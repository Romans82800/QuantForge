#!/usr/bin/env python3
"""Prepare / compare StopLimit EveryTick golden directories.

When MetaTrader 5 is unavailable this still creates the stub layout so a later
capture can drop deals/equity CSVs and run --compare without renaming paths.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_TAG = "golden_stub"
DEFAULT_DIR = ROOT / "parity" / "stop_limit" / DEFAULT_TAG
FIXTURE_IR = ROOT / "fixtures" / "stop_limit_pending_strategy.json"


def prepare(out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest = {
        "schema_version": 1,
        "protocol": "mt5-parity-v2",
        "tag": out_dir.name,
        "symbol": "EURNZD",
        "timeframe": "H1",
        "tester_model": "every_tick_real_ticks",
        "order_kinds": ["buy_stop_limit", "sell_stop_limit"],
        "data_pack": "ICMarkets_EST7_2020_present",
        "status": "awaiting_mt5_capture",
        "files": {
            "ir": "strategy.ir.json",
            "deals": "mt5_deals.csv",
            "equity": "mt5_equity.csv",
            "judge": "qf_judge.json",
            "notes": "notes.md",
        },
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    if FIXTURE_IR.exists():
        shutil.copy2(FIXTURE_IR, out_dir / "strategy.ir.json")
    notes = """# Stop-limit EveryTick golden notes

1. Export `strategy.ir.json` to MQ5 (desktop or `quantforge export`).
2. Strategy Tester: model **Every tick based on real ticks**, bound symbol/broker.
3. Save deals + equity CSVs as `mt5_deals.csv` / `mt5_equity.csv`.
4. Run QuantForge Judge on the same window → `qf_judge.json`.
5. `python scripts/stop_limit_everytick_golden.py --compare {dir}`

External blocker without MT5: leave `status` as awaiting_mt5_capture.
""".format(
        dir=out_dir.as_posix()
    )
    (out_dir / "notes.md").write_text(notes, encoding="utf-8")
    # Placeholder CSVs so the tree is obvious in git status when filled.
    for name in ("mt5_deals.csv", "mt5_equity.csv"):
        path = out_dir / name
        if not path.exists():
            path.write_text("# awaiting MT5 capture\n", encoding="utf-8")
    print(f"prepared {out_dir}")


def compare(out_dir: Path) -> int:
    manifest_path = out_dir / "manifest.json"
    if not manifest_path.exists():
        print(f"missing manifest: {manifest_path}", file=sys.stderr)
        return 2
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    deals = out_dir / manifest["files"]["deals"]
    equity = out_dir / manifest["files"]["equity"]
    judge = out_dir / manifest["files"]["judge"]
    missing = [p.name for p in (deals, equity, judge) if not p.exists() or p.stat().st_size < 32]
    # Placeholder CSVs start with '# awaiting'
    for path in (deals, equity):
        if path.exists() and path.read_text(encoding="utf-8", errors="ignore").lstrip().startswith("#"):
            missing.append(path.name)
    if missing:
        print(
            "golden incomplete — capture still required for: " + ", ".join(sorted(set(missing))),
            file=sys.stderr,
        )
        print("status:", manifest.get("status", "unknown"))
        return 1
    print(
        "inputs present; run `quantforge parity` with mt5-parity-v2 against "
        f"{deals.name} / {equity.name} / {judge.name}"
    )
    print("(automated numeric compare hooks into quantforge-parity in a follow-up)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true", help="create stub golden directory")
    parser.add_argument("--compare", metavar="DIR", help="validate golden dir readiness")
    parser.add_argument(
        "--out",
        type=Path,
        default=DEFAULT_DIR,
        help=f"output directory (default {DEFAULT_DIR})",
    )
    args = parser.parse_args()
    if args.prepare:
        prepare(args.out)
        return 0
    if args.compare:
        return compare(Path(args.compare))
    parser.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
