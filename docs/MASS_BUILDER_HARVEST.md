# Mass Builder long harvest (EURNZD-style)

`DiscoverRunMode::MassBuilder` turns on the SQX-like funnel:

1. **Cheap prefilter** — trailing-window Scout + loose gates (reject no-trades / trash PF)
2. **Full H1 Scout** — existing coarse + deposit gates
3. **Genetic islands** — isolated breeding pots with ring migration
4. **M1 robustness** — databank promotion (walk-forward / MC / neighborhood)

## Kick off (CLI)

```powershell
$env:Path = "C:\Program Files\Git\cmd;C:\Users\Administrator\.cargo\bin;" + $env:Path
$env:QUANTFORGE_DATA_PACK = "C:\Users\Administrator\Documents\QuantForge\ICMarkets_EST7_2020_present"
# Build once with MSVC:
#   cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo build -p quantforge-cli --release'

quantforge discover `
  --source "$env:QUANTFORGE_DATA_PACK\EURNZD_H1.csv" `
  --m1 "$env:QUANTFORGE_DATA_PACK\EURNZD_M1.csv" `
  --broker "$env:QUANTFORGE_DATA_PACK\broker_eurnzd.json" `
  --databank ".\runs\eurnzd_mass_builder\databank.json" `
  --run-mode mass_builder `
  --generations 200 `
  --flatten-at-22 `
  --end-of-day-hour 23
```

Adjust paths to your pack layout. Continue later with `--continue --generations 200` on the same databank.

## Knobs (also in `DiscoverConfig`)

| Field | MassBuilder default behavior |
|-------|------------------------------|
| `enable_cheap_prefilter` | on |
| `prefilter_bar_fraction` | ~0.25 trailing IS bars |
| `island_count` | ≥4 |
| `migration_interval` | 10 generations |
| `migration_elites` | ≥1 per island |
| `batch_size` | ≥500 |
| `early_stop_pot_elites` | none (continuous) |

## Telemetry to watch

- `rejected_prefilter` ≫ `rejected_gate` — funnel is healthy
- `island_migrations` increases every `migration_interval`
- `databank_accepted` grows without pot early-stop freezing the run
