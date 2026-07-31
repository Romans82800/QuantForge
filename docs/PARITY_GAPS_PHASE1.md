# Phase 1 parity gaps (honest)

Target: ≥95% trade-aligned vs MT5 Strategy Tester (`mt5-parity-v2`) on a fixed probe suite covering **market + stop + limit + stop-limit**.

## Landed in this branch

- IR `EntryOrderPolicy::StopLimit { stop_distance, limit_offset, expiry_bars }`
- Scout (`quantforge-eval`) and M1 Judge (`quantforge-tick`) two-stage pending: stop trigger → limit fill
- MQL5 export kind `3` → `CTrade::BuyStopLimit` / `SellStopLimit`
- Fixture `fixtures/stop_limit_pending_strategy.json`
- Unit tests: Scout fill path + export asserts
- `scripts/family_mt5_parity.py` mode `stop_limit` via `emit_family_strategy --mode stop_limit`
- Discover flag `allow_stop_limit_entries` (default **off**; Phase 3 enables in islands)

## Still blocking true 95%

| Gap | Impact |
|-----|--------|
| No live MT5 EveryTick golden CSVs for stop-limit probes yet | Cannot claim measured ≥95% until Strategy Tester runs land |
| Same-bar / selected-TF fill path is OHLC-conservative, not tick path | Remaining misalignment on stop-limit same-bar trigger+fill |
| Filling mode / partial fills / requotes not modeled | Edge cases vs broker |
| Netting accounts / multi-position hedged stacks | Engines assume single magic position |
| Swap reopen / pending modify / OCO | Absent from IR |

## Next concrete steps

1. Generate stop-limit probe EAs, run MT5 tester, archive deals under `parity/` fixtures.
2. Tighten M1 same-minute stop-limit trigger vs limit fill ordering against EveryTick.
3. Phase 2 Builder funnel (cheap reject → islands → databank).
