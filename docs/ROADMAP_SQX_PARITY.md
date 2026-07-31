# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank → retest/what-if/portfolio).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Inventory snapshot (SQX plugins / snippets → QuantForge)

| SQX surface (install) | QuantForge status |
|-----|-----|
| AppBuilder / TaskBuild / Mass Builder | **Present** — Discover Mass Builder + islands + prefilter |
| AppRetester / TaskRetest | **Partial** — Challenge / Judge / multi-symbol gates; no dedicated Retester UI app |
| AppOptimizer / TaskOptimize | **Partial** — MAP-Elites + neighborhood perturbation; no classic grid optimizer UI |
| CrossCheckWalkForward* | **Present** — purged walk-forward in Challenge / Discover robustness |
| CrossCheckMonteCarlo* | **Present** — moving-block / trade MC in Challenge |
| CrossCheckWhatIf | **Present** — `quantforge what-if` + `quantforge_quality::apply_what_if` |
| CrossCheckRetestOnAdditionalMarkets | **Partial** — multi-symbol Discover gates |
| MoneyManagement (FixedSize, RiskFixedPct, …) | **Present** — `FixedCurrency` / `PercentBalance` / `FixedLots` (Discover deposit still stamps fixed $ risk) |
| TradingOptions (weekends, Friday exit, max trades/day) | **Present** — ManagePolicy + Scout/Judge/MQL5 |
| ExitMethods (SL/TP/BE/trail/partial/time) | **Present** |
| PortfolioMaster / PortfolioComposer | **Present** — `quantforge portfolio` packing |
| Databank columns / filters | **Present** — island / entry order / management columns + filters |
| SkinDark | **Partial** — desktop tokens aligned to SQX dark grays/accent |
| NeuralNetwork / Stockpicker / Crypto exchanges | **Out of scope** / external |
| Live MT5 EveryTick goldens | **External** — capture needs authorized tester session |

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
- **Genetic islands**: `island_count` + ring `migration_interval` / `migration_elites`.
- **`DiscoverRunMode::MassBuilder`**: large batch, continuous pot, prefilter + islands + M1 robustness.
- CLI: `--run-mode mass_builder`. How-to: `docs/MASS_BUILDER_HARVEST.md`.

**Still open**

- Databank UX streaming polish at very large elite counts.
- Production soak metrics (candidates/hour) on real packs.

---

## Phase 3 — Complex M1 islands (**landed**)

- `complex_m1_island_count`, band-safe migration, desktop Mass Builder knobs.

---

## Phase 4 — Money management, trading options, What-If (**landed this wave**)

- `RiskPolicy::FixedLots` through Scout / Judge / MQL5 (`VOLUME_MODE` / `FIXED_LOTS`).
- ManagePolicy: `dont_trade_on_weekends`, `exit_on_friday`, `max_trades_per_day`.
- What-If cross-check filters + CLI `quantforge what-if`.
- Discover samples weekend / Friday / max-trades genes (deposit risk remains fixed $).

---

## Remaining parity gaps (honest)

| Gap | Status | Notes |
|-----|--------|-------|
| Market / Stop / Limit / StopLimit | Present | Full IR → Scout → Judge → export |
| HedgedStack / Netting | Present | Multi-slot stack + netting close-on-opposite |
| Tick-file EveryTick | Partial | CSV ingest + flag; live MT5 goldens external |
| Fill simulation | Present | Partial volume + deterministic requote |
| Pending modify / OCO-lite | Present | OrderModify + cancel-on-opposite |
| What-If cross-checks | Present | Biggest/lowest PnL, every Nth, side filters, max/day |
| Strategy Negater | Present | `quantforge negate` flips long/short trees |
| MinMax SL/PT clamps | Present | ManagePolicy min/max stop & TP points |
| Classic parameter optimizer UI | Missing | Neighborhood exists; no SQX AppOptimizer clone |
| Dedicated Retester app | Missing | Challenge/Judge cover rigor; no task graph UI |
| Walk-forward *matrix* UI | Missing | Folds exist; matrix visualization not ported |
| Martingale / ATM MM | Missing | Deliberately deprioritized (risk) |
| Neural / stockpicker / crypto feeds | Out of scope | |
| Live EveryTick goldens | External | Operator capture |

Protocol: `mt5-parity-v2`. Probes: `mql5/QuantForge/`, `scripts/family_mt5_parity.py`, `scripts/stop_limit_everytick_golden.py`.

---

## SQX behavioral reference paths (do not copy source)

- Snippet *names* under `internal\extend\Snippets\SQ\` (MoneyManagement, TradingOptions, WhatIf, …)
- Plugins under `internal\plugins\` (AppBuilder, AppRetester, CrossCheck*, …)
- MT5 export surface: `internal\extend\Code\MetaTrader5\`
- Core engine: binary-only — reimplement from contracts, never decompile
