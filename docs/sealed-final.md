# Sealed-final protocol

`quantforge sealed-final` is a one-shot final test, not a reusable backtest
command.

Before market bars are loaded, QuantForge verifies an immutable split plan and
a machine-produced passing Challenge artifact for the same strategy, broker,
validation hash and split-plan hash. The final configuration must use the exact
Challenge balance and cost assumptions. Its trade, return, profit-factor and
drawdown gates must be at least as strict as Challenge, with at least one gate
strictly tighter.

## One-shot access

After all non-market inputs validate, storage atomically creates:

`<sealed-root>/<strategy-fingerprint>/<split-hash>.sealed-open.json`

Only then does the command load the source data. The claim is never replaced or
removed. It therefore blocks another attempt after a pass, a failed final test,
an evaluation error or a process crash. The report uses the same identity with
the suffix `.sealed-final.json`.

Development and validation bars remain available as past-only indicator
warm-up. The evaluator forbids entries before the sealed boundary, and the
report verifies every scored trade timestamp. Sealed results are never added to
the selection score. Failure records its blockers and leaves the candidate at
`Illuminated`, ineligible for certification.

The ledger is a local application boundary, not cryptographic data custody. A
filesystem owner could choose another root or modify files; production use
should place the ledger under controlled storage with backups and access
restrictions.
