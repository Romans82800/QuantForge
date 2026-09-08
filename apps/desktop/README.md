# QuantForge Desktop

## Campaign recovery and portfolio research

Multi-asset Discover saves campaign recipes and status in the app configuration
directory under `campaigns/`. Reopen a saved campaign to inspect its Databanks,
or Resume to continue each lane from its last recovery/final archive. Resuming
copies existing checkpoints to fresh working files and inherits their verified
trading configuration. Missing checkpoints with recorded evaluations are an
error, not permission to silently replace a cohort. New lanes that never began
may start from the saved recipe. Recovery checkpoints are attempted at generation
boundaries every five active minutes; a crash can lose work since the last one.
Existing campaigns created before this version have no campaign journal and
must still be opened individually.

Campaign strategy previews expose the full partitioned M1 replay, matching
full-period statistics, stored robustness panels, trades and the strategy IR.
Holding strategies can also be replayed. Missing historical test evidence is
shown as missing, never as a passing result. Battery runs now save an immutable
per-candidate JSON record beside the Databank in `<bank>_battery_results/`,
including failed outcomes, configuration, data/broker hashes and available
test evidence. Strict batteries may stop at a failed gate; use the existing
audit-and-graduate mode when every test must run regardless of prior failures.

Portfolio → Multi-asset portfolio accepts multiple saved Databanks. It loads
one market at a time and replays every Databank elite on hash-verified
Development data, then aligns daily marked-to-market changes on the common
UTC calendar period. At least 30 common days are required. A new immutable
report records the source hashes, selected identities, allocation configuration,
daily portfolio path, correlation groups and seeded bootstrap results.
Correlation groups are connected components, not a claim that every member
is strongly correlated with every other; portfolio selection independently
enforces its pairwise limit. Duplicate rules on the same market across banks
are rejected. Strategies with identical rules on different markets remain
distinct. The portfolio uses equal capital weights, not equal volatility or
equal trade risk. Drawdown is measured at daily closes and can understate
intraday drawdown. Currency participation counts capital allocated to both
legs of FX strategies; it is not net notional currency exposure. Maximum
simultaneous positions is calculated from the selected trades' actual intervals.

This release does not establish which filters predict future profitability.
That requires historical development episodes with later validation outcomes;
the per-candidate records provide inputs for that further research. The existing
SL/TP-only recipe controls and MT5 parity workflow remain available. Full
pairwise trade-overlap diagnostics, net currency risk limits and a filter-value
research interface are still outstanding.

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

Data Manager also imports IC Markets downloads directly. Choose the Downloads
folder and QuantForge recursively recognizes `Time,Ask,Bid,Volume` tick CSVs,
aggregates their Ask/Bid midpoint to M1 bars, derives H1 from that M1 stream,
and writes companion metadata under a timestamped output directory. Unrelated
CSV schemas are listed as skipped. The generated bars are research-ready but
still need a real MT5 broker profile before Discover can run; the importer
does not invent contract size, point value, sessions or commission settings.

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
