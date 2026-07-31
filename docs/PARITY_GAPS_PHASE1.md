# Phase 1 parity gaps (honest)

Target: ≥95% trade-aligned vs MT5 Strategy Tester (`mt5-parity-v2`) on a fixed probe suite covering **market + stop + limit + stop-limit**.

## Landed

- IR `EntryOrderPolicy::StopLimit` + Scout / Judge FSM + MQL5 `BuyStopLimit` / `SellStopLimit`
- Fixtures, unit tests, `family_mt5_parity.py --mode stop_limit`
- Discover `allow_stop_limit_entries` (complex islands / Mass Builder)
- Filling-mode preference, OCO-lite, pending OrderModify, HedgedStack, tick-file EveryTick slice
- Numeric golden `--compare` + CI fixture (`scripts/stop_limit_everytick_golden.py`)

## Still blocking measured ≥95%

| Gap | Impact |
|-----|--------|
| No live MT5 EveryTick golden CSVs for stop-limit probes yet | Cannot claim measured ≥95% until Strategy Tester runs land |
| Broker-specific requote / partial-fill realism beyond `FillSimulation` | Edge cases vs live books |

## Operator next step

1. Authorized MT5 session + pack → export stop-limit EA → EveryTick tester.
2. Drop deals/equity into `parity/stop_limit/<tag>/` and run `--compare` (or full `quantforge parity`).
