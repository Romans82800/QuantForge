# QuantForge task graphs (Retester chaining)

SQX `ProjectRetester` / `TaskRetest` / `TaskFiltering` configuration lives inside proprietary plugin JARs. There is no public, redistributable project/task XML schema we can legally clone.

QuantForge therefore ships a **native JSON task graph**:

- Protocol: `quantforge-task-graph-v1`
- Example: [`examples/retester-challenge-matrix-export.qf-task.json`](examples/retester-challenge-matrix-export.qf-task.json)
- Execute end-to-end: `quantforge task-run --graph path.json --work-dir ./runs/task`
- Plan only: `quantforge task-run --graph path.json --dry-run`

Shared inputs live under top-level `inputs` and are merged into each step's `params` (step wins on conflict). Artifacts from earlier steps (`scout`, `challenge`, `wf_matrix`, `judge`, `mq5`, `html_report`, …) are wired automatically when later steps omit explicit paths.

## Step kinds

| Kind | Maps to |
|------|---------|
| `scout` | Completed-bar Scout backtest |
| `challenge` | Validation Challenge battery |
| `walk_forward_matrix` | Fold × lookback WF matrix |
| `judge` | M1 Judge replay |
| `export_mql5` | MQL5 EA pack |
| `databank_filter` | Expression filter over elite JSON |
| `what_if` | What-If trade blotter filters |
| `negate` | Strategy Negater (flip sides) |
| `html_report` | SQX SaverHTML-style results HTML |
| `multi_symbol_retest` | Retest on additional symbol datasets |
| `note` | Documentation node |

`depends_on` forms a DAG. Disabled steps (`enabled: false`) are skipped.

## HTML export

```bash
quantforge export-html --input scout-or-judge.json --out results.html
```

## Desktop

Retester → **4 · Task Graph** runs the same executor (`run_task_graph_workflow`).
Choose a `*.qf-task.json`, a work directory, optional dry-run / stop-on-failure.

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
