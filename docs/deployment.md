# Certified deployment-pack protocol

`quantforge deploy` materializes an MT5-ready directory from one Certified Vault
entry. A raw strategy, databank elite, Challenge report or manually labelled EA
cannot enter this path.

## Certification and parity binding

Before writing anything, deployment:

1. validates the Vault payload manifest and split plan;
2. recomputes the strategy fingerprint, broker hash, certification policy and
   complete certification decision;
3. verifies the Vault payload and deterministic entry identities;
4. re-hashes every gate artifact retained by the Vault entry;
5. requires that certification policy to include incubation, reloads the
   immutable source ledger and recomputes its passing report;
6. reloads and recomputes the external MT5 parity comparison;
7. regenerates MQL5 from the vaulted strategy and broker using the exact export
   configuration stored in that parity artifact;
8. requires the regenerated evidence card and EA source hash to be identical to
   the parity-passed versions.

There are deliberately no deployment-time magic, timeframe, cost or spread
overrides. Any such change would create source or settings that were not covered
by the Certified parity evidence.

## Pack contents

The new deployment directory contains:

- `<Expert>.mq5`;
- `<Expert>.set`;
- `<Expert>.tester.ini`;
- `strategy.ir.json`;
- `broker-spec.json`;
- `export-evidence.json`;
- `risk-pack.json`;
- `CHANGELOG.md`;
- `deployment-manifest.json`.

The deployment manifest identifies the Certified Vault entry, external parity
artifact, paper-incubation artifact, candidate and every payload file's SHA-256
and byte count. The
deployment ID is deterministic for those exact inputs. The directory is built
in a sibling temporary location and renamed into place; an existing target is
never intentionally replaced.

## Safety boundary

`AllowLiveTrading` remains `false` in both generated source and settings. The
risk pack records the strategy risk policy, protective stops, broker volume and
stop limits, spread/deviation/cost assumptions, certification warnings and an
operator notice.

Certification and passed paper incubation demonstrate process controls and
engine agreement on recorded inputs; neither guarantees profitability.
Independent operational review is still required before a person chooses to
enable live orders. QuantForge does not enable them automatically.
