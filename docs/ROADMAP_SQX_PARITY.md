# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder`

Goal: **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for order families, and **(B)** an SQX-scale Builder funnel.

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Do **not** reverse-engineer proprietary JARs or paste licensed sources.

---

## Inventory (SQX → QuantForge)

| SQX surface | Status |
|-----|-----|
| AppBuilder / Mass Builder | **Present** |
| AppRetester / TaskRetest / task-run | **Present** — CLI + Dark UI |
| AppOptimizer / WF matrix / MC / What-If / Negater | **Present** |
| CrossCheckRetestOnAdditionalMarkets | **Present** — Discover gate + `multi-symbol-matrix` CLI/UI |
| MoneyManagement Fixed / Risk% / Martingale / ATR | **Present** |
| Crypto / Stocks / Neural | **Out of scope** |
| Databank ranking + correlation filters | **Present** |
| ProjectRetester XML | **Blocked forever** — QF JSON task graphs instead |
| SaverHTML / SaverPDF | **Present** — `export-html` + `export-results` pack (HTML/CSV/metrics/PDF) |
| ResultsTradeAnalysis / SkinDark | **Present** |
| Live EveryTick goldens | **Present (measured)** — StopLimit, management, stop, market |

---

## Measured EveryTick (model 4)

| Golden | Symbol | Family | Align | Net Δ | DD Δ |
|--------|--------|--------|-------|-------|------|
| `parity/stop_limit/golden_live_eurnzd` | EURNZD | StopLimit | 1/1 | ~0.14% | ~2.1% |
| `parity/management/golden_live_gbpusd` | GBPUSD | Limit+BE/trail/partial | 4/4 | ~0.32% | ~1.7% |
| `parity/stop/golden_live_audusd` | AUDUSD | BuyStop/SellStop | 5/5 | ~0.52% | ~1.3% |
| `parity/market/golden_live_usdjpy` | USDJPY | Market+SL/TP | 8/8 | ~0.86% | ~0.08% |

All **PASS** under mt5-parity-v2 (≥95% economics + trade alignment).

---

## Phase 10 — optional leftovers (**this wave**)

- Market-entry EveryTick golden (USDJPY).
- Saver-style results pack: HTML + trades CSV + metrics JSON + minimal PDF (`quantforge export-results`, Retester tab).
- Cross-symbol matrix with pairwise equity-signature correlations (`quantforge multi-symbol-matrix`, Retester tab).
- ROADMAP closed for in-scope work.

---

## Remaining (honest)

| Item | Status |
|------|--------|
| SQX project/task XML import | **Forever blocked** (proprietary JAR schemas) |
| Neural / crypto connectors | **Forever out of scope** |
| Further symbol goldens | Optional polish only — core families measured |

**In-scope SQX-parity work is effectively complete** aside from forever-blocked surfaces.
