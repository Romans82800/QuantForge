# QuantForge

QuantForge is a local-first systematic strategy research environment targeting
MetaTrader 5. Its core is written in Rust and its desktop shell uses Tauri and
React.

The repository implements the **QuantForge v1 MVP**. The implementation establishes:

- the normative Rust crate layout;
- deterministic content hashes and float quantisation policy;
- validated, hashable broker symbol specifications;
- immutable run manifests and atomic/versioned JSON writes;
- MT5 CSV/TSV ingestion and deterministic data-quality reports;
- a typed export-safe strategy IR with canonical structural fingerprints;
- a deterministic completed-bar OHLC scout with trade and equity outputs;
- typed grammar seeds, parameter mutation and bounded structural mutation;
- parallel, reproducible candidate evaluation with ordered deposits;
- a 972-cell default MAP-Elites behavioral grid;
- global clone and empty-niche equity-correlation rejection;
- novelty-aware parent selection and resumable databank JSON checkpoints;
- deterministic Strategy IR to guarded MQL5 generation;
- live trading disabled by default and broker-aware `OrderCalcProfit` sizing;
- `.set`, tester INI and source-hashed evidence-card exports;
- MetaEditor compilation through native Windows or Wine paths;
- bounded MT5 Strategy Tester execution with fresh-output verification;
- tester deal, equity and terminal-metadata recording;
- aligned trade/equity comparison with a hard machine-readable parity gate;
- a full same-IR M1 Judge with broker stop enforcement and chronological fills;
- decision-bar/M1 reconstruction checks and missing-minute rejection;
- broker-local session gates, scheduled synthetic spreads and rollover swap
  accounting;
- a numerical MT5 indicator reference pack covering all 13 export-safe
  indicators;
- immutable chronological development, validation and sealed-final split plans;
- a typed certification decision that binds every gate to one strategy and
  broker identity;
- hard denial for research overrides, reused artifacts, failed Judge or
  non-external parity, unsafe sealed access and failed indicator parity;
- a Certified-only Vault that re-runs the gate inside storage and refuses a
  second entry for the same evidence;
- validation-only purged walk-forward folds with indicator warm-up but no
  pre-fold entries;
- deterministic cost-shock, moving-block trade bootstrap and parameter
  neighbourhood batteries;
- unavoidable evaluation-count reporting with expected lucky-performance and
  deflated trade-Sharpe proxies;
- a shortlist-only sealed-final evaluator with Challenge-relative stricter
  thresholds;
- a durable pre-open claim that prevents repeated sealed testing after passes,
  failures, crashes or input errors;
- strict semantic adapters for development-only MAP-Elites, validation,
  Challenge, M1 Judge, external MT5 parity, indicator parity and sealed-final
  artifacts;
- a separately hashed validation attestation derived from the immutable
  Challenge baseline;
- a no-clobber certification bundle whose file hashes are rechecked by
  `certify --bundle` before Vault admission;
- deterministic low-correlation portfolio packing from MAP-Elites databanks;
- hard strategy-weight, pairwise-correlation, family-exposure, symbol-exposure
  and strategy-count constraints;
- full seeded circular moving-block portfolio stress records with return tail,
  CVaR and drawdown percentiles;
- Certified-only deployment packs that regenerate the exact external
  parity-passed EA source and settings;
- atomic new-directory materialization of `.mq5`, `.set`, tester, IR, broker,
  evidence, risk, changelog and deployment-manifest files;
- a hard live-disabled deployment default with no post-parity configuration
  overrides;
- append-only paper-incubation starts and daily balance/drawdown/trade records;
- immutable kill rules for daily loss, total drawdown, observation duration,
  trade count and zero-trade streaks;
- one-shot passing or failed incubation finals bound into certification and
  required by deployment;
- a Tauri 2 + React/TypeScript desktop shell with a locked-down local IPC
  surface and generated macOS/Windows/Linux application icons;
- functional Home, Data Lab and Discover workspaces backed by typed Tauri
  commands for broker-bound diagnostics and deterministic new/continued jobs;
- certification-grade desktop discovery that reserves validation and sealed
  partitions before evaluating candidates on development data only;
- a read-only Databank workspace that validates the same run manifests,
  fingerprints, stored gates and niche identities as the CLI;
- native Challenge, sealed-final, M1 Judge, EA export, external/indicator
  parity, incubation, evidence assembly, Vault, Portfolio and Deploy workspaces;
- complete 972-cell behavioral coverage maps, unavoidable evaluation-count
  warnings, rejection telemetry, evidence/novelty clustering, a virtualized
  elite table and lazy IR/equity inspection.

The core v1 workflow is complete in both the CLI and the native desktop. The
desktop calls typed Rust application commands directly; it never shells out to
an ambient executable or bypasses engine validation for visual convenience.

## Reproducible demo

Create a small deterministic databank from the included MT5-format fixture,
continue it for a second epoch, and retain the first checkpoint as a versioned
backup:

```sh
./scripts/quantforge-demo.sh /tmp/quantforge-demo
```

On macOS, pass `--launch` as the second argument to open an already-built native
bundle after the artifacts are ready:

```sh
./scripts/quantforge-demo.sh /tmp/quantforge-demo --launch
```

This fixture is intentionally tiny and demonstrates determinism, continuation,
artifact validation and the desktop workflow—not profitable performance. Its
elites can have zero trades and therefore zero QD score. Use complete broker
history and promotion-grade gates for actual research.

## IC Markets data pack

The local pack lives at `ICMarkets_EST7_2020_present/` (gitignored; ~3.2 GB with
M1). Download the published assets from the
[data-icmarkets-est7-20260803](https://github.com/Romans82800/QuantForge/releases/tag/data-icmarkets-est7-20260803)
release and extract into `~/Documents/QuantForge/ICMarkets_EST7_2020_present/`
(or set `QUANTFORGE_DATA_PACK`).

Source files named `*_TickData.csv` are Ask/Bid quote dumps (often several rows
per minute), not OHLC. QuantForge aggregates midpoint quotes to M1, then builds
H1. Timezone: `ICMarkets/EST+7`. Raw ~6.8 GB CSVs are not stored in git.

## Build

```sh
cargo test --workspace
cargo run -p quantforge-cli -- load-csv path/to/EURUSD_M15.csv --metadata path/to/EURUSD_M15.metadata.csv
cargo run -p quantforge-cli -- data-quality path/to/EURUSD_M15.csv --metadata path/to/EURUSD_M15.metadata.csv
```

Run the desktop with Node.js 20.19+ and the normal Tauri platform prerequisites:

```sh
cd apps/desktop
pnpm install
pnpm tauri dev
```

Use Data Lab to inspect an OHLC export with metadata (or an explicit timezone)
and an optional broker profile. New desktop searches reserve 20% validation and
20% sealed-final history by default, evaluating candidates on the development
partition only. Databank can export an elite's exact strategy IR; Challenge,
Parity Lab, Vault, Portfolio and Deploy then carry that identity through the
full promotion chain. See [the desktop workspace](apps/desktop/README.md).

### Leakage-safe meta-selection research

The research-only `meta-learn` command fits an interpretable logistic model on
IS-side strategy evidence and evaluates it on later validation windows. The
recommended configuration uses 6-month labels for training and a 12-month label
for confirmation. The final sealed window is evaluated only after the model has
been fit; sealed rows can never enter training. Asset identity is excluded by
default and can be enabled explicitly with `--include-asset-identity`.

Build the input directly from existing Databank elites with `meta-build`. Each
origin is an IS snapshot plus one or more later windows sharing its feature
cutoff. The source is truncated before every label end, and incomplete trades
at the boundary are excluded. A minimal spec is:

```json
{
  "origins": [
    {
      "id": "usdjpy-2016",
      "databank": "runs/USDJPY/Databank/USDJPY_H1_2016_databank.json",
      "source": "ICMarketsSC-Demo_USDJPY_H1_2016_present.tsv",
      "metadata": "ICMarketsSC-Demo_USDJPY_H1_2016_present.metadata.csv",
      "m1Source": "ICMarketsSC-Demo_USDJPY_M1_2016_present.tsv",
      "m1Metadata": "ICMarketsSC-Demo_USDJPY_M1_2016_present.metadata.csv",
      "featureSource": "snapshots/USDJPY_H1_through_2023.tsv",
      "featureMetadata": "snapshots/USDJPY_H1_through_2023.metadata.csv",
      "featureM1Source": "snapshots/USDJPY_M1_through_2023.tsv",
      "featureM1Metadata": "snapshots/USDJPY_M1_through_2023.metadata.csv",
      "featureEndDate": "2024-01-01",
      "pool": "elites",
      "broker": "USDJPY.broker.json",
      "asset": "USDJPY",
      "windows": [
        {
          "id": "usdjpy-2016-validation",
          "role": "validation",
          "featureCutoffTimestampMs": 1704067199000,
          "labelStartTimestampMs": 1704067200000,
          "labelEndTimestampMs": 1719792000000,
          "horizonMonths": 6
        }
      ]
    }
  ]
}
```

Run it with:

```sh
cargo run -p quantforge-cli -- meta-build \
  --spec meta-build.json \
  --out meta-learning-input.json
```

Use separate origins for separate historical feature cutoffs. When `m1Source`
is supplied, QuantForge rebuilds the decision timeframe from M1 using the same
grid as Evolve. `featureSource` and `featureM1Source` must be the exact
snapshots used to create the Databank: the rebuilt decision-data hash must
match the Databank and its last bar must not be later than the feature cutoff.
This means a full present-day source cannot be relabeled as an old IS snapshot.
The builder replays the selected Databank pool (`elites` by default); use
`pool: "holding"` when the historical run deferred its heavy robustness
battery. It does not change trading logic or promote candidates.

For new historical Databanks, pass the same cutoff directly to Evolve; H1 and
M1 are clipped together without copying the source files:

```sh
cargo run -p quantforge-cli -- evolve ... \
  --end-date 2024-01-01 \
  --databank runs/USDJPY/Databank/USDJPY_H1_through_2023.json
```

```sh
cargo run -p quantforge-cli -- meta-learn \
  --input meta-learning-input.json \
  --out meta-learning-report.json
```

`meta-learning-input.json` contains chronological `windows`, IS-only
`candidates.features` (including fold, recovery, parameter-neighbourhood,
trade-count, family and complexity evidence), and later `outcomes` keyed by
window ID. The report includes AUC, precision@K, Brier/ECE calibration and the
future-expectancy lift of selected versus unselected candidates. This pipeline
does not alter Discover or trading execution logic.

For the research target that matters most—future expectancy in R—use the
separate `meta-expectancy` command. It fits an interpretable ridge regression
on the same IS-only evidence, ranks candidates by predicted later expectancy,
and reports RMSE, rank correlation and selected-versus-unselected future
expectancy. It uses the same walk-forward and sealed-window rules, and asset
identity remains excluded unless explicitly enabled.

```sh
cargo run -p quantforge-cli -- meta-expectancy \
  --input meta-learning-input.json \
  --out meta-expectancy-report.json
```

The binary survival model and the direct-expectancy model are deliberately
separate research paths. A good survival AUC is not sufficient evidence that
the selected strategies have higher expectancy; the direct-expectancy report's
sealed lift is the relevant decision metric for this objective.

### One-shot H4 production-recipe bakeoff

To decide whether the current Holding robustness battery is removing viable H4
strategies, run one frozen-cohort comparison. The command uses the saved
Databank and battery report, applies only unsealed basic gates and ranking, and
replays every selected strategy once on the sealed final partition with the
existing M1 judge and broker costs. It compares the strict battery, an
`IS expectancy × sqrt(trade count)` lane, and a same-size seeded random control.

```sh
cargo run -p quantforge-cli -- production-bakeoff H1_DATA.tsv \
  --metadata H1_DATA.metadata.csv \
  --m1 M1_DATA.tsv \
  --m1-metadata M1_DATA.metadata.csv \
  --databank runs/USDJPY/Databank/USDJPY_H4_databank.json \
  --battery-report runs/USDJPY/Databank/USDJPY_H4_databank_battery.csv \
  --broker USDJPY.broker.json \
  --expected-cohort-size 52 \
  --out runs/USDJPY/production-bakeoff.json
```

If a matching `<M1_DATA>.quotes.csv` exists, it is used automatically; pass
`--quote-path` to select it explicitly. The output is no-clobber and contains
the cohort IDs, rejection counts, hashes, split boundaries, seed, scoring
formula, all sealed metrics and calendar-year results, plus the precommitted
adoption decision. A mismatch between the saved cohort, battery rows or data
hashes stops the run without writing a report.

### H4 Production Lane v1

Open an H4 Discover archive, go to **Holding**, and choose **Run Production
Lane v1**. The lane freezes the complete Holding cohort, rebuilds H4 from the
archive's bound M1 chronology, and refuses to run unless the reconstructed
Development hash exactly matches the archive.

Each strategy is replayed on Development M1 only. Basic gates plus positive
6- and 12-month median expectancy/coverage determine eligibility; 3-month,
calendar-year, Monte Carlo and parameter-neighbourhood results are retained as
warnings rather than extra automatic rejections. Eligible strategies are
ranked by `Development expectancy R × sqrt(trade count)`, with 12-month median,
6-month median, recovery factor and lower drawdown as deterministic
tie-breakers. The top 20% are promoted subject to the existing niche, family
and correlation limits. Non-selected strategies remain in Holding.

The command writes a no-clobber `*_production_lane_v1_*.json` report beside the
Databank before promotion. It records the source artifact hash, exact split
boundary, fixed configuration, every candidate decision and selected IDs. The
sealed final partition is not loaded into the replay or selector; evaluate it
only later through the existing one-shot Sealed Final workflow after the
recipe and cross-asset shortlist are frozen.

Write a quality report and its run manifest with:

```sh
cargo run -p quantforge-cli -- data-quality data.csv \
  --metadata data.metadata.csv --out quality.json
```

For non-MT5 data without exporter metadata, pass its timezone explicitly with
`--source-timezone Etc/UTC` (or another IANA timezone). Naive timestamps are
never silently interpreted as UTC.

Run the completed-bar scout with explicit strategy, broker and cost inputs:

```sh
cargo run -p quantforge-cli -- scout data.tsv \
  --metadata data.metadata.csv \
  --strategy strategy.json \
  --broker broker.json \
  --commission-per-lot-round-turn 7.0 \
  --slippage-points-per-side 1.0 \
  --out scout-result.json
```

Replay a Scout elite on M1. Both datasets must come from the same broker
timezone, every decision bar must have complete M1 coverage, and recorded M1
spreads are used when present:

```sh
cargo run -p quantforge-cli -- judge EURUSD_M15.tsv \
  --metadata EURUSD_M15.metadata.csv \
  --m1 EURUSD_M1.tsv \
  --m1-metadata EURUSD_M1.metadata.csv \
  --strategy strategy.json \
  --broker broker.json \
  --commission-per-lot-round-turn 7.0 \
  --slippage-points-per-side 1.0 \
  --out judge-result.json
```

Judge evaluates the IR on the decision timeframe, then replays entry, gap,
stop, target and mark-to-market execution minute by minute. Each M1 slice must
reconstruct its parent bar's open, high, low and close within half a tick.
Same-minute stop/target collisions remain conservative and are counted rather
than hidden.

When spread is absent from a bar, both Scout and Judge resolve it in this order:
the matching broker-local `synthetic_spreads` window, then the explicit CLI
fallback. Recorded spread always takes precedence. For example, a broker profile
may contain:

```json
"synthetic_spreads": [
  {
    "day": "monday",
    "open_minute": 0,
    "close_minute": 420,
    "spread_points": 14.0
  },
  {
    "day": "monday",
    "open_minute": 420,
    "close_minute": 1320,
    "spread_points": 7.0
  }
]
```

New entries are allowed only inside `sessions` when that list is non-empty.
Broker-midnight swap is posted into balance and each trade's `swap` field, using
the seven MT5 daily multipliers when present (or the legacy triple-day rule)
without invented weekend rollovers. Points, deposit, symbol/base, profit,
margin-currency and current/open-interest modes are supported when the bound
symbol currencies permit conversion to account currency. MT5 reopen swap modes
remain rejected because they reset the position price basis.

Create a discovery databank with explicit broker-cost assumptions:

```sh
cargo run -p quantforge-cli -- evolve data.tsv \
  --metadata data.metadata.csv \
  --broker broker.json \
  --databank eurusd-bank.json \
  --initial 500 \
  --generations 50 \
  --batch 200 \
  --correlation 0.88 \
  --seed 42 \
  --commission-per-lot-round-turn 7.0 \
  --slippage-points-per-side 1.0
```

Continue it for another bounded epoch. The stored data, broker, grammar, seed,
cost and gate configuration is immutable:

```sh
cargo run -p quantforge-cli -- evolve data.tsv \
  --metadata data.metadata.csv \
  --broker broker.json \
  --databank eurusd-bank.json \
  --continue \
  --generations 50
```

The generator starts with trend, momentum, breakout and mean-reversion grammar
families. Each candidate is evaluated by Scout, checked against minimum-trade,
return, profit-factor and drawdown gates, fingerprinted, assigned a behavioral
niche and deposited in deterministic order. Parent tournaments combine evidence
with novelty so continued search does not optimize a single strategy basin.

Pack eligible databank elites under hard diversification constraints. The v1
packer uses equal weights, searches feasible portfolio sizes deterministically,
and never relaxes a cap to force a result:

```sh
cargo run -p quantforge-cli -- portfolio eurusd-bank.json \
  --broker broker.json \
  --objective risk-adjusted-return \
  --maximum-pairwise-correlation 0.70 \
  --maximum-weight-per-strategy 0.25 \
  --maximum-family-exposure 0.50 \
  --maximum-strategies 10 \
  --minimum-return-percent 2.0 \
  --stress-trials 1000 \
  --stress-block-length 5 \
  --seed 42 \
  --out eurusd-portfolio.json
```

The report contains exact allocations, all selected pairwise correlations,
family and symbol exposures, a reconstructed portfolio return path and every
moving-block stress trial. See [portfolio packing](docs/portfolio.md).

Export a strategy into the corresponding `MQL5/Experts` subdirectory and
compile it. On macOS, the installed MetaTrader Wine environment is detected:

```sh
cargo run -p quantforge-cli -- export \
  --strategy strategy.json \
  --broker broker.json \
  --out "/path/to/MetaTrader 5/MQL5/Experts/QuantForge" \
  --expert-directory QuantForge \
  --expert-name MyStrategy \
  --timeframe M15 \
  --from-date 2021.01.01 \
  --to-date 2026.01.01 \
  --commission-per-lot-round-turn 7.0 \
  --compile
```

The export contains `MyStrategy.mq5`, `.ex5` after compilation, `.set`,
`.tester.ini`, `.evidence.json` and `.compile.json`. The EA refuses non-tester
execution unless `InpAllowLiveTrading` is explicitly enabled.

Run the bounded tester configuration. The generated EA records deals, equity
and terminal metadata under MT5 Common Files:

```sh
cargo run -p quantforge-cli -- mt5-test \
  --tester-ini MyStrategy.tester.ini \
  --evidence MyStrategy.evidence.json \
  --out mt5-run.json \
  --timeout-seconds 1800
```

Compare those outputs against the manifest-bound Scout run:

```sh
cargo run -p quantforge-cli -- parity \
  --scout-result scout-result.json \
  --evidence MyStrategy.evidence.json \
  --mq5 MyStrategy.mq5 \
  --mt5-deals QF_<fingerprint>_deals.csv \
  --mt5-equity QF_<fingerprint>_equity.csv \
  --mt5-metadata QF_<fingerprint>_metadata.csv \
  --trade-timestamp-tolerance-ms 900000 \
  --out parity-report.json
```

The parity command binds the Scout strategy and broker hashes to the export,
re-hashes the exact MQL5 source, verifies mandatory protective-order calls and
checks tester metadata before evaluating trade count, aligned trades, net
profit, drawdown and equity-path divergence.

To validate indicator semantics, compile and run
`mql5/QuantForge/QuantForgeIndicatorParityProbeEA.mq5` with the supplied tester
configuration. Then compare its Common Files output:

```sh
cargo run -p quantforge-cli -- indicator-parity \
  --reference indicator_parity.csv \
  --warmup-rows 1000 \
  --out indicator-parity.json
```

The probe binds terminal build, broker, server, symbol, timeframe and period to
the reference rows. It caught and corrected an ATR smoothing difference during
implementation; the current Rust buffers pass the local IC Markets terminal
reference across SMA, EMA, WMA, RSI, ATR, Donchian high/low, highest/lowest,
standard deviation, z-score, percentile-in-range and rate-of-change.

Freeze the data protocol before discovery. The command stores only partition
boundaries, counts and hashes; the final 20% is marked as sealed and must never
enter a selection score:

```sh
cargo run -p quantforge-cli -- split-plan EURUSD_M15.tsv \
  --metadata EURUSD_M15.metadata.csv \
  --validation-fraction 0.20 \
  --sealed-fraction 0.20 \
  --out EURUSD_M15.split.json
```

Challenge an Illuminated candidate strictly on the validation partition. The
originating research evaluation count is mandatory and becomes part of the
multiple-testing record:

```sh
cargo run -p quantforge-cli -- challenge EURUSD_M15.tsv \
  --metadata EURUSD_M15.metadata.csv \
  --strategy strategy.json \
  --broker broker.json \
  --split-plan EURUSD_M15.split.json \
  --evaluations-touched 4200 \
  --commission-per-lot-round-turn 7.0 \
  --slippage-points-per-side 1.0 \
  --out challenge.json
```

The artifact stores baseline validation results, each fold boundary and purge /
embargo count, every cost point, Monte Carlo tail statistics, every parameter
neighbor and selection-bias metrics. A failure still writes the report for
audit, then returns a failing process status. See [the Challenge protocol](docs/challenge.md).

Open the sealed final partition only after Challenge passes. Cost and balance
arguments must exactly match the Challenge, while at least one final threshold
must be stricter:

```sh
cargo run -p quantforge-cli -- sealed-final EURUSD_M15.tsv \
  --metadata EURUSD_M15.metadata.csv \
  --strategy strategy.json \
  --broker broker.json \
  --split-plan EURUSD_M15.split.json \
  --challenge challenge.json \
  --sealed-root sealed-attempts/ \
  --commission-per-lot-round-turn 7.0 \
  --slippage-points-per-side 1.0
```

Before loading the market data, the command writes an immutable `sealed-open`
claim keyed by strategy and split-plan hashes. The same key can never be opened
again through that ledger, even if evaluation fails or the process stops. The
final report is stored beside the claim. See [the sealed-final protocol](docs/sealed-final.md).

Open a paper-incubation ledger after the strategy and split identity are fixed,
then append one observation per paper-trading day and seal it when the minimum
period is complete:

```sh
cargo run -p quantforge-cli -- incubation-start \
  --strategy strategy.json \
  --broker broker.json \
  --split-plan EURUSD_M15.split.json \
  --root incubation/ \
  --start-date 2026-07-22 \
  --initial-balance 100000

cargo run -p quantforge-cli -- incubation-record \
  --start incubation/<strategy-fingerprint>/<split-hash>/incubation-start.json \
  --date 2026-07-22 \
  --ending-balance 100150 \
  --maximum-drawdown-percent 0.8 \
  --trade-count 2

cargo run -p quantforge-cli -- incubation-finalize \
  --start incubation/<strategy-fingerprint>/<split-hash>/incubation-start.json
```

The workflow is continuous in the operational sense: append a record after
each paper-trading day. It does not yet poll MT5 automatically. A final records
both passes and failures immutably, so a breached ledger cannot be reset and
retried under the same strategy/split identity. See [paper incubation](docs/incubation.md).

When all machine-produced gate reports exist, assemble them. The databank must
have been evolved only on the split plan's development partition; Challenge,
Judge and parity must use its validation partition:

```sh
cargo run -p quantforge-cli -- assemble-evidence \
  --strategy strategy.json \
  --broker broker.json \
  --split-plan EURUSD_M15.split.json \
  --databank databank.json \
  --challenge challenge.json \
  --judge judge.json \
  --parity mt5-parity.json \
  --indicator-parity indicator-parity.json \
  --sealed-final sealed-final.json \
  --incubation incubation-final.json \
  --out-dir assembled-evidence/

cargo run -p quantforge-cli -- certify \
  --strategy strategy.json \
  --broker broker.json \
  --split-plan EURUSD_M15.split.json \
  --bundle assembled-evidence/certification-bundle.json \
  --require-incubation \
  --vault vault/
```

Assembly writes an immutable validation attestation, the typed
`CertificationEvidence`, and a bundle containing the exact path and SHA-256 of
each gate file. It refuses failed reports, candidate/data/cost mismatches,
research overrides, internal-engine parity, incomplete indicator packs and
sealed reports not tied to the exact Challenge artifact. `certify --bundle`
re-hashes those files, so changes after assembly fail before Vault evaluation.

Certification requires passing validation, illumination, Challenge, M1 Judge,
external MT5 Strategy Tester parity, indicator parity and sealed-final claims.
It also requires mandatory protective orders, a pre-sealed shortlist, no sealed
selection score and no research overrides. `--require-incubation` adds the
incubation gate; deployment accepts only entries certified with that policy. A
denied request prints its machine-readable blockers and does
not create a Vault directory. See [the certification protocol](docs/certification.md).

Materialize the exact EA and settings that passed external MT5 parity from the
resulting Certified Vault entry:

```sh
cargo run -p quantforge-cli -- deploy \
  --vault-entry vault/<strategy-fingerprint>/<entry-id>.certified.json \
  --out deployment-packs/MyStrategy/
```

Deployment re-evaluates the vaulted certification, re-hashes every referenced
gate artifact, recomputes the external parity diff, and regenerates the EA using
the stored parity configuration. If the source hash differs, deployment fails
and writes nothing. The completed pack remains live-disabled and includes a
risk pack plus operator changelog. See [deployment packs](docs/deployment.md).

The current Scout and Judge support every export-safe indicator, distinct long/short
entries, market plus stop/limit entries with bar-based expiry, fixed/ATR/range
placement and protective distances, fixed/ATR/R targets, break-even, R/ATR
trailing stops, partial exits, time stops and broker-day flattening. Management
updates use completed bars and become active at the next bar open in Scout,
M1 Judge and generated MT5 code. Favorable extremes for BE/trail/partials are
fill-aware: the entry bar/minute contributes only its close; later bars use
high/low; pre-entry minutes are ignored (no OHLC path lookahead). Stop
modifications that would already trigger at the new bar open are rejected
(matching MT5 `PositionModify`), and entry/SL/TP prices are digit-normalized
like MT5 `NormalizeDouble`. Spread, slippage, commission-aware risk sizing,
conservative same-bar ambiguity, session gates, swap, trade blotters and
mark-to-market equity remain aligned across the engines. MT5 reopen-price swap
modes are still rejected because they require a different position-cost basis.

Discovery's current cheap gates operate on the supplied dataset as a Scout
proxy, with M1 Judge intended for elites only. Promotion-grade discovery must
therefore be run on a development-partition export whose canonical bar hash
matches the split plan. Automatic evidence assembly, semantic cross-artifact
validation, one-shot sealed-final evaluation and bundle-based Vault admission
are implemented. The append-only paper-incubation producer is implemented; it
is optional for research certification but mandatory for deployment. A
Challenge, sealed, parity or incubation report alone is never certification.

## Product invariants

- A run recipe is tied to hashes of its data and broker specification.
- Scored outputs never depend on unordered map iteration.
- Export-safe strategies use completed bars and mandatory protective exits.
- Mutable artifacts are written atomically with a timestamped backup.
- Certification requires both a passing Judge result and an external MT5
  Strategy Tester parity result; neither can substitute for the other.
- Sealed-final data cannot contribute to selection evidence, and a sealed
  failure returns the candidate to `Illuminated`.
- A sealed attempt is claimed before data loading; changing the output filename
  cannot create another attempt under the same sealed ledger.
- Vault admission is Certified-only and immutable through QuantForge's storage
  API. It is an audit boundary, not protection against a machine owner manually
  changing files on disk.

## Export from MetaTrader 5

Compile and attach
[`QuantForgeHistoryExporterEA.mq5`](mql5/QuantForge/QuantForgeHistoryExporterEA.mq5)
to a normal connected chart. Set the symbol, M1 or H1 timeframe, date range,
dataset name, verified broker timezone and commission. The non-trading EA
publishes a TSV and metadata CSV under the terminal's Common Files directory.

See [docs/architecture.md](docs/architecture.md) for the implementation order.
