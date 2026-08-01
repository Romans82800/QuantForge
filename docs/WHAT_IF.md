# What-If cross-checks

SQX-style post-backtest trade filters (`CrossCheckWhatIf`). These do **not** re-simulate the engine; they filter a finished blotter and recompute metrics.

```powershell
quantforge what-if `
  --trades path/to/scout_or_judge.json `
  --exclude-pct-biggest-pl 10 `
  --exclude-short-trades `
  --take-every-nth-trade 2 `
  --out what_if_report.json
```

Filters: `exclude_pct_biggest_pl`, `exclude_pct_lowest_pl`, `exclude_short_trades`, `exclude_long_trades`, `take_every_nth_trade`, `take_max_trades_per_day`.
