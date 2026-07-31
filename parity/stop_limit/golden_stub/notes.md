# Stop-limit EveryTick golden notes

1. Export `strategy.ir.json` to MQ5 (desktop or `quantforge export`).
2. Strategy Tester: model **Every tick based on real ticks**, bound symbol/broker.
3. Save deals + equity CSVs as `mt5_deals.csv` / `mt5_equity.csv`.
4. Run QuantForge Judge on the same window → `qf_judge.json`
   (or drop matching `reference_metrics.json` / `external_metrics.json`).
5. `python scripts/stop_limit_everytick_golden.py --compare parity/stop_limit/golden_stub`

External blocker without MT5: leave `status` as awaiting_mt5_capture.
