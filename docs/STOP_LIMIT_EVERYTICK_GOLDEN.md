# Stop-limit EveryTick golden format (stub)

Measurement toward ≥95% MT5 Strategy Tester alignment for BuyStopLimit / SellStopLimit.

## One-command recipe (when MT5 + pack are available)

```powershell
$env:Path = "C:\Program Files\Git\cmd;C:\Users\Administrator\.cargo\bin;" + $env:Path
$env:QUANTFORGE_DATA_PACK = "C:\Users\Administrator\Documents\QuantForge\ICMarkets_EST7_2020_present"
# Optional: $env:MT5_TERMINAL = "C:\Program Files\MetaTrader 5\terminal64.exe"

python scripts/stop_limit_everytick_golden.py --prepare
# After Strategy Tester EveryTick run, drop deals/equity CSVs into the golden dir:
python scripts/stop_limit_everytick_golden.py --compare parity/stop_limit/golden_stub
```

Without MT5, `--prepare` still writes the stub layout + IR fixture export notes so the compare step is ready once goldens exist.

## Golden directory layout

```
parity/stop_limit/<tag>/
  manifest.json          # symbol, dates, model, hashes
  strategy.ir.json       # QuantForge IR
  expert.mq5             # exported EA (optional until export step)
  mt5_deals.csv          # Strategy Tester deals export
  mt5_equity.csv         # Strategy Tester equity export
  qf_judge.json          # QuantForge Judge (or Scout) result
  notes.md               # human notes
```

## `manifest.json` schema

```json
{
  "schema_version": 1,
  "protocol": "mt5-parity-v2",
  "tag": "golden_stub",
  "symbol": "EURNZD",
  "timeframe": "H1",
  "tester_model": "every_tick_real_ticks",
  "order_kinds": ["buy_stop_limit", "sell_stop_limit"],
  "data_pack": "ICMarkets_EST7_2020_present",
  "status": "awaiting_mt5_capture"
}
```

## Status

- Engine + export for StopLimit: **landed** (Phase 1)
- Archived EveryTick goldens: **external blocker** (needs MT5 terminal + operator capture)
- Compare path: `quantforge parity` / `scripts/stop_limit_everytick_golden.py --compare`
