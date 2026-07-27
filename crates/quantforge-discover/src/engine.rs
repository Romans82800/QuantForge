use crate::archive::{deposit_to_accepted_pool, deposit_to_databank, CandidateEvaluation};
use crate::grammar::{
    apply_search_ranges, build_seed, classify_family, crossover, mutate_with_rng, rng_for,
};
use crate::model::{
    return_drawdown_ratio, Databank, DepositDecision, DiscoverConfig, DiscoverError, Elite,
    GateResult, SearchFamily, SymbolScreenResult,
};
use crate::multi_symbol::{screen_multi_symbol, PackSymbol};
use crate::{DATABANK_SCHEMA_VERSION, GRAMMAR_VERSION};
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::evaluate_strategy;
use quantforge_eval::{ScoutResult, ScoutTelemetry};
use quantforge_ir::StrategyIr;
use quantforge_quality::{deflated_trade_sharpe, expected_max_lucky_sharpe, trade_sharpe_proxy};
use quantforge_tick::{evaluate_strategy_m1, JudgeConfig};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::collections::BTreeMap;

enum CandidateOutcome {
    CoarseRejected,
    AmbiguousRejected,
    PrecisionRejected,
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
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    generation: u64,
    pool: Option<&rayon::ThreadPool>,
) -> Result<(), DiscoverError> {
    let scout = bank.config.scout.clone();
    let gates = &bank.config.gates;
    let discover_config = &bank.config;
    let minimum_return_retention = bank.config.precision.minimum_return_retention;
    let oos1_retention = bank.config.oos1_expectancy_retention;
    let require_m1 = bank.config.require_m1_precision;
    let require_robustness = bank.config.require_m1_robustness;
    let multi_symbol_minimum = bank.config.multi_symbol_minimum_pass;
    let minimum_deflated = bank.config.minimum_deflated_trade_sharpe;
    let evaluations_touched = bank.evaluation_count.max(1);
    let robustness = crate::robustness::RobustnessConfig {
        folds: bank.config.robustness_folds,
        monte_carlo_trials: bank.config.robustness_monte_carlo_trials,
        neighborhood_samples: bank.config.robustness_neighborhood_samples,
        seed: bank.config.seed,
        initial_balance: scout.initial_balance,
        costs: scout.costs.clone(),
        minimum_return_retention,
        minimum_fold_trades: bank.config.deposit_gates.minimum_trades.clamp(1, 2),
        minimum_return_percent: bank.config.deposit_gates.minimum_return_percent,
        minimum_profit_factor: bank.config.deposit_gates.minimum_profit_factor.min(1.0),
        maximum_drawdown_percent: bank.config.deposit_gates.maximum_drawdown_percent.max(30.0),
        minimum_passing_fold_fraction: 0.6,
        minimum_neighborhood_survival_fraction: 0.7,
        parameter_perturbation_fraction: 0.1,
        adx_period_min: bank
            .config
            .search_ranges
            .indicator_period
            .minimum
            .round()
            .max(2.0) as u16,
        adx_period_max: bank
            .config
            .search_ranges
            .indicator_period
            .maximum
            .round()
            .max(2.0) as u16,
        adx_period_step: bank
            .config
            .search_ranges
            .indicator_period
            .step
            .round()
            .max(1.0) as u16,
        adx_threshold_min: bank.config.search_ranges.adx_threshold.minimum,
        adx_threshold_max: bank.config.search_ranges.adx_threshold.maximum,
        adx_threshold_step: bank.config.search_ranges.adx_threshold.step,
        calendar_year_folds: bank.config.calendar_year_folds,
    };
    let evaluate_batch = || {
        candidates
            .into_par_iter()
            .map(|strategy| {
                let result = (|| {
                    let coarse = evaluate_strategy(&strategy, dataset, broker, &scout)
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

                    // SQX Selected-TF path: deposit from H1/IS metrics; M1 fidelity is deferred
                    // unless require_m1_precision is on (legacy in-loop) or robustness is on.
                    let deposit_result = if require_m1 {
                        let precise = evaluate_strategy_m1(
                            &strategy,
                            dataset,
                            m1_dataset,
                            broker,
                            &JudgeConfig {
                                initial_balance: scout.initial_balance,
                                costs: scout.costs.clone(),
                                allow_execution_gaps: true,
                            },
                        )
                        .map_err(|error| error.to_string())?;
                        let precise_result = ScoutResult {
                            trades: precise.trades,
                            equity: precise.equity,
                            metrics: precise.metrics,
                            telemetry: ScoutTelemetry::default(),
                        };
                        let retention = if coarse.metrics.return_percent > 0.0 {
                            precise_result.metrics.return_percent / coarse.metrics.return_percent
                        } else if precise_result.metrics.return_percent
                            >= coarse.metrics.return_percent
                        {
                            1.0
                        } else {
                            0.0
                        };
                        if !precision_passes(
                            &precise_result.metrics,
                            retention,
                            gates,
                            minimum_return_retention,
                        ) {
                            return Ok(CandidateOutcome::PrecisionRejected);
                        }
                        precise_result
                    } else {
                        coarse
                    };

                    if !crate::archive::passes_gate_config(
                        &deposit_result,
                        &discover_config.deposit_gates,
                    ) {
                        return Ok(CandidateOutcome::DepositGateRejected);
                    }

                    // Pot path stops here. M1 robustness + OOS1 run only after a
                    // candidate actually enters the pot (see sequential loop below).
                    let is_expectancy = deposit_result.metrics.expectancy;
                    Ok(CandidateOutcome::Accepted {
                        result: Box::new(deposit_result),
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
    let evaluated = match pool {
        Some(pool) => pool.install(evaluate_batch),
        None => evaluate_batch(),
    };

    for (strategy, result) in evaluated {
        bank.evaluation_count += 1;
        match result {
            Ok(CandidateOutcome::Accepted {
                result,
                is_expectancy,
                observed_trade_sharpe,
                expected_max_lucky_sharpe,
                deflated_trade_sharpe,
                multi_symbol_results,
            }) => {
                // Deposit to pot first with H1 metrics only — do not wait on M1.
                let pot_evaluation = CandidateEvaluation {
                    strategy: strategy.clone(),
                    result: *result,
                    generation,
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
                };
                let pot_decision = deposit_to_accepted_pool(bank, pot_evaluation.clone())?;
                bank.telemetry.record(pot_decision);
                let pot_ok = matches!(
                    pot_decision,
                    DepositDecision::AcceptedToPot | DepositDecision::ReplacedInPot
                );
                if !pot_ok {
                    // Niche/clone/correlation reject — skip expensive M1 battery.
                    continue;
                }

                // Databank path only: the M1 replay is the promotion result.
                // H1/M15 remains useful for cheaply filling the breeding pot,
                // but it must never become the metric/equity curve of a
                // promoted elite.
                let databank_gate = if require_robustness {
                    crate::robustness::run_m1_predeposit_robustness(
                        &strategy,
                        dataset,
                        m1_dataset,
                        broker,
                        &robustness,
                        &pot_evaluation.result.metrics,
                    )
                } else {
                    // Explicit research-only escape hatch. Production defaults
                    // require the branch above and therefore archive M1.
                    Ok(pot_evaluation.result.clone())
                };

                let (m1_is_result, oos1_expectancy, oos1_expectancy_ratio, oos1_ok) =
                    if let Ok(m1_is_result) = &databank_gate {
                        if let Some(oos1) = oos1_dataset {
                            match evaluate_strategy_m1(
                                &strategy,
                                oos1,
                                m1_dataset,
                                broker,
                                &JudgeConfig {
                                    initial_balance: scout.initial_balance,
                                    costs: scout.costs.clone(),
                                    allow_execution_gaps: true,
                                },
                            ) {
                                Ok(oos1_result) => {
                                    let oos1_expectancy = oos1_result.metrics.expectancy;
                                    let ok = passes_oos1_pick(
                                        m1_is_result.metrics.expectancy,
                                        oos1_expectancy,
                                        oos1_retention,
                                    );
                                    let ratio = (m1_is_result.metrics.expectancy > 0.0)
                                        .then_some(
                                            oos1_expectancy / m1_is_result.metrics.expectancy,
                                        )
                                        .filter(|value| value.is_finite());
                                    (Some(m1_is_result.clone()), Some(oos1_expectancy), ratio, ok)
                                }
                                Err(error) => {
                                    *bank
                                        .telemetry
                                        .evaluation_errors
                                        .entry(error.to_string())
                                        .or_default() += 1;
                                    bank.telemetry.record(DepositDecision::RejectedEvaluation);
                                    continue;
                                }
                            }
                        } else {
                            (Some(m1_is_result.clone()), None, None, true)
                        }
                    } else {
                        (None, None, None, false)
                    };

                if let Err(reject) = &databank_gate {
                    match reject {
                        crate::robustness::RobustnessReject::M1Fidelity => {
                            bank.telemetry.record(DepositDecision::RejectedM1Fidelity);
                        }
                        crate::robustness::RobustnessReject::WalkForward => {
                            bank.telemetry.record(DepositDecision::RejectedWalkForward);
                        }
                        crate::robustness::RobustnessReject::MonteCarlo => {
                            bank.telemetry.record(DepositDecision::RejectedMonteCarlo);
                        }
                        crate::robustness::RobustnessReject::ParamNeighborhood => {
                            bank.telemetry
                                .record(DepositDecision::RejectedParamNeighborhood);
                        }
                    }
                } else if !oos1_ok {
                    bank.telemetry.record(DepositDecision::RejectedOos1);
                } else if let Some(m1_is_result) = m1_is_result {
                    let bank_evaluation = CandidateEvaluation {
                        // M1 result and IS expectancy are what the archived
                        // strategy is ranked and displayed by.  This makes an
                        // exported MT5 comparison apples-to-apples.
                        result: m1_is_result.clone(),
                        is_expectancy: m1_is_result.metrics.expectancy,
                        oos1_expectancy,
                        oos1_expectancy_ratio,
                        gate_results: build_gate_results(
                            m1_is_result.metrics.expectancy,
                            oos1_expectancy,
                            oos1_expectancy_ratio,
                            oos1_ok,
                            None,
                            &multi_symbol_results,
                            bank.config.multi_symbol_minimum_pass,
                            deflated_trade_sharpe,
                            bank.config.minimum_deflated_trade_sharpe,
                        ),
                        ..pot_evaluation
                    };
                    let bank_decision = deposit_to_databank(bank, bank_evaluation)?;
                    bank.telemetry.record(bank_decision);
                }
            }
            Ok(CandidateOutcome::CoarseRejected) => {
                bank.telemetry.record(DepositDecision::RejectedGate);
            }
            Ok(CandidateOutcome::AmbiguousRejected) => {
                bank.telemetry.record(DepositDecision::RejectedAmbiguous);
            }
            Ok(CandidateOutcome::PrecisionRejected) => {
                bank.telemetry.record(DepositDecision::RejectedPrecision);
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
    Ok(())
}

/// Promotion pick gate: OOS1 expectancy must retain at least `retention` of IS
/// expectancy. Both windows must show positive expectancy.
pub(crate) fn passes_oos1_pick(is_expectancy: f64, oos1_expectancy: f64, retention: f64) -> bool {
    is_expectancy.is_finite()
        && oos1_expectancy.is_finite()
        && is_expectancy > 0.0
        && oos1_expectancy > 0.0
        && oos1_expectancy >= retention * is_expectancy
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
    mut config: DiscoverConfig,
    generations: u64,
) -> Result<Databank, DiscoverError> {
    config.apply_run_mode();
    config.validate()?;
    broker.validate()?;
    for market in pack {
        market.broker.validate()?;
    }
    let search_family = config.search_family;
    let mut bank = Databank {
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
        telemetry: Default::default(),
    };

    let initial = (0..bank.config.initial_candidates)
        .map(|index| {
            let mut seeded = crate::grammar::generate_seed_for_family(
                bank.config.seed,
                index as u64,
                search_family,
            );
            let mut rng = rng_for(bank.config.seed, 99, index as u64);
            apply_search_ranges(&mut seeded, &mut rng, &bank.config.search_ranges);
            apply_production_policy(seeded, &bank.config)
        })
        .collect();
    let pool = build_worker_pool(bank.config.worker_threads)?;
    evaluate_and_deposit(
        &mut bank,
        initial,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        pack,
        primary_symbol,
        0,
        pool.as_ref(),
    )?;
    run_generations(
        &mut bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        pack,
        primary_symbol,
        generations,
        pool.as_ref(),
    )?;
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
    mut bank: Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    additional_generations: u64,
) -> Result<Databank, DiscoverError> {
    validate_resume(&bank, dataset, m1_dataset, broker)?;
    let pool = build_worker_pool(bank.config.worker_threads)?;
    run_generations(
        &mut bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        pack,
        primary_symbol,
        additional_generations,
        pool.as_ref(),
    )?;
    Ok(bank)
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
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    count: u64,
    pool: Option<&rayon::ThreadPool>,
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
            broker,
            pack,
            primary_symbol,
            generation,
            pool,
        )?;
        bank.completed_generations = generation;
    }
    Ok(())
}

fn breed_generation(bank: &Databank, generation: u64) -> Vec<StrategyIr> {
    let pot_size = bank.accepted_pool.len();
    let breeding_unlocked = pot_size >= bank.config.mutate_after_elites;
    let search_family = bank.config.search_family;
    let max_atoms = search_family.spec().max_atoms.max(1);
    (0..bank.config.batch_size)
        .map(|index| {
            let sequence = generation
                .wrapping_mul(1_000_000)
                .wrapping_add(index as u64);
            let mut rng = rng_for(bank.config.seed, generation + 10, index as u64);
            let keep_filling = !breeding_unlocked
                || bank.accepted_pool.is_empty()
                || rng.gen_bool(bank.config.random_fill_fraction);
            if keep_filling {
                let mut seeded = build_seed(
                    search_family,
                    &mut rng,
                    format!("g{generation}-{index}"),
                    max_atoms,
                    true,
                );
                apply_search_ranges(&mut seeded, &mut rng, &bank.config.search_ranges);
                return apply_production_policy(seeded, &bank.config);
            }

            let first_index = tournament(bank, &mut rng, Some(search_family.style()));
            let first = &bank.accepted_pool[first_index];
            let preferred_family = Some(search_family.style());
            let second_index = tournament(bank, &mut rng, preferred_family);
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
                bank.config.allow_cross_family_mutation,
                search_family,
            );
            child.id = format!("g{generation}-{index}");
            apply_search_ranges(&mut child, &mut rng, &bank.config.search_ranges);
            apply_production_policy(child, &bank.config)
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
    strategy.manage.flatten_end_of_day = config.flatten_at_22;
    strategy.manage.max_one_entry_per_day = config.max_one_entry_per_day;
    strategy.meta.thesis_hint = match config.search_family {
        SearchFamily::TrendPullback => "trend_pullback".into(),
        SearchFamily::MomentumBurst => "momentum_burst".into(),
        SearchFamily::DonchianBreakout => "donchian_breakout".into(),
        SearchFamily::MeanReversionBand => "mean_reversion_band".into(),
        SearchFamily::ZScoreReversion => "zscore_reversion".into(),
        SearchFamily::SessionOrb => "session_orb".into(),
        SearchFamily::ImpulseCandle => "impulse_candle".into(),
        SearchFamily::VolSqueezeBreak => "vol_squeeze_break".into(),
        SearchFamily::SupplyDemandReclaim => "supply_demand_reclaim".into(),
        SearchFamily::SweepReclaim => "sweep_reclaim".into(),
    };
    if config.simple_exits {
        enforce_simple_exits(&mut strategy, &config.search_ranges);
    } else {
        enforce_execution_feature_flags(&mut strategy, config);
    }
    strategy
}

/// Restrict the grammar to the individually enabled execution genes.  The
/// selector is derived from the immutable candidate id, so it is deterministic
/// across replays while still giving the search both an off-state and several
/// parameterized on-states for every enabled module.
fn enforce_execution_feature_flags(
    strategy: &mut StrategyIr,
    config: &crate::model::DiscoverConfig,
) {
    use quantforge_ir::{EntryDistancePolicy, EntryOrderPolicy, PartialExit, TrailingPolicy};

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
                period: crate::FROZEN_ATR_PERIOD,
                multiplier: 0.5,
            },
            4,
        ),
    };
    // Market is always available.  A pending entry becomes a selectable gene
    // only when its specific order kind is enabled.
    strategy.entry.order = match (config.allow_stop_entries, config.allow_limit_entries) {
        (false, false) => EntryOrderPolicy::Market,
        (true, false) if selector % 3 == 0 => EntryOrderPolicy::Market,
        (false, true) if selector % 3 == 0 => EntryOrderPolicy::Market,
        (true, false) => EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        },
        (false, true) => EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        },
        (true, true) => match selector % 3 {
            0 => EntryOrderPolicy::Market,
            1 => EntryOrderPolicy::Stop {
                distance,
                expiry_bars,
            },
            _ => EntryOrderPolicy::Limit {
                distance,
                expiry_bars,
            },
        },
    };

    if !config.allow_break_even || selector.rotate_left(7) % 3 == 0 {
        strategy.manage.break_even_at_r = None;
    } else if strategy.manage.break_even_at_r.is_none() {
        strategy.manage.break_even_at_r = Some([0.75, 1.0, 1.25, 1.5][selector as usize % 4]);
    }
    if !config.allow_trailing_stops || selector.rotate_left(13) % 3 == 0 {
        strategy.manage.trailing = None;
    } else if strategy.manage.trailing.is_none() {
        strategy.manage.trailing = Some(TrailingPolicy::RiskMultiple {
            activate_at_r: [1.0, 1.5, 2.0][selector as usize % 3],
            distance_r: [0.5, 0.75, 1.0][selector.rotate_left(19) as usize % 3],
        });
    }
    if !config.allow_partial_exits || selector.rotate_left(23) % 3 == 0 {
        strategy.manage.partial_exits.clear();
    } else if strategy.manage.partial_exits.is_empty() {
        strategy.manage.partial_exits.push(PartialExit {
            at_r: [0.75, 1.0, 1.25, 1.5][selector.rotate_left(29) as usize % 4],
            fraction: [0.25, 0.5, 0.75][selector.rotate_left(31) as usize % 3],
        });
    }
}

/// Selected-timeframe compatibility profile.
///
/// Signals use completed bars and entries occur at the next bar open. Market
/// entries avoid inventing an unknowable intrabar path for stop/limit fills,
/// which is the largest source of Selected-TF versus M1 divergence. Pending
/// orders remain supported by IR, evaluators and exporters, but are reserved
/// for workflows that discover and validate directly on M1/ticks.
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
    // Point stops are not cross-symbol portable; force ATR multiples at frozen period.
    strategy.stops.stop_loss = match &strategy.stops.stop_loss {
        quantforge_ir::StopLossPolicy::AtrMultiple { multiplier, .. } => {
            quantforge_ir::StopLossPolicy::AtrMultiple {
                period: ranges.atr_period.minimum.round().max(1.0) as u16,
                multiplier: clamp_to_range(*multiplier, &ranges.atr_stop_multiple),
            }
        }
        quantforge_ir::StopLossPolicy::FixedPoints { .. }
        | quantforge_ir::StopLossPolicy::RangeMultiple { .. } => {
            quantforge_ir::StopLossPolicy::AtrMultiple {
                period: ranges.atr_period.minimum.round().max(1.0) as u16,
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
        quantforge_ir::TakeProfitPolicy::AtrMultiple { multiplier, .. } => {
            quantforge_ir::TakeProfitPolicy::AtrMultiple {
                period: ranges.atr_period.minimum.round().max(1.0) as u16,
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
        && return_drawdown_ratio(metrics) >= gates.minimum_return_drawdown
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
            search_family: crate::SearchFamily::TrendPullback,
            run_mode: crate::DiscoverRunMode::FullHarvest,
            allow_cross_family_mutation: false,
            early_stop_pot_elites: None,
            trial_budget_warning: crate::TRIAL_BUDGET_WARNING,
            gates: GateConfig {
                minimum_trades: 0,
                maximum_drawdown_percent: 100.0,
                minimum_return_percent: -100.0,
                minimum_profit_factor: 0.0,
                minimum_return_drawdown: 0.0,
            },
            deposit_gates: GateConfig {
                minimum_trades: 0,
                maximum_drawdown_percent: 100.0,
                minimum_return_percent: -100.0,
                minimum_profit_factor: 0.0,
                minimum_return_drawdown: 0.0,
            },
            precision: crate::model::PrecisionGateConfig {
                minimum_return_retention: 0.0,
            },
            search_ranges: crate::model::SearchRangeProfile::default(),
            oos1_expectancy_retention: 0.0,
            require_m1_precision: false,
            simple_exits: true,
            allow_break_even: false,
            allow_trailing_stops: false,
            allow_partial_exits: false,
            allow_stop_entries: false,
            allow_limit_entries: false,
            flatten_at_22: false,
            // Fixture series is short; pendings need multiple fills/day to illuminate niches.
            max_one_entry_per_day: false,
            mutate_after_elites: 0,
            random_fill_fraction: 0.0,
            worker_threads: 1,
            require_m1_robustness: false,
            robustness_folds: 3,
            robustness_monte_carlo_trials: 50,
            robustness_neighborhood_samples: 2,
            calendar_year_folds: false,
            minimum_deflated_trade_sharpe: None,
            multi_symbol_minimum_pass: 0,
            scout: ScoutConfig {
                initial_balance: 10_000.0,
                same_bar_policy: SameBarPolicy::Conservative,
                costs: CostModel::default(),
            },
        }
    }

    #[test]
    fn evolution_is_reproducible_and_illuminates_multiple_niches() {
        let dataset = dataset();
        let first = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        let second = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        assert_eq!(first, second);
        assert!(first.coverage() >= 4);
        assert_eq!(first.evaluation_count, 64);
        let non_market: Vec<_> = first
            .elites
            .iter()
            .chain(first.accepted_pool.iter())
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
    fn checkpoint_resume_matches_uninterrupted_evolution() {
        let dataset = dataset();
        let uninterrupted = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        let checkpoint = evolve_new(&dataset, None, &dataset, &broker(), config(), 1).unwrap();
        let resumed =
            continue_evolution(checkpoint, &dataset, None, &dataset, &broker(), 1).unwrap();
        assert_eq!(uninterrupted, resumed);
    }

    #[test]
    fn persisted_databank_integrity_rejects_fingerprint_tampering() {
        let dataset = dataset();
        let mut bank = evolve_new(&dataset, None, &dataset, &broker(), config(), 1).unwrap();
        bank.validate_integrity().unwrap();
        bank.elites[0].structural_fingerprint = ContentHash::sha256("tampered");
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
        };
        let gates = GateConfig {
            minimum_trades: 20,
            maximum_drawdown_percent: 30.0,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            minimum_return_drawdown: 0.0,
        };
        let retention = metrics.return_percent / 21.5217;
        assert!(!precision_passes(&metrics, retention, &gates, 0.95));
    }

    #[test]
    fn oos1_pick_requires_positive_retention_of_is_expectancy() {
        assert!(passes_oos1_pick(10.0, 7.0, 0.7));
        assert!(!passes_oos1_pick(10.0, 6.9, 0.7));
        assert!(!passes_oos1_pick(10.0, -1.0, 0.7));
        assert!(!passes_oos1_pick(-2.0, 5.0, 0.7));
        assert!(!passes_oos1_pick(f64::NAN, 5.0, 0.7));
    }

    #[test]
    fn close_at_22_is_a_job_policy_not_an_evolvable_gene() {
        let dataset = dataset();
        let mut config = config();
        config.flatten_at_22 = true;
        let bank = evolve_new(&dataset, None, &dataset, &broker(), config, 1).unwrap();
        assert!(bank
            .elites
            .iter()
            .all(|elite| elite.strategy.manage.flatten_end_of_day));
        bank.validate_integrity().unwrap();
    }

    #[test]
    fn max_one_entry_per_day_is_a_job_policy_not_an_evolvable_gene() {
        let dataset = dataset();
        let mut config = config();
        config.max_one_entry_per_day = true;
        let bank = evolve_new(&dataset, None, &dataset, &broker(), config, 1).unwrap();
        assert!(bank
            .elites
            .iter()
            .all(|elite| elite.strategy.manage.max_one_entry_per_day));
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
        assert!(candidates
            .iter()
            .any(|strategy| strategy.manage.break_even_at_r.is_some()));
        assert!(candidates
            .iter()
            .any(|strategy| strategy.manage.trailing.is_some()));
        assert!(candidates
            .iter()
            .any(|strategy| !strategy.manage.partial_exits.is_empty()));
        assert!(candidates.iter().any(|strategy| matches!(
            strategy.entry.order,
            quantforge_ir::EntryOrderPolicy::Stop { .. }
        )));
        assert!(candidates.iter().any(|strategy| matches!(
            strategy.entry.order,
            quantforge_ir::EntryOrderPolicy::Limit { .. }
        )));
    }

    #[test]
    fn complex_execution_requires_m1_precision() {
        let mut config = DiscoverConfig::default();
        config.allow_trailing_stops = true;
        assert!(config.validate().is_err());
    }
}
