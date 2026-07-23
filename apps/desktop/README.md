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

New searches enable the certification-grade 60/20/20 split by default. Older
databanks evaluated on full history remain readable, but evidence assembly will
correctly reject them as non-development-only research. The desktop never
shells out to an ambient CLI or weakens promotion validation for convenience.
