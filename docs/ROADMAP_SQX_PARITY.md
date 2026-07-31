# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank → retest/what-if/portfolio).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Inventory snapshot (SQX plugins / snippets → QuantForge)

| SQX surface (install) | QuantForge status |
|-----|-----|
| AppBuilder / TaskBuild / Mass Builder | **Present** — Discover Mass Builder + islands + prefilter |
| AppRetester / TaskRetest | **Present** — Retester workspace + in-process `quantforge task-run` executor |
| AppOptimizer / TaskOptimize | **Present** — Optimizer workspace: param neighborhood + WF matrix |
| CrossCheckWalkForward* | **Present** — purged WF in Challenge / Discover robustness |
| CrossCheckWalkForwardMatrix | **Present** — engine + CLI + Dark UI |
| CrossCheckMonteCarlo* | **Present** |
| CrossCheckWhatIf | **Present** |
| CrossCheckNegater | **Present** |
| CrossCheckRetestOnAdditionalMarkets | **Partial** — multi-symbol Discover gates + task step |
| MoneyManagement Fixed / Risk% / Martingale / ATR | **Present** — `FixedCurrency` / `PercentBalance` / `FixedLots` / `Martingale` / `AtrRiskPercent` |
| MoneyManagement Crypto / Stocks / Picker | **Out of scope** — exchange / stockpicker APIs |
| TradingOptions | **Present** |
| ExitMethods | **Present** |
| PortfolioMaster / PortfolioComposer | **Present** |
| Databank columns / ranking filters | **Present** — expression DSL (`PF > 1.5 AND Drawdown < 20`) in UI + `quantforge databank-filter` |
| SettingsFiltering / TaskFiltering | **Partial** — expression DSL covers ranking filters; no SQX task XML import |
| ProjectRetester XML | **Documented alternative** — QF JSON task graph (SQX XML not publicly schema-documented) |
| SaverHTML / results HTML | **Present** — `html_report` task step + `quantforge export-html` |
| SkinDark | **Partial** |
| NeuralNetwork / Crypto exchanges | **Out of scope** |
| Live MT5 EveryTick goldens | **External / in progress** — `--capture` harness + gitignored demo login |

---

## Phase 1–5 — landed (see prior sections in git history)

MT5 order parity, Mass Builder, complex M1 islands, FixedLots / weekends / What-If / Negater, Martingale, Optimizer/Retester shells, WF matrix.

---

## Phase 6 — ATR MM, ranking DSL, task graph (**landed**)

- `RiskPolicy::AtrRiskPercent` through Scout / Judge / MQL5 volume mode `3`.
- Databank filter expression language (Rust + Dark UI box + CLI).
- QF-native Retester task graph JSON + planner.

---

## Phase 7 — in-process task executor + HTML export (**this wave**)

- `run_task_graph` executes Scout / Challenge / WF matrix / Judge / Export / Filter / What-If / Negate / HTML / multi-symbol steps in-process.
- `quantforge task-run` defaults to execute (`--dry-run` optional); `--work-dir` for artifacts.
- `quantforge export-html` for SaverHTML-style reports.
- Windows-native defaults for `export --compile` / `mt5-test` (no Wine required).
- StopLimit EveryTick golden `--capture` path (credentials via env or gitignored `.mt5-demo.local` only).

---

## Remaining parity gaps (honest)

| Gap | Status | Notes |
|-----|--------|-------|
| ATR / volatility MM | Present | SQX ATRRiskBasedSizing-style |
| Ranking filter DSL | Present | AND/OR/NOT + column aliases |
| Retester task chaining | Present | In-process executor |
| SQX project/task XML import | Blocked | Schema inside proprietary JARs |
| Crypto / Stocks MM | Out of scope | |
| Full in-process task executor | Present | |
| Live EveryTick goldens | External | Capture harness ready; needs successful tester run |
| Neural / crypto connectors | Out of scope | |

Protocol: `mt5-parity-v2`. Probes: `mql5/QuantForge/`, `scripts/family_mt5_parity.py`, `scripts/stop_limit_everytick_golden.py`.
