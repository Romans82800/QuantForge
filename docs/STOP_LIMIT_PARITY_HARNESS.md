# Stop-limit MT5 parity measurement

Phase 1 wired BuyStopLimit/SellStopLimit end-to-end. This note is the **measurement** path toward ≥95% trade alignment vs Strategy Tester (EveryTick / M1).

## Quick path (automated family harness)

From the repo root, with `quantforge.exe` on `PATH` and MetaTrader 5 configured as in `scripts/family_mt5_parity.py`:

```powershell
$env:QUANTFORGE_DATA_PACK = "C:\Users\Administrator\Documents\QuantForge\ICMarkets_EST7_2020_present"
python scripts/family_mt5_parity.py --mode stop_limit --continue
```

Or emit a single seed:

```powershell
cargo run -p quantforge-discover --example emit_family_strategy -- `
  --family trend_pullback --sequence 0 --mode stop_limit --out stop_limit.ir.json
```

Then export via CLI / desktop and run Strategy Tester with model **Every tick based on real ticks** (or **1 minute OHLC** for a faster smoke).

## Fixture

- IR: `fixtures/stop_limit_pending_strategy.json`
- Export must contain `g_trade.BuyStopLimit` / `SellStopLimit` and `QFEntryLimitOffset`

## What to archive for goldens

For each probe, save under something like `parity/stop_limit/<tag>/`:

1. Exported `.mq5` + `.set`
2. Tester deals / report CSV used by `quantforge parity`
3. Judge (or Scout) run JSON from QuantForge
4. Notes: symbol, dates, tester model (EveryTick vs M1), spread

Compare with protocol `mt5-parity-v2` (`crates/quantforge-parity`).

## If MT5 is flaky / unavailable

Do **not** block the product branch. Unit tests already cover:

- Scout two-stage trigger→limit fill
- MQL5 export kind `3`

Stub golden layout + one-command recipe: `docs/STOP_LIMIT_EVERYTICK_GOLDEN.md` /
`python scripts/stop_limit_everytick_golden.py --prepare`.

Re-run the family harness when a stable MT5 terminal + data pack is available. Remaining measured gaps are listed in `docs/PARITY_GAPS_PHASE1.md`.
