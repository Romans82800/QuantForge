#!/usr/bin/env python3
"""Verify that an SQX retest reproduces the backtest stored inside each .sqx.

Every .sqx carries a <Fingerprint> recording the net profit, trade count and
drawdown of its original backtest. If a retest on re-imported data does not
reproduce those numbers, the data or cost settings differ from the build run and
every downstream robustness pass/fail is meaningless. This compares the two.

  baseline: read expected values out of a folder of .sqx files
  compare:  diff a Databank CSV export against that baseline

The Databank export's column names vary by SQX version and locale, so columns are
matched by fuzzy header lookup rather than fixed names.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
import zipfile
from pathlib import Path

# The payload is an inner self-closing <Fingerprint strategyName=... /> nested inside
# a <Fingerprint type="...StrategyFingerprint"> wrapper, so require the attribute.
FINGERPRINT = re.compile(r"<Fingerprint\b([^>]*\bstrategyName=\"[^>]*?)/?>")
ATTR = re.compile(r'([\w]+)="([^"]*)"')


def read_fingerprint(path: Path) -> dict[str, str] | None:
    try:
        with zipfile.ZipFile(path) as zf:
            xml = zf.read("settings.xml").decode("utf-8", errors="ignore")
    except Exception:
        return None
    m = FINGERPRINT.search(xml)
    if not m:
        return None
    fp = dict(ATTR.findall(m.group(1)))
    sym = re.search(r'<Symbol type="String">(.*?)</Symbol>', xml)
    fp["symbol"] = sym.group(1) if sym else ""
    return fp


def build_baseline(folder: Path) -> list[dict]:
    out = []
    for path in sorted(folder.rglob("*")):
        if path.suffix.lower() != ".sqx":
            continue
        fp = read_fingerprint(path)
        if not fp:
            print(f"  ! unreadable: {path.name}", file=sys.stderr)
            continue
        out.append(
            {
                "file": path.name,
                "strategy": fp.get("strategyName", ""),
                "symbol": fp.get("symbol", ""),
                "trades": int(float(fp["trades"])) if fp.get("trades") else None,
                "profit": float(fp["profit"]) if fp.get("profit") else None,
                "drawdown": float(fp["drawdown"]) if fp.get("drawdown") else None,
                "fitness": float(fp["fitness"]) if fp.get("fitness") else None,
            }
        )
    return out


def norm(text: str) -> str:
    return re.sub(r"[^a-z0-9]", "", text.lower())


def find_column(headers: list[str], *candidates: str) -> str | None:
    normed = {norm(h): h for h in headers}
    for cand in candidates:
        if norm(cand) in normed:
            return normed[norm(cand)]
    for cand in candidates:
        c = norm(cand)
        for n, original in normed.items():
            if c and c in n:
                return original
    return None


def parse_number(raw: str | None) -> float | None:
    if raw is None:
        return None
    cleaned = re.sub(r"[^\d.\-]", "", raw.replace(",", ""))
    try:
        return float(cleaned)
    except ValueError:
        return None


def compare(baseline: list[dict], export: Path, tol_pct: float) -> int:
    rows = list(csv.DictReader(export.open(encoding="utf-8-sig")))
    if not rows:
        print(f"no rows in {export}", file=sys.stderr)
        return 1
    headers = list(rows[0].keys())
    col_name = find_column(headers, "Name", "Strategy", "Strategy name")
    col_profit = find_column(headers, "Net profit", "NetProfit", "Profit")
    col_trades = find_column(headers, "# of trades", "Number of trades", "Trades")
    col_dd = find_column(headers, "Drawdown", "Max DD", "Max drawdown")

    print(f"matched columns: name={col_name!r} profit={col_profit!r} "
          f"trades={col_trades!r} drawdown={col_dd!r}\n")
    if not col_name:
        print("could not find a strategy-name column; headers were:", headers)
        return 1

    actual = {norm(r[col_name]): r for r in rows if r.get(col_name)}
    ok = mismatched = missing = 0

    for b in baseline:
        row = actual.get(norm(b["strategy"])) or actual.get(
            norm(Path(b["file"]).stem)
        )
        if row is None:
            print(f"MISSING  {b['strategy']:<24} not present in export")
            missing += 1
            continue
        deltas = []
        for label, key, col in (
            ("profit", "profit", col_profit),
            ("trades", "trades", col_trades),
            ("drawdown", "drawdown", col_dd),
        ):
            if not col or b[key] is None:
                continue
            got = parse_number(row.get(col))
            if got is None:
                continue
            want = float(b[key])
            if abs(want) < 1e-9:
                diff_pct = 0.0 if abs(got) < 1e-9 else 100.0
            else:
                diff_pct = abs(got - want) / abs(want) * 100.0
            deltas.append((label, want, got, diff_pct))
        worst = max((d[3] for d in deltas), default=0.0)
        if worst <= tol_pct:
            ok += 1
            print(f"OK       {b['strategy']:<24} worst delta {worst:5.2f}%")
        else:
            mismatched += 1
            print(f"MISMATCH {b['strategy']:<24} worst delta {worst:5.2f}%")
            for label, want, got, diff_pct in deltas:
                flag = "  <-- " if diff_pct > tol_pct else "      "
                print(f"{flag}{label:<9} expected {want:>14,.2f}  got {got:>14,.2f}"
                      f"  ({diff_pct:.2f}%)")

    total = len(baseline)
    print(f"\n{ok}/{total} reproduced within {tol_pct}%  "
          f"({mismatched} mismatched, {missing} missing)")
    if mismatched or missing:
        print("\nA mismatch means the retest data or costs differ from the build run.")
        print("Check the data symbol, spread/commission/swap and backtest precision")
        print("before trusting any robustness result from this batch.")
    return 0 if ok == total else 2


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("baseline", help="extract expected values from .sqx files")
    b.add_argument("folder", type=Path)
    b.add_argument("--json", type=Path)

    c = sub.add_parser("compare", help="diff a Databank CSV export against baseline")
    c.add_argument("baseline_json", type=Path)
    c.add_argument("export_csv", type=Path)
    c.add_argument("--tolerance", type=float, default=1.0,
                   help="max %% deviation to count as reproduced (default 1.0)")

    args = ap.parse_args()

    if args.cmd == "baseline":
        rows = build_baseline(args.folder)
        if not rows:
            print("no readable .sqx found", file=sys.stderr)
            return 1
        print(f"{'strategy':<24} {'symbol':<10} {'trades':>7} {'profit':>14} "
              f"{'drawdown':>13}")
        for r in rows:
            print(f"{r['strategy']:<24} {r['symbol']:<10} {r['trades'] or 0:>7} "
                  f"{r['profit'] or 0:>14,.2f} {r['drawdown'] or 0:>13,.2f}")
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(rows, indent=2))
            print(f"\nwrote {args.json}")
        return 0

    baseline = json.loads(args.baseline_json.read_text())
    return compare(baseline, args.export_csv, args.tolerance)


if __name__ == "__main__":
    raise SystemExit(main())
