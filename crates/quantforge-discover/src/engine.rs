use crate::archive::{
    CandidateEvaluation, deposit_to_accepted_pool, deposit_to_databank, deposit_to_holding,
    deposit_to_specialist_pool, refresh_fingerprint_coverage_map, remove_holding_by_fingerprint,
};
use crate::grammar::{
    apply_search_ranges, build_seed, classify_family, crossover, mutate_with_rng, rng_for,
};
#[cfg(test)]
use crate::model::recovery_factor;
use crate::model::{
    Databank, DepositDecision, DiscoverConfig, DiscoverError, Elite, GateConfig, GateResult,
    SearchFamily, SymbolScreenResult,
};
use crate::multi_symbol::{PackSymbol, screen_multi_symbol};
use crate::{DATABANK_SCHEMA_VERSION, GRAMMAR_VERSION};
use quantforge_broker::SymbolSpecification;
use quantforge_data::{BarDataset, QuoteBarDataset, bar_content_hash};
use quantforge_eval::ScoutResult;
use quantforge_eval::{IndicatorBufferCache, evaluate_strategy_cached};
use quantforge_ir::StrategyIr;
use quantforge_quality::{
    deflated_trade_sharpe, expected_max_lucky_sharpe, perturb_strategy_parameters,
    trade_sharpe_proxy,
};
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1, evaluate_strategy_m1_with_quotes};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Side-queue overflow when all promotion worker slots are busy. Pot admits are
/// retried next generation instead of being dropped.
const PROMOTION_OVERFLOW_CAPACITY: usize = 2048;

/// After breeding unlocks, cap random seeding so more batch slots breed from pot parents.
const POST_BREED_RANDOM_FILL_CAP: f64 = 0.40;

enum CandidateOutcome {
    CoarseRejected,
    AmbiguousRejected,
    DepositGateRejected,
    MultiSymbolRejected {
        #[allow(dead_code)]
        results: Vec<SymbolScreenResult>,
    },
    DeflatedSharpeRejected {
        #[allow(dead_code)]
        observed: Option<f64>,
        #[allow(dead_code)]
        expected: f64,
        #[allow(dead_code)]
        deflated: Option<f64>,
        #[allow(dead_code)]
        multi_symbol_results: Vec<SymbolScreenResult>,
    },
    Accepted {
        result: Box<ScoutResult>,
        island_id: u16,
        is_expectancy: f64,
        observed_trade_sharpe: Option<f64>,
        expected_max_lucky_sharpe: f64,
        deflated_trade_sharpe: Option<f64>,
        multi_symbol_results: Vec<SymbolScreenResult>,
    },
}

#[allow(clippy::too_many_arguments)]
fn evaluate_and_deposit(
    bank: &mut Databank,
    candidates: Vec<StrategyIr>,
    dataset: &BarDataset,
    certification_oos1: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    generation: u64,
    scout_pool: Option<&rayon::ThreadPool>,
    promotion: &PromotionPipeline,
    indicator_cache: &Arc<IndicatorBufferCache>,
) -> Result<(), DiscoverError> {
    // Apply finished promotions before scouting so databank / telemetry stay current
    // without blocking the H1 worker pool on in-flight M1 work.
    promotion.drain_completed(bank)?;

    let mut scout = bank.config.scout.clone();
    // Both the coarse gate and the multi-symbol screen cap drawdown at
    // `gates.maximum_drawdown_percent`, so a candidate past that ceiling is already
    // rejected and the rest of its replay is wasted. Only the coarse screen uses
    // this config; promotion keeps full metrics.
    scout.abandon_above_drawdown_percent = Some(
        bank.config
            .gates
            .maximum_drawdown_percent
            .max(bank.config.deposit_gates.maximum_drawdown_percent),
    );
    let scout = scout;
    let owned_gates = bank.config.gates.clone();
    let gates = &owned_gates;
    let discover_config = &bank.config;
    let multi_symbol_minimum = bank.config.multi_symbol_minimum_pass;
    let minimum_deflated = bank.config.minimum_deflated_trade_sharpe;
    let evaluations_touched = bank.evaluation_count.max(1);
    let evaluate_batch = || {
        candidates
            .into_par_iter()
            .enumerate()
            .map(|(index, strategy)| {
                let island_id = (index % discover_config.effective_island_count()) as u16;
                let result = (|| {
                    let coarse = evaluate_strategy_cached(
                        &strategy,
                        dataset,
                        broker,
                        &scout,
                        indicator_cache.as_ref(),
                    )
                    .map_err(|error| error.to_string())?;
                    if !crate::archive::passes_gates(&coarse, discover_config) {
                        return Ok::<_, String>(CandidateOutcome::CoarseRejected);
                    }
                    // SQX automatic dismissal: ≥25% same-bar open+close trades.
                    if ambiguous_trade_fraction(&coarse) >= 0.25 {
                        return Ok(CandidateOutcome::AmbiguousRejected);
                    }

                    // Cheap cross-symbol H1 screen before any M1 work.
                    let multi = if multi_symbol_minimum > 0 && !pack.is_empty() {
                        screen_multi_symbol(&strategy, primary_symbol, &coarse, pack, &scout, gates)
                    } else {
                        crate::multi_symbol::MultiSymbolScreen {
                            results: Vec::new(),
                            passing: 1,
                            pooled_profits: coarse
                                .trades
                                .iter()
                                .map(|trade| trade.net_profit)
                                .collect(),
                        }
                    };
                    if multi_symbol_minimum > 0 && multi.passing < multi_symbol_minimum {
                        return Ok(CandidateOutcome::MultiSymbolRejected {
                            results: multi.results,
                        });
                    }

                    let observed = trade_sharpe_proxy(&multi.pooled_profits);
                    let expected = expected_max_lucky_sharpe(evaluations_touched);
                    let deflated =
                        deflated_trade_sharpe(&multi.pooled_profits, evaluations_touched);
                    if let Some(floor) = minimum_deflated {
                        if deflated.is_none_or(|value| value < floor) {
                            return Ok(CandidateOutcome::DeflatedSharpeRejected {
                                observed,
                                expected,
                                deflated,
                                multi_symbol_results: multi.results,
                            });
                        }
                    }

                    // Pot admission is Selected-TF / H1 Development only.
                    // Development robustness → M1 waits until breeding unlocks.
                    if !crate::archive::passes_gate_config(&coarse, &discover_config.deposit_gates)
                    {
                        return Ok(CandidateOutcome::DepositGateRejected);
                    }

                    let is_expectancy = coarse.metrics.expectancy;
                    Ok(CandidateOutcome::Accepted {
                        result: Box::new(coarse),
                        island_id,
                        is_expectancy,
                        observed_trade_sharpe: observed,
                        expected_max_lucky_sharpe: expected,
                        deflated_trade_sharpe: deflated,
                        multi_symbol_results: multi.results,
                    })
                })();
                (strategy, result)
            })
            .collect::<Vec<_>>()
    };
    let evaluated = match scout_pool {
        Some(pool) => pool.install(evaluate_batch),
        None => evaluate_batch(),
    };

    // Pass 1: H1 gates → breeding pot only. Never touch the databank here.
    let mut pot_promotions = Vec::new();
    for (strategy, result) in evaluated {
        bank.evaluation_count += 1;
        match result {
            Ok(CandidateOutcome::Accepted {
                result,
                island_id,
                is_expectancy,
                observed_trade_sharpe,
                expected_max_lucky_sharpe,
                deflated_trade_sharpe,
                multi_symbol_results,
            }) => {
                let pot_evaluation = CandidateEvaluation {
                    strategy: strategy.clone(),
                    result: *result,
                    generation,
                    island_id,
                    is_expectancy,
                    oos1_expectancy: None,
                    oos1_expectancy_ratio: None,
                    observed_trade_sharpe,
                    expected_max_lucky_sharpe: Some(expected_max_lucky_sharpe),
                    deflated_trade_sharpe,
                    multi_symbol_results: multi_symbol_results.clone(),
                    gate_results: build_gate_results(
                        is_expectancy,
                        None,
                        None,
                        false,
                        None,
                        &multi_symbol_results,
                        bank.config.multi_symbol_minimum_pass,
                        deflated_trade_sharpe,
                        bank.config.minimum_deflated_trade_sharpe,
                    ),
                    robustness: None,
                };
                let pot_decision = deposit_to_accepted_pool(bank, pot_evaluation.clone())?;
                bank.telemetry.record(pot_decision);
                // Only real pot admits need M1 replay. Diversity rejects cannot
                // enter the breeding pot and must not steal promotion slots.
                match pot_decision {
                    DepositDecision::AcceptedToPot | DepositDecision::ReplacedInPot => {
                        pot_promotions.push(pot_evaluation);
                    }
                    _ => {}
                }
            }
            Ok(CandidateOutcome::CoarseRejected) => {
                bank.telemetry.record(DepositDecision::RejectedGate);
            }
            Ok(CandidateOutcome::AmbiguousRejected) => {
                bank.telemetry.record(DepositDecision::RejectedAmbiguous);
            }
            Ok(CandidateOutcome::DepositGateRejected) => {
                bank.telemetry.record(DepositDecision::RejectedDepositGate);
            }
            Ok(CandidateOutcome::MultiSymbolRejected { .. }) => {
                bank.telemetry.record(DepositDecision::RejectedMultiSymbol);
            }
            Ok(CandidateOutcome::DeflatedSharpeRejected { .. }) => {
                bank.telemetry
                    .record(DepositDecision::RejectedDeflatedSharpe);
            }
            Err(error) => {
                *bank.telemetry.evaluation_errors.entry(error).or_default() += 1;
                bank.telemetry.record(DepositDecision::RejectedEvaluation);
            }
        }
    }

    // Discovery is development-only. Once breeding unlocks, the side pool runs
    // M1 80/130 into Holding. Plateau / fold-R / CPCV / MC wait for the battery.
    let breeding_unlocked = bank.accepted_pool.len() >= bank.config.mutate_after_elites;
    if breeding_unlocked && !pot_promotions.is_empty() {
        let context = promotion.context_for(
            &bank.config,
            dataset,
            certification_oos1,
            m1_dataset,
            quote_dataset,
            broker,
        );
        promotion.drain_overflow(Arc::clone(&context), bank)?;
        for pot_evaluation in pot_promotions {
            promotion.try_enqueue_or_buffer(pot_evaluation, Arc::clone(&context), bank)?;
        }
    }
    promotion.snapshot_telemetry(bank);
    Ok(())
}

enum PromotionOutcome {
    DevelopmentExpectancyRejected,
    DatabankGateRejected,
    RobustnessRejected {
        reject: crate::robustness::RobustnessReject,
    },
    HoldingApproved {
        candidate: CandidateEvaluation,
    },
    DevelopmentApproved {
        candidate: CandidateEvaluation,
        oos1_passed: bool,
        oos1_expectancy: Option<f64>,
        oos1_expectancy_ratio: Option<f64>,
    },
    EvaluationError {
        message: String,
    },
}

/// Immutable inputs shared by every in-flight promotion job.
struct PromotionContext {
    dataset: Arc<BarDataset>,
    #[allow(dead_code)]
    oos1: Option<Arc<BarDataset>>,
    m1: Arc<BarDataset>,
    quotes: Option<Arc<QuoteBarDataset>>,
    broker: Arc<SymbolSpecification>,
    deposit_gates: GateConfig,
    #[allow(dead_code)]
    oos1_expectancy_retention: f64,
    minimum_development_expectancy_r: f64,
    build_to_holding: bool,
    require_m1_robustness: bool,
    robustness: crate::robustness::RobustnessConfig,
}

struct PromotionShared {
    completed: Mutex<Vec<PromotionOutcome>>,
    overflow: Mutex<VecDeque<CandidateEvaluation>>,
    /// Waiting + running promotion jobs. Cap this for backpressure.
    inflight: AtomicUsize,
    /// Actively executing on the promotion pool (subset of inflight).
    running: AtomicUsize,
    capacity: usize,
    wake: Condvar,
}

/// Side pool for Development CPCV/robustness → M1 → OOS1 validation while scouting continues.
struct PromotionPipeline {
    pool: Option<rayon::ThreadPool>,
    shared: Arc<PromotionShared>,
    /// One immutable market-data context for the lifetime of the session.
    ///
    /// Previously `evaluate_and_deposit` cloned every Development/OOS1/M1/
    /// quote dataset once per generation.  A backed-up promotion queue then
    /// retained dozens of complete copies and could exhaust workstation RAM
    /// during an overnight run.  Resume validation guarantees that a session
    /// cannot change feeds, so sharing this context is both safe and exact.
    context: Mutex<Option<Arc<PromotionContext>>>,
}

impl PromotionPipeline {
    fn new(worker_threads: usize, capacity: usize) -> Result<Self, DiscoverError> {
        Ok(Self {
            pool: build_worker_pool(worker_threads.max(1))?,
            shared: Arc::new(PromotionShared {
                completed: Mutex::new(Vec::new()),
                overflow: Mutex::new(VecDeque::new()),
                inflight: AtomicUsize::new(0),
                running: AtomicUsize::new(0),
                capacity: capacity.max(1),
                wake: Condvar::new(),
            }),
            context: Mutex::new(None),
        })
    }

    fn queue_depth(&self) -> usize {
        self.shared.inflight.load(Ordering::SeqCst)
    }

    fn inflight_running(&self) -> usize {
        self.shared.running.load(Ordering::SeqCst)
    }

    fn snapshot_telemetry(&self, bank: &mut Databank) {
        bank.telemetry.promotion_queue_depth = self.queue_depth() as u64;
        bank.telemetry.promotion_inflight = self.inflight_running() as u64;
    }

    #[allow(clippy::too_many_arguments)]
    fn context_for(
        &self,
        config: &DiscoverConfig,
        dataset: &BarDataset,
        oos1_dataset: Option<&BarDataset>,
        m1_dataset: &BarDataset,
        quote_dataset: Option<&QuoteBarDataset>,
        broker: &SymbolSpecification,
    ) -> Arc<PromotionContext> {
        let mut cached = self
            .context
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(context) = cached.as_ref() {
            return Arc::clone(context);
        }
        let context = Arc::new(PromotionContext {
            dataset: Arc::new(dataset.clone()),
            oos1: oos1_dataset.map(|data| Arc::new(data.clone())),
            m1: Arc::new(m1_dataset.clone()),
            quotes: quote_dataset.map(|data| Arc::new(data.clone())),
            broker: Arc::new(broker.clone()),
            deposit_gates: config.deposit_gates.clone(),
            oos1_expectancy_retention: config.oos1_expectancy_retention,
            minimum_development_expectancy_r: config.minimum_development_expectancy_r,
            build_to_holding: config.build_to_holding,
            require_m1_robustness: config.require_m1_robustness,
            robustness: robustness_config_from_discover(config),
        });
        *cached = Some(Arc::clone(&context));
        context
    }

    #[allow(dead_code)]
    fn enqueue(
        &self,
        pot_evaluation: CandidateEvaluation,
        context: Arc<PromotionContext>,
        bank: &mut Databank,
    ) -> Result<(), DiscoverError> {
        // Pot admits wait for a slot. Diversity overflow must not stall H1.
        loop {
            self.drain_completed(bank)?;
            let depth = self.shared.inflight.load(Ordering::SeqCst);
            if depth < self.shared.capacity {
                break;
            }
            bank.telemetry.promotion_backpressure_events += 1;
            self.snapshot_telemetry(bank);
            let guard = self
                .shared
                .completed
                .lock()
                .map_err(|_| DiscoverError::InvalidConfig("promotion lock poisoned".into()))?;
            let (_guard, _) = self
                .shared
                .wake
                .wait_timeout(guard, Duration::from_millis(25))
                .map_err(|_| DiscoverError::InvalidConfig("promotion wait poisoned".into()))?;
        }
        self.spawn_promotion(pot_evaluation, context, bank)
    }

    fn try_enqueue_or_buffer(
        &self,
        pot_evaluation: CandidateEvaluation,
        context: Arc<PromotionContext>,
        bank: &mut Databank,
    ) -> Result<(), DiscoverError> {
        self.drain_completed(bank)?;
        if self.shared.inflight.load(Ordering::SeqCst) >= self.shared.capacity {
            bank.telemetry.promotion_backpressure_events += 1;
            self.snapshot_telemetry(bank);
            self.push_overflow(pot_evaluation, bank)
        } else {
            self.spawn_promotion(pot_evaluation, context, bank)
        }
    }

    /// Enqueue if a slot is free. Returns false when the M1 queue is full so
    /// the caller can buffer the job for the next generation.
    fn try_enqueue(
        &self,
        pot_evaluation: CandidateEvaluation,
        context: Arc<PromotionContext>,
        bank: &mut Databank,
    ) -> Result<bool, DiscoverError> {
        self.drain_completed(bank)?;
        if self.shared.inflight.load(Ordering::SeqCst) >= self.shared.capacity {
            bank.telemetry.promotion_backpressure_events += 1;
            self.snapshot_telemetry(bank);
            return Ok(false);
        }
        self.spawn_promotion(pot_evaluation, context, bank)?;
        Ok(true)
    }

    fn push_overflow(
        &self,
        pot_evaluation: CandidateEvaluation,
        bank: &mut Databank,
    ) -> Result<(), DiscoverError> {
        let mut overflow =
            self.shared.overflow.lock().map_err(|_| {
                DiscoverError::InvalidConfig("promotion overflow lock poisoned".into())
            })?;
        if overflow.len() >= PROMOTION_OVERFLOW_CAPACITY {
            overflow.pop_front();
            bank.telemetry.promotion_overflow_dropped += 1;
        }
        overflow.push_back(pot_evaluation);
        Ok(())
    }

    fn drain_overflow(
        &self,
        context: Arc<PromotionContext>,
        bank: &mut Databank,
    ) -> Result<(), DiscoverError> {
        loop {
            self.drain_completed(bank)?;
            if self.shared.inflight.load(Ordering::SeqCst) >= self.shared.capacity {
                break;
            }
            let next = {
                let mut overflow = self.shared.overflow.lock().map_err(|_| {
                    DiscoverError::InvalidConfig("promotion overflow lock poisoned".into())
                })?;
                overflow.pop_front()
            };
            match next {
                Some(pot_evaluation) => {
                    self.spawn_promotion(pot_evaluation, Arc::clone(&context), bank)?;
                }
                None => break,
            }
        }
        Ok(())
    }

    fn spawn_promotion(
        &self,
        pot_evaluation: CandidateEvaluation,
        context: Arc<PromotionContext>,
        bank: &mut Databank,
    ) -> Result<(), DiscoverError> {
        self.shared.inflight.fetch_add(1, Ordering::SeqCst);
        bank.telemetry.promotions_enqueued += 1;
        self.snapshot_telemetry(bank);

        let shared = Arc::clone(&self.shared);
        let run = move || {
            shared.running.fetch_add(1, Ordering::SeqCst);
            let outcome = promote_one(pot_evaluation, &context);
            shared.running.fetch_sub(1, Ordering::SeqCst);
            if let Ok(mut completed) = shared.completed.lock() {
                completed.push(outcome);
            }
            shared.inflight.fetch_sub(1, Ordering::SeqCst);
            shared.wake.notify_all();
        };

        match &self.pool {
            Some(pool) => {
                pool.spawn(run);
            }
            None => {
                // No dedicated pool: run inline so gates still apply.
                run();
            }
        }
        Ok(())
    }

    fn drain_completed(&self, bank: &mut Databank) -> Result<(), DiscoverError> {
        let outcomes = {
            let mut completed = self
                .shared
                .completed
                .lock()
                .map_err(|_| DiscoverError::InvalidConfig("promotion lock poisoned".into()))?;
            std::mem::take(&mut *completed)
        };
        for outcome in outcomes {
            apply_promotion_outcome(bank, outcome)?;
            bank.telemetry.promotions_completed += 1;
        }
        self.snapshot_telemetry(bank);
        Ok(())
    }

    fn flush(&self, bank: &mut Databank) -> Result<(), DiscoverError> {
        loop {
            self.drain_completed(bank)?;
            if self.shared.inflight.load(Ordering::SeqCst) == 0 {
                self.drain_completed(bank)?;
                self.snapshot_telemetry(bank);
                return Ok(());
            }
            let guard = self
                .shared
                .completed
                .lock()
                .map_err(|_| DiscoverError::InvalidConfig("promotion lock poisoned".into()))?;
            let (_guard, _) = self
                .shared
                .wake
                .wait_timeout(guard, Duration::from_millis(50))
                .map_err(|_| DiscoverError::InvalidConfig("promotion wait poisoned".into()))?;
        }
    }
}

fn promote_one(
    pot_evaluation: CandidateEvaluation,
    context: &PromotionContext,
) -> PromotionOutcome {
    let strategy = &pot_evaluation.strategy;
    let broker = context.broker.as_ref();
    let robustness = &context.robustness;

    // Holding path: M1 80/130 retention only. The full Development battery
    // and optional OOS1 validation run later, after a user starts it from
    // Holding.
    if context.build_to_holding {
        let m1_outcome = match crate::robustness::run_m1_holding_admission(
            strategy,
            context.dataset.as_ref(),
            context.m1.as_ref(),
            context.quotes.as_deref(),
            broker,
            robustness,
            &pot_evaluation.result.metrics,
        ) {
            Err(reject) => return PromotionOutcome::RobustnessRejected { reject },
            Ok(outcome) => (outcome.result, outcome.evidence),
        };
        if !crate::archive::passes_gate_config(&m1_outcome.0, &context.deposit_gates) {
            return PromotionOutcome::DatabankGateRejected;
        }
        let development_expectancy = m1_outcome.0.metrics.expectancy;
        if !passes_development_expectancy(
            development_expectancy,
            context.minimum_development_expectancy_r,
        ) {
            return PromotionOutcome::DevelopmentExpectancyRejected;
        }
        return PromotionOutcome::HoldingApproved {
            candidate: CandidateEvaluation {
                strategy: pot_evaluation.strategy.clone(),
                result: m1_outcome.0,
                generation: pot_evaluation.generation,
                island_id: pot_evaluation.island_id,
                is_expectancy: development_expectancy,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: pot_evaluation.observed_trade_sharpe,
                expected_max_lucky_sharpe: pot_evaluation.expected_max_lucky_sharpe,
                deflated_trade_sharpe: pot_evaluation.deflated_trade_sharpe,
                multi_symbol_results: pot_evaluation.multi_symbol_results.clone(),
                gate_results: pot_evaluation.gate_results.clone(),
                robustness: m1_outcome.1,
            },
        };
    }

    // Direct-to-databank path: optional full battery, then optional OOS1.
    let m1_outcome = if context.require_m1_robustness {
        match crate::robustness::run_m1_predeposit_robustness(
            strategy,
            context.dataset.as_ref(),
            context.m1.as_ref(),
            context.quotes.as_deref(),
            broker,
            robustness,
            &pot_evaluation.result.metrics,
            true,
        ) {
            Err(reject) => return PromotionOutcome::RobustnessRejected { reject },
            Ok(outcome) => (outcome.result, outcome.evidence),
        }
    } else {
        match evaluate_strategy_m1_with_optional_quotes(
            strategy,
            context.dataset.as_ref(),
            context.m1.as_ref(),
            context.quotes.as_deref(),
            broker,
            &JudgeConfig {
                initial_balance: robustness.initial_balance,
                costs: robustness.costs.clone(),
                allow_execution_gaps: false,
                indicator_engine: robustness.indicator_engine,
                entry_window: robustness.entry_window,
            },
        ) {
            Err(error) => {
                return PromotionOutcome::EvaluationError {
                    message: error.to_string(),
                };
            }
            Ok(result) => (
                ScoutResult {
                    trades: result.trades,
                    equity: result.equity,
                    metrics: result.metrics,
                    telemetry: Default::default(),
                },
                None,
            ),
        }
    };
    if !crate::archive::passes_gate_config(&m1_outcome.0, &context.deposit_gates) {
        return PromotionOutcome::DatabankGateRejected;
    }
    let development_expectancy = m1_outcome.0.metrics.expectancy;
    if !passes_development_expectancy(
        development_expectancy,
        context.minimum_development_expectancy_r,
    ) {
        return PromotionOutcome::DevelopmentExpectancyRejected;
    }

    let development_candidate = CandidateEvaluation {
        strategy: pot_evaluation.strategy.clone(),
        result: m1_outcome.0.clone(),
        generation: pot_evaluation.generation,
        island_id: pot_evaluation.island_id,
        is_expectancy: development_expectancy,
        oos1_expectancy: None,
        oos1_expectancy_ratio: None,
        observed_trade_sharpe: pot_evaluation.observed_trade_sharpe,
        expected_max_lucky_sharpe: pot_evaluation.expected_max_lucky_sharpe,
        deflated_trade_sharpe: pot_evaluation.deflated_trade_sharpe,
        multi_symbol_results: pot_evaluation.multi_symbol_results.clone(),
        gate_results: pot_evaluation.gate_results.clone(),
        robustness: m1_outcome.1.clone(),
    };

    // OOS1 is a post-Development validation gate. It never participates in
    // scouting, breeding, or ranking. A zero OOS1 reserve is represented by
    // `None`, which leaves the two-way Development/OOS2 protocol intact.
    let Some(oos1) = context.oos1.as_deref() else {
        return PromotionOutcome::DevelopmentApproved {
            candidate: development_candidate,
            oos1_passed: true,
            oos1_expectancy: None,
            oos1_expectancy_ratio: None,
        };
    };
    let Some(oos1_start_ms) = oos1.bars.first().map(|bar| bar.timestamp_ms) else {
        return PromotionOutcome::EvaluationError {
            message: "OOS1 validation partition is empty".into(),
        };
    };
    // Replay Development + OOS1 together only to preserve indicator warm-up,
    // open positions, and execution continuity at the boundary. The admitted
    // Development metrics above remain the fitness reference.
    let validation_decision = join_datasets(context.dataset.as_ref(), oos1);
    let validation = match evaluate_strategy_m1_with_optional_quotes(
        strategy,
        &validation_decision,
        context.m1.as_ref(),
        context.quotes.as_deref(),
        broker,
        &JudgeConfig {
            initial_balance: robustness.initial_balance,
            costs: robustness.costs.clone(),
            allow_execution_gaps: false,
            indicator_engine: robustness.indicator_engine,
            entry_window: robustness.entry_window,
        },
    ) {
        Ok(result) => result,
        Err(error) => {
            return PromotionOutcome::EvaluationError {
                message: format!("M1 OOS1 validation replay failed: {error}"),
            };
        }
    };
    let oos1_expectancy = expectancy_from(&validation.trades, oos1_start_ms);
    if !passes_oos1_pick(
        development_expectancy,
        oos1_expectancy,
        context.oos1_expectancy_retention,
    ) {
        return PromotionOutcome::DevelopmentApproved {
            candidate: development_candidate,
            oos1_passed: false,
            oos1_expectancy: Some(oos1_expectancy),
            oos1_expectancy_ratio: Some(oos1_expectancy / development_expectancy),
        };
    }
    PromotionOutcome::DevelopmentApproved {
        candidate: development_candidate,
        oos1_passed: true,
        oos1_expectancy: Some(oos1_expectancy),
        oos1_expectancy_ratio: Some(oos1_expectancy / development_expectancy),
    }
}

fn apply_promotion_outcome(
    bank: &mut Databank,
    outcome: PromotionOutcome,
) -> Result<(), DiscoverError> {
    match outcome {
        PromotionOutcome::DevelopmentExpectancyRejected => {
            bank.telemetry
                .record(DepositDecision::RejectedDevelopmentExpectancy);
        }
        PromotionOutcome::DatabankGateRejected => {
            bank.telemetry.record(DepositDecision::RejectedDepositGate);
        }
        PromotionOutcome::RobustnessRejected { reject } => match reject {
            crate::robustness::RobustnessReject::M1Fidelity => {
                bank.telemetry.record(DepositDecision::RejectedM1Fidelity);
            }
            crate::robustness::RobustnessReject::FoldStability => {
                bank.telemetry
                    .record(DepositDecision::RejectedFoldStability);
            }
            crate::robustness::RobustnessReject::WalkForward => {
                bank.telemetry.record(DepositDecision::RejectedWalkForward);
            }
            crate::robustness::RobustnessReject::Cpcv => {
                bank.telemetry.record(DepositDecision::RejectedCpcv);
            }
            crate::robustness::RobustnessReject::MonteCarlo => {
                bank.telemetry.record(DepositDecision::RejectedMonteCarlo);
            }
            crate::robustness::RobustnessReject::ParamNeighborhood => {
                bank.telemetry
                    .record(DepositDecision::RejectedParamNeighborhood);
            }
        },
        PromotionOutcome::EvaluationError { message } => {
            *bank.telemetry.evaluation_errors.entry(message).or_default() += 1;
            bank.telemetry.record(DepositDecision::RejectedEvaluation);
        }
        PromotionOutcome::HoldingApproved { candidate } => {
            let decision = deposit_to_holding(bank, candidate)?;
            bank.telemetry.record(decision);
        }
        PromotionOutcome::DevelopmentApproved {
            candidate,
            oos1_passed,
            oos1_expectancy,
            oos1_expectancy_ratio,
        } => {
            // OOS1 fields are absent from the Development specialist parent.
            // This makes leakage structurally impossible in its breeding lane.
            let specialist_decision = deposit_to_specialist_pool(bank, candidate.clone())?;
            match specialist_decision {
                DepositDecision::AcceptedToPot => bank.telemetry.specialist_accepted += 1,
                DepositDecision::ReplacedInPot => bank.telemetry.specialist_replaced += 1,
                _ => {}
            }
            if !oos1_passed {
                bank.telemetry.record(DepositDecision::RejectedOos1);
                return Ok(());
            }
            let m1_expectancy = candidate.result.metrics.expectancy;
            let mut bank_evaluation = candidate;
            bank_evaluation.oos1_expectancy = oos1_expectancy;
            bank_evaluation.oos1_expectancy_ratio = oos1_expectancy_ratio;
            bank_evaluation.gate_results = build_gate_results(
                m1_expectancy,
                oos1_expectancy,
                oos1_expectancy_ratio,
                oos1_expectancy.is_some(),
                None,
                &bank_evaluation.multi_symbol_results,
                bank.config.multi_symbol_minimum_pass,
                bank_evaluation.deflated_trade_sharpe,
                bank.config.minimum_deflated_trade_sharpe,
            );
            let bank_decision = deposit_to_databank(bank, bank_evaluation)?;
            bank.telemetry.record(bank_decision);
        }
    }
    Ok(())
}

fn robustness_config_from_discover(config: &DiscoverConfig) -> crate::robustness::RobustnessConfig {
    let search = &config.search_ranges;
    crate::robustness::RobustnessConfig {
        folds: config.robustness_folds,
        monte_carlo_trials: config.robustness_monte_carlo_trials,
        monte_carlo_block_length: config.robustness_monte_carlo_block_length,
        monte_carlo_skip_trade_probability: config.robustness_monte_carlo_skip_trade_probability,
        monte_carlo_minimum_p80_profit_retention: config
            .robustness_monte_carlo_p80_profit_retention,
        monte_carlo_max_drawdown_ratio: config.robustness_monte_carlo_max_drawdown_ratio,
        neighborhood_samples: config.robustness_neighborhood_samples,
        seed: config.seed,
        initial_balance: config.scout.initial_balance,
        costs: config.scout.costs.clone(),
        entry_window: config.scout.entry_window,
        minimum_return_retention: config.precision.minimum_return_retention,
        minimum_fold_trades: config.deposit_gates.minimum_trades.clamp(1, 2),
        minimum_return_percent: config.deposit_gates.minimum_return_percent,
        minimum_profit_factor: config.deposit_gates.minimum_profit_factor.min(1.0),
        maximum_drawdown_percent: config.deposit_gates.maximum_drawdown_percent.max(30.0),
        minimum_passing_fold_fraction: 0.6,
        minimum_neighborhood_survival_fraction: config
            .minimum_neighborhood_survival_fraction
            .max(0.55)
            .min(1.0),
        parameter_perturbation_fraction: config.robustness_perturbation_fraction,
        adx_period_min: search.indicator_period.minimum.round().max(2.0) as u16,
        adx_period_max: search.indicator_period.maximum.round().max(2.0) as u16,
        adx_period_step: search.indicator_period.step.round().max(1.0) as u16,
        adx_threshold_min: search.adx_threshold.minimum,
        adx_threshold_max: search.adx_threshold.maximum,
        adx_threshold_step: search.adx_threshold.step,
        indicator_engine: config.scout.indicator_engine,
        calendar_year_folds: config.calendar_year_folds,
    }
}

/// OOS1 validation requires positive expectancy and configurable retention
/// relative to the M1 Development replay. It must never feed breeding or rank.
pub(crate) fn passes_oos1_pick(is_expectancy: f64, oos1_expectancy: f64, retention: f64) -> bool {
    is_expectancy.is_finite()
        && oos1_expectancy.is_finite()
        && is_expectancy > 0.0
        && oos1_expectancy > 0.0
        && oos1_expectancy >= retention * is_expectancy
}

fn passes_development_expectancy(expectancy: f64, minimum_r: f64) -> bool {
    expectancy.is_finite()
        && minimum_r.is_finite()
        && minimum_r >= 0.0
        && expectancy >= minimum_r * crate::FIXED_RISK_PER_TRADE
}

/// Why an on-demand Holding → Databank battery failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldingBatteryReject {
    NotInHolding,
    Robustness(crate::robustness::RobustnessReject),
    DepositGate,
    DevelopmentExpectancy,
    Oos1,
    /// Battery passed but Databank inventory rejected the deposit (clone / correlation).
    DatabankDeposit(DepositDecision),
    Evaluation(String),
}

impl HoldingBatteryReject {
    pub fn kill_bucket(&self) -> &'static str {
        match self {
            Self::Robustness(reject) => reject.kill_bucket(),
            Self::DepositGate => "deposit",
            Self::DevelopmentExpectancy => "expectancy",
            Self::Oos1 => "oos1",
            Self::DatabankDeposit(decision) => match decision {
                DepositDecision::RejectedCorrelated => "correlation",
                DepositDecision::RejectedClone => "clone",
                _ => "databank",
            },
            Self::NotInHolding | Self::Evaluation(_) => "other",
        }
    }
}

impl std::fmt::Display for HoldingBatteryReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInHolding => write!(f, "not in Holding"),
            Self::Robustness(reject) => write!(f, "{reject}"),
            Self::DepositGate => write!(f, "deposit gates"),
            Self::DevelopmentExpectancy => write!(f, "Development expectancy floor"),
            Self::Oos1 => write!(f, "OOS1 retention"),
            Self::DatabankDeposit(decision) => write!(f, "Databank deposit: {decision:?}"),
            Self::Evaluation(message) => write!(f, "evaluation: {message}"),
        }
    }
}

/// Rank Holding for the overnight factory: more trades × non-negative R-expectancy first.
pub fn holding_factory_score(trade_count: usize, expectancy_r: f64) -> f64 {
    trade_count as f64 * expectancy_r.max(0.0)
}

/// Move every current Holding entry into the Databank without running the
/// deferred robustness battery. This is an explicit research override used by
/// the desktop Holding page; Discover's normal promotion path is unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingBypassResult {
    pub promoted: usize,
    pub replaced: usize,
}

pub fn promote_all_holding_without_robustness(bank: &mut Databank) -> HoldingBypassResult {
    let selected = bank
        .holding
        .iter()
        .map(|elite| elite.structural_fingerprint.to_string())
        .collect::<BTreeSet<_>>();
    promote_selected_holding_without_robustness(bank, &selected, None)
}

/// Promote only the named Holding entries while leaving every non-selected
/// candidate in Holding. `selection_gate` records the Development-only recipe
/// that chose the cohort; it must not contain future/holdout evidence.
pub fn promote_selected_holding_without_robustness(
    bank: &mut Databank,
    selected: &BTreeSet<String>,
    selection_gate: Option<GateResult>,
) -> HoldingBypassResult {
    let holding = std::mem::take(&mut bank.holding);
    let mut remaining = Vec::with_capacity(holding.len());
    let mut promoted = 0;
    let mut replaced = 0;

    for mut elite in holding {
        if !selected.contains(elite.structural_fingerprint.as_str()) {
            remaining.push(elite);
            continue;
        }
        elite
            .gate_results
            .retain(|gate| gate.name != "robustness_bypass");
        if !elite
            .gate_results
            .iter()
            .any(|gate| gate.name == "robustness_audit")
        {
            elite.gate_results.push(GateResult {
                name: "robustness_bypass".into(),
                passed: false,
                detail: "Manually graduated from Holding; deferred robustness battery was not run."
                    .into(),
            });
        }
        if let Some(gate) = selection_gate.as_ref() {
            elite
                .gate_results
                .retain(|existing| existing.name != gate.name);
            elite.gate_results.push(gate.clone());
        }
        if let Some(index) = bank
            .elites
            .iter()
            .position(|existing| existing.structural_fingerprint == elite.structural_fingerprint)
        {
            bank.elites[index] = elite;
            replaced += 1;
            bank.telemetry.record(DepositDecision::ReplacedInDatabank);
        } else {
            bank.elites.push(elite);
            promoted += 1;
            bank.telemetry.record(DepositDecision::AcceptedToDatabank);
        }
    }

    bank.holding = remaining;
    refresh_fingerprint_coverage_map(&bank.holding, &mut bank.holding_coverage_map);
    refresh_fingerprint_coverage_map(&bank.elites, &mut bank.coverage_map);
    HoldingBypassResult { promoted, replaced }
}

/// Successful Holding battery: strategy removed from Holding and deposited.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingBatteryResult {
    pub fingerprint: quantforge_core::ContentHash,
    pub decision: DepositDecision,
    pub elite: Elite,
}

/// Outcome of the non-gating full-battery audit. The candidate remains in
/// Holding until the caller explicitly graduates it; this result never feeds
/// breeding, ranking, or OOS selection.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingBatteryAuditResult {
    pub fingerprint: quantforge_core::ContentHash,
    pub passed: bool,
    pub reason: Option<String>,
}

/// Run the same Development/M1 battery as the strict Holding gate, but only
/// record the result on the Holding candidate. It does not remove, promote, or
/// reject the candidate.
pub fn audit_holding_battery(
    bank: &mut Databank,
    fingerprint: &quantforge_core::ContentHash,
    dataset: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
) -> Result<HoldingBatteryAuditResult, HoldingBatteryReject> {
    let elite = bank
        .holding
        .iter()
        .find(|entry| &entry.structural_fingerprint == fingerprint)
        .cloned()
        .ok_or(HoldingBatteryReject::NotInHolding)?;
    let robustness = robustness_config_from_discover(&bank.config);
    let prior_h1 = elite
        .robustness
        .as_ref()
        .map(|evidence| evidence.m1_retention.selected_timeframe_metrics.clone())
        .unwrap_or_else(|| elite.metrics.clone());
    let outcome = crate::robustness::run_m1_predeposit_robustness_audit(
        &elite.strategy,
        dataset,
        m1_dataset,
        quote_dataset,
        broker,
        &robustness,
        &prior_h1,
        false,
    );
    let (passed, reason, evidence, mut gates) = match outcome {
        Err(reject) => (false, Some(reject.to_string()), None, Vec::new()),
        Ok(outcome) => {
            let deposit_passed = crate::archive::passes_gate_config(
                &outcome.result,
                &bank.config.deposit_gates,
            );
            let expectancy_passed = passes_development_expectancy(
                outcome.result.metrics.expectancy,
                bank.config.minimum_development_expectancy_r,
            );
            let mut blockers = outcome.blockers;
            if !deposit_passed {
                blockers.push("deposit gates".into());
            }
            if !expectancy_passed {
                blockers.push("Development expectancy floor".into());
            }
            let mut gates = outcome.gates;
            gates.push(GateResult {
                name: "robustness_audit_deposit_gates".into(),
                passed: deposit_passed,
                detail: "Development deposit gates evaluated as audit evidence only.".into(),
            });
            gates.push(GateResult {
                name: "robustness_audit_development_expectancy".into(),
                passed: expectancy_passed,
                detail: format!(
                    "Development expectancy {:.3}R; required {:.3}R.",
                    outcome.result.metrics.expectancy,
                    bank.config.minimum_development_expectancy_r,
                ),
            });
            let passed = blockers.is_empty();
            let reason = (!passed).then(|| blockers.join("; "));
            (passed, reason, Some(outcome.evidence), gates)
        }
    };
    let audited = bank
        .holding
        .iter_mut()
        .find(|entry| &entry.structural_fingerprint == fingerprint)
        .ok_or(HoldingBatteryReject::NotInHolding)?;
    if evidence.is_some() {
        audited.robustness = evidence;
    }
    audited
        .gate_results
        .retain(|gate| !gate.name.starts_with("robustness_audit"));
    if gates.is_empty() {
        gates.push(GateResult {
            name: "robustness_audit".into(),
            passed,
            detail: match &reason {
                Some(reason) => format!(
                    "Full Development/M1 battery audit could not complete: {reason}. This was recorded only; the strategy can still be graduated."
                ),
                None => "Full Development/M1 battery audit passed. This was recorded only; it was not used as a selection gate.".into(),
            },
        });
    }
    audited.gate_results.extend(gates);
    Ok(HoldingBatteryAuditResult {
        fingerprint: fingerprint.clone(),
        passed,
        reason,
    })
}

/// Run the full M1 robustness battery on one Holding candidate and, on pass,
/// promote it into the Databank elites archive. When the immutable run split
/// contains OOS1, validate there only after the Development battery passes.
pub fn run_holding_battery_and_promote(
    bank: &mut Databank,
    fingerprint: &quantforge_core::ContentHash,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
) -> Result<HoldingBatteryResult, HoldingBatteryReject> {
    let elite = bank
        .holding
        .iter()
        .find(|entry| &entry.structural_fingerprint == fingerprint)
        .cloned()
        .ok_or(HoldingBatteryReject::NotInHolding)?;
    let robustness = robustness_config_from_discover(&bank.config);
    // Do not re-scout Selected-TF and re-gate M1 fidelity here. Holding admission
    // already passed that check against the pot H1; a fresh scout can disagree
    // and falsely reject as M1Fidelity. Battery only runs CPCV / WFO / MC /
    // ±param (and OOS1 below). Use stored Holding M1 metrics as the retention
    // audit reference when no Selected-TF snapshot was kept on the elite.
    let prior_h1 = elite
        .robustness
        .as_ref()
        .map(|evidence| evidence.m1_retention.selected_timeframe_metrics.clone())
        .unwrap_or_else(|| elite.metrics.clone());
    let outcome = crate::robustness::run_m1_predeposit_robustness(
        &elite.strategy,
        dataset,
        m1_dataset,
        quote_dataset,
        broker,
        &robustness,
        &prior_h1,
        false,
    )
    .map_err(HoldingBatteryReject::Robustness)?;
    if !crate::archive::passes_gate_config(&outcome.result, &bank.config.deposit_gates) {
        return Err(HoldingBatteryReject::DepositGate);
    }
    let development_expectancy = outcome.result.metrics.expectancy;
    if !passes_development_expectancy(
        development_expectancy,
        bank.config.minimum_development_expectancy_r,
    ) {
        return Err(HoldingBatteryReject::DevelopmentExpectancy);
    }

    let mut candidate = CandidateEvaluation {
        strategy: elite.strategy.clone(),
        result: outcome.result,
        generation: elite.discovered_generation,
        island_id: elite.island_id,
        is_expectancy: development_expectancy,
        oos1_expectancy: None,
        oos1_expectancy_ratio: None,
        observed_trade_sharpe: elite.observed_trade_sharpe,
        expected_max_lucky_sharpe: elite.expected_max_lucky_sharpe,
        deflated_trade_sharpe: elite.deflated_trade_sharpe,
        multi_symbol_results: elite.multi_symbol_results.clone(),
        gate_results: elite.gate_results.clone(),
        robustness: outcome.evidence,
    };

    if let Some(oos1) = oos1_dataset {
        let Some(oos1_start_ms) = oos1.bars.first().map(|bar| bar.timestamp_ms) else {
            return Err(HoldingBatteryReject::Evaluation(
                "OOS1 validation partition is empty".into(),
            ));
        };
        let validation_decision = join_datasets(dataset, oos1);
        let validation = evaluate_strategy_m1_with_optional_quotes(
            &elite.strategy,
            &validation_decision,
            m1_dataset,
            quote_dataset,
            broker,
            &JudgeConfig {
                initial_balance: robustness.initial_balance,
                costs: robustness.costs.clone(),
                allow_execution_gaps: false,
                indicator_engine: robustness.indicator_engine,
                entry_window: robustness.entry_window,
            },
        )
        .map_err(|error| {
            HoldingBatteryReject::Evaluation(format!("M1 OOS1 validation replay failed: {error}"))
        })?;
        let oos1_expectancy = expectancy_from(&validation.trades, oos1_start_ms);
        if !passes_oos1_pick(
            development_expectancy,
            oos1_expectancy,
            bank.config.oos1_expectancy_retention,
        ) {
            return Err(HoldingBatteryReject::Oos1);
        }
        candidate.oos1_expectancy = Some(oos1_expectancy);
        candidate.oos1_expectancy_ratio = Some(oos1_expectancy / development_expectancy);
        candidate.gate_results = build_gate_results(
            development_expectancy,
            candidate.oos1_expectancy,
            candidate.oos1_expectancy_ratio,
            true,
            None,
            &candidate.multi_symbol_results,
            bank.config.multi_symbol_minimum_pass,
            candidate.deflated_trade_sharpe,
            bank.config.minimum_deflated_trade_sharpe,
        );
    }

    let decision = deposit_to_databank(bank, candidate)
        .map_err(|error| HoldingBatteryReject::Evaluation(error.to_string()))?;
    match decision {
        DepositDecision::AcceptedToDatabank | DepositDecision::ReplacedInDatabank => {
            remove_holding_by_fingerprint(bank, fingerprint)
                .ok_or(HoldingBatteryReject::NotInHolding)?;
            bank.telemetry.record(decision);
            let promoted = bank
                .elites
                .iter()
                .find(|entry| &entry.structural_fingerprint == fingerprint)
                .cloned()
                .ok_or_else(|| {
                    HoldingBatteryReject::Evaluation("promoted elite missing after deposit".into())
                })?;
            Ok(HoldingBatteryResult {
                fingerprint: fingerprint.clone(),
                decision,
                elite: promoted,
            })
        }
        other => {
            bank.telemetry.record(other);
            Err(HoldingBatteryReject::DatabankDeposit(other))
        }
    }
}

/// Concatenate decision partitions for a validation replay.  A segmented
/// timeline can interleave validation and training windows, so the caller's
/// filtered datasets are not necessarily in chronological order.  Sort and
/// deduplicate here before replay; feeding `IST` followed by `ISV1` would make
/// the M1 judge seek backwards and report false missing-open errors.
#[allow(dead_code)]
fn join_datasets(first: &BarDataset, second: &BarDataset) -> BarDataset {
    let mut bars = Vec::with_capacity(first.bars.len() + second.bars.len());
    bars.extend_from_slice(&first.bars);
    bars.extend_from_slice(&second.bars);
    bars.sort_unstable_by_key(|bar| bar.timestamp_ms);
    bars.dedup_by_key(|bar| bar.timestamp_ms);
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: first.delimiter,
        source_timezone: first.source_timezone.clone(),
        bars,
    }
}

#[allow(dead_code)]
fn expectancy_from(trades: &[quantforge_eval::Trade], timestamp_ms: i64) -> f64 {
    expectancy_for(trades, |entry| entry >= timestamp_ms)
}

fn expectancy_for(trades: &[quantforge_eval::Trade], include: impl Fn(i64) -> bool) -> f64 {
    let mut profit = 0.0;
    let mut count = 0usize;
    for trade in trades {
        if include(trade.entry_timestamp_ms) {
            profit += trade.net_profit;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        profit / count as f64
    }
}

/// Fraction of trades that open and close on the same decision bar (SQX ambiguous).
pub(crate) fn ambiguous_trade_fraction(result: &ScoutResult) -> f64 {
    let total = result.trades.len();
    if total == 0 {
        return 0.0;
    }
    let ambiguous = result
        .trades
        .iter()
        .filter(|trade| trade.bars_held == 0)
        .count();
    ambiguous as f64 / total as f64
}

fn evaluate_strategy_m1_with_optional_quotes(
    strategy: &StrategyIr,
    decision_dataset: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    judge: &JudgeConfig,
) -> Result<quantforge_tick::JudgeResult, quantforge_tick::JudgeError> {
    match quote_dataset {
        Some(quotes) => evaluate_strategy_m1_with_quotes(
            strategy,
            decision_dataset,
            m1_dataset,
            quotes,
            broker,
            judge,
        ),
        None => evaluate_strategy_m1(strategy, decision_dataset, m1_dataset, broker, judge),
    }
}

pub fn evolve_new(
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: DiscoverConfig,
    generations: u64,
) -> Result<Databank, DiscoverError> {
    evolve_new_with_pack(
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        &[],
        &broker.symbol,
        config,
        generations,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn evolve_new_with_pack(
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    config: DiscoverConfig,
    generations: u64,
) -> Result<Databank, DiscoverError> {
    evolve_new_with_pack_and_quotes(
        dataset,
        oos1_dataset,
        m1_dataset,
        None,
        broker,
        pack,
        primary_symbol,
        config,
        generations,
    )
}

pub fn new_databank(
    dataset: &BarDataset,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    mut config: DiscoverConfig,
) -> Result<Databank, DiscoverError> {
    config.apply_run_mode();
    config.validate()?;
    broker.validate()?;
    Ok(Databank {
        schema_version: DATABANK_SCHEMA_VERSION,
        grammar_version: GRAMMAR_VERSION.into(),
        data_hash: dataset.data_hash.clone(),
        execution_data_hash: m1_dataset.data_hash.clone(),
        broker_spec_hash: broker.content_hash()?,
        config,
        completed_generations: 0,
        evaluation_count: 0,
        elites: Vec::new(),
        coverage_map: BTreeMap::new(),
        accepted_pool: Vec::new(),
        accepted_coverage_map: BTreeMap::new(),
        specialist_pool: Vec::new(),
        specialist_coverage_map: BTreeMap::new(),
        holding: Vec::new(),
        holding_coverage_map: BTreeMap::new(),
        telemetry: Default::default(),
    })
}

fn generate_initial_population(config: &DiscoverConfig) -> Vec<StrategyIr> {
    (0..config.initial_candidates)
        .map(|index| {
            let island_id = index % config.effective_island_count();
            let mut rng = rng_for(config.seed, 99, index as u64);
            let mut seeded = build_seed(
                SearchFamily::Universal,
                &mut rng,
                format!("i{island_id}-seed-{index}"),
                config.universal_grammar.maximum_entry_conditions.max(1),
                true,
                config.market_entries_only(),
                &config.universal_grammar,
            );
            apply_search_ranges(&mut seeded, &mut rng, &config.search_ranges);
            apply_production_policy(seeded, config)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn evolve_new_with_pack_and_quotes(
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    config: DiscoverConfig,
    generations: u64,
) -> Result<Databank, DiscoverError> {
    for market in pack {
        market.broker.validate()?;
    }
    let mut bank = new_databank(dataset, m1_dataset, broker, config)?;
    let initial = generate_initial_population(&bank.config);
    let pool = build_worker_pool(bank.config.resolved_scout_worker_threads())?;
    let promotion = PromotionPipeline::new(
        bank.config.resolved_promotion_worker_threads(),
        bank.config.promotion_queue_capacity,
    )?;
    // One cache per decision dataset, reused by every candidate in the run.
    let indicator_cache = Arc::new(IndicatorBufferCache::new(dataset.bars.len()));
    evaluate_and_deposit(
        &mut bank,
        initial,
        dataset,
        oos1_dataset,
        m1_dataset,
        quote_dataset,
        broker,
        pack,
        primary_symbol,
        0,
        pool.as_ref(),
        &promotion,
        &indicator_cache,
    )?;
    run_generations(
        &mut bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        quote_dataset,
        broker,
        pack,
        primary_symbol,
        generations,
        pool.as_ref(),
        &promotion,
        &indicator_cache,
    )?;
    promotion.flush(&mut bank)?;
    Ok(bank)
}

pub fn continue_evolution(
    bank: Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    additional_generations: u64,
) -> Result<Databank, DiscoverError> {
    continue_evolution_with_pack(
        bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        &[],
        &broker.symbol,
        additional_generations,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn continue_evolution_with_pack(
    bank: Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    additional_generations: u64,
) -> Result<Databank, DiscoverError> {
    continue_evolution_with_pack_and_quotes(
        bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        None,
        broker,
        pack,
        primary_symbol,
        additional_generations,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn continue_evolution_with_pack_and_quotes(
    mut bank: Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    additional_generations: u64,
) -> Result<Databank, DiscoverError> {
    validate_resume(&bank, dataset, m1_dataset, broker)?;
    let pool = build_worker_pool(bank.config.resolved_scout_worker_threads())?;
    let promotion = PromotionPipeline::new(
        bank.config.resolved_promotion_worker_threads(),
        bank.config.promotion_queue_capacity,
    )?;
    let indicator_cache = Arc::new(IndicatorBufferCache::new(dataset.bars.len()));
    run_generations(
        &mut bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        quote_dataset,
        broker,
        pack,
        primary_symbol,
        additional_generations,
        pool.as_ref(),
        &promotion,
        &indicator_cache,
    )?;
    promotion.flush(&mut bank)?;
    Ok(bank)
}

/// Long-lived worker pools and cross-candidate indicator cache for a caller that
/// advances one generation at a time.
///
/// H1 scouting runs on the scout pool. After breeding unlocks, Development
/// robustness → M1 promotions drain on a dedicated pool.
/// [`continue_evolution_with_pack`] builds both per call, which is right for a
/// batch of generations but throws the indicator cache away every generation
/// when a UI advances singly to checkpoint and refresh progress. Holding one
/// session for the whole run keeps `ATR(14)`-style buffers warm across
/// generations instead of recomputing them from the first bar each time.
pub struct EvolutionSession {
    scout_pool: Option<rayon::ThreadPool>,
    promotion: PromotionPipeline,
    indicator_cache: Arc<IndicatorBufferCache>,
}

impl EvolutionSession {
    /// `decision_bars` must be the bar count of the one decision dataset this
    /// session will be advanced against; buffers are keyed to those bars.
    pub fn new(config: &DiscoverConfig, decision_bars: usize) -> Result<Self, DiscoverError> {
        // Holding admission is one M1 replay; the old 2-worker / 8-slot cap was
        // sized for the full neighborhood battery sitting on this queue, which
        // Quota no longer does. Scout throughput stays independently configurable.
        let promotion_workers = config.resolved_session_promotion_workers();
        let promotion_capacity = config.resolved_session_promotion_capacity();
        Ok(Self {
            scout_pool: build_worker_pool(config.resolved_scout_worker_threads())?,
            promotion: PromotionPipeline::new(promotion_workers, promotion_capacity)?,
            indicator_cache: Arc::new(IndicatorBufferCache::new(decision_bars)),
        })
    }

    /// Wait for every queued promotion to finish and deposit results.
    pub fn flush_promotions(&self, bank: &mut Databank) -> Result<(), DiscoverError> {
        self.promotion.flush(bank)
    }

    pub fn promotion_queue_depth(&self) -> usize {
        self.promotion.queue_depth()
    }

    pub fn promotion_inflight(&self) -> usize {
        self.promotion.inflight_running()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &self,
        bank: Databank,
        dataset: &BarDataset,
        oos1_dataset: Option<&BarDataset>,
        m1_dataset: &BarDataset,
        broker: &SymbolSpecification,
        pack: &[PackSymbol],
        primary_symbol: &str,
        additional_generations: u64,
    ) -> Result<Databank, DiscoverError> {
        self.advance_with_quotes(
            bank,
            dataset,
            oos1_dataset,
            m1_dataset,
            None,
            broker,
            pack,
            primary_symbol,
            additional_generations,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn advance_with_quotes(
        &self,
        mut bank: Databank,
        dataset: &BarDataset,
        oos1_dataset: Option<&BarDataset>,
        m1_dataset: &BarDataset,
        quote_dataset: Option<&QuoteBarDataset>,
        broker: &SymbolSpecification,
        pack: &[PackSymbol],
        primary_symbol: &str,
        additional_generations: u64,
    ) -> Result<Databank, DiscoverError> {
        validate_resume(&bank, dataset, m1_dataset, broker)?;
        run_generations(
            &mut bank,
            dataset,
            oos1_dataset,
            m1_dataset,
            quote_dataset,
            broker,
            pack,
            primary_symbol,
            additional_generations,
            self.scout_pool.as_ref(),
            &self.promotion,
            &self.indicator_cache,
        )?;
        Ok(bank)
    }

    /// Evaluate the grammar seed population in scout-sized chunks so a live
    /// worker can publish Looked-at before the first generation starts.
    ///
    /// Does not flush M1 holding admissions: those keep running on the session
    /// promotion pool while later generations scout.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_initial_population<F>(
        &self,
        bank: &mut Databank,
        dataset: &BarDataset,
        oos1_dataset: Option<&BarDataset>,
        m1_dataset: &BarDataset,
        quote_dataset: Option<&QuoteBarDataset>,
        broker: &SymbolSpecification,
        pack: &[PackSymbol],
        primary_symbol: &str,
        on_progress: &mut F,
    ) -> Result<(), DiscoverError>
    where
        F: FnMut(&Databank) -> Result<bool, DiscoverError>,
    {
        let initial = generate_initial_population(&bank.config);
        let chunk = bank.config.batch_size.max(64);
        for piece in initial.chunks(chunk) {
            evaluate_and_deposit(
                bank,
                piece.to_vec(),
                dataset,
                oos1_dataset,
                m1_dataset,
                quote_dataset,
                broker,
                pack,
                primary_symbol,
                0,
                self.scout_pool.as_ref(),
                &self.promotion,
                &self.indicator_cache,
            )?;
            if !on_progress(bank)? {
                return Ok(());
            }
        }
        Ok(())
    }
}

fn build_worker_pool(worker_threads: usize) -> Result<Option<rayon::ThreadPool>, DiscoverError> {
    if worker_threads == 0 {
        return Ok(None);
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(worker_threads)
        .build()
        .map(Some)
        .map_err(|error| DiscoverError::InvalidConfig(format!("worker thread pool: {error}")))
}

#[allow(clippy::too_many_arguments)]
fn run_generations(
    bank: &mut Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    count: u64,
    scout_pool: Option<&rayon::ThreadPool>,
    promotion: &PromotionPipeline,
    indicator_cache: &Arc<IndicatorBufferCache>,
) -> Result<(), DiscoverError> {
    for _ in 0..count {
        if let Some(limit) = bank.config.early_stop_pot_elites {
            if bank.accepted_pool.len() >= limit {
                break;
            }
        }
        let generation = bank.completed_generations + 1;
        let batch = breed_generation(bank, generation);
        evaluate_and_deposit(
            bank,
            batch,
            dataset,
            oos1_dataset,
            m1_dataset,
            quote_dataset,
            broker,
            pack,
            primary_symbol,
            generation,
            scout_pool,
            promotion,
            indicator_cache,
        )?;
        bank.completed_generations = generation;
        if bank.config.migration_interval > 0
            && generation % bank.config.migration_interval == 0
            && bank.config.effective_island_count() > 1
        {
            crate::islands::migrate_islands(bank);
        }
    }
    Ok(())
}

fn breed_generation(bank: &Databank, generation: u64) -> Vec<StrategyIr> {
    let pot_size = bank.accepted_pool.len();
    let breeding_unlocked = pot_size >= bank.config.mutate_after_elites;
    let max_atoms = bank
        .config
        .universal_grammar
        .maximum_entry_conditions
        .max(1);
    let market_only = bank.config.market_entries_only();
    let island_count = bank.config.effective_island_count();
    (0..bank.config.batch_size)
        .map(|index| {
            let island_id = (index % island_count) as u16;
            let sequence = generation
                .wrapping_mul(1_000_000)
                .wrapping_add(index as u64);
            let mut rng = rng_for(bank.config.seed, generation + 10, index as u64);
            let fresh_seed = |rng: &mut ChaCha8Rng| {
                let mut seeded = build_seed(
                    SearchFamily::Universal,
                    rng,
                    format!("i{island_id}-g{generation}-{index}"),
                    max_atoms,
                    true,
                    market_only,
                    &bank.config.universal_grammar,
                );
                apply_search_ranges(&mut seeded, rng, &bank.config.search_ranges);
                apply_production_policy(seeded, &bank.config)
            };
            let random_fill = if breeding_unlocked {
                bank.config
                    .random_fill_fraction
                    .min(POST_BREED_RANDOM_FILL_CAP)
            } else {
                bank.config.random_fill_fraction
            };
            let keep_filling =
                !breeding_unlocked || bank.accepted_pool.is_empty() || rng.gen_bool(random_fill);
            if keep_filling {
                return fresh_seed(&mut rng);
            }

            // A separate exploitation lane starts only from candidates that
            // passed the complete Development battery. It freezes the logical
            // tree and execution modules and perturbs existing numeric genes.
            let specialist_probability = 0.25;
            if !bank.specialist_pool.is_empty() && rng.gen_bool(specialist_probability) {
                let parent = crate::islands::tournament_in_elites(
                    &bank.specialist_pool,
                    &bank.config,
                    &mut rng,
                    island_id,
                )
                .unwrap_or_else(|| tournament_in(&bank.specialist_pool, &bank.config, &mut rng));
                if let Ok(mut child) = perturb_strategy_parameters(
                    &bank.specialist_pool[parent].strategy,
                    bank.config.robustness_perturbation_fraction,
                    sequence as usize,
                    bank.config.seed ^ generation,
                ) {
                    child.id = format!("i{island_id}-g{generation}-{index}-specialist");
                    return child;
                }
            }

            let first_index = crate::islands::tournament_on_island(bank, &mut rng, island_id)
                .unwrap_or_else(|| tournament(bank, &mut rng, None));
            let first = &bank.accepted_pool[first_index];
            let second_index = crate::islands::tournament_on_island(bank, &mut rng, island_id)
                .unwrap_or_else(|| tournament(bank, &mut rng, None));
            let crossed = crossover(
                &first.strategy,
                &bank.accepted_pool[second_index].strategy,
                &mut rng,
            );
            let mut child = mutate_with_rng(
                &crossed,
                &mut rng,
                bank.config.structural_mutation_probability,
                sequence,
                false,
                SearchFamily::Universal,
                &bank.config.universal_grammar,
            );
            child.id = format!("i{island_id}-g{generation}-{index}");
            apply_search_ranges(&mut child, &mut rng, &bank.config.search_ranges);
            let child = apply_production_policy(child, &bank.config);
            crate::grammar::fit_within_ir_limits(
                child,
                bank.config.universal_grammar.minimum_entry_conditions,
            )
            .unwrap_or_else(|| fresh_seed(&mut rng))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_gate_results(
    is_expectancy: f64,
    oos1_expectancy: Option<f64>,
    oos1_ratio: Option<f64>,
    oos1_ok: bool,
    robustness_reject: Option<&crate::robustness::RobustnessReject>,
    multi_symbol: &[SymbolScreenResult],
    multi_symbol_minimum: usize,
    deflated: Option<f64>,
    deflated_floor: Option<f64>,
) -> Vec<GateResult> {
    let mut gates = vec![GateResult {
        name: "is_deposit".into(),
        passed: true,
        detail: format!("IS expectancy {is_expectancy:.4} R"),
    }];
    if let Some(oos1) = oos1_expectancy {
        gates.push(GateResult {
            name: "oos1_retention".into(),
            passed: oos1_ok,
            detail: format!(
                "OOS1 {oos1:.4} R · ratio {}",
                oos1_ratio
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "n/a".into())
            ),
        });
    }
    if multi_symbol_minimum > 0 {
        let passing = multi_symbol.iter().filter(|row| row.passed).count();
        gates.push(GateResult {
            name: "multi_symbol".into(),
            passed: passing >= multi_symbol_minimum,
            detail: format!("{passing}/{multi_symbol_minimum} pack symbols"),
        });
    }
    match robustness_reject {
        None => gates.push(GateResult {
            name: "m1_robustness".into(),
            passed: true,
            detail: "passed or not required".into(),
        }),
        Some(reject) => gates.push(GateResult {
            name: "m1_robustness".into(),
            passed: false,
            detail: format!("{reject:?}"),
        }),
    }
    if let Some(floor) = deflated_floor {
        gates.push(GateResult {
            name: "deflated_sharpe".into(),
            passed: deflated.is_some_and(|value| value >= floor),
            detail: format!(
                "deflated {} · floor {floor:.3}",
                deflated
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "n/a".into())
            ),
        });
    }
    gates
}

fn apply_production_policy(
    mut strategy: StrategyIr,
    config: &crate::model::DiscoverConfig,
) -> StrategyIr {
    strategy.manage.flatten_end_of_day = config.flatten_at_22 || config.sl_tp_only_exits;
    strategy.manage.end_of_day_hour = config.end_of_day_hour;
    strategy.manage.max_one_entry_per_day = config.max_one_entry_per_day;
    strategy.meta.thesis_hint = format!("{:?}", classify_family(&strategy)).to_ascii_lowercase();
    if config.sl_tp_only_exits {
        enforce_sl_tp_only_exits(&mut strategy);
    } else if config.simple_exits {
        enforce_simple_exits(&mut strategy, &config.search_ranges);
    } else {
        enforce_execution_feature_flags(&mut strategy, config);
    }
    enforce_fixed_pip_protection(&mut strategy, config);
    strategy
}

/// Remove every exit degree of freedom except the protective SL/TP pair and
/// the common broker-local end-of-day flatten. This runs after every seed and
/// mutation, so no hidden exit gene can survive into evaluation or export.
fn enforce_sl_tp_only_exits(strategy: &mut StrategyIr) {
    strategy.exit = None;
    strategy.exit_long = None;
    strategy.exit_short = None;
    strategy.manage.time_stop_bars = None;
    strategy.manage.trailing = None;
    strategy.manage.break_even_at_r = None;
    strategy.manage.partial_exits.clear();
}

/// Fixed distances are deliberately a small, explicit alternative to the
/// volatility-scaled SL/TP pair. The desktop stores the bound broker's pip
/// size with the run recipe, so EURUSD and JPY pairs use the same human pip
/// ladder despite different quote precisions.
fn enforce_fixed_pip_protection(strategy: &mut StrategyIr, config: &crate::model::DiscoverConfig) {
    if !config.allow_fixed_pip_stops {
        return;
    }
    let selector = strategy
        .id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        });
    // Keep an equal-sized ATR/R comparison arm. A fixed stop and target are a
    // single protective-exit family, rather than two independently tuned ways
    // to fit the same sample.
    if execution_gene_lane(selector, 0x6669_7865_645f_736c) & 1 != 0 {
        return;
    }
    let pip_points = config.fixed_pip_size_points;
    let fixed_pips = |lane: u64, range: &crate::model::SearchRange| {
        let steps = ((range.maximum - range.minimum) / range.step).floor().max(0.0) as u64;
        range.minimum + (lane % (steps + 1)) as f64 * range.step
    };
    let stop_pips = fixed_pips(
        execution_gene_lane(selector, 0x6669_7865_645f_7374),
        &config.search_ranges.fixed_stop_pips,
    );
    let target_pips = fixed_pips(
        execution_gene_lane(selector, 0x6669_7865_645f_7470),
        &config.search_ranges.fixed_target_pips,
    );
    strategy.stops.stop_loss = quantforge_ir::StopLossPolicy::FixedPoints {
        points: stop_pips * pip_points,
    };
    strategy.stops.take_profit = quantforge_ir::TakeProfitPolicy::FixedPoints {
        points: target_pips * pip_points,
    };
}

#[derive(Clone, Copy)]
enum EntryOrderKind {
    Market,
    Stop,
    Limit,
}

/// Derive an independent deterministic lane from a candidate selector.
///
/// Rotating the same FNV hash is not enough separation when candidate ids are
/// sequential: modulo-small decisions (order kind and on/off feature flags)
/// can become perfectly correlated. SplitMix64's avalanche step gives each
/// execution gene its own reproducible stream without coupling it to another.
fn execution_gene_lane(selector: u64, salt: u64) -> u64 {
    let mut value = selector ^ salt;
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Restrict the grammar to the individually enabled execution genes.  The
/// selector is derived from the immutable candidate id, so it is deterministic
/// across replays while still giving the search both an off-state and several
/// parameterized on-states for every enabled module.
fn enforce_execution_feature_flags(
    strategy: &mut StrategyIr,
    config: &crate::model::DiscoverConfig,
) {
    use quantforge_ir::{EntryDistancePolicy, EntryOrderPolicy};

    let selector = strategy
        .id
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        });

    let (distance, expiry_bars) = match &strategy.entry.order {
        EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        }
        | EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        } => (distance.clone(), *expiry_bars),
        EntryOrderPolicy::Market => (
            EntryDistancePolicy::AtrMultiple {
                // This is only a temporary value for the market branch.  The
                // search-range pass has already assigned the actual ATR gene.
                period: config.search_ranges.atr_period.minimum.round().max(1.0) as u16,
                multiplier: 0.5,
            },
            4,
        ),
    };
    // Every enabled order kind is an equally weighted gene, so unchecking
    // market yields a pure stop-only or limit-only run and vice versa.
    let mut enabled = Vec::with_capacity(3);
    if config.allow_market_entries {
        enabled.push(EntryOrderKind::Market);
    }
    if config.allow_stop_entries {
        enabled.push(EntryOrderKind::Stop);
    }
    if config.allow_limit_entries {
        enabled.push(EntryOrderKind::Limit);
    }
    // `validate` rejects a run with nothing enabled; market keeps replay safe.
    let order_lane = execution_gene_lane(selector, 0x6f72_6465_725f_7479);
    let chosen = enabled
        .get(order_lane as usize % enabled.len().max(1))
        .copied()
        .unwrap_or(EntryOrderKind::Market);
    strategy.entry.order = match chosen {
        EntryOrderKind::Market => EntryOrderPolicy::Market,
        EntryOrderKind::Stop => EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        },
        EntryOrderKind::Limit => EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        },
    };

    let break_even_lane = execution_gene_lane(selector, 0x6272_6561_6b5f_6576);
    let trailing_lane = execution_gene_lane(selector, 0x7472_6169_6c69_6e67);
    let partial_lane = execution_gene_lane(selector, 0x7061_7274_6961_6c73);
    let exit_rule_lane = execution_gene_lane(selector, 0x696e_6469_6361_746f);
    let time_stop_lane = execution_gene_lane(selector, 0x7469_6d65_5f73_746f);

    if !config.allow_indicator_exit_rules || exit_rule_lane % 3 == 0 {
        strategy.exit = None;
        strategy.exit_long = None;
        strategy.exit_short = None;
    }
    if !config.allow_time_stops || time_stop_lane % 3 == 0 {
        strategy.manage.time_stop_bars = None;
    } else if strategy.manage.time_stop_bars.is_none() {
        let range = &config.search_ranges.time_stop_bars;
        let (minimum, maximum) = if range.minimum == 0.0 && range.maximum == 0.0 && range.step == 0.0 {
            (4_u16, 16_u16)
        } else {
            let minimum = range.minimum.round().max(1.0) as u16;
            let maximum = range.maximum.round().max(f64::from(minimum)) as u16;
            (minimum, maximum)
        };
        let span = usize::from(maximum.saturating_sub(minimum)) + 1;
        strategy.manage.time_stop_bars = Some(minimum + (time_stop_lane as usize % span) as u16);
    }

    if !config.allow_break_even || break_even_lane % 3 == 0 {
        strategy.manage.break_even_at_r = None;
    } else if strategy.manage.break_even_at_r.is_none() {
        strategy.manage.break_even_at_r =
            Some([0.75, 1.0, 1.25, 1.5][break_even_lane as usize % 4]);
    }
    if !config.allow_trailing_stops || trailing_lane % 3 == 0 {
        strategy.manage.trailing = None;
    } else if strategy.manage.trailing.is_none() {
        strategy.manage.trailing = Some(quantforge_ir::TrailingPolicy::RiskMultiple {
            activate_at_r: [1.0, 1.5, 2.0][trailing_lane as usize % 3],
            distance_r: [0.5, 0.75, 1.0]
                [execution_gene_lane(selector, 0x7472_6169_6c5f_7061) as usize % 3],
        });
    }
    if !config.allow_partial_exits || partial_lane % 3 == 0 {
        strategy.manage.partial_exits.clear();
    } else if strategy.manage.partial_exits.is_empty() {
        strategy.manage.partial_exits.push(quantforge_ir::PartialExit {
            at_r: [0.75, 1.0, 1.25, 1.5][partial_lane as usize % 4],
            fraction: [0.25, 0.5, 0.75]
                [execution_gene_lane(selector, 0x7061_7274_5f66_7261) as usize % 3],
        });
    }
}

/// Selected-timeframe compatibility profile.
///
/// Signals use completed bars and entries occur at the next bar open. Market
/// entries avoid inventing an unknowable intrabar path for stop/limit fills.
/// Pending entries remain searchable on Selected-TF; M1 fidelity / MT5 parity
/// are final gates after Discover (SQX RetestWithHigherPrecision pattern), not
/// admission blockers during breeding.
fn enforce_simple_exits(strategy: &mut StrategyIr, ranges: &crate::model::SearchRangeProfile) {
    strategy.entry.order = quantforge_ir::EntryOrderPolicy::Market;
    strategy.manage.trailing = None;
    strategy.manage.break_even_at_r = None;
    strategy.manage.partial_exits.clear();
    let bars = strategy.manage.time_stop_bars.unwrap_or(8).clamp(
        ranges.time_stop_bars.minimum.round().max(1.0) as u16,
        ranges.time_stop_bars.maximum.round().clamp(1.0, 16.0) as u16,
    );
    strategy.manage.time_stop_bars = Some(bars);
    // Point stops are not cross-symbol portable; force ATR multiples while
    // preserving the candidate's searchable ATR lookback.
    let atr_period = |period: u16| {
        (f64::from(period).clamp(ranges.atr_period.minimum, ranges.atr_period.maximum)
            / ranges.atr_period.step)
            .round()
            .mul_add(ranges.atr_period.step, ranges.atr_period.minimum)
            .round()
            .max(1.0) as u16
    };
    strategy.stops.stop_loss = match &strategy.stops.stop_loss {
        quantforge_ir::StopLossPolicy::AtrMultiple { period, multiplier } => {
            quantforge_ir::StopLossPolicy::AtrMultiple {
                period: atr_period(*period),
                multiplier: clamp_to_range(*multiplier, &ranges.atr_stop_multiple),
            }
        }
        quantforge_ir::StopLossPolicy::FixedPoints { .. }
        | quantforge_ir::StopLossPolicy::RangeMultiple { .. } => {
            quantforge_ir::StopLossPolicy::AtrMultiple {
                period: atr_period(ranges.atr_period.minimum.round().max(1.0) as u16),
                multiplier: clamp_to_range(2.0, &ranges.atr_stop_multiple),
            }
        }
    };
    strategy.stops.take_profit = match &strategy.stops.take_profit {
        quantforge_ir::TakeProfitPolicy::RiskMultiple { multiple } => {
            quantforge_ir::TakeProfitPolicy::RiskMultiple {
                multiple: clamp_to_range(*multiple, &ranges.risk_target_multiple),
            }
        }
        quantforge_ir::TakeProfitPolicy::AtrMultiple { period, multiplier } => {
            quantforge_ir::TakeProfitPolicy::AtrMultiple {
                period: atr_period(*period),
                multiplier: clamp_to_range(*multiplier, &ranges.atr_target_multiple),
            }
        }
        quantforge_ir::TakeProfitPolicy::FixedPoints { .. } => {
            quantforge_ir::TakeProfitPolicy::RiskMultiple {
                multiple: clamp_to_range(2.0, &ranges.risk_target_multiple),
            }
        }
    };
}

fn clamp_to_range(value: f64, range: &crate::model::SearchRange) -> f64 {
    let steps = ((value.clamp(range.minimum, range.maximum) - range.minimum) / range.step).round();
    (range.minimum + steps * range.step).clamp(range.minimum, range.maximum)
}

fn tournament(
    bank: &Databank,
    rng: &mut ChaCha8Rng,
    preferred_family: Option<crate::model::FamilyStyle>,
) -> usize {
    let pool: Vec<usize> = bank
        .accepted_pool
        .iter()
        .enumerate()
        .filter_map(|(index, elite)| {
            preferred_family
                .is_none_or(|family| classify_family(&elite.strategy) == family)
                .then_some(index)
        })
        .collect();
    let pool = if pool.is_empty() {
        (0..bank.accepted_pool.len()).collect()
    } else {
        pool
    };

    let mut winner = pool[rng.gen_range(0..pool.len())];
    for _ in 1..bank.config.tournament_size {
        let contender = pool[rng.gen_range(0..pool.len())];
        if selection_is_better(
            &bank.accepted_pool[contender],
            &bank.accepted_pool[winner],
            bank.config.novelty_weight,
        ) {
            winner = contender;
        }
    }
    winner
}

fn tournament_in(pool: &[Elite], config: &DiscoverConfig, rng: &mut ChaCha8Rng) -> usize {
    let mut winner = rng.gen_range(0..pool.len());
    for _ in 1..config.tournament_size {
        let contender = rng.gen_range(0..pool.len());
        if selection_is_better(&pool[contender], &pool[winner], config.novelty_weight) {
            winner = contender;
        }
    }
    winner
}

fn selection_is_better(left: &Elite, right: &Elite, novelty_weight: f64) -> bool {
    let left_score = left.evidence.total + novelty_weight * left.novelty;
    let right_score = right.evidence.total + novelty_weight * right.novelty;
    left_score
        .total_cmp(&right_score)
        .then_with(|| {
            left.structural_fingerprint
                .cmp(&right.structural_fingerprint)
        })
        .is_gt()
}

#[cfg(test)]
fn precision_passes(
    metrics: &quantforge_eval::BacktestMetrics,
    return_retention: f64,
    gates: &crate::model::GateConfig,
    minimum_return_retention: f64,
) -> bool {
    let effective_profit_factor = metrics.profit_factor.unwrap_or({
        if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::INFINITY
        } else {
            0.0
        }
    });
    metrics.trade_count >= gates.minimum_trades
        && metrics.return_percent > gates.minimum_return_percent
        && metrics.max_drawdown_percent <= gates.maximum_drawdown_percent
        && effective_profit_factor >= gates.minimum_profit_factor
        && recovery_factor(metrics) >= gates.minimum_recovery_factor
        && return_retention.is_finite()
        && return_retention >= minimum_return_retention
}

fn validate_resume(
    bank: &Databank,
    dataset: &BarDataset,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
) -> Result<(), DiscoverError> {
    bank.config.validate()?;
    if bank.schema_version != DATABANK_SCHEMA_VERSION {
        return Err(DiscoverError::IncompatibleDatabank(format!(
            "schema version {} is not supported",
            bank.schema_version
        )));
    }
    if bank.grammar_version != GRAMMAR_VERSION {
        return Err(DiscoverError::IncompatibleDatabank(format!(
            "grammar {} does not match {}",
            bank.grammar_version, GRAMMAR_VERSION
        )));
    }
    if bank.data_hash != dataset.data_hash {
        return Err(DiscoverError::IncompatibleDatabank(
            "data hash changed".into(),
        ));
    }
    if bank.execution_data_hash != m1_dataset.data_hash {
        return Err(DiscoverError::IncompatibleDatabank(
            "M1 execution data hash changed".into(),
        ));
    }
    if bank.broker_spec_hash != broker.content_hash()? {
        return Err(DiscoverError::IncompatibleDatabank(
            "broker specification hash changed".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GateConfig;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_core::ContentHash;
    use quantforge_data::Bar;
    use quantforge_eval::{CostModel, SameBarPolicy, ScoutConfig};

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Discovery fixture".into(),
            symbol: "TEST".into(),
            digits: 2,
            point: 0.01,
            tick_size: 0.01,
            tick_value: 1.0,
            contract_size: 1.0,
            volume_min: 0.01,
            volume_step: 0.01,
            volume_max: 100.0,
            stops_level_points: 0,
            freeze_level_points: 0,
            filling_modes: vec![FillingMode::FillOrKill],
            trade_mode: TradeMode::Full,
            margin_initial_per_lot: None,
            swap_mode: SwapMode::Disabled,
            swap_long: 0.0,
            swap_short: 0.0,
            triple_swap_day: DayOfWeek::Wednesday,
            swap_multipliers: vec![],
            sessions: vec![],
            timezone: "Etc/UTC".into(),
            account_currency: "USD".into(),
            base_currency: "USD".into(),
            profit_currency: "USD".into(),
            margin_currency: "USD".into(),
            synthetic_spreads: vec![],
        }
    }

    fn dataset() -> BarDataset {
        let bars: Vec<_> = (0..320)
            .map(|index| {
                let wave = ((index as f64) / 7.0).sin() * 0.8;
                let open = 100.0 + wave + index as f64 * 0.002;
                let close = open + (((index % 5) as f64) - 2.0) * 0.08;
                Bar {
                    timestamp_ms: index as i64 * 60_000,
                    open,
                    high: open.max(close) + 0.25,
                    low: open.min(close) - 0.25,
                    close,
                    tick_volume: 100,
                    real_volume: 0,
                    spread_points: Some(1),
                }
            })
            .collect();
        BarDataset {
            data_hash: ContentHash::sha256(serde_json::to_vec(&bars).unwrap()),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars,
        }
    }

    fn config() -> DiscoverConfig {
        DiscoverConfig {
            initial_candidates: 32,
            batch_size: 16,
            correlation_threshold: 1.0,
            novelty_weight: 2.0,
            tournament_size: 3,
            structural_mutation_probability: 0.3,
            seed: 1234,
            run_mode: crate::DiscoverRunMode::FullHarvest,
            universal_grammar: crate::model::UniversalGrammarConfig::default(),
            target_databank_elites: None,
            early_stop_pot_elites: None,
            trial_budget_warning: crate::TRIAL_BUDGET_WARNING,
            gates: GateConfig {
                minimum_trades: 0,
                maximum_drawdown_percent: 100.0,
                minimum_return_percent: -100.0,
                minimum_profit_factor: 0.0,
                minimum_recovery_factor: 0.0,
            },
            deposit_gates: GateConfig {
                minimum_trades: 0,
                maximum_drawdown_percent: 100.0,
                minimum_return_percent: -100.0,
                minimum_profit_factor: 0.0,
                minimum_recovery_factor: 0.0,
            },
            precision: crate::model::PrecisionGateConfig {
                minimum_return_retention: 0.0,
            },
            search_ranges: crate::model::SearchRangeProfile::default(),
            oos1_expectancy_retention: 0.0,
            minimum_development_expectancy_r: 0.0,
            require_m1_precision: true,
            simple_exits: true,
            sl_tp_only_exits: false,
            allow_fixed_pip_stops: false,
            fixed_pip_size_points: 10.0,
            allow_indicator_exit_rules: false,
            allow_time_stops: false,
            allow_break_even: false,
            allow_trailing_stops: false,
            allow_partial_exits: false,
            allow_market_entries: true,
            allow_stop_entries: false,
            allow_limit_entries: false,
            flatten_at_22: false,
            end_of_day_hour: 23,
            // Fixture series is short; pendings need multiple fills/day to illuminate niches.
            max_one_entry_per_day: false,
            // Keep breeding locked in unit fixtures so tests exercise the H1 pot
            // path without paying for the post-breed M1 robustness battery.
            mutate_after_elites: 10_000,
            random_fill_fraction: 0.0,
            worker_threads: 1,
            promotion_worker_threads: 1,
            promotion_queue_capacity: 8,
            max_accepted_pool_elites: 10_000,
            max_specialist_pool_elites: 2_000,
            max_databank_elites: 5_000,
            max_holding_elites: 5_000,
            max_elites_per_niche: 2,
            max_promoted_per_niche: 4,
            max_per_entry_family: 8,
            build_to_holding: false,
            require_m1_robustness: true,
            robustness_folds: 3,
            robustness_monte_carlo_trials: 50,
            robustness_monte_carlo_block_length: 5,
            robustness_monte_carlo_skip_trade_probability:
                crate::robustness::MONTE_CARLO_SKIP_TRADE_PROBABILITY,
            robustness_monte_carlo_p80_profit_retention:
                crate::robustness::MONTE_CARLO_P80_PROFIT_RETENTION,
            robustness_monte_carlo_max_drawdown_ratio:
                crate::robustness::MONTE_CARLO_MAX_DRAWDOWN_RATIO,
            robustness_neighborhood_samples: 2,
            robustness_perturbation_fraction: 0.20,
            minimum_neighborhood_survival_fraction: 0.7,
            calendar_year_folds: false,
            minimum_deflated_trade_sharpe: None,
            multi_symbol_minimum_pass: 0,
            history_start_year: crate::model::default_history_start_year(),
            island_count: 1,
            migration_interval: 0,
            migration_elites: 2,
            general_island_count: 0,
            refinement_island_count: 0,
            exploration_island_count: 0,
            scout: ScoutConfig {
                initial_balance: 10_000.0,
                same_bar_policy: SameBarPolicy::Conservative,
                costs: CostModel::default(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn sl_tp_only_profile_removes_every_searchable_exit_gene() {
        let mut strategy = crate::grammar::generate_seed(7, 11);
        strategy.manage.time_stop_bars = Some(12);
        strategy.manage.break_even_at_r = Some(1.0);
        strategy.manage.trailing = Some(quantforge_ir::TrailingPolicy::RiskMultiple {
            activate_at_r: 1.0,
            distance_r: 0.5,
        });
        strategy.manage.partial_exits.push(quantforge_ir::PartialExit {
            at_r: 1.0,
            fraction: 0.5,
        });
        let mut constrained = config();
        constrained.sl_tp_only_exits = true;

        let result = super::apply_production_policy(strategy, &constrained);

        assert!(result.exit.is_none());
        assert!(result.exit_long.is_none());
        assert!(result.exit_short.is_none());
        assert!(result.manage.time_stop_bars.is_none());
        assert!(result.manage.break_even_at_r.is_none());
        assert!(result.manage.trailing.is_none());
        assert!(result.manage.partial_exits.is_empty());
        assert!(result.manage.flatten_end_of_day);
    }

    #[test]
    fn fixed_pip_protection_is_an_explicit_symbol_scaled_sl_tp_arm() {
        let mut constrained = config();
        constrained.sl_tp_only_exits = true;
        constrained.allow_fixed_pip_stops = true;
        constrained.fixed_pip_size_points = 10.0;

        let candidates: Vec<_> = (0..48)
            .map(|index| {
                super::apply_production_policy(
                    crate::grammar::generate_seed(19, index),
                    &constrained,
                )
            })
            .collect();
        let fixed: Vec<_> = candidates
            .iter()
            .filter(|strategy| {
                matches!(strategy.stops.stop_loss, quantforge_ir::StopLossPolicy::FixedPoints { .. })
                    && matches!(strategy.stops.take_profit, quantforge_ir::TakeProfitPolicy::FixedPoints { .. })
            })
            .collect();
        assert!(!fixed.is_empty());
        assert!(fixed.iter().all(|strategy| {
            matches!(strategy.stops.stop_loss, quantforge_ir::StopLossPolicy::FixedPoints { points } if points >= 100.0)
                && matches!(strategy.stops.take_profit, quantforge_ir::TakeProfitPolicy::FixedPoints { points } if points >= 150.0)
                && strategy.exit.is_none()
                && strategy.manage.time_stop_bars.is_none()
        }));
    }

    #[test]
    fn evolution_is_reproducible_and_illuminates_multiple_niches() {
        let dataset = dataset();
        let first = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        let second = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        assert_eq!(first, second);
        assert!(first.pot_size() >= 4);
        assert!(
            first.elites.is_empty(),
            "databank stays empty before breeding"
        );
        assert_eq!(first.evaluation_count, 64);
        let non_market: Vec<_> = first
            .accepted_pool
            .iter()
            .filter(|elite| {
                !matches!(
                    elite.strategy.entry.order,
                    quantforge_ir::EntryOrderPolicy::Market
                )
            })
            .map(|elite| &elite.strategy.entry.order)
            .collect();
        assert!(
            non_market.is_empty(),
            "non-market production entries: {non_market:?}"
        );
        assert_eq!(first.completed_generations, 2);
    }

    #[test]
    fn pot_fills_without_databank_before_breeding() {
        let dataset = dataset();
        let bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        assert!(
            !bank.accepted_pool.is_empty(),
            "H1 gates should fill the breeding pot"
        );
        assert!(
            bank.elites.is_empty(),
            "nothing enters the databank before breeding unlocks"
        );
        bank.validate_integrity().unwrap();
    }

    #[test]
    fn m1_precision_flag_does_not_change_selected_tf_pot() {
        let dataset = dataset();
        let mut precise_config = config();
        precise_config.require_m1_precision = true;
        precise_config.precision.minimum_return_retention = 1.0;
        let precise = evolve_new(&dataset, None, &dataset, &broker(), precise_config, 2).unwrap();
        let scout = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();

        let pot_of = |bank: &Databank| -> Vec<quantforge_core::ContentHash> {
            bank.accepted_pool
                .iter()
                .map(|elite| elite.structural_fingerprint.clone())
                .collect()
        };
        assert!(!precise.accepted_pool.is_empty());
        assert_eq!(
            pot_of(&precise),
            pot_of(&scout),
            "pot admission must depend on Selected-TF evidence only"
        );
        assert!(precise.elites.is_empty());
        assert!(scout.elites.is_empty());
        precise.validate_integrity().unwrap();
    }

    #[test]
    fn checkpoint_resume_matches_uninterrupted_evolution() {
        let dataset = dataset();
        let uninterrupted = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        let checkpoint = evolve_new(&dataset, None, &dataset, &broker(), config(), 1).unwrap();
        let resumed =
            continue_evolution(checkpoint, &dataset, None, &dataset, &broker(), 1).unwrap();
        assert_eq!(uninterrupted, resumed);
    }

    #[test]
    fn a_reused_session_advancing_singly_matches_batch_evolution() {
        let dataset = dataset();
        let batch = evolve_new(&dataset, None, &dataset, &broker(), config(), 3).unwrap();
        let mut stepped = evolve_new(&dataset, None, &dataset, &broker(), config(), 1).unwrap();
        let session = EvolutionSession::new(&stepped.config, dataset.bars.len()).unwrap();
        for _ in 0..2 {
            stepped = session
                .advance(
                    stepped,
                    &dataset,
                    None,
                    &dataset,
                    &broker(),
                    &[],
                    &broker().symbol,
                    1,
                )
                .unwrap();
        }
        assert_eq!(batch, stepped);
    }

    #[test]
    fn persisted_databank_integrity_rejects_fingerprint_tampering() {
        let dataset = dataset();
        let mut bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 1).unwrap();
        bank.validate_integrity().unwrap();
        assert!(!bank.accepted_pool.is_empty());
        bank.accepted_pool[0].structural_fingerprint = ContentHash::sha256("tampered");
        assert!(bank.validate_integrity().is_err());
    }

    #[test]
    fn precision_gate_rejects_the_observed_false_h1_edge() {
        let metrics = quantforge_eval::BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 100_311.5,
            net_profit: 311.5,
            return_percent: 0.3115,
            trade_count: 76,
            winning_trades: 38,
            losing_trades: 38,
            win_rate: 50.0,
            profit_factor: Some(1.0125),
            max_drawdown: 8_824.0,
            max_drawdown_percent: 8.824,
            sharpe_ratio: Some(0.03),
            expectancy: 311.5 / 76.0,
            expectancy_r: 0.0,
            median_r: 0.0,
        };
        let gates = GateConfig {
            minimum_trades: 20,
            maximum_drawdown_percent: 30.0,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            minimum_recovery_factor: 0.0,
        };
        let retention = metrics.return_percent / 21.5217;
        assert!(!precision_passes(&metrics, retention, &gates, 0.95));
    }

    #[test]
    fn oos1_pick_requires_positive_retention_of_is_expectancy() {
        assert!(passes_oos1_pick(10.0, 7.0, 0.7));
        assert!(passes_oos1_pick(10.0, 12.0, 1.2));
        assert!(!passes_oos1_pick(10.0, 6.9, 0.7));
        assert!(!passes_oos1_pick(10.0, 11.9, 1.2));
        assert!(!passes_oos1_pick(10.0, -1.0, 0.7));
        assert!(!passes_oos1_pick(-2.0, 5.0, 0.7));
        assert!(!passes_oos1_pick(f64::NAN, 5.0, 0.7));
    }

    #[test]
    fn minimum_development_expectancy_is_expressed_in_fixed_r() {
        assert!(passes_development_expectancy(200.0, 0.2));
        assert!(!passes_development_expectancy(199.99, 0.2));
        assert!(!passes_development_expectancy(f64::NAN, 0.2));
    }

    #[test]
    fn close_at_22_is_a_job_policy_not_an_evolvable_gene() {
        let dataset = dataset();
        let mut config = config();
        config.flatten_at_22 = true;
        let bank = evolve_new(&dataset, None, &dataset, &broker(), config, 1).unwrap();
        assert!(
            bank.elites
                .iter()
                .all(|elite| elite.strategy.manage.flatten_end_of_day)
        );
        bank.validate_integrity().unwrap();
    }

    #[test]
    fn max_one_entry_per_day_is_a_job_policy_not_an_evolvable_gene() {
        let dataset = dataset();
        let mut config = config();
        config.max_one_entry_per_day = true;
        let bank = evolve_new(&dataset, None, &dataset, &broker(), config, 1).unwrap();
        assert!(
            bank.elites
                .iter()
                .all(|elite| elite.strategy.manage.max_one_entry_per_day)
        );
        bank.validate_integrity().unwrap();
    }

    #[test]
    fn execution_modules_are_distinct_opt_in_genes() {
        let mut config = DiscoverConfig::default();
        config.simple_exits = false;
        config.require_m1_precision = true;
        config.allow_break_even = true;
        config.allow_trailing_stops = true;
        config.allow_partial_exits = true;
        config.allow_stop_entries = true;
        config.allow_limit_entries = true;
        config.validate().unwrap();

        let candidates: Vec<_> = (0..96)
            .map(|index| {
                apply_production_policy(
                    crate::grammar::generate_seed_for_family(
                        42,
                        index,
                        SearchFamily::TrendPullback,
                    ),
                    &config,
                )
            })
            .collect();
        assert!(
            candidates
                .iter()
                .any(|strategy| strategy.manage.break_even_at_r.is_some())
        );
        assert!(
            candidates
                .iter()
                .any(|strategy| strategy.manage.trailing.is_some())
        );
        assert!(
            candidates
                .iter()
                .any(|strategy| !strategy.manage.partial_exits.is_empty())
        );
        assert!(candidates.iter().any(|strategy| matches!(
            strategy.entry.order,
            quantforge_ir::EntryOrderPolicy::Stop { .. }
        )));
        assert!(candidates.iter().any(|strategy| matches!(
            strategy.entry.order,
            quantforge_ir::EntryOrderPolicy::Limit { .. }
        )));

        let management_enabled = |strategy: &&StrategyIr| {
            strategy.manage.break_even_at_r.is_some()
                || strategy.manage.trailing.is_some()
                || !strategy.manage.partial_exits.is_empty()
        };
        assert!(
            candidates
                .iter()
                .filter(management_enabled)
                .any(|strategy| {
                    matches!(
                        strategy.entry.order,
                        quantforge_ir::EntryOrderPolicy::Market
                    )
                })
        );
        assert!(candidates.iter().any(|strategy| {
            matches!(
                strategy.entry.order,
                quantforge_ir::EntryOrderPolicy::Stop { .. }
                    | quantforge_ir::EntryOrderPolicy::Limit { .. }
            ) && strategy.manage.break_even_at_r.is_none()
                && strategy.manage.trailing.is_none()
                && strategy.manage.partial_exits.is_empty()
        }));
        assert!(candidates.iter().any(|strategy| {
            strategy.manage.break_even_at_r.is_some()
                && strategy.manage.trailing.is_none()
                && strategy.manage.partial_exits.is_empty()
        }));
        assert!(candidates.iter().any(|strategy| {
            strategy.manage.break_even_at_r.is_none()
                && strategy.manage.trailing.is_some()
                && strategy.manage.partial_exits.is_empty()
        }));
        assert!(candidates.iter().any(|strategy| {
            strategy.manage.break_even_at_r.is_none()
                && strategy.manage.trailing.is_none()
                && !strategy.manage.partial_exits.is_empty()
        }));
    }

    #[test]
    fn complex_execution_no_longer_requires_m1_during_discover() {
        let mut config = DiscoverConfig::default();
        config.allow_trailing_stops = true;
        config.require_m1_precision = false;
        config.validate().unwrap();
    }

    #[test]
    fn entry_order_kinds_are_independently_selectable() {
        use quantforge_ir::EntryOrderPolicy;

        let population = |config: &DiscoverConfig| -> Vec<StrategyIr> {
            (0..96)
                .map(|index| {
                    apply_production_policy(
                        crate::grammar::generate_seed_for_family(
                            42,
                            index,
                            SearchFamily::TrendPullback,
                        ),
                        config,
                    )
                })
                .collect()
        };
        let counts = |strategies: &[StrategyIr]| -> (usize, usize, usize) {
            let market = strategies
                .iter()
                .filter(|value| matches!(value.entry.order, EntryOrderPolicy::Market))
                .count();
            let stop = strategies
                .iter()
                .filter(|value| matches!(value.entry.order, EntryOrderPolicy::Stop { .. }))
                .count();
            let limit = strategies
                .iter()
                .filter(|value| matches!(value.entry.order, EntryOrderPolicy::Limit { .. }))
                .count();
            (market, stop, limit)
        };

        for (market, stop, limit) in [
            (false, true, false),
            (false, false, true),
            (false, true, true),
            (true, true, false),
        ] {
            let mut config = DiscoverConfig::default();
            config.simple_exits = false;
            config.require_m1_precision = true;
            config.allow_market_entries = market;
            config.allow_stop_entries = stop;
            config.allow_limit_entries = limit;
            config.validate().unwrap();

            let (markets, stops, limits) = counts(&population(&config));
            assert_eq!(
                markets > 0,
                market,
                "market share for {market}/{stop}/{limit}"
            );
            assert_eq!(stops > 0, stop, "stop share for {market}/{stop}/{limit}");
            assert_eq!(limits > 0, limit, "limit share for {market}/{stop}/{limit}");
        }
    }

    #[test]
    fn a_run_with_no_entry_order_kind_is_rejected() {
        let mut config = DiscoverConfig::default();
        config.require_m1_precision = true;
        config.allow_market_entries = false;
        assert!(config.validate().is_err());
    }

    #[test]
    fn high_performance_mode_restores_historical_island_topology() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::HighPerformanceIslands;
        config.apply_run_mode();
        assert_eq!(config.island_count, 4);
        assert_eq!(config.migration_interval, 10);
        assert_eq!(config.migration_elites, 2);
        assert!(config.initial_candidates >= 2_000);
        assert!(config.batch_size >= 500);
        assert!(config.require_m1_robustness);
        assert!(config.require_m1_precision);
    }

    #[test]
    fn quota_mode_keeps_the_robustness_battery_discriminating() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::QuotaHarvest;
        // Old presets clamped these down to 80/5/0.5, which passed everything.
        config.robustness_monte_carlo_trials = 80;
        config.robustness_neighborhood_samples = 5;
        config.minimum_neighborhood_survival_fraction = 0.5;
        config.apply_run_mode();
        // A P95 drawdown read off fewer than ~1k resampled paths is noise.
        assert!(config.robustness_monte_carlo_trials >= 1_000);
        // Ten deterministic axis neighbors come first; 200 H1-cached samples
        // make the ret/DD histogram dense enough for the orig/median band.
        assert!(config.robustness_neighborhood_samples >= 200);
        assert!(config.minimum_neighborhood_survival_fraction >= 0.55);
    }

    #[test]
    fn quota_holding_is_not_capped_by_family_or_niche_inventory() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::QuotaHarvest;
        config.apply_run_mode();
        assert_eq!(config.target_databank_elites, Some(400));
        assert!(config.build_to_holding);
        assert!(config.max_holding_elites >= 2_000);
        assert!(config.max_elites_per_niche >= 8);
    }

    #[test]
    fn quota_mode_keeps_an_explicit_holding_target() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::QuotaHarvest;
        config.target_databank_elites = Some(500);
        config.multi_symbol_minimum_pass = 6;
        config.apply_run_mode();
        assert_eq!(config.target_databank_elites, Some(500));
        assert!(config.build_to_holding);
        assert_eq!(config.multi_symbol_minimum_pass, 0);
    }

    #[test]
    fn quota_mode_keeps_a_large_explicit_holding_target() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::QuotaHarvest;
        config.target_databank_elites = Some(2_000);
        config.apply_run_mode();
        assert_eq!(config.target_databank_elites, Some(2_000));
    }

    #[test]
    fn factory_score_ranks_busy_positive_expectancy_ahead_of_heroes() {
        assert!(super::holding_factory_score(200, 0.12) > super::holding_factory_score(40, 0.40));
        assert_eq!(super::holding_factory_score(500, -0.10), 0.0);
    }

    #[test]
    fn explicit_robustness_bypass_moves_the_complete_holding_cohort() {
        let dataset = dataset();
        let mut bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 0).unwrap();
        bank.holding = bank.accepted_pool.iter().take(3).cloned().collect();
        assert!(!bank.holding.is_empty());
        let expected: Vec<_> = bank
            .holding
            .iter()
            .map(|elite| elite.structural_fingerprint.clone())
            .collect();

        let result = super::promote_all_holding_without_robustness(&mut bank);

        assert_eq!(result.promoted, expected.len());
        assert_eq!(result.replaced, 0);
        assert!(bank.holding.is_empty());
        assert!(bank.holding_coverage_map.is_empty());
        assert_eq!(bank.elites.len(), expected.len());
        assert_eq!(bank.coverage_map.len(), expected.len());
        assert!(bank.elites.iter().all(|elite| {
            elite
                .gate_results
                .iter()
                .any(|gate| gate.name == "robustness_bypass" && !gate.passed)
        }));
        assert!(expected.iter().all(|fingerprint| {
            bank.elites
                .iter()
                .any(|elite| &elite.structural_fingerprint == fingerprint)
        }));
    }

    #[test]
    fn audited_graduation_preserves_the_audit_instead_of_marking_a_bypass() {
        let dataset = dataset();
        let mut bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 0).unwrap();
        bank.holding = bank.accepted_pool.iter().take(1).cloned().collect();
        let fingerprint = bank.holding[0].structural_fingerprint.to_string();
        bank.holding[0].gate_results.push(GateResult {
            name: "robustness_audit".into(),
            passed: false,
            detail: "Recorded failure.".into(),
        });

        let result = super::promote_selected_holding_without_robustness(
            &mut bank,
            &BTreeSet::from([fingerprint]),
            None,
        );

        assert_eq!(result.promoted, 1);
        assert!(bank.elites[0]
            .gate_results
            .iter()
            .any(|gate| gate.name == "robustness_audit" && !gate.passed));
        assert!(!bank.elites[0]
            .gate_results
            .iter()
            .any(|gate| gate.name == "robustness_bypass"));
    }

    #[test]
    fn battery_audit_records_its_result_without_moving_the_holding_candidate() {
        let dataset = dataset();
        let mut bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 0).unwrap();
        bank.holding = bank.accepted_pool.iter().take(1).cloned().collect();
        let fingerprint = bank.holding[0].structural_fingerprint.clone();

        let result = super::audit_holding_battery(
            &mut bank,
            &fingerprint,
            &dataset,
            &dataset,
            None,
            &broker(),
        )
        .unwrap();

        assert_eq!(result.fingerprint, fingerprint);
        assert_eq!(bank.holding.len(), 1);
        assert!(bank.elites.is_empty());
        assert!(bank.holding[0]
            .gate_results
            .iter()
            .any(|gate| gate.name == "robustness_audit" && gate.passed == result.passed));
    }

    #[test]
    fn production_lane_promotion_moves_only_selected_holding_names() {
        let dataset = dataset();
        let mut bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 0).unwrap();
        bank.holding = bank.accepted_pool.iter().take(3).cloned().collect();
        assert!(bank.holding.len() >= 2);
        let selected_fingerprint = bank.holding[0].structural_fingerprint.to_string();
        let untouched_fingerprint = bank.holding[1].structural_fingerprint.clone();
        let selected = BTreeSet::from([selected_fingerprint]);

        let result = super::promote_selected_holding_without_robustness(
            &mut bank,
            &selected,
            Some(GateResult {
                name: "production_lane_v1".into(),
                passed: true,
                detail: "Selected using Development evidence only.".into(),
            }),
        );

        assert_eq!(result.promoted, 1);
        assert_eq!(result.replaced, 0);
        assert!(
            bank.holding
                .iter()
                .any(|elite| elite.structural_fingerprint == untouched_fingerprint)
        );
        let promoted = bank
            .elites
            .iter()
            .find(|elite| selected.contains(elite.structural_fingerprint.as_str()))
            .unwrap();
        assert!(
            promoted
                .gate_results
                .iter()
                .any(|gate| { gate.name == "production_lane_v1" && gate.passed })
        );
        assert!(
            promoted
                .gate_results
                .iter()
                .any(|gate| { gate.name == "robustness_bypass" && !gate.passed })
        );
    }

    #[test]
    fn high_performance_islands_skips_pack_replay_overhead() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::HighPerformanceIslands;
        config.multi_symbol_minimum_pass = 6;
        config.apply_run_mode();
        assert_eq!(config.multi_symbol_minimum_pass, 0);
    }

    #[test]
    fn holding_admission_does_not_inherit_the_full_battery_worker_cap() {
        let mut holding = DiscoverConfig::default();
        holding.build_to_holding = true;
        holding.promotion_worker_threads = 8;
        holding.promotion_queue_capacity = 64;
        assert_eq!(holding.resolved_session_promotion_workers(), 8);
        assert_eq!(holding.resolved_session_promotion_capacity(), 64);

        let mut battery = DiscoverConfig::default();
        battery.build_to_holding = false;
        battery.promotion_worker_threads = 8;
        battery.promotion_queue_capacity = 64;
        assert_eq!(battery.resolved_session_promotion_workers(), 8);
        assert_eq!(battery.resolved_session_promotion_capacity(), 64);
    }

    #[test]
    fn only_general_islands_and_migration_controls_are_honored() {
        let mut config = DiscoverConfig::default();
        config.run_mode = crate::DiscoverRunMode::HighPerformanceIslands;
        config.general_island_count = 3;
        config.refinement_island_count = 2;
        config.exploration_island_count = 1;
        config.migration_interval = 17;
        config.migration_elites = 3;
        config.apply_run_mode();
        assert_eq!(config.effective_island_count(), 3);
        assert_eq!(config.refinement_island_count, 0);
        assert_eq!(config.exploration_island_count, 0);
        assert_eq!(config.migration_interval, 17);
        assert_eq!(config.migration_elites, 3);
    }

    #[test]
    fn the_parameter_jitter_must_stay_inside_a_meaningful_band() {
        let mut config = DiscoverConfig::default();
        config.require_m1_robustness = true;
        for fraction in [0.0, 1.5, f64::NAN] {
            config.robustness_perturbation_fraction = fraction;
            assert!(config.validate().is_err(), "{fraction} should be rejected");
        }
        config.robustness_perturbation_fraction = 0.35;
        config.validate().unwrap();
    }

    #[test]
    fn promotion_queue_capacity_must_be_positive() {
        let mut config = DiscoverConfig::default();
        config.promotion_queue_capacity = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn promotion_context_is_shared_across_generations() {
        let dataset = dataset();
        let broker = broker();
        let config = config();
        let pipeline = PromotionPipeline::new(1, 2).unwrap();
        let first = pipeline.context_for(&config, &dataset, None, &dataset, None, &broker);
        let second = pipeline.context_for(&config, &dataset, None, &dataset, None, &broker);

        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first.dataset, &second.dataset));
        assert!(Arc::ptr_eq(&first.m1, &second.m1));
    }

    #[test]
    fn oos1_is_kept_out_of_breeding_archives() {
        let dataset = dataset();
        let mut cfg = config();
        cfg.mutate_after_elites = 1;
        cfg.require_m1_robustness = false;
        let oos1 = dataset.clone();
        let bank = evolve_new(&dataset, Some(&oos1), &dataset, &broker(), cfg, 2)
            .expect("OOS1 is deferred until after the Development battery");
        assert!(
            bank.accepted_pool
                .iter()
                .all(|elite| elite.oos1_expectancy.is_none())
        );
        assert!(
            bank.elites
                .iter()
                .all(|elite| elite.oos1_expectancy.is_none())
        );
    }

    #[test]
    fn scout_generations_advance_while_promotions_are_configured() {
        let dataset = dataset();
        let mut cfg = config();
        cfg.mutate_after_elites = 1;
        cfg.require_m1_robustness = false;
        cfg.promotion_worker_threads = 1;
        cfg.promotion_queue_capacity = 8;
        let session = EvolutionSession::new(&cfg, dataset.bars.len()).unwrap();
        let oos1 = dataset.clone();
        let mut bank = evolve_new(&dataset, Some(&oos1), &dataset, &broker(), cfg, 0).unwrap();
        // Seed pot without generations, then step once via session.
        bank = session
            .advance(
                bank,
                &dataset,
                Some(&oos1),
                &dataset,
                &broker(),
                &[],
                "TEST",
                1,
            )
            .unwrap();
        assert_eq!(bank.completed_generations, 1);
        // Scout finished the generation without waiting for a flush of every
        // promotion; flush still applies the same deposit path.
        session.flush_promotions(&mut bank).unwrap();
        assert_eq!(
            bank.telemetry.promotions_enqueued,
            bank.telemetry.promotions_completed
        );
    }
}
