//! SQX-style strategy Negater — flip long/short entry (and exit) sides.
//!
//! Used as a robustness / cross-check transform: a strategy that only works one
//! way and collapses when negated is often curve-fit. This does not mutate IR
//! identity fields beyond side-specific signal trees.

use quantforge_ir::{EntrySignals, Side, StrategyIr};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NegateMode {
    /// Swap long ↔ short entry/exit trees and invert `side`.
    FlipSides,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NegateReport {
    pub protocol_version: String,
    pub mode: NegateMode,
    pub original_side: Side,
    pub negated_side: Side,
    pub strategy: StrategyIr,
}

pub const NEGATE_PROTOCOL_VERSION: &str = "negate-v1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NegateError {
    #[error("cannot negate Both-side strategy with FlipSides (ambiguous)")]
    AmbiguousBoth,
}

/// Return a negated clone of `strategy` (original unchanged).
pub fn negate_strategy(
    strategy: &StrategyIr,
    mode: NegateMode,
) -> Result<NegateReport, NegateError> {
    match mode {
        NegateMode::FlipSides => {
            if strategy.side == Side::Both {
                return Err(NegateError::AmbiguousBoth);
            }
            let mut out = strategy.clone();
            let entry = EntrySignals {
                long: strategy.entry.short.clone(),
                short: strategy.entry.long.clone(),
                order: strategy.entry.order.clone(),
            };
            out.entry = entry;
            std::mem::swap(&mut out.exit_long, &mut out.exit_short);
            out.side = match strategy.side {
                Side::LongOnly => Side::ShortOnly,
                Side::ShortOnly => Side::LongOnly,
                Side::Both => Side::Both,
            };
            out.id = format!("{}__negated", strategy.id);
            Ok(NegateReport {
                protocol_version: NEGATE_PROTOCOL_VERSION.into(),
                mode,
                original_side: strategy.side,
                negated_side: out.side,
                strategy: out,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_core::STRATEGY_IR_VERSION;
    use quantforge_ir::{
        BoolExpr, ComparisonOp, EntrySignals, NumericExpr, PriceField, ProtectiveStops, RiskPolicy,
        StopLossPolicy, StrategyMeta, TakeProfitPolicy,
    };

    fn long_only() -> StrategyIr {
        StrategyIr {
            id: "n".into(),
            version: STRATEGY_IR_VERSION,
            entry: EntrySignals {
                long: Some(BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Price {
                        field: PriceField::Close,
                        shift: 1,
                    },
                    right: NumericExpr::Constant { value: 0.0 },
                }),
                short: None,
                order: Default::default(),
            },
            exit: None,
            exit_long: Some(BoolExpr::Compare {
                comparison: ComparisonOp::LessThan,
                left: NumericExpr::Price {
                    field: PriceField::Close,
                    shift: 1,
                },
                right: NumericExpr::Constant { value: 0.0 },
            }),
            exit_short: None,
            filters: vec![],
            side: Side::LongOnly,
            risk: RiskPolicy::FixedCurrency { amount: 10.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::FixedPoints { points: 2.0 },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
            },
            manage: Default::default(),
            meta: StrategyMeta {
                thesis_hint: "t".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    #[test]
    fn flip_sides_swaps_trees_and_side() {
        let report = negate_strategy(&long_only(), NegateMode::FlipSides).unwrap();
        assert_eq!(report.negated_side, Side::ShortOnly);
        assert!(report.strategy.entry.long.is_none());
        assert!(report.strategy.entry.short.is_some());
        assert!(report.strategy.exit_short.is_some());
        assert!(report.strategy.exit_long.is_none());
    }
}
