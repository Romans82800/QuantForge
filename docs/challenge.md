# Challenge protocol

`quantforge challenge` moves one fixed, Illuminated strategy through a
deterministic robustness battery. It reads the full source only to verify its
split-plan identity, then extracts and scores the validation partition. Sealed
bars are never included in the baseline, folds, cost shocks, Monte Carlo input
or parameter neighborhoods.

## Battery

The v1 report contains:

- a baseline validation Scout run and validation gates;
- contiguous purged walk-forward folds with configurable pre-fold purge and
  post-fold embargo accounting;
- spread, slippage and commission shocks at increasing multipliers;
- moving-block bootstrap of the ordered validation trade-profit sequence;
- deterministic bounded perturbations of indicator periods, constants, risk,
  stop, target and time-stop parameters;
- the originating `evaluations_touched`, an analytic expected maximum lucky
  Sharpe and a deflated trade-Sharpe proxy.

Earlier validation bars remain available to later folds as past-only indicator
warm-up, but the evaluator forbids entries before each fold begins. The strategy
is fixed during Challenge; no fitting occurs on the reported residual training
bars.

Challenge passes only when baseline gates, the configured fraction of folds,
cost-shock survival, Monte Carlo tails and parameter-neighborhood survival all
pass. A deflated-Sharpe floor can also be enabled. Every raw component is stored
alongside the final blockers.

## Statistical boundary

This is a strong purged walk-forward k-fold implementation, which the product
spec permits as the v1 alternative to combinatorial purged CV. It is not full
CPCV path enumeration. Monte Carlo v1 resamples contiguous blocks of trade
outcomes; moving-block bar resampling and optional cross-seed reappearance are
future extensions. These method names are recorded explicitly in JSON so the
evidence cannot be mistaken for a stronger protocol.
