# Live MT5 StopLimit golden (EveryTick model 4)

Measured on ICMarketsSC-Demo portable terminal (login via gitignored .mt5-demo.local only).
Window: 2024.01.01 → 2024.02.01, symbol EURNZD, H1 decisions / M1 judge.

Equity series in qf_judge.json / parity_report.json are downsampled for git; full MT5 equity remains in mt5_equity.csv.

# Stop-limit EveryTick golden notes

1. Export `strategy.ir.json` to MQ5 (desktop or `quantforge export`).
2. Strategy Tester: model **Every tick based on real ticks**, bound symbol/broker.
3. Save deals + equity CSVs as `mt5_deals.csv` / `mt5_equity.csv`.
4. Run QuantForge Judge on the same window → `qf_judge.json`
   (or drop matching `reference_metrics.json` / `external_metrics.json`).
5. `python scripts/stop_limit_everytick_golden.py --compare parity/stop_limit/golden_live_eurnzd`

External blocker without MT5: leave `status` as awaiting_mt5_capture.
