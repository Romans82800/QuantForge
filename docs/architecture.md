# QuantForge implementation order

The implementation follows a vertical-slice-first order so MT5 truth is tested
before significant desktop work.

1. **Deterministic foundations**: shared types, data and broker hashes, run
   manifests, data ingestion, quality checks and strategy IR.
2. **Scout slice**: feature calculation, next-bar-open execution, cost model,
   trade blotter and deterministic golden tests.
3. **Discovery**: grammar generation, mutation, crossover, MAP-Elites deposit,
   correlation rejection and resumable RNG state.
4. **Truth core**: replay elite decision signals through complete M1 bars,
   validate parent-bar reconstruction and compare every export-safe numerical
   indicator against buffers exported by MT5. Resolve broker-local sessions,
   scheduled spreads and overnight swap identically in Scout and Judge.
5. **Parity slice**: export one IR to guarded MQL5, compile it, run the MT5
   Strategy Tester and compare aligned trades rather than aggregate totals only.
6. **Certification control plane**: immutable chronological split plans, typed
   promotion evidence, sealed-use rules and a Certified-only Vault whose storage
   API re-runs the gate.
7. **Challenge**: purged validation, cost shocks, Monte Carlo, parameter
   neighbourhoods and versioned multiple-testing adjustments.
8. **Sealed final**: require a passing Challenge shortlist, claim access before
   loading bars, score only post-boundary entries and retain pass or demotion as
   the single attempt for that strategy/split identity.
9. **Evidence assembly**: derive a distinct validation attestation, semantically
   verify every upstream artifact and freeze exact file hashes into a bundle
   consumed directly by certification.
10. **Portfolio packing**: select databank elites under hard correlation,
    weight, symbol and family caps, then retain a complete seeded moving-block
    stress record.
11. **Deployment packs**: accept only an intact Certified Vault entry and
    reproduce the exact parity-passed MT5 source, settings, risk controls and
    audit chain with live trading disabled.
12. **Incubation ledger**: implemented append-only paper-trading starts and
    daily observations, one-shot final evaluation, certification binding and a
    deployment requirement before operational live review.
13. **Desktop**: the Tauri shell exposes the complete workflow through typed
    local commands, including promotion, parity, certification and guarded
    deployment. It never delegates to an ambient CLI process.

## Certification state machine

`Scouted -> Accepted -> Illuminated -> Challenged -> Parity-Passed -> Certified -> Deployed`

An internal judge cannot create a `Parity-Passed` result. That state requires a
versioned external MT5 tester run tied to terminal build, data, broker profile,
EA source and settings hashes. Incubation is an optional certification policy,
not a separate strategy grade. A failed or improperly used sealed test demotes
the candidate to `Illuminated`.

## Broker timestamp rule

MT5 OHLC exports contain broker-server wall times. Every import must therefore
provide either the paired QuantForge exporter metadata or an explicit IANA
timezone. Ambiguous or nonexistent local times at daylight-saving transitions
are rejected instead of guessed. The normalized UTC timestamps are included in
the data hash.
