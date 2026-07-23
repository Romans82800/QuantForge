# Paper-incubation protocol

Paper incubation is an append-only operational evidence gate between historical
certification work and deployment. It records observed paper-account outcomes;
it is not another backtest and is not a profitability guarantee.

## Immutable lifecycle

`quantforge incubation-start` binds one ledger to the exact strategy
fingerprint, broker specification and chronological split-plan hash. Its start
date, initial balance and kill rules cannot be edited through QuantForge. The
deterministic location is:

`<root>/<strategy-fingerprint>/<split-hash>/incubation-start.json`

`quantforge incubation-record` appends a dated JSON artifact under
`observations/`. Starting balance is taken from the prior record, dates must be
strictly increasing, and existing files are never replaced. Each record stores
ending balance, maximum observed drawdown, trade count and an optional note.

`quantforge incubation-finalize` re-hashes and reloads the complete source
ledger, evaluates it, and writes `incubation-final.json` once. A failed result is
written before the command returns failure. Further records and another final
are refused, preventing a breached history from being silently retried.

## Default kill rules

- maximum daily loss: 2%;
- maximum total observed drawdown: 10%;
- minimum observation days: 30;
- minimum total trades: 20;
- maximum consecutive zero-trade days: 5.

Rules are fixed when the ledger starts. Finalization reports every blocker:
insufficient duration, daily-loss breach, total-drawdown breach, insufficient
trades or collapsed trade frequency.

## Continuous operation and promotion

The current workflow is continuously appended by the operator, normally once
after each paper-trading day. Automatic polling from an MT5 terminal is not yet
implemented, so account statements and recorded values remain an operational
trust boundary.

Supply the passing final to `assemble-evidence --incubation`. Then certify with
`--require-incubation`. Deployment rejects Vault entries created without that
policy and re-verifies the source start, every observation, the final report and
their SHA-256 bindings before writing a pack.
