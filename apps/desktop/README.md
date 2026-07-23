# QuantForge Desktop

The desktop shell is a Tauri 2 + React/TypeScript research cockpit over the
same Rust engines and artifacts used by the CLI. Home links the active workflow;
Data Lab parses, hashes, grades and broker-binds real OHLC sources; Discover
runs deterministic new or continued MAP-Elites jobs with live progress and
checkpoint controls; Databank validates and interrogates the resulting archive.

## Run

Node.js 20.19+ and the normal Tauri platform prerequisites are required.

```sh
cd apps/desktop
pnpm install
pnpm tauri dev
```

Frontend checks and the Rust backend can be verified independently:

```sh
pnpm check
pnpm test
cargo test -p quantforge-desktop
```

All required workspaces are active. The desktop covers broker-bound data
inspection, development-only discovery, Databank inspection/IR export,
Challenge, one-shot sealed final, M1 Judge, guarded MQL5 export, external and
indicator parity, paper incubation, evidence assembly, Certified Vault
admission, portfolio packing and deployment-pack generation.

The Databank workspace supports checkbox selection, deterministic top-N
selection and no-clobber batch export. A batch folder contains one exact
`*.strategy.ir.json` artifact per selected elite plus
`quantforge-strategy-batch.json`, which records the hashes, metrics and paths
needed to audit the set. The workspace retains the Evidence × Novelty
behavioral map for diversity analysis and separately plots cumulative equity
curves reconstructed from the stored 64-point M1 signatures.

Databank rows expose Return / DD and an M1 equity Sharpe proxy and can be
ranked by either measure. Return / DD is net return percentage divided by
maximum drawdown percentage. New discovery jobs can set a minimum Return / DD
gate; candidates must satisfy it on both the decision-timeframe screen and the
M1 precision replay. Newly evaluated results persist the Sharpe proxy directly,
while older databanks derive it from their stored M1 signature.

Selecting an elite also exposes direct handoffs to Challenge and M1 / MT5.
Those handoffs carry the decision and M1 data paths, metadata, broker profile,
cost assumptions, initial balance and newly exported Strategy IR forward from
the loaded databank. Output paths remain intentionally blank because
QuantForge artifacts are immutable and never silently overwritten. An older
databank without recorded M1 source context can still be opened, but its M1
path must be selected once at the Judge stage.

New searches enable the certification-grade 60/20/20 split by default. Older
databanks evaluated on full history remain readable, but evidence assembly will
correctly reject them as non-development-only research. The desktop never
shells out to an ambient CLI or weakens promotion validation for convenience.
