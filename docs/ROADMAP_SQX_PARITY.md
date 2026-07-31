# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank → retest/what-if/portfolio).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Inventory snapshot (SQX plugins / snippets → QuantForge)

| SQX surface (install) | QuantForge status |
|-----|-----|
| AppBuilder / TaskBuild / Mass Builder | **Present** — Discover Mass Builder + islands + prefilter |
| AppRetester / TaskRetest | **Present** — Retester workspace + QF-native task graph JSON (`docs/TASK_GRAPH.md`) |
| AppOptimizer / TaskOptimize | **Present** — Optimizer workspace: param neighborhood + WF matrix |
| CrossCheckWalkForward* | **Present** — purged WF in Challenge / Discover robustness |
| CrossCheckWalkForwardMatrix | **Present** — engine + CLI + Dark UI |
| CrossCheckMonteCarlo* | **Present** |
| CrossCheckWhatIf | **Present** |
| CrossCheckNegater | **Present** |
| CrossCheckRetestOnAdditionalMarkets | **Partial** — multi-symbol Discover gates |
| MoneyManagement Fixed / Risk% / Martingale / ATR | **Present** — `FixedCurrency` / `PercentBalance` / `FixedLots` / `Martingale` / `AtrRiskPercent` |
| MoneyManagement Crypto / Stocks / Picker | **Out of scope** — exchange / stockpicker APIs |
| TradingOptions | **Present** |
| ExitMethods | **Present** |
| PortfolioMaster / PortfolioComposer | **Present** |
| Databank columns / ranking filters | **Present** — expression DSL (`PF > 1.5 AND Drawdown < 20`) in UI + `quantforge databank-filter` |
| SettingsFiltering / TaskFiltering | **Partial** — expression DSL covers ranking filters; no SQX task XML import |
| ProjectRetester XML | **Documented alternative** — QF JSON task graph (SQX XML not publicly schema-documented) |
| SkinDark | **Partial** |
| NeuralNetwork / Crypto exchanges | **Out of scope** |
| Live MT5 EveryTick goldens | **External** |

---

## Phase 1–5 — landed (see prior sections in git history)

MT5 order parity, Mass Builder, complex M1 islands, FixedLots / weekends / What-If / Negater, Martingale, Optimizer/Retester shells, WF matrix.

---

## Phase 6 — ATR MM, ranking DSL, task graph (**landed this wave**)

- `RiskPolicy::AtrRiskPercent` through Scout / Judge / MQL5 volume mode `3`.
- Databank filter expression language (Rust + Dark UI box + CLI).
- QF-native Retester task graph JSON + `quantforge task-run` + `docs/TASK_GRAPH.md`.

---

## Remaining parity gaps (honest)

| Gap | Status | Notes |
|-----|--------|-------|
| ATR / volatility MM | Present | SQX ATRRiskBasedSizing-style |
| Ranking filter DSL | Present | AND/OR/NOT + column aliases |
| Retester task chaining | Present | QF JSON graph; not SQX XML |
| SQX project/task XML import | Blocked | Schema inside proprietary JARs |
| Crypto / Stocks MM | Out of scope | |
| Full in-process task executor | Partial | `task-run` plans/validates; steps still invoke dedicated commands |
| Live EveryTick goldens | External | |
| Neural / crypto connectors | Out of scope | |

Protocol: `mt5-parity-v2`. Probes: `mql5/QuantForge/`, `scripts/family_mt5_parity.py`, `scripts/stop_limit_everytick_golden.py`.
