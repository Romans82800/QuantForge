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
| Databank columns / ranking filters | **Present** — expression DSL + UI |
| DatabankFilterByCorrelation | **Present** — `quantforge databank-correlate` + Databank UI action |
| SettingsFiltering / TaskFiltering | **Partial** — expression DSL covers ranking filters; no SQX task XML import |
| ProjectRetester XML | **Documented alternative** — QF JSON task graph (SQX XML not publicly schema-documented) |
| SaverHTML / results HTML | **Present** — `html_report` task step + `quantforge export-html` |
| ResultsTradeAnalysis | **Present** — denser trade list (R, bars, volume, commission) + win/avg/expectancy strip; equity+balance+DD chart |
| SkinDark | **Present** — single SQX-blue Dark palette (teal override removed) |
| NeuralNetwork / Crypto exchanges | **Out of scope** |
| Live MT5 EveryTick goldens | **Present (measured)** — StopLimit EURNZD; management GBPUSD; Stop AUDUSD |

---

## Phase 1–8 — landed

MT5 order parity, Mass Builder, ATR MM, ranking DSL, in-process task executor, HTML export, measured StopLimit EveryTick, Retester Task Graph UI.

---

## Phase 9 — broaden goldens + Results / correlation (**this wave**)

- Generalized EveryTick capture (`--strategy` / `--family` / `--expert-name`).
- Live goldens: `parity/management/golden_live_gbpusd` (limit+BE/trail/partial), `parity/stop/golden_live_audusd` (BuyStop/SellStop).
- Databank correlation filter (CLI + Dark UI).
- Results trade analysis denser; equity chart shows balance + drawdown band; SkinDark accent unified.

### Measured EveryTick (model 4) summary

| Golden | Symbol | Family | Trades | Net Δ | DD Δ | Align | Verdict |
|--------|--------|--------|--------|-------|------|-------|---------|
| `golden_live_eurnzd` | EURNZD | StopLimit | 1/1 | ~0.14% | ~2.1% | 1/1 | **PASS** |
| `golden_live_gbpusd` | GBPUSD | Management (limit+BE/trail/partial) | 4/4 | ~0.32% | ~1.7% | 4/4 | **PASS** |
| `golden_live_audusd` | AUDUSD | Stop pending | 5/5 | ~0.52% | ~1.3% | 5/5 | **PASS** |

---

## Remaining parity gaps (honest)

| Gap | Status | Notes |
|-----|--------|-------|
| SQX project/task XML import | Blocked | Schema inside proprietary JARs |
| Crypto / Stocks MM / Neural | Out of scope | |
| Market-only EveryTick golden archive | Optional polish | Engine covered; family harness exists |
| SaverPDF | Optional | HTML present |
| Multi-symbol Retester matrix UI | Partial | Discover gate + task step present |

Protocol: `mt5-parity-v2`. Capture: `scripts/stop_limit_everytick_golden.py`.
