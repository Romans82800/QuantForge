use crate::grammar::{entry_condition_count, exit_condition_count};
use crate::model::{
    BehaviorDescriptor, Databank, DepositDecision, DiscoverConfig, Elite, EvidenceComponents,
    GateResult, LongShortSkewBucket, NicheKey, RobustnessEvidence, ScoutFitnessMode,
    ThreeLevelBucket, recovery_factor,
};
use quantforge_core::{FloatPolicy, quantize};
use quantforge_eval::{PositionSide, ScoutResult};
use quantforge_ir::StrategyIr;

#[derive(Clone)]
pub(crate) struct CandidateEvaluation {
    pub strategy: StrategyIr,
    pub result: ScoutResult,
    pub generation: u64,
    pub island_id: u16,
    pub is_expectancy: f64,
    pub oos1_expectancy: Option<f64>,
    pub oos1_expectancy_ratio: Option<f64>,
    pub observed_trade_sharpe: Option<f64>,
    pub expected_max_lucky_sharpe: Option<f64>,
    pub deflated_trade_sharpe: Option<f64>,
    pub multi_symbol_results: Vec<crate::model::SymbolScreenResult>,
    pub gate_results: Vec<GateResult>,
    /// Present only on the databank path, where the M1 robustness battery ran.
    pub robustness: Option<RobustnessEvidence>,
}

pub(crate) fn deposit_to_accepted_pool(
    bank: &mut Databank,
    candidate: CandidateEvaluation,
) -> Result<DepositDecision, quantforge_ir::IrError> {
    if !passes_gate_config(&candidate.result, &bank.config.deposit_gates) {
        return Ok(DepositDecision::RejectedDepositGate);
    }
    // Breeding pot: island-local niche bags so each island can keep its own
    // parents. Family cap still applies so one grammar does not eat the bag.
    let decision = deposit_into_stack(
        &mut bank.accepted_pool,
        &mut bank.accepted_coverage_map,
        bank.config.correlation_threshold,
        bank.config.max_elites_per_niche,
        bank.config.max_per_entry_family.saturating_mul(2).max(16),
        0,
        true,
        bank.config.scout_fitness_mode,
        candidate,
        DepositDecision::AcceptedToPot,
        DepositDecision::ReplacedInPot,
    )?;
    trim_pool(
        &mut bank.accepted_pool,
        bank.config.max_accepted_pool_elites,
    );
    refresh_fingerprint_coverage_map(&bank.accepted_pool, &mut bank.accepted_coverage_map);
    Ok(decision)
}

pub(crate) fn deposit_to_databank(
    bank: &mut Databank,
    candidate: CandidateEvaluation,
) -> Result<DepositDecision, quantforge_ir::IrError> {
    if !passes_gate_config(&candidate.result, &bank.config.deposit_gates) {
        return Ok(DepositDecision::RejectedDepositGate);
    }
    // Databank is the overnight output: fingerprint uniqueness plus equity-path
    // correlation (0.5) as the clone janitor. Niches and families stay on the pot.
    let decision = deposit_into_stack(
        &mut bank.elites,
        &mut bank.coverage_map,
        0.5,
        0,
        0,
        0,
        false,
        bank.config.scout_fitness_mode,
        candidate,
        DepositDecision::AcceptedToDatabank,
        DepositDecision::ReplacedInDatabank,
    )?;
    trim_pool(&mut bank.elites, bank.config.max_databank_elites);
    refresh_fingerprint_coverage_map(&bank.elites, &mut bank.coverage_map);
    Ok(decision)
}

pub(crate) fn deposit_to_holding(
    bank: &mut Databank,
    candidate: CandidateEvaluation,
) -> Result<DepositDecision, quantforge_ir::IrError> {
    if !passes_gate_config(&candidate.result, &bank.config.deposit_gates) {
        return Ok(DepositDecision::RejectedDepositGate);
    }
    // Fingerprint clone-replace only. Correlation is a breeding-pot rule; applying
    // it here stalled Holding at ~100 names on one USDJPY sleeve.
    let decision = deposit_into_stack(
        &mut bank.holding,
        &mut bank.holding_coverage_map,
        1.0,
        0,
        0,
        0,
        false,
        bank.config.scout_fitness_mode,
        candidate,
        DepositDecision::AcceptedToHolding,
        DepositDecision::ReplacedInHolding,
    )?;
    trim_pool(&mut bank.holding, bank.config.max_holding_elites);
    refresh_fingerprint_coverage_map(&bank.holding, &mut bank.holding_coverage_map);
    Ok(decision)
}

pub(crate) fn remove_holding_by_fingerprint(
    bank: &mut Databank,
    fingerprint: &quantforge_core::ContentHash,
) -> Option<Elite> {
    let index = bank
        .holding
        .iter()
        .position(|elite| &elite.structural_fingerprint == fingerprint)?;
    let removed = bank.holding.swap_remove(index);
    refresh_fingerprint_coverage_map(&bank.holding, &mut bank.holding_coverage_map);
    Some(removed)
}

pub(crate) fn deposit_to_specialist_pool(
    bank: &mut Databank,
    candidate: CandidateEvaluation,
) -> Result<DepositDecision, quantforge_ir::IrError> {
    let decision = deposit_into_stack(
        &mut bank.specialist_pool,
        &mut bank.specialist_coverage_map,
        bank.config.correlation_threshold,
        bank.config.max_elites_per_niche,
        bank.config.max_per_entry_family.saturating_mul(2).max(16),
        0,
        true,
        bank.config.scout_fitness_mode,
        candidate,
        DepositDecision::AcceptedToPot,
        DepositDecision::ReplacedInPot,
    )?;
    trim_pool(
        &mut bank.specialist_pool,
        bank.config.max_specialist_pool_elites,
    );
    refresh_fingerprint_coverage_map(&bank.specialist_pool, &mut bank.specialist_coverage_map);
    Ok(decision)
}

fn trim_pool(entries: &mut Vec<Elite>, limit: usize) {
    while entries.len() > limit {
        let worst = entries
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                left.evidence
                    .total
                    .total_cmp(&right.evidence.total)
                    .then_with(|| left.novelty.total_cmp(&right.novelty))
                    .then_with(|| right.complexity.cmp(&left.complexity))
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        entries.swap_remove(worst);
    }
}

/// Fingerprint-keyed stacking bag (no niche uniqueness / replacement).
fn deposit_into_stack(
    entries: &mut Vec<Elite>,
    coverage_map: &mut std::collections::BTreeMap<String, quantforge_core::ContentHash>,
    correlation_threshold: f64,
    max_per_niche: usize,
    max_per_family: usize,
    peer_family_count: usize,
    niche_per_island: bool,
    scout_fitness_mode: ScoutFitnessMode,
    candidate: CandidateEvaluation,
    accepted: DepositDecision,
    replaced: DepositDecision,
) -> Result<DepositDecision, quantforge_ir::IrError> {
    let fingerprint = candidate
        .strategy
        .structural_fingerprint(FloatPolicy::default())?;
    let family = entry_family_key(&candidate.strategy);
    let descriptor = descriptor(&candidate.strategy, &candidate.result);
    let niche = niche_key(&descriptor);
    let evidence = evidence(
        &candidate.strategy,
        &candidate.result,
        scout_fitness_mode,
    );
    let signature = equity_signature(&candidate.result, 64);
    let maximum_correlation = entries
        .iter()
        .filter(|elite| elite.structural_fingerprint != fingerprint)
        .map(|elite| correlation(&signature, &elite.equity_signature))
        .fold(0.0_f64, f64::max);
    let novelty = quantized(1.0 - maximum_correlation.clamp(0.0, 1.0));
    let complexity = candidate.strategy.complexity().score;
    let island_id = candidate.island_id;

    let fingerprint_index = entries
        .iter()
        .position(|elite| elite.structural_fingerprint == fingerprint);
    if let Some(index) = fingerprint_index {
        let existing = &entries[index];
        if !better_than(
            &evidence,
            novelty,
            complexity,
            candidate.result.metrics.trade_count,
            existing,
        ) {
            return Ok(DepositDecision::RejectedClone);
        }
        entries[index] = make_elite(
            candidate,
            fingerprint.clone(),
            descriptor,
            niche,
            evidence,
            novelty,
            complexity,
            signature,
        );
        refresh_fingerprint_coverage_map(entries, coverage_map);
        return Ok(replaced);
    }

    if maximum_correlation > correlation_threshold {
        return Ok(DepositDecision::RejectedCorrelated);
    }

    let family_indexes: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, elite)| entry_family_key(&elite.strategy) == family)
        .map(|(index, _)| index)
        .collect();
    if max_per_family > 0 && family_indexes.len() + peer_family_count >= max_per_family {
        if family_indexes.is_empty() {
            // Peers already filled this family; this pool cannot add another.
            return Ok(DepositDecision::RejectedFamilyNotImproved);
        }
        let worst_index = family_indexes
            .into_iter()
            .min_by(|left, right| {
                entries[*left]
                    .evidence
                    .total
                    .total_cmp(&entries[*right].evidence.total)
                    .then_with(|| entries[*left].novelty.total_cmp(&entries[*right].novelty))
            })
            .unwrap_or(0);
        if !better_than(
            &evidence,
            novelty,
            complexity,
            candidate.result.metrics.trade_count,
            &entries[worst_index],
        ) {
            return Ok(DepositDecision::RejectedFamilyNotImproved);
        }
        entries[worst_index] = make_elite(
            candidate,
            fingerprint,
            descriptor,
            niche,
            evidence,
            novelty,
            complexity,
            signature,
        );
        refresh_fingerprint_coverage_map(entries, coverage_map);
        return Ok(replaced);
    }

    if max_per_niche > 0 {
        let niche_indexes: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, elite)| {
                elite.niche == niche
                    && (!niche_per_island || elite.island_id == island_id)
            })
            .map(|(index, _)| index)
            .collect();
        if niche_indexes.len() >= max_per_niche {
            let worst_index = niche_indexes
                .into_iter()
                .min_by(|left, right| {
                    entries[*left]
                        .evidence
                        .total
                        .total_cmp(&entries[*right].evidence.total)
                        .then_with(|| entries[*left].novelty.total_cmp(&entries[*right].novelty))
                })
                .unwrap_or(0);
            if !better_than(
                &evidence,
                novelty,
                complexity,
                candidate.result.metrics.trade_count,
                &entries[worst_index],
            ) {
                return Ok(DepositDecision::RejectedNicheNotImproved);
            }
            entries[worst_index] = make_elite(
                candidate,
                fingerprint,
                descriptor,
                niche,
                evidence,
                novelty,
                complexity,
                signature,
            );
            refresh_fingerprint_coverage_map(entries, coverage_map);
            return Ok(replaced);
        }
    }

    entries.push(make_elite(
        candidate,
        fingerprint,
        descriptor,
        niche,
        evidence,
        novelty,
        complexity,
        signature,
    ));
    refresh_fingerprint_coverage_map(entries, coverage_map);
    Ok(accepted)
}

fn make_elite(
    candidate: CandidateEvaluation,
    fingerprint: quantforge_core::ContentHash,
    descriptor: BehaviorDescriptor,
    niche: NicheKey,
    evidence: EvidenceComponents,
    novelty: f64,
    complexity: usize,
    signature: Vec<f64>,
) -> Elite {
    Elite {
        strategy: candidate.strategy,
        structural_fingerprint: fingerprint,
        descriptor,
        niche,
        evidence,
        novelty,
        complexity,
        metrics: candidate.result.metrics,
        is_expectancy: candidate.is_expectancy,
        oos1_expectancy: candidate.oos1_expectancy,
        oos1_expectancy_ratio: candidate.oos1_expectancy_ratio,
        fold_r: crate::fold_r::calendar_year_fold_r(&candidate.result.trades),
        observed_trade_sharpe: candidate.observed_trade_sharpe,
        expected_max_lucky_sharpe: candidate.expected_max_lucky_sharpe,
        deflated_trade_sharpe: candidate.deflated_trade_sharpe,
        multi_symbol_results: candidate.multi_symbol_results,
        gate_results: candidate.gate_results,
        robustness: candidate.robustness,
        equity_signature: signature,
        discovered_generation: candidate.generation,
        island_id: candidate.island_id,
    }
}

pub(crate) fn passes_gates(result: &ScoutResult, config: &DiscoverConfig) -> bool {
    passes_gate_config(result, &config.gates)
}

pub(crate) fn passes_gate_config(result: &ScoutResult, gates: &crate::model::GateConfig) -> bool {
    let metrics = &result.metrics;
    let effective_profit_factor = metrics.profit_factor.unwrap_or({
        if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::INFINITY
        } else {
            0.0
        }
    });
    metrics.trade_count >= gates.minimum_trades
        && metrics.max_drawdown_percent <= gates.maximum_drawdown_percent
        && metrics.return_percent > gates.minimum_return_percent
        && effective_profit_factor >= gates.minimum_profit_factor
        && recovery_factor(metrics) >= gates.minimum_recovery_factor
}

fn descriptor(strategy: &StrategyIr, result: &ScoutResult) -> BehaviorDescriptor {
    let trade_count = result.trades.len();
    let bar_count = result.equity.len().max(1);
    let long_count = result
        .trades
        .iter()
        .filter(|trade| trade.side == PositionSide::Long)
        .count();
    let short_count = trade_count.saturating_sub(long_count);
    BehaviorDescriptor {
        entry_conditions: entry_condition_count(strategy),
        exit_conditions: exit_condition_count(strategy),
        trades_per_1000_bars: quantized(trade_count as f64 / bar_count as f64 * 1_000.0),
        average_bars_held: if trade_count == 0 {
            0.0
        } else {
            quantized(
                result
                    .trades
                    .iter()
                    .map(|trade| trade.bars_held as f64)
                    .sum::<f64>()
                    / trade_count as f64,
            )
        },
        drawdown_percent: quantized(result.metrics.max_drawdown_percent),
        win_rate_percent: quantized(result.metrics.win_rate),
        long_short_skew: if trade_count == 0 {
            0.0
        } else {
            quantized((long_count as f64 - short_count as f64) / trade_count as f64)
        },
    }
}

pub(crate) fn niche_key(value: &BehaviorDescriptor) -> NicheKey {
    NicheKey {
        entry_conditions: value.entry_conditions,
        // Descriptor frequency is trades / equity points * 1000. Equity is M1-
        // dense, so values cluster near ~0.05–0.15 under one-entry-per-day.
        // Legacy 5/20 thresholds put every H1 swing strategy in "low".
        trade_frequency: three_level(value.trades_per_1000_bars, 0.05, 0.12),
        hold_time: three_level(value.average_bars_held, 4.0, 24.0),
        drawdown: three_level(value.drawdown_percent, 5.0, 15.0),
        win_rate: three_level(value.win_rate_percent, 35.0, 55.0),
        long_short_skew: if value.long_short_skew < -0.25 {
            LongShortSkewBucket::ShortHeavy
        } else if value.long_short_skew > 0.25 {
            LongShortSkewBucket::LongHeavy
        } else {
            LongShortSkewBucket::Balanced
        },
    }
}

/// Sorted unique indicator operators on the long (and short) entry tree.
/// Example: `ema+minus_di+plus_di`. Used to stop one grammar family dominating.
pub fn entry_family_key(strategy: &StrategyIr) -> String {
    let mut ops = std::collections::BTreeSet::new();
    if let Some(expr) = strategy.entry.long.as_ref() {
        collect_indicator_ops(expr, &mut ops);
    }
    if let Some(expr) = strategy.entry.short.as_ref() {
        collect_indicator_ops(expr, &mut ops);
    }
    if ops.is_empty() {
        "empty".into()
    } else {
        ops.into_iter().collect::<Vec<_>>().join("+")
    }
}

fn collect_indicator_ops(expr: &quantforge_ir::BoolExpr, out: &mut std::collections::BTreeSet<String>) {
    use quantforge_ir::BoolExpr;
    match expr {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            collect_numeric_ops(left, out);
            collect_numeric_ops(right, out);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            collect_numeric_ops(value, out);
            collect_numeric_ops(lower, out);
            collect_numeric_ops(upper, out);
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                collect_indicator_ops(child, out);
            }
        }
        BoolExpr::Not { child } => collect_indicator_ops(child, out),
    }
}

fn collect_numeric_ops(expr: &quantforge_ir::NumericExpr, out: &mut std::collections::BTreeSet<String>) {
    use quantforge_ir::NumericExpr;
    if let NumericExpr::Indicator { value } = expr {
        out.insert(indicator_operator_name(value));
    }
}

fn indicator_operator_name(expr: &quantforge_ir::IndicatorExpr) -> String {
    match serde_json::to_value(expr) {
        Ok(serde_json::Value::Object(fields)) => fields
            .get("operator")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_owned(),
        _ => "unknown".into(),
    }
}

fn three_level(value: f64, first: f64, second: f64) -> ThreeLevelBucket {
    if value < first {
        ThreeLevelBucket::Low
    } else if value < second {
        ThreeLevelBucket::Medium
    } else {
        ThreeLevelBucket::High
    }
}

/// Log-scaled, uncapped sample-depth weight used in scout ranking.
pub(crate) fn trade_count_evidence(trade_count: usize) -> f64 {
    (trade_count as f64 + 1.0).ln() * 3.0
}

fn evidence(
    strategy: &StrategyIr,
    result: &ScoutResult,
    mode: ScoutFitnessMode,
) -> EvidenceComponents {
    match mode {
        ScoutFitnessMode::RawIs => evidence_raw_is(strategy, result),
        ScoutFitnessMode::StableFold => evidence_stable_fold(strategy, result),
    }
}

/// SQX-style scout fitness: pooled IS expectancy/return/PF dominate; fold
/// stability is only a light tie-break.
fn evidence_raw_is(strategy: &StrategyIr, result: &ScoutResult) -> EvidenceComponents {
    let pooled_r = if result.metrics.expectancy_r.is_finite() && result.metrics.expectancy_r != 0.0
    {
        result.metrics.expectancy_r
    } else {
        crate::fold_r::calendar_year_fold_r(&result.trades).pooled_r
    };
    let expectancy_component = (pooled_r / 0.20).tanh() * 30.0;
    let return_pct_component = (result.metrics.return_percent / 20.0).tanh() * 15.0;
    let net_profit_component =
        (result.metrics.net_profit / (result.metrics.initial_balance * 0.20)).tanh() * 10.0;
    let recovery_component = (recovery_factor(&result.metrics) / 5.0).tanh() * 12.0;
    let sharpe_component = (result.metrics.sharpe_ratio.unwrap_or(0.0) / 3.0).tanh() * 8.0;
    let fold = crate::fold_r::calendar_year_fold_r(&result.trades);
    let stability_penalty = if fold.usable {
        (fold.fold_spread / 0.25).tanh() * 2.0
            + ((fold.max_year_share - 0.55).max(0.0) / 0.30).tanh() * 2.0
    } else {
        0.0
    };
    let return_component = expectancy_component
        + return_pct_component
        + net_profit_component
        + recovery_component
        + sharpe_component
        - stability_penalty;
    let effective_profit_factor = result.metrics.profit_factor.unwrap_or({
        if result.metrics.net_profit > 0.0 {
            10.0
        } else {
            0.0
        }
    });
    let profit_factor_component = if effective_profit_factor > 0.0 {
        ((effective_profit_factor - 1.0) / 0.75).tanh() * 22.0
    } else {
        -20.0
    };
    let trade_count_bonus = trade_count_evidence(result.metrics.trade_count);
    let drawdown_penalty = (result.metrics.max_drawdown_percent / 20.0).tanh() * 18.0;
    let complexity_penalty = strategy.complexity().score as f64 * 0.35;
    let total = return_component + profit_factor_component + trade_count_bonus
        - drawdown_penalty
        - complexity_penalty;
    EvidenceComponents {
        return_component: quantized(return_component),
        profit_factor_component: quantized(profit_factor_component),
        trade_count_bonus: quantized(trade_count_bonus),
        drawdown_penalty: quantized(drawdown_penalty),
        complexity_penalty: quantized(complexity_penalty),
        total: quantized(total),
    }
}

fn evidence_stable_fold(strategy: &StrategyIr, result: &ScoutResult) -> EvidenceComponents {
    // Rank the R that showed up across Development years, not pooled IS dollars.
    // A 0.4R IS gift concentrated in one year must lose to a 0.10R that repeats.
    let fold = crate::fold_r::calendar_year_fold_r(&result.trades);
    let expectancy_r = fold.rank_r();
    let expectancy_component = (expectancy_r / 0.25).tanh() * 20.0;
    let stability_penalty = if fold.usable {
        (fold.fold_spread / 0.15).tanh() * 8.0
            + if fold.has_negative_fold { 6.0 } else { 0.0 }
            + ((fold.max_year_share - 0.40).max(0.0) / 0.20).tanh() * 6.0
    } else {
        0.0
    };
    let recovery_component = (recovery_factor(&result.metrics) / 5.0).tanh() * 15.0;
    let sharpe_component = (result.metrics.sharpe_ratio.unwrap_or(0.0) / 3.0).tanh() * 10.0;
    let return_component =
        expectancy_component + recovery_component + sharpe_component - stability_penalty;
    let effective_profit_factor = result.metrics.profit_factor.unwrap_or({
        if result.metrics.net_profit > 0.0 {
            10.0
        } else {
            0.0
        }
    });
    let profit_factor_component = if effective_profit_factor > 0.0 {
        ((effective_profit_factor - 1.0) / 0.75).tanh() * 20.0
    } else {
        -20.0
    };
    // Uncapped log confidence: thin books (~100 trades) cannot rank like deep
    // ones with the same fold-R. ~100→14 pts, ~300→17, ~1000→21 (grows forever).
    let trade_count_bonus = trade_count_evidence(result.metrics.trade_count);
    let drawdown_penalty = (result.metrics.max_drawdown_percent / 20.0).tanh() * 20.0;
    let complexity_penalty = strategy.complexity().score as f64 * 0.40;
    let total = return_component + profit_factor_component + trade_count_bonus
        - drawdown_penalty
        - complexity_penalty;
    EvidenceComponents {
        return_component: quantized(return_component),
        profit_factor_component: quantized(profit_factor_component),
        trade_count_bonus: quantized(trade_count_bonus),
        drawdown_penalty: quantized(drawdown_penalty),
        complexity_penalty: quantized(complexity_penalty),
        total: quantized(total),
    }
}

fn equity_signature(result: &ScoutResult, target_points: usize) -> Vec<f64> {
    if result.equity.is_empty() {
        return Vec::new();
    }
    let mut previous = result.metrics.initial_balance;
    let deltas: Vec<f64> = result
        .equity
        .iter()
        .map(|point| {
            let delta = point.equity - previous;
            previous = point.equity;
            delta
        })
        .collect();
    let chunk_size = deltas.len().div_ceil(target_points).max(1);
    deltas
        .chunks(chunk_size)
        .take(target_points)
        .map(|chunk| quantized(chunk.iter().sum::<f64>() / chunk.len() as f64))
        .collect()
}

fn correlation(left: &[f64], right: &[f64]) -> f64 {
    let length = left.len().min(right.len());
    if length < 2 {
        return 0.0;
    }
    let left = &left[..length];
    let right = &right[..length];
    let left_mean = left.iter().sum::<f64>() / length as f64;
    let right_mean = right.iter().sum::<f64>() / length as f64;
    let mut covariance = 0.0;
    let mut left_variance = 0.0;
    let mut right_variance = 0.0;
    for (left, right) in left.iter().zip(right) {
        let left_delta = left - left_mean;
        let right_delta = right - right_mean;
        covariance += left_delta * right_delta;
        left_variance += left_delta * left_delta;
        right_variance += right_delta * right_delta;
    }
    let denominator = (left_variance * right_variance).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        (covariance / denominator).clamp(-1.0, 1.0).max(0.0)
    }
}

fn better_than(
    evidence: &EvidenceComponents,
    novelty: f64,
    complexity: usize,
    trade_count: usize,
    champion: &Elite,
) -> bool {
    evidence
        .total
        .total_cmp(&champion.evidence.total)
        .then_with(|| novelty.total_cmp(&champion.novelty))
        .then_with(|| champion.complexity.cmp(&complexity))
        .then_with(|| trade_count.cmp(&champion.metrics.trade_count))
        .is_gt()
}

pub(crate) fn refresh_fingerprint_coverage_map(
    entries: &[Elite],
    coverage_map: &mut std::collections::BTreeMap<String, quantforge_core::ContentHash>,
) {
    *coverage_map = entries
        .iter()
        .map(|elite| {
            (
                elite.structural_fingerprint.to_string(),
                elite.structural_fingerprint.clone(),
            )
        })
        .collect();
}

pub fn niche_label(niche: &NicheKey) -> String {
    format!(
        "entry{}/{:?}/{:?}/{:?}/{:?}/{:?}",
        niche.entry_conditions,
        niche.trade_frequency,
        niche.hold_time,
        niche.drawdown,
        niche.win_rate,
        niche.long_short_skew
    )
    .to_ascii_lowercase()
}

fn quantized(value: f64) -> f64 {
    quantize(value, FloatPolicy::default().score_quantum).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Databank, DiscoverTelemetry, GateConfig, ScoutFitnessMode};
    use crate::{DATABANK_SCHEMA_VERSION, GRAMMAR_VERSION, generate_seed};
    use quantforge_core::ContentHash;
    use quantforge_eval::{BacktestMetrics, EquityPoint, ScoutConfig, ScoutResult, ScoutTelemetry};
    use std::collections::BTreeMap;

    fn bank(correlation_threshold: f64) -> Databank {
        Databank {
            schema_version: DATABANK_SCHEMA_VERSION,
            grammar_version: GRAMMAR_VERSION.into(),
            data_hash: ContentHash::sha256("data"),
            execution_data_hash: ContentHash::sha256("m1-data"),
            broker_spec_hash: ContentHash::sha256("broker"),
            config: DiscoverConfig {
                correlation_threshold,
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
                scout: ScoutConfig::default(),
                ..DiscoverConfig::default()
            },
            completed_generations: 0,
            evaluation_count: 0,
            accepted_pool: Vec::new(),
            accepted_coverage_map: BTreeMap::new(),
            specialist_pool: Vec::new(),
            specialist_coverage_map: BTreeMap::new(),
            holding: Vec::new(),
            holding_coverage_map: BTreeMap::new(),
            elites: Vec::new(),
            coverage_map: BTreeMap::new(),
            telemetry: DiscoverTelemetry::default(),
        }
    }

    fn profitable_result() -> ScoutResult {
        ScoutResult {
            trades: Vec::new(),
            equity: vec![
                EquityPoint {
                    timestamp_ms: 1,
                    balance: 100_000.0,
                    equity: 100_000.0,
                },
                EquityPoint {
                    timestamp_ms: 2,
                    balance: 101_000.0,
                    equity: 101_000.0,
                },
                EquityPoint {
                    timestamp_ms: 3,
                    balance: 102_000.0,
                    equity: 102_000.0,
                },
            ],
            metrics: BacktestMetrics {
                initial_balance: 100_000.0,
                ending_balance: 102_000.0,
                net_profit: 2_000.0,
                return_percent: 2.0,
                trade_count: 0,
                winning_trades: 0,
                losing_trades: 0,
                win_rate: 0.0,
                profit_factor: None,
                max_drawdown: 0.0,
                max_drawdown_percent: 0.0,
                sharpe_ratio: None,
                expectancy: 0.0,
                expectancy_r: 0.0,
                median_r: 0.0,
            },
            telemetry: ScoutTelemetry::default(),
        }
    }

    #[test]
    fn raw_is_fitness_prefers_higher_pooled_expectancy() {
        let strategy = generate_seed(7, 0);
        let mut high = profitable_result();
        high.metrics.trade_count = 220;
        high.metrics.expectancy_r = 0.18;
        high.metrics.return_percent = 12.0;
        high.metrics.net_profit = 12_000.0;
        high.metrics.profit_factor = Some(1.4);
        let mut low = profitable_result();
        low.metrics.trade_count = 220;
        low.metrics.expectancy_r = 0.06;
        low.metrics.return_percent = 4.0;
        low.metrics.net_profit = 4_000.0;
        low.metrics.profit_factor = Some(1.15);
        let high_score = super::evidence(&strategy, &high, ScoutFitnessMode::RawIs);
        let low_score = super::evidence(&strategy, &low, ScoutFitnessMode::RawIs);
        assert!(high_score.total > low_score.total);
    }

    #[test]
    fn trade_count_evidence_grows_without_cap() {
        let at_103 = super::trade_count_evidence(103);
        let at_350 = super::trade_count_evidence(350);
        let at_1000 = super::trade_count_evidence(1_000);
        assert!(at_350 > at_103);
        assert!(at_1000 > at_350);
        // Old capped formula topped out near 5 pts; 1000 vs 103 must beat that gap.
        assert!(at_1000 - at_103 > 5.0);
    }

    #[test]
    fn correlation_distinguishes_same_inverse_and_flat_paths() {
        assert!((correlation(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]) - 1.0).abs() < 1e-12);
        assert_eq!(correlation(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]), 0.0);
        assert_eq!(correlation(&[1.0, 1.0], &[2.0, 2.0]), 0.0);
    }

    #[test]
    fn default_grid_includes_named_and_universal_grammar_niches() {
        assert_eq!(crate::model::FamilyStyle::ALL.len() * 3usize.pow(5), 2673);
    }

    #[test]
    fn recovery_factor_gate_rejects_weak_efficiency() {
        let mut result = profitable_result();
        result.metrics.max_drawdown_percent = 2.0;
        result.metrics.max_drawdown = 2_000.0;
        let mut config = bank(0.88).config;
        config.gates.minimum_recovery_factor = 1.01;
        assert!(!passes_gates(&result, &config));
        config.gates.minimum_recovery_factor = 1.0;
        assert!(passes_gates(&result, &config));
    }

    #[test]
    fn duplicate_structures_and_correlated_empty_niches_are_rejected() {
        let mut bank = bank(0.88);
        let trend = generate_seed(42, 0);
        let first = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: trend.clone(),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(first, DepositDecision::AcceptedToPot);

        let clone = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: trend,
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(clone, DepositDecision::RejectedClone);

        let correlated_other_family = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(42, 1),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(correlated_other_family, DepositDecision::RejectedCorrelated);
    }

    #[test]
    fn pot_accepts_distinct_strategies_in_the_same_niche() {
        let mut bank = bank(0.99);
        let first = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(7, 0),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(first, DepositDecision::AcceptedToPot);

        // Uncorrelated equity, same niche buckets (coarse descriptors), different IR.
        let mut second_result = profitable_result();
        for (index, point) in second_result.equity.iter_mut().enumerate() {
            point.equity = 100_000.0 + (index as f64).cos() * 250.0 + index as f64 * 3.0;
        }
        second_result.metrics.return_percent = 1.5;
        let second = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(7, 3),
                result: second_result,
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(second, DepositDecision::AcceptedToPot);
        assert_eq!(bank.accepted_pool.len(), 2);
        assert_eq!(bank.accepted_coverage_map.len(), 2);
    }

    #[test]
    fn pot_niche_cap_is_per_island() {
        let mut bank = bank(0.99);
        bank.config.max_elites_per_niche = 1;
        bank.config.max_per_entry_family = 64;
        let first = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(7, 0),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(first, DepositDecision::AcceptedToPot);

        let mut other_island = profitable_result();
        for (index, point) in other_island.equity.iter_mut().enumerate() {
            point.equity = 100_000.0 + (index as f64).sin() * 180.0 + index as f64 * 4.0;
        }
        other_island.metrics.return_percent = 1.2;
        let accepted = deposit_to_accepted_pool(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(7, 3),
                result: other_island,
                generation: 0,
                island_id: 1,
                is_expectancy: 1.2,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(accepted, DepositDecision::AcceptedToPot);
        assert_eq!(bank.accepted_pool.len(), 2);
        let islands: Vec<u16> = bank.accepted_pool.iter().map(|elite| elite.island_id).collect();
        assert!(islands.contains(&0) && islands.contains(&1));
    }

    #[test]
    fn databank_stacks_distinct_strategies_in_the_same_niche() {
        let mut bank = bank(0.99);
        let first = deposit_to_databank(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(11, 0),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(first, DepositDecision::AcceptedToDatabank);

        let mut second_result = profitable_result();
        for (index, point) in second_result.equity.iter_mut().enumerate() {
            point.equity = 102_000.0 - index as f64 * 1_000.0;
        }
        second_result.metrics.return_percent = 1.8;
        let second = deposit_to_databank(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(11, 4),
                result: second_result,
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(second, DepositDecision::AcceptedToDatabank);
        assert_eq!(bank.elites.len(), 2);
        assert_eq!(bank.coverage_map.len(), 2);
        assert_ne!(
            bank.elites[0].structural_fingerprint,
            bank.elites[1].structural_fingerprint
        );
    }

    #[test]
    fn holding_stacks_uncorrelated_names_in_the_same_entry_family() {
        let mut bank = bank(0.99);
        bank.config.max_per_entry_family = 1;
        bank.config.max_promoted_per_niche = 1;
        let family = entry_family_key(&generate_seed(7, 0));
        let first = deposit_to_holding(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(7, 0),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(first, DepositDecision::AcceptedToHolding);

        let mut second_result = profitable_result();
        for (index, point) in second_result.equity.iter_mut().enumerate() {
            point.equity = 100_000.0 + (index as f64) * 17.0 + (index as f64).cos() * 40.0;
        }
        second_result.metrics.return_percent = 0.5;
        second_result.metrics.net_profit = 500.0;
        let same_family = (0..2048)
            .map(|offset| generate_seed(7, offset))
            .find(|strategy| {
                entry_family_key(strategy) == family
                    && strategy.structural_fingerprint(quantforge_core::FloatPolicy::default()).ok()
                        != generate_seed(7, 0)
                            .structural_fingerprint(quantforge_core::FloatPolicy::default())
                            .ok()
            })
            .expect("another seed in the same entry family");
        let second = deposit_to_holding(
            &mut bank,
            CandidateEvaluation {
                strategy: same_family,
                result: second_result,
                generation: 1,
                island_id: 0,
                is_expectancy: 0.5,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(second, DepositDecision::AcceptedToHolding);
        assert_eq!(bank.holding.len(), 2);
    }

    #[test]
    fn holding_accepts_correlated_distinct_fingerprints() {
        let mut bank = bank(0.50);
        let first = deposit_to_holding(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(7, 0),
                result: profitable_result(),
                generation: 0,
                island_id: 0,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(first, DepositDecision::AcceptedToHolding);
        let second = deposit_to_holding(
            &mut bank,
            CandidateEvaluation {
                strategy: generate_seed(11, 4),
                result: profitable_result(),
                generation: 1,
                island_id: 1,
                is_expectancy: 1.0,
                oos1_expectancy: None,
                oos1_expectancy_ratio: None,
                observed_trade_sharpe: None,
                expected_max_lucky_sharpe: None,
                deflated_trade_sharpe: None,
                multi_symbol_results: Vec::new(),
                gate_results: Vec::new(),
                robustness: None,
            },
        )
        .unwrap();
        assert_eq!(second, DepositDecision::AcceptedToHolding);
        assert_eq!(bank.holding.len(), 2);
    }
}
