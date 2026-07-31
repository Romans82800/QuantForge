# QuantForge × SQX / MT5 Parity Roadmap

Branch: `cursor/sqx-parity-builder` (from `cursor/is-oos1-oos2-parallel-challenge`)

Goal: ship **(A)** ≥95% MT5 Strategy Tester trade-aligned parity for all order types, and **(B)** an SQX-scale Builder funnel (cheap prefilter → mass generation → genetic islands → databank).

Legal: SQX at `C:\StrategyQuantX144` is a **behavioral reference only**. Reimplement contracts from public MT5 semantics + observed export shapes. Do **not** reverse-engineer `SQUANT.dat` or paste SQX licensed Java/FreeMarker/snippet sources into QuantForge.

---

## Phase 1 — MT5 order-type + execution parity (in progress)

**Scope**

- End-to-end **BuyStopLimit / SellStopLimit** (IR → Scout → M1 Judge → MQL5 export → unit/parity harness).
- Expand order-type probe coverage: market + limit + stop + stop-limit.
- Honest gap list toward 95% trade alignment vs Strategy Tester (EveryTick / M1).

**Success metrics**

- Fixed probe suite: ≥95% trades aligned (count + side + entry/exit within protocol `mt5-parity-v2` tolerances) on market, stop, limit, and stop-limit fixtures.
- Scout and Judge both honor two-stage stop-limit (trigger → limit fill) with expiry/cancel parity to existing pending rules.
- Exported EA uses `CTrade::BuyStopLimit` / `SellStopLimit` with stop + limit prices.

**Non-goals (Phase 1)**

- Genetic islands / SQX Builder throughput (Phase 2).
- Enabling pending/BE/trail in default Discover genes (Phase 3).
- Tick-level EveryTick engine rewrite (tighten same-bar/minute approximations only as needed for probes).

---

## Phase 2 — SQX-like mass Builder

**Scope**

- Cheap reject stage (fast Scout / filters) before expensive M1 Judge.
- Mass generation + **genetic islands** (replacing / extending MAP-Elites as the scale path).
- Databank UX throughput for storing, ranking, and promoting candidates.

**Success metrics**

- Sustained candidates/hour at SQX-comparable funnel ratios (cheap reject ≫ Judge).
- Island isolation + periodic migration documented and measurable.
- Databank write path keeps up with generation without blocking search.

---

## Phase 3 — Discover enables pending / BE / trail with M1 precision islands

**Scope**

- Turn on stop / limit / stop-limit / BE / trail / partials as Discover genes (still opt-in flags).
- Route enabled execution genes through M1-precision islands.

**Success metrics**

- Feature-flagged runs produce valid IR + export for all enabled entry kinds.
- Robustness / challenge gates remain green on M1 Judge for promoted elites.

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
