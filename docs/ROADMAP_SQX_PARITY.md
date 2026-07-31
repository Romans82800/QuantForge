# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank → retest/what-if/portfolio).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Inventory snapshot (SQX plugins / snippets → QuantForge)

| SQX surface (install) | QuantForge status |
|-----|-----|
| AppBuilder / TaskBuild / Mass Builder | **Present** — Discover Mass Builder + islands + prefilter |
| AppRetester / TaskRetest | **Present** — Retester workspace: M1 Judge, Challenge, WF Matrix, EA export, MT5 compare |
| AppOptimizer / TaskOptimize | **Present** — Optimizer workspace: param neighborhood + WF matrix (Scout); MAP-Elites still powers Discover |
| CrossCheckWalkForward* | **Present** — purged WF in Challenge / Discover robustness |
| CrossCheckWalkForwardMatrix | **Present** — `run_walk_forward_matrix` + CLI `quantforge wf-matrix` + Dark UI grid |
| CrossCheckMonteCarlo* | **Present** — moving-block / trade MC in Challenge |
| CrossCheckWhatIf | **Present** — `quantforge what-if` + `quantforge_quality::apply_what_if` |
| CrossCheckNegater | **Present** — `quantforge negate` |
| CrossCheckRetestOnAdditionalMarkets | **Partial** — multi-symbol Discover gates |
| MoneyManagement (FixedSize, RiskFixedPct, SimpleMartingaleMM, …) | **Present** — `FixedCurrency` / `PercentBalance` / `FixedLots` / `Martingale` (Scout/Judge/MQL5) |
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

- End-to-end **BuyStopLimit / SellStopLimit** (IR → Scout → Judge → MQL5 export → unit tests + fixtures).
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

## Phase 4 — Money management, trading options, What-If (**landed**)

- `RiskPolicy::FixedLots` through Scout / Judge / MQL5 (`VOLUME_MODE` / `FIXED_LOTS`).
- ManagePolicy: `dont_trade_on_weekends`, `exit_on_friday`, `max_trades_per_day`.
- What-If cross-check filters + CLI `quantforge what-if`.
- Discover samples weekend / Friday / max-trades genes (deposit risk remains fixed $).

---

## Phase 5 — Martingale MM, Optimizer / Retester shells, WF matrix (**landed this wave**)

- `RiskPolicy::Martingale { base_lots, multiplier, max_steps }` — Scout loss-streak sizing, Judge, MQL5 volume mode `2`, Challenge perturb arms, export placeholders.
- Desktop **Optimizer** workspace (neighborhood table + WF matrix).
- Desktop **Retester** gains Challenge + WF Matrix tabs (alongside Judge / export / parity).
- `quantforge-quality::run_walk_forward_matrix` + CLI `wf-matrix`.

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
| Classic parameter optimizer UI | Present | Thin shell over neighborhood Scout + results table |
| Dedicated Retester app | Present | Challenge / Judge / WF matrix / export / compare shells |
| Walk-forward *matrix* | Present | Engine + CLI + Dark UI grid |
| Martingale MM | Present | SimpleMartingale-style streak sizing (not full ATM/grid ladder) |
| ATR / crypto / stock MM variants | Missing | Lower priority vs FX lot policies |
| Full SQX task-graph Retester | Partial | Shells wired; no SQX project XML import |
| Ranking filter DSL parity | Partial | Databank columns exist; SQX expression filters not cloned |
| Neural / stockpicker / crypto feeds | Out of scope | |
| Live EveryTick goldens | External | Operator capture |

Protocol: `mt5-parity-v2`. Probes: `mql5/QuantForge/`, `scripts/family_mt5_parity.py`, `scripts/stop_limit_everytick_golden.py`.

---

## SQX behavioral reference paths (do not copy source)

- Snippet *names* under `internal\extend\Snippets\SQ\` (MoneyManagement, TradingOptions, WhatIf, …)
- Plugins under `internal\plugins\` (AppBuilder, AppRetester, CrossCheck*, …)
- MT5 export surface: `internal\extend\Code\MetaTrader5\`
- Core engine: binary-only — reimplement from contracts, never decompile
