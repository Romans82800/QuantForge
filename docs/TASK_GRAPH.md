# QuantForge task graphs (Retester chaining)

SQX `ProjectRetester` / `TaskRetest` / `TaskFiltering` configuration lives inside proprietary plugin JARs. There is no public, redistributable project/task XML schema we can legally clone.

QuantForge therefore ships a **native JSON task graph**:

- Protocol: `quantforge-task-graph-v1`
- Example: [`examples/retester-challenge-matrix-export.qf-task.json`](examples/retester-challenge-matrix-export.qf-task.json)
- Validate / plan: `quantforge task-run --graph path.json` (or `--example`)

## Step kinds

| Kind | Maps to |
|------|---------|
| `challenge` | `quantforge challenge` / Retester Challenge tab |
| `walk_forward_matrix` | `quantforge wf-matrix` / Optimizer & Retester WF Matrix |
| `judge` | `quantforge judge` / Retester M1 Judge |
| `export_mql5` | `quantforge export` / EA Export |
| `databank_filter` | `quantforge databank-filter` |
| `note` | Documentation node |

`depends_on` forms a DAG. Disabled steps (`enabled: false`) are skipped.

## Databank ranking filters

SQX-style expressions over elite columns:

```text
PF > 1.5 AND Drawdown < 20 AND Trades >= 30
grade == 'certified' OR NOT (Drawdown > 30)
```

CLI:

```bash
quantforge databank-filter --elites elites.json --expr "PF > 1.5 AND Drawdown < 20"
```

Desktop Databank has a matching expression box beside search.
