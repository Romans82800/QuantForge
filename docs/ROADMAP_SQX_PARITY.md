# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Phase 1 — MT5 order-type + execution parity (**landed**)

**Shipped**

- End-to-end **BuyStopLimit / SellStopLimit** (IR → Scout → M1 Judge → MQL5 export → unit tests + fixtures).
- Order-type coverage expanded: market + limit + stop + stop-limit (`family_mt5_parity.py --mode stop_limit`).
- Measurement notes: `docs/STOP_LIMIT_PARITY_HARNESS.md`, gaps: `docs/PARITY_GAPS_PHASE1.md`.

**Still open for measured ≥95%**

- Archive live Strategy Tester EveryTick goldens and run `quantforge parity` on the stop-limit suite.

---

## Phase 2 — SQX-like mass Builder (**landed core**)

**Shipped**

- **Cheap prefilter**: trailing-window Scout + `prefilter_gates` before full IS (`enable_cheap_prefilter`).
- **Genetic islands**: `island_count` + ring `migration_interval` / `migration_elites` on the breeding pot (`crates/quantforge-discover/src/islands.rs`).
- **`DiscoverRunMode::MassBuilder`**: large batch, continuous pot (no early pot stop), prefilter + islands + M1 robustness for databank.
- CLI: `--run-mode mass_builder` (aliases `builder`, `mass`).
- How to run a long harvest: `docs/MASS_BUILDER_HARVEST.md`.

**Success metrics (partial)**

- Funnel telemetry: `rejected_prefilter`, `island_migrations`.
- Continuous harvest knobs ready for EURNZD-style multi-day runs via `--continue`.

**Still open**

- Databank UX throughput polish (desktop ranking/streaming).
- Production soak metrics (candidates/hour) on real packs.

---

## Phase 3 — Discover enables pending / BE / trail with M1 precision islands (**landed**)

**Shipped**

- `complex_m1_island_count`: highest-numbered islands sample pending / BE / trail / partials; lower islands stay Selected-TF market-only.
- Mass Builder sets ~half islands to complex_m1, enables all allow_* flags, keeps global `require_m1_precision=false` (M1 forced on complex promotion).
- Band-safe island migration (simple↔simple, complex↔complex).
- CLI / desktop / Tauri: Mass Builder run mode + island / prefilter / complex_m1 knobs.

**Success metrics**

- Feature-flagged runs produce valid IR + export for enabled entry kinds on complex islands.
- Simple islands never emit pending/BE/trail/partial genes when mixed profiles are active.
- Robustness / M1 gates still apply to promoted complex elites.

---

## Remaining parity gaps (honest, toward 95%)

| Gap | Status | Notes |
|-----|--------|-------|
| Market / Stop / Limit entries | Present | IR + Scout + Judge + export |
| BuyStopLimit / SellStopLimit | Phase 1 | Two-price pending FSM + MQL5 StopLimit |
| Swap / reopen modes | Missing | Not in IR |
| EveryTick vs M1 same-bar fill path | Partial | Conservative same-bar; not true tick replay |
| Filling modes (FOK/IOC/RETURN) | Partial | Export uses `SetTypeFillingBySymbol` |
| Netting vs hedging | Partial | Engines assume hedged single-position model |
| Partial fills / requotes | Missing | Idealized fills + adverse slippage points |
| Pending modification / OCO | Missing | Place + expire + fill only |

Protocol: `mt5-parity-v2` in `crates/quantforge-parity`. Probes: `mql5/QuantForge/`, `scripts/family_mt5_parity.py`.

---

## SQX behavioral reference paths (do not copy source)

- Blocks / trading options: `C:\StrategyQuantX144\internal\extend\Snippets\SQ\`
- MT5 export surface: `C:\StrategyQuantX144\internal\extend\Code\MetaTrader5\` (`Main.tpl`, `SQ.mqh`, Enter*, TradingOptions)
- Core engine: binary-only — reimplement from contracts, never decompile

Public MT5 StopLimit contract used by Phase 1:

- **BuyStopLimit**: stop trigger above Ask; when triggered, Buy Limit at limit price (limit \< stop).
- **SellStopLimit**: stop trigger below Bid; when triggered, Sell Limit at limit price (limit \> stop).
- `CTrade::BuyStopLimit(volume, price, stoplimit, …)` / `SellStopLimit(volume, price, stoplimit, …)`.
