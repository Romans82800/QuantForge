use crate::model::{
    ExportBundle, ExportError, ExportEvidenceCard, ExportStyle, ExportSupportFile, Mql5ExportConfig,
};
use crate::{EXPORT_SCHEMA_VERSION, EXPORT_TARGET};
use include_dir::{include_dir, Dir};
use quantforge_broker::{SymbolSpecification, TradeMode};
use quantforge_core::{ContentHash, FloatPolicy};
use quantforge_ir::{
    BoolExpr, ComparisonOp, ContextValue, EntryDistancePolicy, EntryOrderPolicy, IndicatorExpr,
    IrLimits, NumericExpr, PriceField, RiskPolicy, Side, StopLossPolicy, StrategyIr,
    TakeProfitPolicy, TrailingPolicy,
};

const TEMPLATE: &str = include_str!("template.mq5");
const SQX_TEMPLATE: &str = include_str!("sqx_template.mq5");
const SQX_RUNTIME: &str = include_str!("sqx_runtime.mqh");
/// MACD, Bollinger, Ichimoku, QQE, VWAP and CCI helpers. Both templates share
/// one copy so the two export styles cannot drift apart.
const EXTENDED_INDICATORS: &str = include_str!("extended_indicators.mqh");
/// `iMACD` needs a signal period even when only the main line is read; the main
/// buffer does not depend on it.
const DEFAULT_MACD_SIGNAL_PERIOD: u16 = 9;
static SQX_INDICATORS: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../mql5/QuantForge/Indicators");

fn sqx_runtime_inline() -> String {
    SQX_RUNTIME
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("#ifndef __QUANTFORGE_SQX_RUNTIME_MQH__")
                || trimmed.starts_with("#define __QUANTFORGE_SQX_RUNTIME_MQH__")
                || trimmed == "#endif"
                || trimmed == "#include <Trade/Trade.mqh>")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn generate_bundle(
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &Mql5ExportConfig,
) -> Result<ExportBundle, ExportError> {
    config.validate()?;
    broker.validate()?;
    strategy.validate_export_safe(IrLimits::default())?;
    validate_trade_mode(strategy, broker)?;

    let strategy = strategy.canonicalized(FloatPolicy::default())?;
    let strategy_fingerprint = strategy.structural_fingerprint(FloatPolicy::default())?;
    let broker_spec_hash = broker.content_hash()?;
    let fingerprint_short = &strategy_fingerprint.as_str()[..12];
    let parity_prefix = format!("QuantForge\\QF_{fingerprint_short}");
    let template = match config.export_style {
        ExportStyle::Quantforge => TEMPLATE,
        ExportStyle::Sqx => SQX_TEMPLATE,
    };

    let mut source = template.to_owned();
    if matches!(config.export_style, ExportStyle::Sqx) {
        source = source.replace("@@SQX_RUNTIME@@", &sqx_runtime_inline());
    }
    source = source.replace("@@QF_EXTENDED_INDICATORS@@", EXTENDED_INDICATORS);
    let (filling_modes, filling_mode_count) = filling_modes_mql(broker);
    for (placeholder, value) in [
        ("@@EXPERT_NAME@@", config.expert_name.clone()),
        ("@@ALLOW_LIVE@@", "false".into()),
        ("@@MAGIC@@", config.magic.to_string()),
        ("@@DEVIATION@@", config.deviation_points.to_string()),
        (
            "@@MAX_SPREAD@@",
            mql_double(config.max_spread_points.unwrap_or(0.0)),
        ),
        (
            "@@SLIPPAGE@@",
            mql_double(config.estimated_slippage_points_per_side),
        ),
        (
            "@@COMMISSION@@",
            mql_double(config.commission_per_lot_round_turn),
        ),
        (
            "@@ENTRY_WINDOW_START@@",
            config.entry_window_start_hour.to_string(),
        ),
        (
            "@@ENTRY_WINDOW_END@@",
            config.entry_window_end_hour.to_string(),
        ),
        ("@@PARITY_PREFIX@@", mql_string(&parity_prefix)),
        ("@@STRATEGY_FINGERPRINT@@", strategy_fingerprint.to_string()),
        ("@@BROKER_FINGERPRINT@@", broker_spec_hash.to_string()),
        (
            "@@LONG_SIGNAL@@",
            strategy
                .entry
                .long
                .as_ref()
                .map(|value| bool_expr(value, "extra_shift"))
                .unwrap_or_else(|| "false".into()),
        ),
        (
            "@@SHORT_SIGNAL@@",
            strategy
                .entry
                .short
                .as_ref()
                .map(|value| bool_expr(value, "extra_shift"))
                .unwrap_or_else(|| "false".into()),
        ),
        (
            "@@LONG_EXIT_SIGNAL@@",
            strategy
                .long_exit()
                .map(|value| bool_expr(value, "extra_shift"))
                .unwrap_or_else(|| "false".into()),
        ),
        (
            "@@SHORT_EXIT_SIGNAL@@",
            strategy
                .short_exit()
                .map(|value| bool_expr(value, "extra_shift"))
                .unwrap_or_else(|| "false".into()),
        ),
        ("@@FILTERS@@", filters(&strategy.filters)),
        ("@@STOP_DISTANCE@@", stop_distance(&strategy)),
        ("@@TARGET_DISTANCE@@", target_distance(&strategy)),
        ("@@RISK_BUDGET@@", risk_budget(&strategy)),
        ("@@ENTRY_ORDER_KIND@@", entry_order_kind(&strategy).to_string()),
        ("@@ENTRY_DISTANCE@@", entry_distance(&strategy)),
        ("@@ENTRY_LIMIT_OFFSET@@", entry_limit_offset(&strategy)),
        ("@@ENTRY_EXPIRY@@", entry_expiry(&strategy).to_string()),
        (
            "@@BREAK_EVEN_R@@",
            mql_double(strategy.manage.break_even_at_r.unwrap_or(0.0)),
        ),
        ("@@TRAILING_KIND@@", trailing_kind(&strategy).to_string()),
        ("@@TRAILING_ACTIVATE_R@@", trailing_activate_r(&strategy)),
        ("@@TRAILING_DISTANCE@@", trailing_distance(&strategy)),
        (
            "@@EOD_HOUR@@",
            strategy.manage.end_of_day_hour.to_string(),
        ),
        (
            "@@FLATTEN_EOD@@",
            if strategy.manage.flatten_end_of_day {
                "true".into()
            } else {
                "false".into()
            },
        ),
        (
            "@@MAX_ONE_ENTRY_PER_DAY@@",
            if strategy.manage.max_one_entry_per_day {
                "true".into()
            } else {
                "false".into()
            },
        ),
        (
            "@@PARTIAL_COUNT@@",
            strategy.manage.partial_exits.len().to_string(),
        ),
        ("@@PARTIAL_AT_R@@", partial_at_r(&strategy)),
        ("@@PARTIAL_FRACTION@@", partial_fraction(&strategy)),
        (
            "@@TIME_STOP@@",
            strategy.manage.time_stop_bars.unwrap_or(0).to_string(),
        ),
        ("@@FINGERPRINT_SHORT@@", fingerprint_short.into()),
        ("@@FILLING_MODES@@", filling_modes),
        ("@@FILLING_MODE_COUNT@@", filling_mode_count.to_string()),
        ("@@SYMBOL@@", mql_string(&broker.symbol)),
        ("@@BROKER_TIMEZONE@@", broker.timezone.clone()),
    ] {
        source = source.replace(placeholder, &value);
    }
    if source.contains("@@") {
        return Err(ExportError::InvalidConfig(
            "internal MQL5 template contains an unresolved placeholder".into(),
        ));
    }

    let source_hash = ContentHash::sha256(source.as_bytes());
    let set_file = set_file(config, &parity_prefix);
    let tester_ini = tester_ini(config, broker, &parity_prefix);
    let evidence = ExportEvidenceCard {
        schema_version: EXPORT_SCHEMA_VERSION,
        target: EXPORT_TARGET.into(),
        strategy_fingerprint,
        broker_spec_hash,
        source_hash,
        strategy_ir_version: strategy.version,
        expert_name: config.expert_name.clone(),
        symbol: broker.symbol.clone(),
        timeframe: config.timeframe.clone(),
        live_trading_default: false,
        mandatory_stop_loss: true,
        mandatory_take_profit: true,
        parity_deals_file: format!("{parity_prefix}_deals.csv"),
        parity_equity_file: format!("{parity_prefix}_equity.csv"),
        parity_metadata_file: format!("{parity_prefix}_metadata.csv"),
        export_style: config.export_style,
        config: config.clone(),
    };
    let support_files = match config.export_style {
        ExportStyle::Sqx => sqx_support_files(),
        ExportStyle::Quantforge => Vec::new(),
    };
    Ok(ExportBundle {
        source,
        set_file,
        tester_ini,
        evidence,
        support_files,
    })
}

fn sqx_support_files() -> Vec<ExportSupportFile> {
    let mut files = vec![ExportSupportFile {
        relative_path: "sqx_runtime.mqh".into(),
        contents: SQX_RUNTIME.into(),
    }];
    for entry in SQX_INDICATORS.files() {
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "mq5")
        {
            let name = entry
                .path()
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(contents) = entry.contents_utf8() {
                files.push(ExportSupportFile {
                    relative_path: format!("Indicators/{name}"),
                    contents: contents.into(),
                });
            }
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn validate_trade_mode(
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
) -> Result<(), ExportError> {
    let incompatible = matches!(
        (broker.trade_mode, strategy.side),
        (TradeMode::Disabled | TradeMode::CloseOnly, _)
            | (TradeMode::LongOnly, Side::ShortOnly | Side::Both)
            | (TradeMode::ShortOnly, Side::LongOnly | Side::Both)
    );
    if incompatible {
        Err(ExportError::InvalidConfig(
            "strategy side is incompatible with the bound broker trade mode".into(),
        ))
    } else {
        Ok(())
    }
}

fn bool_expr(expression: &BoolExpr, shift: &str) -> String {
    match expression {
        BoolExpr::Compare {
            comparison,
            left,
            right,
        } => format!(
            "{}({},{})",
            match comparison {
                ComparisonOp::GreaterThan => "QFGreater",
                ComparisonOp::LessThan => "QFLess",
            },
            numeric_expr(left, shift),
            numeric_expr(right, shift)
        ),
        BoolExpr::CrossAbove { left, right } | BoolExpr::CrossBelow { left, right } => {
            let function = if matches!(expression, BoolExpr::CrossAbove { .. }) {
                "QFCrossAbove"
            } else {
                "QFCrossBelow"
            };
            format!(
                "{function}({},{},{},{})",
                numeric_expr(left, shift),
                numeric_expr(right, shift),
                numeric_expr(left, &format!("({shift}+1)")),
                numeric_expr(right, &format!("({shift}+1)"))
            )
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => format!(
            "QFBetween({},{},{})",
            numeric_expr(value, shift),
            numeric_expr(lower, shift),
            numeric_expr(upper, shift)
        ),
        BoolExpr::And { children } => format!(
            "({})",
            children
                .iter()
                .map(|child| bool_expr(child, shift))
                .collect::<Vec<_>>()
                .join(" && ")
        ),
        BoolExpr::Or { children } => format!(
            "({})",
            children
                .iter()
                .map(|child| bool_expr(child, shift))
                .collect::<Vec<_>>()
                .join(" || ")
        ),
        BoolExpr::Not { child } => format!("(!{})", bool_expr(child, shift)),
    }
}

fn numeric_expr(expression: &NumericExpr, extra_shift: &str) -> String {
    match expression {
        NumericExpr::Price { field, shift } => format!(
            "QFPrice({},({}+{}))",
            price_field(*field),
            shift,
            extra_shift
        ),
        NumericExpr::Indicator { value } => indicator_expr(value, extra_shift),
        NumericExpr::Context { value, shift } => format!(
            "QFContext({},({}+{}))",
            match value {
                ContextValue::SessionHour => 0,
                ContextValue::DayOfWeek => 1,
            },
            shift,
            extra_shift
        ),
        NumericExpr::Constant { value } => mql_double(*value),
    }
}

fn indicator_expr(indicator: &IndicatorExpr, extra_shift: &str) -> String {
    match *indicator {
        IndicatorExpr::Sma {
            source,
            period,
            shift,
        } => ma_expr("MODE_SMA", source, period, shift, extra_shift),
        IndicatorExpr::Ema {
            source,
            period,
            shift,
        } => ma_expr("MODE_EMA", source, period, shift, extra_shift),
        IndicatorExpr::Wma {
            source,
            period,
            shift,
        } => ma_expr("MODE_LWMA", source, period, shift, extra_shift),
        IndicatorExpr::Rsi {
            source,
            period,
            shift,
        } => format!(
            "QFRSI({},{},({}+{}))",
            applied_price(source),
            period,
            shift,
            extra_shift
        ),
        IndicatorExpr::Atr { period, shift } => {
            format!("QFATR({},({}+{}))", period, shift, extra_shift)
        }
        IndicatorExpr::Adx { period, shift } => {
            format!("QFADX({},({}+{}))", period, shift, extra_shift)
        }
        IndicatorExpr::PlusDi { period, shift } => {
            format!("QFPlusDI({},({}+{}))", period, shift, extra_shift)
        }
        IndicatorExpr::MinusDi { period, shift } => {
            format!("QFMinusDI({},({}+{}))", period, shift, extra_shift)
        }
        IndicatorExpr::DonchianHigh { period, shift } => format!(
            "QFExtreme(MODE_HIGH,1,{},({}+{}),true)",
            period, shift, extra_shift
        ),
        IndicatorExpr::DonchianLow { period, shift } => format!(
            "QFExtreme(MODE_LOW,2,{},({}+{}),false)",
            period, shift, extra_shift
        ),
        IndicatorExpr::Highest {
            source,
            period,
            shift,
        } => extreme_expr(source, period, shift, extra_shift, true),
        IndicatorExpr::Lowest {
            source,
            period,
            shift,
        } => extreme_expr(source, period, shift, extra_shift, false),
        IndicatorExpr::StandardDeviation {
            source,
            period,
            shift,
        } => format!(
            "QFStdDev({},{},({}+{}))",
            applied_price(source),
            period,
            shift,
            extra_shift
        ),
        IndicatorExpr::ZScore {
            source,
            period,
            shift,
        } => format!(
            "QFZScore({},{},{},({}+{}))",
            price_field(source),
            applied_price(source),
            period,
            shift,
            extra_shift
        ),
        IndicatorExpr::PercentileInRange {
            source,
            period,
            shift,
        } => format!(
            "QFPercentile({},{},{},({}+{}))",
            price_field(source),
            series_mode(source),
            period,
            shift,
            extra_shift
        ),
        IndicatorExpr::RateOfChange {
            source,
            period,
            shift,
        } => format!(
            "QFROC({},{},({}+{}))",
            price_field(source),
            period,
            shift,
            extra_shift
        ),
        IndicatorExpr::SessionRangeHigh {
            start_hour,
            range_bars,
            shift,
        } => format!(
            "QFSessionRangeHigh({},{},({}+{}))",
            start_hour, range_bars, shift, extra_shift
        ),
        IndicatorExpr::SessionRangeLow {
            start_hour,
            range_bars,
            shift,
        } => format!(
            "QFSessionRangeLow({},{},({}+{}))",
            start_hour, range_bars, shift, extra_shift
        ),
        IndicatorExpr::BodyRangeRatio { shift } => {
            format!("QFBodyRangeRatio(({}+{}))", shift, extra_shift)
        }
        IndicatorExpr::CloseLocationInBar { shift } => {
            format!("QFCloseLocationInBar(({}+{}))", shift, extra_shift)
        }
        IndicatorExpr::AtrPercentile {
            atr_period,
            lookback,
            shift,
        } => format!(
            "QFAtrPercentile({},{},({}+{}))",
            atr_period, lookback, shift, extra_shift
        ),
        IndicatorExpr::SwingBaseZoneHigh {
            swing_left,
            swing_right,
            base_bars,
            shift,
        } => format!(
            "QFSwingBaseZoneHigh({},{},{},({}+{}))",
            swing_left, swing_right, base_bars, shift, extra_shift
        ),
        IndicatorExpr::SwingBaseZoneLow {
            swing_left,
            swing_right,
            base_bars,
            shift,
        } => format!(
            "QFSwingBaseZoneLow({},{},{},({}+{}))",
            swing_left, swing_right, base_bars, shift, extra_shift
        ),
        IndicatorExpr::LiquiditySweepScore { period, shift } => format!(
            "QFLiquiditySweepScore({},({}+{}))",
            period, shift, extra_shift
        ),
        IndicatorExpr::MacdMain {
            source,
            fast_period,
            slow_period,
            shift,
        } => format!(
            "QFMacdMain({},{},{},{},({}+{}))",
            applied_price(source),
            fast_period,
            slow_period,
            DEFAULT_MACD_SIGNAL_PERIOD,
            shift,
            extra_shift
        ),
        IndicatorExpr::MacdSignal {
            source,
            fast_period,
            slow_period,
            signal_period,
            shift,
        } => format!(
            "QFMacdSignal({},{},{},{},({}+{}))",
            applied_price(source),
            fast_period,
            slow_period,
            signal_period,
            shift,
            extra_shift
        ),
        IndicatorExpr::MacdHistogram {
            source,
            fast_period,
            slow_period,
            signal_period,
            shift,
        } => format!(
            "QFMacdHistogram({},{},{},{},({}+{}))",
            applied_price(source),
            fast_period,
            slow_period,
            signal_period,
            shift,
            extra_shift
        ),
        IndicatorExpr::BollingerMid {
            source,
            period,
            shift,
        } => format!(
            "QFBollingerMid({},{},({}+{}))",
            applied_price(source),
            period,
            shift,
            extra_shift
        ),
        IndicatorExpr::BollingerUpper {
            source,
            period,
            deviation_tenths,
            shift,
        } => format!(
            "QFBollingerUpper({},{},{},({}+{}))",
            applied_price(source),
            period,
            deviation_tenths,
            shift,
            extra_shift
        ),
        IndicatorExpr::BollingerLower {
            source,
            period,
            deviation_tenths,
            shift,
        } => format!(
            "QFBollingerLower({},{},{},({}+{}))",
            applied_price(source),
            period,
            deviation_tenths,
            shift,
            extra_shift
        ),
        IndicatorExpr::BollingerBandwidth {
            source,
            period,
            deviation_tenths,
            shift,
        } => format!(
            "QFBollingerBandwidth({},{},{},({}+{}))",
            applied_price(source),
            period,
            deviation_tenths,
            shift,
            extra_shift
        ),
        IndicatorExpr::IchimokuTenkan { period, shift } => format!(
            "QFIchimokuTenkan({},({}+{}))",
            period, shift, extra_shift
        ),
        IndicatorExpr::IchimokuKijun { period, shift } => {
            format!("QFIchimokuKijun({},({}+{}))", period, shift, extra_shift)
        }
        IndicatorExpr::IchimokuSenkouA {
            tenkan_period,
            kijun_period,
            shift,
        } => format!(
            "QFIchimokuSenkouA({},{},({}+{}))",
            tenkan_period, kijun_period, shift, extra_shift
        ),
        IndicatorExpr::IchimokuSenkouB {
            period,
            kijun_period,
            shift,
        } => format!(
            "QFIchimokuSenkouB({},{},({}+{}))",
            period, kijun_period, shift, extra_shift
        ),
        IndicatorExpr::QqeLine {
            rsi_period,
            smoothing_period,
            shift,
        } => format!(
            "QFQqeLine({},{},({}+{}))",
            rsi_period, smoothing_period, shift, extra_shift
        ),
        IndicatorExpr::QqeTrail {
            rsi_period,
            smoothing_period,
            factor_tenths,
            shift,
        } => format!(
            "QFQqeTrail({},{},{},({}+{}))",
            rsi_period, smoothing_period, factor_tenths, shift, extra_shift
        ),
        IndicatorExpr::Vwap { period, shift } => {
            format!("QFVwap({},({}+{}))", period, shift, extra_shift)
        }
        IndicatorExpr::Cci { period, shift } => {
            format!("QFCci({},({}+{}))", period, shift, extra_shift)
        }
    }
}

fn ma_expr(method: &str, source: PriceField, period: u16, shift: u16, extra_shift: &str) -> String {
    format!(
        "QFMA({method},{},{},({}+{}))",
        applied_price(source),
        period,
        shift,
        extra_shift
    )
}

fn extreme_expr(
    source: PriceField,
    period: u16,
    shift: u16,
    extra_shift: &str,
    maximum: bool,
) -> String {
    format!(
        "QFExtreme({},{},{},({}+{}),{})",
        series_mode(source),
        price_field(source),
        period,
        shift,
        extra_shift,
        maximum
    )
}

fn filters(filters: &[BoolExpr]) -> String {
    if filters.is_empty() {
        "true".into()
    } else {
        filters
            .iter()
            .map(|value| bool_expr(value, "extra_shift"))
            .collect::<Vec<_>>()
            .join(" && ")
    }
}

fn stop_distance(strategy: &StrategyIr) -> String {
    match strategy.stops.stop_loss {
        StopLossPolicy::FixedPoints { points } => format!("{}*_Point", mql_double(points)),
        StopLossPolicy::AtrMultiple { period, multiplier } => {
            format!("QFATR({},1)*{}", period, mql_double(multiplier))
        }
        StopLossPolicy::RangeMultiple { period, multiplier } => {
            format!("QFAverageRange({},1)*{}", period, mql_double(multiplier))
        }
    }
}

fn target_distance(strategy: &StrategyIr) -> String {
    match strategy.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => {
            format!("stop_distance*{}", mql_double(multiple))
        }
        TakeProfitPolicy::FixedPoints { points } => format!("{}*_Point", mql_double(points)),
        TakeProfitPolicy::AtrMultiple { period, multiplier } => {
            format!("QFATR({},1)*{}", period, mql_double(multiplier))
        }
    }
}

fn risk_budget(strategy: &StrategyIr) -> String {
    match strategy.risk {
        RiskPolicy::FixedCurrency { amount } => mql_double(amount),
        RiskPolicy::PercentBalance { percent } => format!(
            "AccountInfoDouble(ACCOUNT_BALANCE)*{}/100.0",
            mql_double(percent)
        ),
    }
}

fn entry_order_kind(strategy: &StrategyIr) -> u8 {
    match strategy.entry.order {
        EntryOrderPolicy::Market => 0,
        EntryOrderPolicy::Stop { .. } => 1,
        EntryOrderPolicy::Limit { .. } => 2,
        EntryOrderPolicy::StopLimit { .. } => 3,
    }
}

/// Preferred MT5 filling enums from the bound broker profile.
/// Returns `(modes_csv, count)` — when count is 0 the EA falls back to symbol autodetection.
fn filling_modes_mql(broker: &quantforge_broker::SymbolSpecification) -> (String, usize) {
    use quantforge_broker::FillingMode;
    let modes: Vec<&'static str> = broker
        .filling_modes
        .iter()
        .filter_map(|mode| match mode {
            FillingMode::FillOrKill => Some("ORDER_FILLING_FOK"),
            FillingMode::ImmediateOrCancel => Some("ORDER_FILLING_IOC"),
            FillingMode::Return => Some("ORDER_FILLING_RETURN"),
            FillingMode::BookOrCancel => None,
        })
        .collect();
    let count = modes.len();
    // Array initializer must stay non-empty for MQL5 compile; count gates use.
    let csv = if modes.is_empty() {
        "ORDER_FILLING_FOK".into()
    } else {
        modes.join(", ")
    };
    (csv, count)
}

fn entry_distance(strategy: &StrategyIr) -> String {
    let policy = match &strategy.entry.order {
        EntryOrderPolicy::Market => return "0.0".into(),
        EntryOrderPolicy::Stop { distance, .. } | EntryOrderPolicy::Limit { distance, .. } => {
            distance
        }
        EntryOrderPolicy::StopLimit { stop_distance, .. } => stop_distance,
    };
    distance_expr(policy)
}

fn entry_limit_offset(strategy: &StrategyIr) -> String {
    match &strategy.entry.order {
        EntryOrderPolicy::StopLimit { limit_offset, .. } => distance_expr(limit_offset),
        _ => "0.0".into(),
    }
}

fn distance_expr(policy: &EntryDistancePolicy) -> String {
    match *policy {
        EntryDistancePolicy::FixedPoints { points } => format!("{}*_Point", mql_double(points)),
        EntryDistancePolicy::AtrMultiple { period, multiplier } => {
            format!("QFATR({},1)*{}", period, mql_double(multiplier))
        }
        EntryDistancePolicy::RangeMultiple { period, multiplier } => {
            format!("QFAverageRange({},1)*{}", period, mql_double(multiplier))
        }
    }
}

fn entry_expiry(strategy: &StrategyIr) -> u16 {
    match strategy.entry.order {
        EntryOrderPolicy::Market => 0,
        EntryOrderPolicy::Stop { expiry_bars, .. }
        | EntryOrderPolicy::Limit { expiry_bars, .. }
        | EntryOrderPolicy::StopLimit { expiry_bars, .. } => expiry_bars,
    }
}

fn trailing_kind(strategy: &StrategyIr) -> u8 {
    match strategy.manage.trailing {
        None => 0,
        Some(TrailingPolicy::RiskMultiple { .. }) => 1,
        Some(TrailingPolicy::AtrMultiple { .. }) => 2,
    }
}

fn trailing_activate_r(strategy: &StrategyIr) -> String {
    let value = match strategy.manage.trailing {
        None => 0.0,
        Some(TrailingPolicy::RiskMultiple { activate_at_r, .. })
        | Some(TrailingPolicy::AtrMultiple { activate_at_r, .. }) => activate_at_r,
    };
    mql_double(value)
}

fn trailing_distance(strategy: &StrategyIr) -> String {
    match strategy.manage.trailing {
        None => "0.0".into(),
        Some(TrailingPolicy::RiskMultiple { distance_r, .. }) => {
            format!("g_initial_risk*{}", mql_double(distance_r))
        }
        Some(TrailingPolicy::AtrMultiple {
            period, multiplier, ..
        }) => format!("QFATR({},1)*{}", period, mql_double(multiplier)),
    }
}

fn partial_at_r(strategy: &StrategyIr) -> String {
    if strategy.manage.partial_exits.is_empty() {
        return "return 0.0;".into();
    }
    let mut output = String::new();
    for (index, partial) in strategy.manage.partial_exits.iter().enumerate() {
        output.push_str(&format!(
            "if(index=={index}) return {};\n   ",
            mql_double(partial.at_r)
        ));
    }
    output.push_str("return 0.0;");
    output
}

fn partial_fraction(strategy: &StrategyIr) -> String {
    if strategy.manage.partial_exits.is_empty() {
        return "return 0.0;".into();
    }
    let mut output = String::new();
    for (index, partial) in strategy.manage.partial_exits.iter().enumerate() {
        output.push_str(&format!(
            "if(index=={index}) return {};\n   ",
            mql_double(partial.fraction)
        ));
    }
    output.push_str("return 0.0;");
    output
}

fn set_file(config: &Mql5ExportConfig, parity_prefix: &str) -> String {
    format!(
        "InpAllowLiveTrading=false||false||0||true||N\n\
         InpMagic={}||{}||1||{}||N\n\
         InpDeviationPoints={}||{}||1||{}||N\n\
         InpMaxSpreadPoints={}||{}||1||{}||N\n\
         InpEstimatedSlippagePointsPerSide={}||{}||0.1||{}||N\n\
         InpCommissionPerLotRoundTurn={}||{}||0.1||{}||N\n\
         InpEntryWindowStartHour={}||{}||1||{}||N\n\
         InpEntryWindowEndHour={}||{}||1||{}||N\n\
         InpParityPrefix={}\n",
        config.magic,
        config.magic,
        config.magic,
        config.deviation_points,
        config.deviation_points,
        config.deviation_points,
        mql_double(config.max_spread_points.unwrap_or(0.0)),
        mql_double(config.max_spread_points.unwrap_or(0.0)),
        mql_double(config.max_spread_points.unwrap_or(0.0)),
        mql_double(config.estimated_slippage_points_per_side),
        mql_double(config.estimated_slippage_points_per_side),
        mql_double(config.estimated_slippage_points_per_side),
        mql_double(config.commission_per_lot_round_turn),
        mql_double(config.commission_per_lot_round_turn),
        mql_double(config.commission_per_lot_round_turn),
        config.entry_window_start_hour,
        config.entry_window_start_hour,
        config.entry_window_start_hour,
        config.entry_window_end_hour,
        config.entry_window_end_hour,
        config.entry_window_end_hour,
        parity_prefix
    )
}

fn tester_ini(
    config: &Mql5ExportConfig,
    broker: &SymbolSpecification,
    parity_prefix: &str,
) -> String {
    let mut output = format!(
        "[Tester]\nExpert={}\\{}.ex5\nSymbol={}\nPeriod={}\nOptimization=0\nModel={}\nDeposit={}\nCurrency={}\nLeverage=1:{}\nVisual=0\nReport=MQL5\\Files\\QuantForge\\{}_report\nReplaceReport=1\nShutdownTerminal=1\n",
        config.expert_directory,
        config.expert_name,
        broker.symbol,
        config.timeframe,
        config.tester.model,
        mql_double(config.tester.deposit),
        config.tester.currency.to_ascii_uppercase(),
        config.tester.leverage,
        config.expert_name
    );
    if let Some(from) = &config.tester.from_date {
        output.push_str(&format!("FromDate={from}\n"));
    }
    if let Some(to) = &config.tester.to_date {
        output.push_str(&format!("ToDate={to}\n"));
    }
    output.push_str("\n[TesterInputs]\n");
    output.push_str(&set_file(config, parity_prefix));
    output
}

fn price_field(value: PriceField) -> u8 {
    match value {
        PriceField::Open => 0,
        PriceField::High => 1,
        PriceField::Low => 2,
        PriceField::Close => 3,
    }
}

fn applied_price(value: PriceField) -> &'static str {
    match value {
        PriceField::Open => "PRICE_OPEN",
        PriceField::High => "PRICE_HIGH",
        PriceField::Low => "PRICE_LOW",
        PriceField::Close => "PRICE_CLOSE",
    }
}

fn series_mode(value: PriceField) -> &'static str {
    match value {
        PriceField::Open => "MODE_OPEN",
        PriceField::High => "MODE_HIGH",
        PriceField::Low => "MODE_LOW",
        PriceField::Close => "MODE_CLOSE",
    }
}

fn mql_double(value: f64) -> String {
    format!("{value:.12}")
}

fn mql_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode};
    use quantforge_core::STRATEGY_IR_VERSION;
    use quantforge_ir::{
        EntryDistancePolicy, EntryOrderPolicy, EntrySignals, ManagePolicy, PartialExit,
        ProtectiveStops, StrategyMeta, TrailingPolicy,
    };

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Fixture".into(),
            symbol: "EURUSD".into(),
            digits: 5,
            point: 0.00001,
            tick_size: 0.00001,
            tick_value: 1.0,
            contract_size: 100_000.0,
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
            base_currency: "EUR".into(),
            profit_currency: "USD".into(),
            margin_currency: "EUR".into(),
            synthetic_spreads: vec![],
        }
    }

    fn strategy() -> StrategyIr {
        let fast = NumericExpr::Indicator {
            value: IndicatorExpr::Ema {
                source: PriceField::Close,
                period: 12,
                shift: 1,
            },
        };
        let slow = NumericExpr::Indicator {
            value: IndicatorExpr::Ema {
                source: PriceField::Close,
                period: 48,
                shift: 1,
            },
        };
        StrategyIr {
            id: "fixture".into(),
            version: STRATEGY_IR_VERSION,
            entry: EntrySignals {
                long: Some(BoolExpr::CrossAbove {
                    left: fast.clone(),
                    right: slow.clone(),
                }),
                short: Some(BoolExpr::CrossBelow {
                    left: fast,
                    right: slow,
                }),
                order: Default::default(),
            },
            exit: None,
            exit_long: None,
            exit_short: None,
            filters: Vec::new(),
            side: Side::Both,
            risk: RiskPolicy::FixedCurrency { amount: 100.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::AtrMultiple {
                    period: 14,
                    multiplier: 2.0,
                },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
            },
            manage: ManagePolicy::default(),
            meta: StrategyMeta {
                thesis_hint: "fixture".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    #[test]
    fn source_is_deterministic_guarded_and_contains_protective_orders() {
        let first = generate_bundle(&strategy(), &broker(), &Mql5ExportConfig::default()).unwrap();
        let second = generate_bundle(&strategy(), &broker(), &Mql5ExportConfig::default()).unwrap();
        assert_eq!(first, second);
        assert!(first.source.contains("InpAllowLiveTrading=false"));
        assert!(first.source.contains("OrderCalcProfit"));
        assert!(
            first
                .source
                .contains("g_trade.Buy(volume,_Symbol,0.0,stop,target")
        );
        assert!(first.source.contains("QFCrossAbove"));
        assert!(first.source.contains("SqATR"));
        assert!(first.source.contains("checkBarOpen"));
        assert!(first.source.contains("sqPrice"));
        assert!(first.source.contains("sqValid"));
        assert!(!first.source.contains("#include \"sqx_runtime.mqh\""));
        assert!(first.source.contains("LongEntrySignal"));
        assert!(first.source.contains("g_decision_bars_seen<320"));
        assert!(!first.source.contains("@@"));
        assert!(first.evidence.mandatory_stop_loss);
        assert!(first.evidence.mandatory_take_profit);
    }

    #[test]
    fn indicator_handles_are_created_once_and_released_on_deinit() {
        for style in [ExportStyle::Sqx, ExportStyle::Quantforge] {
            let bundle = generate_bundle(
                &strategy(),
                &broker(),
                &Mql5ExportConfig {
                    export_style: style,
                    ..Mql5ExportConfig::default()
                },
            )
            .unwrap();
            // Every handle must come from a cache lookup, never from a bare
            // constructor inside a per-tick accessor.
            for constructor in ["iCustom(", "iMA(", "iRSI(", "iATR(", "iStdDev(", "iADXWilder("] {
                let creations = bundle.source.matches(constructor).count();
                let cache_stores = bundle
                    .source
                    .matches(&format!("Remember(key,{constructor}"))
                    .count();
                assert_eq!(
                    creations, cache_stores,
                    "{style:?} creates {constructor} outside the handle cache"
                );
            }
            assert!(bundle.source.contains("ReleaseHandles"));
        }
    }

    #[test]
    fn suggested_expert_names_are_unique_per_candidate_and_mql_safe() {
        assert_eq!(
            crate::suggested_expert_name("EURUSD", "g898-84", 990_001),
            "EURUSD_g898_84"
        );
        assert_eq!(
            crate::suggested_expert_name("EURUSD", "g12-7", 990_002),
            "EURUSD_g12_7"
        );
        // A missing id still yields a distinct, compilable name.
        assert_eq!(
            crate::suggested_expert_name("XAU/USD", "", 42),
            "XAU_USD_m42"
        );
        assert_eq!(crate::suggested_expert_name("30YR", "g1-1", 7), "QF_30YR_g1_1");
        for name in [
            crate::suggested_expert_name("EURUSD", "g898-84", 1),
            crate::suggested_expert_name("", "", 1),
        ] {
            assert!(
                name.chars()
                    .all(|value| value.is_ascii_alphanumeric() || value == '_')
            );
        }
    }

    #[test]
    fn side_specific_exits_are_exported_under_the_matching_position_type() {
        let mut strategy = strategy();
        strategy.exit_long = Some(BoolExpr::Compare {
            comparison: ComparisonOp::LessThan,
            left: NumericExpr::Indicator {
                value: IndicatorExpr::Rsi {
                    source: PriceField::Close,
                    period: 14,
                    shift: 1,
                },
            },
            right: NumericExpr::Constant { value: 45.0 },
        });
        strategy.exit_short = Some(BoolExpr::Compare {
            comparison: ComparisonOp::GreaterThan,
            left: NumericExpr::Indicator {
                value: IndicatorExpr::Rsi {
                    source: PriceField::Close,
                    period: 14,
                    shift: 1,
                },
            },
            right: NumericExpr::Constant { value: 55.0 },
        });
        let bundle =
            generate_bundle(&strategy, &broker(), &Mql5ExportConfig::default()).unwrap();
        assert!(bundle.source.contains("position_type == POSITION_TYPE_BUY"));
        assert!(bundle.source.contains("position_type == POSITION_TYPE_SELL"));
        assert!(bundle.source.contains("45.000000000000"));
        assert!(bundle.source.contains("55.000000000000"));
        assert!(!bundle.source.contains("@@"));
    }

    #[test]
    fn new_family_indicators_emit_qf_helpers() {
        let mut strategy = strategy();
        strategy.entry.long = Some(BoolExpr::And {
            children: vec![
                BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Indicator {
                        value: IndicatorExpr::BodyRangeRatio { shift: 1 },
                    },
                    right: NumericExpr::Constant { value: 0.6 },
                },
                BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Indicator {
                        value: IndicatorExpr::LiquiditySweepScore {
                            period: 14,
                            shift: 1,
                        },
                    },
                    right: NumericExpr::Constant { value: 0.0 },
                },
                BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Price {
                        field: PriceField::Close,
                        shift: 1,
                    },
                    right: NumericExpr::Indicator {
                        value: IndicatorExpr::SessionRangeHigh {
                            start_hour: 9,
                            range_bars: 2,
                            shift: 1,
                        },
                    },
                },
            ],
        });
        let bundle = generate_bundle(&strategy, &broker(), &Mql5ExportConfig::default()).unwrap();
        assert!(bundle.source.contains("QFBodyRangeRatio"));
        assert!(bundle.source.contains("QFLiquiditySweepScore"));
        assert!(bundle.source.contains("QFSessionRangeHigh"));
        assert!(!bundle.support_files.is_empty());
        assert!(!bundle.source.contains("@@"));
    }

    #[test]
    fn management_and_pending_entries_are_exported() {
        let mut strategy = strategy();
        strategy.manage.break_even_at_r = Some(1.0);
        strategy.manage.flatten_end_of_day = true;
        strategy.manage.max_one_entry_per_day = true;
        strategy.manage.trailing = Some(TrailingPolicy::RiskMultiple {
            activate_at_r: 1.5,
            distance_r: 0.75,
        });
        strategy.manage.partial_exits = vec![PartialExit {
            at_r: 1.0,
            fraction: 0.5,
        }];
        strategy.entry.order = EntryOrderPolicy::Stop {
            distance: EntryDistancePolicy::AtrMultiple {
                period: 14,
                multiplier: 0.5,
            },
            expiry_bars: 3,
        };
        let bundle = generate_bundle(&strategy, &broker(), &Mql5ExportConfig::default()).unwrap();
        assert!(bundle.source.contains("return 1;"));
        assert!(bundle.source.contains("return 3;"));
        assert!(bundle.source.contains("g_trade.BuyStop"));
        assert!(bundle.source.contains("QFManagePosition"));
        assert!(bundle.source.contains("return true;"));
        assert!(bundle.source.contains("QFInCloseBlackout()"));
        assert!(bundle.source.contains("current.hour>=23"));
        assert!(bundle.source.contains("QFMaxOneEntryPerDay"));
        assert!(bundle.source.contains("QFEntryDayExhausted"));
        assert!(bundle.source.contains("QFInMandatoryEntryWindow"));
        assert!(bundle.source.contains("InpEntryWindowStartHour=2"));
        assert!(bundle.source.contains("InpEntryWindowEndHour=19"));
        assert!(
            bundle
                .source
                .contains("current.hour>=InpEntryWindowStartHour && current.hour<InpEntryWindowEndHour")
        );
        assert!(bundle.source.contains("QFMarkEntrySignalTaken"));
    }

    #[test]
    fn stop_limit_entries_are_exported() {
        let mut strategy = strategy();
        strategy.entry.order = EntryOrderPolicy::StopLimit {
            stop_distance: EntryDistancePolicy::AtrMultiple {
                period: 14,
                multiplier: 0.5,
            },
            limit_offset: EntryDistancePolicy::AtrMultiple {
                period: 14,
                multiplier: 0.25,
            },
            expiry_bars: 4,
        };
        let bundle = generate_bundle(&strategy, &broker(), &Mql5ExportConfig::default()).unwrap();
        assert!(bundle.source.contains("return 3;"));
        assert!(bundle.source.contains("g_trade.BuyStopLimit"));
        assert!(bundle.source.contains("g_trade.SellStopLimit"));
        assert!(bundle.source.contains("QFEntryLimitOffset"));
        assert!(!bundle.source.contains("@@"));
    }

    #[test]
    fn a_custom_entry_window_reaches_the_expert_and_its_set_file() {
        let config = Mql5ExportConfig {
            entry_window_start_hour: 2,
            entry_window_end_hour: 23,
            ..Mql5ExportConfig::default()
        };
        let bundle = generate_bundle(&strategy(), &broker(), &config).unwrap();
        assert!(bundle.source.contains("InpEntryWindowStartHour=2"));
        assert!(bundle.source.contains("InpEntryWindowEndHour=23"));
        assert!(bundle.set_file.contains("InpEntryWindowEndHour=23||23||1||23||N"));
    }

    #[test]
    fn an_inverted_entry_window_is_rejected() {
        let config = Mql5ExportConfig {
            entry_window_start_hour: 22,
            entry_window_end_hour: 3,
            ..Mql5ExportConfig::default()
        };
        assert!(generate_bundle(&strategy(), &broker(), &config).is_err());
    }

    /// The export crate cannot depend on the engine, so its mirrored defaults
    /// are pinned here instead.
    #[test]
    fn entry_window_defaults_match_the_engine() {
        use crate::model::{MANDATORY_ENTRY_WINDOW_END_HOUR, MANDATORY_ENTRY_WINDOW_START_HOUR};

        let engine = quantforge_eval::EntryWindow::default();
        assert_eq!(engine.start_hour, MANDATORY_ENTRY_WINDOW_START_HOUR);
        assert_eq!(engine.end_hour, MANDATORY_ENTRY_WINDOW_END_HOUR);
        let config = Mql5ExportConfig::default();
        assert_eq!(config.entry_window_start_hour, engine.start_hour);
        assert_eq!(config.entry_window_end_hour, engine.end_hour);
    }
}
