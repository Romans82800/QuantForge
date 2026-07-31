# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank → retest/what-if/portfolio).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Inventory snapshot (SQX plugins / snippets → QuantForge)

| SQX surface (install) | QuantForge status |
|-----|-----|
| AppBuilder / TaskBuild / Mass Builder | **Present** — Discover Mass Builder + islands + prefilter |
| AppRetester / TaskRetest | **Present** — Retester workspace + in-process `quantforge task-run` + Dark UI Task Graph tab |
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
| Live MT5 EveryTick goldens | **Present (measured)** — StopLimit EURNZD EveryTick model 4 PASS on ICMarketsSC-Demo; M1 OHLC also archived |

---

## Phase 1–5 — landed (see prior sections in git history)

MT5 order parity, Mass Builder, complex M1 islands, FixedLots / weekends / What-If / Negater, Martingale, Optimizer/Retester shells, WF matrix.

---

## Phase 6 — ATR MM, ranking DSL, task graph (**landed**)

- `RiskPolicy::AtrRiskPercent` through Scout / Judge / MQL5 volume mode `3`.
- Databank filter expression language (Rust + Dark UI box + CLI).
- QF-native Retester task graph JSON + planner.

---

## Phase 7 — in-process task executor + HTML export (**landed**)

- `run_task_graph` executes Scout / Challenge / WF matrix / Judge / Export / Filter / What-If / Negate / HTML / multi-symbol steps in-process.
- `quantforge task-run` defaults to execute (`--dry-run` optional); `--work-dir` for artifacts.
- `quantforge export-html` for SaverHTML-style reports.
- Windows-native defaults for `export --compile` / `mt5-test` (no Wine required).
- StopLimit EveryTick golden `--capture` path (credentials via env or gitignored `.mt5-demo.local` only).

---

## Phase 8 — measured EveryTick golden + Retester UI (**this wave**)

- MQL5 StopLimit via `OrderOpen(... ORDER_TYPE_*_STOP_LIMIT ...)` for MetaTrader builds without `BuyStopLimit` helpers.
- `mt5-test` waits for fresh deals/equity/metadata (LiveUpdate-safe) and supports `--portable`.
- Live goldens committed under `parity/stop_limit/golden_live_eurnzd` (model 4) and `golden_live_eurnzd_m1` (model 1) — logins scrubbed; equity series downsampled in JSON.
- Dark UI **Task Graph** tab wires the same executor as CLI.

### Measured StopLimit EveryTick (model 4) — EURNZD 2024.01–02

| Metric | Result |
|--------|--------|
| Trade count | exact match (1 vs 1) |
| Trade alignment | PASS (60s timestamp tolerance) |
| Net profit relative Δ | ~0.14% |
| Max DD relative Δ | ~2.1% |
| Equity path | PASS |
| Verdict | **PASS** (well above ≥95% goal for this family probe) |

---

## Remaining parity gaps (honest)

| Gap | Status | Notes |
|-----|--------|-------|
| ATR / volatility MM | Present | SQX ATRRiskBasedSizing-style |
| Ranking filter DSL | Present | AND/OR/NOT + column aliases |
| Retester task chaining | Present | In-process executor + UI |
| SQX project/task XML import | Blocked | Schema inside proprietary JARs |
| Crypto / Stocks MM | Out of scope | |
| Full in-process task executor | Present | |
| Live EveryTick goldens (StopLimit) | Present | Measured PASS on demo; broaden to more symbols/order kinds next |
| Other order-family EveryTick goldens | Partial | Market/limit/stop families covered earlier; expand live captures |
| Neural / crypto connectors | Out of scope | |
| SaverPDF / full ResultsTradeAnalysis suite | Partial / polish | HTML present; PDF + deep trade views optional |
| DatabankFilterByCorrelation UI | Partial | Portfolio correlation exists; dedicated databank action polish remains |

Protocol: `mt5-parity-v2`. Probes: `mql5/QuantForge/`, `scripts/family_mt5_parity.py`, `scripts/stop_limit_everytick_golden.py`.
