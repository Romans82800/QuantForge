#!/usr/bin/env python3
"""Group .sqx strategies by the backtest config stored in their settings.xml.

Each .sqx is a zip archive whose settings.xml records the project configuration of
the strategy's last backtest (symbol, timeframe, history range, precision, costs).
Strategies sharing a config can be retested as a single SQX batch, so this reports
how many distinct batches a library actually needs.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
import zipfile
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

SCALAR_KEYS = (
    "Symbol",
    "Timeframe",
    "HistoryFrom",
    "HistoryTo",
    "BacktestPrecision",
    "Precision",
    "Slippage",
    "MinDistance",
    "MoneyManagement.InitialCapital",
    "BrokerOption.PickerBroker",
    "StrategyName",
    "Complexity",
)

# Config fields that define a retest batch. Precision/costs are reported but kept out
# of the key so a batch is not split by settings you would override during a retest.
GROUP_KEYS = ("Symbol", "Timeframe", "HistoryFrom", "HistoryTo", "Swap")

SWAP_RE = re.compile(r"<Swap\b[^>]*/?>")


def scalar(xml: str, key: str) -> str | None:
    m = re.search(
        rf"<{re.escape(key)}(?:\s[^>]*)?>(.{{0,200}}?)</{re.escape(key)}>", xml, re.S
    )
    if not m:
        return None
    return re.sub(r"\s+", " ", m.group(1)).strip()


def swap_signature(xml: str) -> str | None:
    m = SWAP_RE.search(xml)
    return re.sub(r"\s+", " ", m.group()) if m else None


def epoch_ms_to_date(value: str | None) -> str | None:
    try:
        ms = int(value)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        return None
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).strftime("%Y-%m-%d")


def read_config(path: Path) -> dict[str, str | None]:
    with zipfile.ZipFile(path) as zf:
        xml = zf.read("settings.xml").decode("utf-8", errors="ignore")
    cfg: dict[str, str | None] = {k: scalar(xml, k) for k in SCALAR_KEYS}
    cfg["Swap"] = swap_signature(xml)
    cfg["HistoryFromDate"] = epoch_ms_to_date(cfg["HistoryFrom"])
    cfg["HistoryToDate"] = epoch_ms_to_date(cfg["HistoryTo"])
    return cfg


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("root", type=Path, help="directory to scan recursively for .sqx")
    ap.add_argument("--csv", type=Path, help="write per-strategy rows here")
    ap.add_argument("--json", type=Path, help="write group summary here")
    args = ap.parse_args()

    files = sorted(p for p in args.root.rglob("*") if p.suffix.lower() == ".sqx")
    if not files:
        print(f"no .sqx files under {args.root}", file=sys.stderr)
        return 1

    rows: list[dict[str, str]] = []
    groups: dict[tuple, list[Path]] = defaultdict(list)
    failures: list[tuple[Path, str]] = []

    for path in files:
        try:
            cfg = read_config(path)
        except Exception as exc:  # unreadable/corrupt archive
            failures.append((path, f"{type(exc).__name__}: {exc}"))
            continue
        key = tuple(cfg.get(k) for k in GROUP_KEYS)
        groups[key].append(path)
        rows.append(
            {
                "file": str(path),
                "symbol": cfg["Symbol"] or "",
                "timeframe": cfg["Timeframe"] or "",
                "history_from": cfg["HistoryFromDate"] or "",
                "history_to": cfg["HistoryToDate"] or "",
                "precision": cfg["BacktestPrecision"] or "",
                "slippage": cfg["Slippage"] or "",
                "min_distance": cfg["MinDistance"] or "",
                "initial_capital": cfg["MoneyManagement.InitialCapital"] or "",
                "swap": cfg["Swap"] or "",
                "complexity": cfg["Complexity"] or "",
                "group": " | ".join(str(v) for v in key),
            }
        )

    print(f"scanned {len(files)} .sqx files")
    print(f"readable {len(rows)}, unreadable {len(failures)}")
    print(f"distinct retest configs: {len(groups)}\n")

    ordered = sorted(groups.items(), key=lambda kv: len(kv[1]), reverse=True)
    print(f"{'count':>6}  symbol  tf    history range")
    for key, members in ordered:
        symbol, tf, hfrom, hto, _swap = key
        print(
            f"{len(members):>6}  {symbol or '?':<7} {tf or '?':<5} "
            f"{epoch_ms_to_date(hfrom) or '?'} -> {epoch_ms_to_date(hto) or '?'}"
        )

    by_symbol = Counter(r["symbol"] for r in rows)
    print("\nper symbol:")
    for symbol, count in by_symbol.most_common():
        print(f"{count:>6}  {symbol or '?'}")

    if failures:
        print("\nunreadable files (first 10):")
        for path, err in failures[:10]:
            print(f"  {path.name}: {err}")

    if args.csv:
        args.csv.parent.mkdir(parents=True, exist_ok=True)
        with args.csv.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
            writer.writeheader()
            writer.writerows(rows)
        print(f"\nwrote {args.csv}")

    if args.json:
        summary = [
            {
                "symbol": key[0],
                "timeframe": key[1],
                "history_from": epoch_ms_to_date(key[2]),
                "history_to": epoch_ms_to_date(key[3]),
                "swap": key[4],
                "count": len(members),
                "files": [str(p) for p in members],
            }
            for key, members in ordered
        ]
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(f"wrote {args.json}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
