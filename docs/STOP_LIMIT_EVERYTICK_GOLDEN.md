# Stop-limit EveryTick golden format

Measurement toward ≥95% MT5 Strategy Tester alignment for BuyStopLimit / SellStopLimit.

## One-command recipe

```powershell
$env:Path = "C:\Program Files\Git\cmd;C:\Users\Administrator\.cargo\bin;C:\Users\Administrator\AppData\Local\Programs\Python\Python312;" + $env:Path
$env:QUANTFORGE_DATA_PACK = "C:\Users\Administrator\Documents\QuantForge\ICMarkets_EST7_2020_present"

# Stub layout for a live capture:
python scripts/stop_limit_everytick_golden.py --prepare

# Automated demo capture (credentials from env or gitignored `.mt5-demo.local` ONLY):
#   MT5_LOGIN / MT5_PASSWORD / MT5_SERVER   or   .mt5-demo.local
python scripts/stop_limit_everytick_golden.py --capture --symbol EURNZD --from-date 2024.01.01 --to-date 2024.02.01

# Self-contained CI fixture (no MT5):
python scripts/stop_limit_everytick_golden.py --write-fixture
python scripts/stop_limit_everytick_golden.py --compare parity/stop_limit/golden_numeric_fixture
# → PASS / FAIL via inline mt5-parity-v2 tolerances (or `quantforge parity` when full inputs exist)
```

After Strategy Tester EveryTick capture, drop deals/equity (+ Judge) into the golden dir and run `--compare`.

**Never commit** MT5 passwords, account numbers, or `.mt5-demo.local`. Use env vars or the gitignored local file.

## Compare modes

1. **Full `quantforge parity`** — when `evidence.json`, `expert.mq5`, `mt5_metadata.json`, `qf_judge.json`, deals, and equity are all present.
2. **Inline metrics** — compares `reference_metrics.json` / `qf_judge.json` vs `external_metrics.json` or deals+equity-derived metrics (trade count, net profit, max drawdown, equity path). Writes `compare_report.json`.

`--prepare` still creates the awaiting-capture stub and does not require MT5.

## Golden directory layout

```
parity/stop_limit/<tag>/
  manifest.json
  strategy.ir.json
  expert.mq5                 # optional until export
  evidence.json              # optional; enables full quantforge parity
  mt5_metadata.json          # optional; enables full quantforge parity
  mt5_deals.csv
  mt5_equity.csv
  qf_judge.json              # or reference_metrics.json
  external_metrics.json      # optional sidecar
  compare_report.json        # written by --compare
  notes.md
```

## Tick-file Scout replay

```powershell
quantforge scout ... --same-bar-policy every_tick_ohlc
quantforge scout ... --tick-file ticks.csv --enable-tick-file-replay
quantforge scout ... --position-accounting hedged_stack --max-open-positions 3
```

Tick CSV: `timestamp_ms,bid,ask` (header optional).

## MT5 operator capture (when terminal is present but login-gated)

Terminal found at `C:\Program Files\MetaTrader 5\terminal64.exe`.

1. Log into the broker account that matches the QuantForge pack/broker profile (non-interactive capture is blocked without an authorized session).
2. `quantforge export` the stop-limit IR → copy EA under `MQL5/Experts`.
3. Strategy Tester: model **Every tick based on real ticks**, same symbol/session/window as the Judge recipe.
4. Export deals + equity CSVs into the golden dir; run Judge → `qf_judge.json`.
5. `python scripts/stop_limit_everytick_golden.py --compare <dir>`

## Status

- Engine + export for StopLimit: **landed**
- HedgedStack multi-slot book + tick-file EveryTick slice: **landed**
- Numeric `--compare` (fixture + inline / parity): **landed**
- Archived live EveryTick goldens: **external** (MT5 login + operator capture)
