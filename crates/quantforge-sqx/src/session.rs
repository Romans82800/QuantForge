use chrono::{NaiveDate, Timelike};
use quantforge_broker::BrokerClock;
use quantforge_data::Bar;

/// Broker-local session range high/low carried forward after the opening window.
pub fn session_range_series(
    bars: &[Bar],
    clock: &BrokerClock,
    start_hour: u8,
    range_bars: usize,
    high: bool,
) -> Vec<f64> {
    let mut output = vec![f64::NAN; bars.len()];
    if range_bars == 0 {
        return output;
    }
    let mut day_cursor: Option<NaiveDate> = None;
    let mut window_start: Option<usize> = None;
    let mut frozen: Option<f64> = None;

    for (index, bar) in bars.iter().enumerate() {
        let Ok(local) = clock.local_datetime(bar.timestamp_ms) else {
            output[index] = frozen.unwrap_or(f64::NAN);
            continue;
        };
        let day = local.date();
        if day_cursor != Some(day) {
            day_cursor = Some(day);
            window_start = None;
            frozen = None;
        }
        if window_start.is_none() && local.hour() as u8 >= start_hour {
            window_start = Some(index);
        }
        if let Some(start) = window_start {
            if index + 1 >= start + range_bars {
                let window = &bars[start..start + range_bars];
                frozen = Some(if high {
                    window.iter().map(|bar| bar.high).fold(f64::NEG_INFINITY, f64::max)
                } else {
                    window.iter().map(|bar| bar.low).fold(f64::INFINITY, f64::min)
                });
            }
        }
        output[index] = frozen.unwrap_or(f64::NAN);
    }
    output
}
