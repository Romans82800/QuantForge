# QuantForge research handoff

This is a factual handoff for another agent. It separates measured results
from conclusions and from work that was only proposed.

## Objective

The user's objective is not to maximize in-sample score. It is to produce H4
strategies or a selector that generalizes to a later holdout, with a target of
roughly `0.1R` or more out of sample and a majority of selected strategies or
holdout blocks positive. Asset identity should not be used by a selector unless
explicitly requested.

Important leakage rule: the final sealed partition may be opened once after the
recipe is frozen. The USDJPY H4 sealed bakeoff below has now been viewed, so it
must be treated as a research result, not as a virgin holdout for a later
recipe change.

## Data and execution

- Primary recent experiment: USDJPY, ICMarkets, H1 decision source plus M1
  execution source, 2016-present.
- H4 decision bars were reconstructed from M1 using the quote-attached H4
  path used by the desktop application.
- Broker costs: commission `7.0` per lot round turn, existing broker profile,
  recorded bid/ask quote sidecar where available.
- Promotion split in the saved H4 artifact: `validation_fraction=0.0`,
  `sealed_fraction=0.30`.
- The H4 2016 sealed bakeoff covers the later calendar blocks 2023-2026.
- The CLI now refuses to write an artifact if the output already exists, if
  the saved split is not promotion-grade, if the H4 data hash cannot be
  reconstructed exactly, or if the frozen cohort size does not match the
  requested expected size.

## Robustness battery results

The battery short-circuits on the first failed gate. Therefore later gates do
not get measured for candidates rejected earlier.

### USDJPY H1, 2016-present

Artifact:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H1_2016_databank_1787150329289.json`

Battery:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H1_2016_databank_1787150329289_battery.csv`

- 122 candidates tested.
- 100 passed; 22 rejected.
- Rejected: 19 Monte Carlo, 3 sequential walk-forward.
- Saved databank: 102 elites and 22 Holding candidates after 162,000
  evaluations and 158 generations.
- The artifact has `promotion_split=true`, `validation_fraction=0.0`, and
  `sealed_fraction=0.3`.
- The saved elites do not contain non-null OOS1 expectancy values, so this is
  not evidence of H1 holdout performance.

### USDJPY H4, 2016-present, earlier run

Battery:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H4_2016_databank_1787270222383_battery.csv`

- 24 tested, 0 passed.
- 22 rejected by calendar-year fold stability.
- 2 rejected by parameter neighborhood / Ret/DD band.

### USDJPY H4, 2016-present, final frozen cohort

Artifact:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H4_2016_databank_1787271032087.json`

Battery:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H4_2016_databank_1787271032087_battery.csv`

- 52 tested, 0 passed.
- 44 rejected by calendar-year fold stability.
- 8 rejected by parameter neighborhood / Ret/DD band.
- All 52 passed the basic eligibility gates used by the bakeoff.
- Saved artifact: 52 Holding candidates, 0 promoted elites, 80 completed
  generations.

### USDJPY H4, 2020-present

Artifact:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H4_2020_databank_1787270725395.json`

Battery:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/USDJPY/Databank/USDJPY_H4_2020_databank_1787270725395_battery.csv`

- 91 tested, 0 passed.
- 61 rejected by calendar-year fold stability.
- 27 rejected by parameter neighborhood / Ret/DD band.
- 2 rejected by Monte Carlo.
- 1 rejected by CPCV folds.

### Other saved battery runs

- CHFJPY H1 2016 run `1787174909924`: 3 tested, 0 passed, all parameter
  neighborhood.
- CHFJPY H1 2016 run `1787177728010`: 84 tested, 4 passed, 78 parameter
  neighborhood rejects, 2 Monte Carlo rejects.
- CHFJPY H1 2016 run `1787178670813`: 9 tested, 0 passed, all parameter
  neighborhood.
- CHFJPY H1 2016 run `1787179958054`: 5 tested, 0 passed, all parameter
  neighborhood.
- CHFJPY H1 2020 run `1787179101344`: 208 tested, 1 passed, 205 parameter
  neighborhood rejects, 2 calendar-year fold rejects.
- EURJPY H1 2016 run `1787222719008`: 8 tested, 3 passed, 5 Monte Carlo
  rejects.

These results show that the battery can pass candidates on some H1 runs, but
the H4 cohorts tested here were eliminated almost completely before later
tests could be observed.

## One-shot H4 production-recipe bakeoff

Implementation:
`/Users/danielagbonkpolor/Documents/QuantForge.worktrees/qf-ui-simplification/crates/quantforge-discover/src/production_bakeoff.rs`

CLI implementation:
`/Users/danielagbonkpolor/Documents/QuantForge.worktrees/qf-ui-simplification/crates/quantforge-cli/src/main.rs`

Report:
`/tmp/USDJPY_H4_production_bakeoff_20260821_v2.json`

Frozen inputs:

- 52 exact battery rows / candidate fingerprints.
- Seed `42`.
- Selection fraction `20%`, giving an 11-strategy budget.
- Simple score: `IS expectancy_R * sqrt(IS trade count)`.
- Tiebreaks: median calendar-year R, neighborhood survival evidence,
  recovery factor, lower drawdown, deterministic index.
- Random control uses the same eligible pool, budget, diversity constraints,
  and seed.
- Sealed results were evaluated through the existing M1 judge once per
  selected strategy union.

### Bakeoff results

Strict battery arm:

- Selected/evaluated: `0/0`.
- Strict pass count: `0/52`.
- This is represented as a valid empty arm, not silently relaxed.

Simple rank arm:

- Selected/evaluated: `11/11`.
- Median sealed expectancy: `+0.0532R`.
- Mean sealed expectancy: `+0.0464R`.
- Positive-strategy rate: `72.7%`.
- Trades: `1,468`.
- Net profit: `+75,105.60` on equal-risk aggregate initial balance
  `1,100,000`.
- Aggregate return: `+6.83%`.
- Aggregate max drawdown: `2.79%`.
- Aggregate recovery factor: `2.31`.
- Positive calendar blocks: `2023, 2024, 2025, 2026` (`100%`).

Random control:

- Selected/evaluated: `11/11`.
- Median sealed expectancy: `+0.0302R`.
- Mean sealed expectancy: `+0.0084R`.
- Positive-strategy rate: `72.7%`.
- Trades: `924`.
- Net profit: `+27,841.73`.
- Aggregate return: `+2.53%`.
- Aggregate max drawdown: `1.72%`.
- Aggregate recovery factor: `1.46`.
- Positive calendar blocks: `2024, 2025, 2026`; 2023 was negative (`75%`).

Decision:

- Simple versus random median lift: `+0.0230R`.
- Required lift: `+0.10R`.
- Simple positive aggregate: yes.
- Simple positive median: yes.
- Simple drawdown rule: yes.
- Simple positive calendar-year rule: yes.
- Adopt simple lane: **no**.

Interpretation: the current strict battery is too destructive on this cohort,
but the simple selector is not strong enough to replace it. The current
experiment does not prove that the strict battery improves holdout outcomes,
because its arm selected zero, and it does not prove that the simple rank is a
reliable production selector because the lift was only `0.023R`.

## ML / meta-learning results

Implementation:
`/Users/danielagbonkpolor/Documents/QuantForge.worktrees/qf-ui-simplification/crates/quantforge-discover/src/meta_learning.rs`

The ML pipeline used IS-only features and later-window labels. Asset identity
was excluded in every saved report. Features included IS expectancy, return,
trade count, PF, Sharpe, drawdown, recovery, return/DD, median R, fold median
R, fold spread, fold pass fraction, negative-fold flag, parameter-neighborhood
metrics, M1 retention, complexity, entry/exit counts and family indicators.

The classifier is an interpretable logistic-style model trained by iterative
gradient updates. The direct expectancy model is a regularized linear/ridge
style ranker. Neither saw the final evaluation-window outcomes during
selection.

### USDJPY small meta dataset

Input:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/USDJPY/meta-learning-input.json`

Report:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/USDJPY/meta-learning-report.json`

- 5 windows, 178 candidates/outcomes.
- `topK=20%`, asset identity excluded, minimum future trades 10,
  minimum retention 0.7.
- Validation 2021: AUC `0.700`, precision@K `0.200`, selected lift
  `+0.688R`.
- Validation 2022: AUC `0.967`, precision@K `0.500`, selected lift
  `+0.202R`.
- Validation 2024: AUC `0.818`, precision@K `0.333`, selected lift
  `+0.277R`.
- Sealed 2025: 10 rows, 0 positive labels, 2 selected, precision `0.0`,
  selected lift `-0.024R`.
- Final sealed Brier score `0.0037`; ECE `0.0388`.

This is too small and too sparse in the sealed window to support deployment.

### USDJPY large meta dataset

Input:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/USDJPY/meta-learning-large-input.json`

Classifier report:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/USDJPY/meta-learning-large-report.json`

Direct expectancy report:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/USDJPY/meta-expectancy-report.json`

Dataset: 5 windows, 847 candidates/outcomes; asset identity excluded;
`topK=20%`.

Classifier validation:

- 2021: AUC `0.825`, precision@K `0.200`, lift `+0.573R`.
- 2022: AUC `0.767`, precision@K `0.250`, lift `-0.484R`.
- 2024: AUC `0.818`, precision@K `0.333`, lift `+0.277R`.

Classifier sealed 2025:

- 191 rows, 7 positives, 39 selected.
- AUC `0.873`.
- Precision@K `0.154` (6 of 39 selected positive labels).
- Selected future expectancy `-0.095R`.
- Unselected future expectancy `+0.117R`.
- Selected lift `-0.213R`.
- Brier score `0.0658`; ECE `0.0972`.

Direct expectancy validation:

- 2021 lift `-0.271R`, rank correlation `-0.290`.
- 2022 lift `-0.260R`, rank correlation `-0.066`.
- 2024 lift `+0.063R`, rank correlation `+0.196`.

Direct expectancy sealed 2025:

- Selected expectancy `+0.050R` versus unselected `+0.080R`.
- Lift `-0.030R`.
- Rank correlation `-0.023`.
- Selected positive expectancy rate `46.2%`; unselected `45.5%`.

Conclusion: the ML models produced respectable-looking validation AUC in some
windows, but did not improve future expectancy on the sealed USDJPY window.
AUC alone was not useful evidence of a profitable selector.

### EURGBP large meta dataset

Input/report paths:

- `/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/EURGBP/meta-learning-large-input.json`
- `/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/EURGBP/meta-learning-large-report.json`
- `/Users/danielagbonkpolor/Documents/QuantForge/runs/meta-learning/EURGBP/meta-expectancy-report.json`

- 3 windows, 342 candidates/outcomes, asset identity excluded.
- Classifier validation AUC `0.526`, precision@K `0.043`, lift `-0.119R`.
- Classifier sealed 2025: AUC `0.693`, precision@K `0.083`, selected future
  expectancy `-0.224R` versus unselected `-0.023R`, lift `-0.201R`.
- Direct expectancy validation lift `-0.155R`.
- Direct expectancy sealed selected `-0.168R` versus unselected `-0.038R`,
  lift `-0.130R`.
- Direct sealed selected positive rate `25%`; unselected positive rate about
  `6.7%` by the saved outcome label, but both selected and unselected
  expectancies were negative.

Conclusion: no evidence that the selector generalized on EURGBP.

## Methodology / strategy-construction tests

### AUDUSD execution-module ablation

Report:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/methodology/audusd-execution-module-ablation-20260727.json`

- 2,160 draws, 180 cells, 4,320 evaluations.
- Best cell: SessionOrb / limit entry / 2 atoms; OOS1 pass `20%`, median
  retention `-0.989`.
- No factor contrast survived FDR 10%.
- Saved recommendation: prefer the simplest production recipe unless a family
  cell clearly dominates.

### GBPUSD factor grid

Report:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/methodology/gbpusd-factor-grid.json`

- 4,800 draws, 120 cells, 9,600 evaluations.
- Best cell: ZScoreReversion / simple market / 3 atoms; OOS1 pass `71%`,
  median retention `3.374`.
- Significant contrasts at FDR <=10% included pending-entry versus simple
  market and 2/3 atoms versus 1 atom.
- Family ranking by mean OOS1 pass among screened cells: MeanReversionBand,
  ZScoreReversion, VolSqueezeBreak, ImpulseCandle, SupplyDemandReclaim.

These factor-grid results are methodology evidence, not final sealed portfolio
proof. They also contain small-cell and multiple-comparison risks.

### Factory comparison sanity runs

Saved reports:

- `/Users/danielagbonkpolor/Documents/QuantForge/runs/factory-comparison/USDJPY/current-holding-broad.json`
- `/Users/danielagbonkpolor/Documents/QuantForge/runs/factory-comparison/USDJPY/simple-control.json`
- Equivalent EURGBP reports in the same directory.

For both USDJPY and EURGBP comparison runs, the quick 10-generation runs made
6,000 evaluations and ended with 0 Holding candidates and 0 elites in both the
current-broad and simple-control lanes. These were pipeline/factory sanity
runs, not useful OOS performance tests.

## Execution/parity validation

Saved report:
`/Users/danielagbonkpolor/Documents/QuantForge/runs/family-mt5-parity/ALL_RESULTS.json`

- 20 family/mode cases across 10 strategy families and market/pending modes.
- 20/20 acceptable.
- 20/20 trade-count checks passed.
- MT5 versus QuantForge trade-count delta ranged from `-1` to `+1`.

This supports execution/parity confidence, not strategy generalization.

## Implementation tests for the bakeoff

The new production-bakeoff module has six focused tests:

- invalid selection fraction is rejected;
- median calculation is deterministic;
- strict battery counts are taken only from frozen battery rows;
- sealed results cannot change eligibility or ranking;
- selection is deterministic and simple/random receive the same budget;
- zero-survivor strict battery is represented as a valid empty arm.

Additional validation completed:

- `cargo test -p quantforge-tick --lib`: 10 passed;
- `cargo test -p quantforge-discover production_bakeoff`: 6 passed;
- `cargo test -p quantforge-cli`: all CLI/integration tests passed;
- `cargo check -p quantforge-cli`: passed.

## What is and is not established

Established:

- H4 candidate generation currently produces cohorts that can pass basic gates
  but are eliminated by strict robustness checks.
- A simple IS score selected a profitable USDJPY sealed portfolio in one
  already-observed bakeoff, but its improvement over random was only `0.023R`.
- The saved ML selectors did not improve sealed future expectancy on the larger
  USDJPY or EURGBP datasets.
- Execution parity is close enough for the strategy research results to be
  meaningful.

Not established:

- Which individual robustness gate improves final holdout performance. The
  battery short-circuits and the current H4 cohorts mostly die at fold or
  neighbourhood gates.
- That the current simple rank should replace the production battery.
- That ML can select strategies with `0.1R+` future expectancy.
- That any new recipe now has a clean untouched holdout. The USDJPY H4 sealed
  bakeoff has been viewed and is contaminated for future recipe selection.

Earlier discussions also considered raw expectancy, fold expectancy,
`expectancy * trade_count`, H1-versus-H4 selection, a permutation-null test,
and several UI/performance comparisons. No separate machine-readable result
for those comparisons was found in the saved run artifacts, so they are not
reported here as measured conclusions. The saved bakeoff directly measured
`expectancy_R * sqrt(trade_count)` against a seeded random control.

## Recommended next direction

Do not keep tuning thresholds on the observed H4 holdout. For the next clean
research cycle, use Development-only chronological walk-forward as the main
future-like selector, keep M1 fidelity mandatory, use parameter neighbourhood
and Monte Carlo as a smaller fragility screen, and record CPCV/calendar-fold
results as diagnostics rather than letting them eliminate nearly every H4
candidate. Freeze that recipe before using a genuinely untouched final time
block. Improve the strategy factory if the new cohort still cannot reach the
user's `0.1R` / majority-positive target.
