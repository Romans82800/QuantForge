# Certification and Vault protocol

QuantForge treats certification as a reproducible evidence decision, not a
label a caller can assign directly.

## Data split

`quantforge split-plan` creates three contiguous chronological partitions:

1. development;
2. validation;
3. sealed final.

The artifact records the full dataset identity plus each partition's time
range, bar count and canonical bar hash. It refuses unordered bars, inconsistent
dataset hashes, empty partitions and data-quality failures unless a research
override is explicitly recorded. Any such override later blocks certification.

The split plan is an audit mechanism. It does not encrypt the original market
data; operating-system access to the source dataset remains outside this local
application boundary.

## Required promotion evidence

One `CertificationEvidence` bundle must bind these distinct artifacts to the
same strategy fingerprint and broker-specification hash:

- validation;
- MAP-Elites illumination;
- Challenge battery;
- M1 Judge;
- external MT5 Strategy Tester parity;
- MT5 indicator parity;
- sealed final test;
- incubation, when required by policy.

The gate denies promotion when an artifact is reused for multiple stages, a
protocol or binding differs, any research override exists, validation or sealed
data hashes differ from the split plan, the external engine is not MT5, parity
lacks protective orders, the sealed data was opened before shortlisting, or the
sealed result contributed to the selection score. High evaluation counts are
recorded as a selection-bias warning.

## Automatic assembly

`quantforge assemble-evidence` consumes the strategy, broker, split plan and the
machine-produced databank, Challenge, Judge, external parity, indicator parity,
sealed-final and optional incubation artifacts. It does not trust a manually
asserted pass flag. It checks, among other invariants:

- the candidate is an exact elite in a no-override databank whose data hash is
  the development partition;
- the databank evaluation count and Scout cost model equal Challenge;
- Challenge is internally consistent, passing and bound to validation;
- Judge uses M1 execution without gaps or overrides, the same cost model and
  metrics that clear the Challenge baseline thresholds;
- parity compares that exact Challenge baseline to an engine named
  `mt5-strategy-tester`, recomputes the diff, and retains mandatory stop and
  target evidence;
- the MT5 indicator artifact contains passing results for all 13 export-safe
  fields on the same symbol;
- sealed-final passes, was shortlisted before access, was not used in
  selection, and names the SHA-256 of the exact Challenge file.
- when supplied, incubation passes its immutable kill rules and every embedded
  observation exactly matches the hash and bytes of its append-only source
  artifact.

After every check passes, assembly writes three no-clobber files:

- `validation-attestation.json`, a separately hashed projection of the
  Challenge validation baseline;
- `certification-evidence.json`, the typed gate input;
- `certification-bundle.json`, containing the evidence path/hash and each gate
  artifact path/hash.

Use `quantforge certify --bundle <path>` for admission. Certification verifies
the bundle manifest and re-hashes the evidence and every artifact before it
evaluates the policy. Moving or changing a file after assembly is therefore a
hard failure. The original `--evidence` plus repeated `--artifact` interface is
retained for compatibility, but automatic assembly is the promotion-grade
path.

Passing `--incubation <incubation-final.json>` adds the eighth gate. Use
`certify --require-incubation` for promotion-grade admission. Research-only
certification may omit the gate, but such a Vault entry cannot be deployed.

## Vault semantics

Storage calls the certification evaluator again before writing. A denial creates
no candidate directory. A success writes:

`<vault>/<strategy-fingerprint>/<entry-id>.certified.json`

The entry contains the decision, exact strategy and broker objects, split plan,
evidence bundle, source hashes, artifact paths/hashes and run manifest. Entry ID
is derived from the strategy and evidence identity, so retrying the same
certification cannot create a second version, even if other payload metadata
changes. Writes use no-clobber atomic persistence.

Vault immutability is enforced by QuantForge's API. A user with filesystem
ownership can still alter or remove local files, so external backups, signing
and access control remain deployment concerns.
