use crate::archive::{CandidateEvaluation, deposit};
use crate::grammar::{build_seed, classify_family, crossover, mutate_with_rng, rng_for};
use crate::model::{
    Databank, DepositDecision, DiscoverConfig, DiscoverError, Elite, return_drawdown_ratio,
};
use crate::{DATABANK_SCHEMA_VERSION, GRAMMAR_VERSION};
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::evaluate_strategy;
use quantforge_eval::{ScoutResult, ScoutTelemetry};
use quantforge_ir::StrategyIr;
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::collections::BTreeMap;

enum CandidateOutcome {
    CoarseRejected,
    PrecisionRejected,
    Oos1Rejected,
    Accepted {
        result: Box<ScoutResult>,
        is_expectancy: f64,
        oos1_expectancy: Option<f64>,
        oos1_expectancy_ratio: Option<f64>,
    },
}

fn evaluate_and_deposit(
    bank: &mut Databank,
    candidates: Vec<StrategyIr>,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    generation: u64,
) -> Result<(), DiscoverError> {
    let scout = &bank.config.scout;
    let gates = &bank.config.gates;
    let discover_config = &bank.config;
    let minimum_return_retention = bank.config.precision.minimum_return_retention;
    let oos1_retention = bank.config.oos1_expectancy_retention;
    let evaluated: Vec<_> = candidates
        .into_par_iter()
        .map(|strategy| {
            let result = (|| {
                let coarse = evaluate_strategy(&strategy, dataset, broker, scout)
                    .map_err(|error| error.to_string())?;
                if !crate::archive::passes_gates(&coarse, discover_config) {
                    return Ok::<_, String>(CandidateOutcome::CoarseRejected);
                }
                let precise = evaluate_strategy_m1(
                    &strategy,
                    dataset,
                    m1_dataset,
                    broker,
                    &JudgeConfig {
                        initial_balance: scout.initial_balance,
                        costs: scout.costs.clone(),
                        allow_execution_gaps: false,
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
                } else if precise_result.metrics.return_percent >= coarse.metrics.return_percent {
                    1.0
                } else {
                    0.0
                };
                let precision_passed = precision_passes(
                    &precise_result.metrics,
                    retention,
                    gates,
                    minimum_return_retention,
                );
                if !precision_passed {
                    return Ok(CandidateOutcome::PrecisionRejected);
                }

                let is_expectancy = precise_result.metrics.expectancy;
                let (oos1_expectancy, oos1_expectancy_ratio) = if let Some(oos1) = oos1_dataset {
                    let oos1_result = evaluate_strategy(&strategy, oos1, broker, scout)
                        .map_err(|error| error.to_string())?;
                    let oos1_expectancy = oos1_result.metrics.expectancy;
                    if !passes_oos1_pick(is_expectancy, oos1_expectancy, oos1_retention) {
                        return Ok(CandidateOutcome::Oos1Rejected);
                    }
                    let ratio = (is_expectancy > 0.0)
                        .then_some(oos1_expectancy / is_expectancy)
                        .filter(|value| value.is_finite());
                    (Some(oos1_expectancy), ratio)
                } else {
                    (None, None)
                };

                Ok(CandidateOutcome::Accepted {
                    result: Box::new(precise_result),
                    is_expectancy,
                    oos1_expectancy,
                    oos1_expectancy_ratio,
                })
            })();
            (strategy, result)
        })
        .collect();

    for (strategy, result) in evaluated {
        bank.evaluation_count += 1;
        let decision = match result {
            Ok(CandidateOutcome::Accepted {
                result,
                is_expectancy,
                oos1_expectancy,
                oos1_expectancy_ratio,
            }) => deposit(
                bank,
                CandidateEvaluation {
                    strategy,
                    result: *result,
                    generation,
                    is_expectancy,
                    oos1_expectancy,
                    oos1_expectancy_ratio,
                },
            )?,
            Ok(CandidateOutcome::CoarseRejected) => DepositDecision::RejectedGate,
            Ok(CandidateOutcome::PrecisionRejected) => DepositDecision::RejectedPrecision,
            Ok(CandidateOutcome::Oos1Rejected) => DepositDecision::RejectedOos1,
            Err(error) => {
                *bank.telemetry.evaluation_errors.entry(error).or_default() += 1;
                DepositDecision::RejectedEvaluation
            }
        };
        bank.telemetry.record(decision);
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

pub fn evolve_new(
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: DiscoverConfig,
    generations: u64,
) -> Result<Databank, DiscoverError> {
    config.validate()?;
    broker.validate()?;
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
        telemetry: Default::default(),
    };

    let initial = (0..bank.config.initial_candidates)
        .map(|index| {
            apply_production_policy(
                crate::grammar::generate_seed(bank.config.seed, index as u64),
                &bank.config,
            )
        })
        .collect();
    evaluate_and_deposit(
        &mut bank,
        initial,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        0,
    )?;
    run_generations(
        &mut bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        generations,
    )?;
    Ok(bank)
}

pub fn continue_evolution(
    mut bank: Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    additional_generations: u64,
) -> Result<Databank, DiscoverError> {
    validate_resume(&bank, dataset, m1_dataset, broker)?;
    run_generations(
        &mut bank,
        dataset,
        oos1_dataset,
        m1_dataset,
        broker,
        additional_generations,
    )?;
    Ok(bank)
}

fn run_generations(
    bank: &mut Databank,
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    count: u64,
) -> Result<(), DiscoverError> {
    for _ in 0..count {
        let generation = bank.completed_generations + 1;
        let batch = breed_generation(bank, generation);
        evaluate_and_deposit(
            bank,
            batch,
            dataset,
            oos1_dataset,
            m1_dataset,
            broker,
            generation,
        )?;
        bank.completed_generations = generation;
    }
    Ok(())
}

fn breed_generation(bank: &Databank, generation: u64) -> Vec<StrategyIr> {
    (0..bank.config.batch_size)
        .map(|index| {
            let sequence = generation
                .wrapping_mul(1_000_000)
                .wrapping_add(index as u64);
            let mut rng = rng_for(bank.config.seed, generation + 10, index as u64);
            if bank.elites.is_empty() {
                let family = match index % 4 {
                    0 => crate::model::FamilyStyle::Trend,
                    1 => crate::model::FamilyStyle::Momentum,
                    2 => crate::model::FamilyStyle::Breakout,
                    _ => crate::model::FamilyStyle::MeanReversion,
                };
                return apply_production_policy(
                    build_seed(family, &mut rng, format!("g{generation}-{index}")),
                    &bank.config,
                );
            }

            let first_index = tournament(bank, &mut rng, None);
            let first = &bank.elites[first_index];
            let preferred_family = rng.gen_bool(0.75).then(|| classify_family(&first.strategy));
            let second_index = tournament(bank, &mut rng, preferred_family);
            let crossed = crossover(
                &first.strategy,
                &bank.elites[second_index].strategy,
                &mut rng,
            );
            let mut child = mutate_with_rng(
                &crossed,
                &mut rng,
                bank.config.structural_mutation_probability,
                sequence,
            );
            child.id = format!("g{generation}-{index}");
            apply_production_policy(child, &bank.config)
        })
        .collect()
}

fn apply_production_policy(
    mut strategy: StrategyIr,
    config: &crate::model::DiscoverConfig,
) -> StrategyIr {
    strategy.manage.flatten_end_of_day = config.flatten_at_22;
    strategy
}

fn tournament(
    bank: &Databank,
    rng: &mut ChaCha8Rng,
    preferred_family: Option<crate::model::FamilyStyle>,
) -> usize {
    let pool: Vec<usize> = bank
        .elites
        .iter()
        .enumerate()
        .filter_map(|(index, elite)| {
            preferred_family
                .is_none_or(|family| classify_family(&elite.strategy) == family)
                .then_some(index)
        })
        .collect();
    let pool = if pool.is_empty() {
        (0..bank.elites.len()).collect()
    } else {
        pool
    };

    let mut winner = pool[rng.gen_range(0..pool.len())];
    for _ in 1..bank.config.tournament_size {
        let contender = pool[rng.gen_range(0..pool.len())];
        if selection_is_better(
            &bank.elites[contender],
            &bank.elites[winner],
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
            gates: GateConfig {
                minimum_trades: 0,
                maximum_drawdown_percent: 100.0,
                minimum_return_percent: -100.0,
                minimum_profit_factor: 0.0,
                minimum_return_drawdown: 0.0,
            },
            precision: crate::model::PrecisionGateConfig {
                minimum_return_retention: 0.0,
            },
            oos1_expectancy_retention: 0.0,
            flatten_at_22: false,
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
        assert_eq!(first.completed_generations, 2);
    }

    #[test]
    fn checkpoint_resume_matches_uninterrupted_evolution() {
        let dataset = dataset();
        let uninterrupted = evolve_new(&dataset, None, &dataset, &broker(), config(), 2).unwrap();
        let checkpoint = evolve_new(&dataset, None, &dataset, &broker(), config(), 1).unwrap();
        let resumed = continue_evolution(checkpoint, &dataset, None, &dataset, &broker(), 1).unwrap();
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
        assert!(
            bank.elites
                .iter()
                .all(|elite| elite.strategy.manage.flatten_end_of_day)
        );
        bank.validate_integrity().unwrap();
    }
}
