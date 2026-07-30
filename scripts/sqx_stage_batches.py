#!/usr/bin/env python3
"""Stage .sqx strategies into one folder per retest config, smallest batch first.

Reads the group summary produced by sqx_config_groups.py and links each strategy
into SQX_all/_batches/<NN>_<feed>/. Hard links are used so a batch costs no extra
disk and SQX sees ordinary files; symlinks are the fallback across filesystems.

Batches whose data feed is not loaded in SQX are suffixed __NODATA, since a retest
cannot reproduce the original backtest without the feed it was built on.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sqlite3
from pathlib import Path

SQX_DATA_DB = Path(
    "/Applications/StrategyQuantXB144_arm.app/Contents/Resources/user/data/data.db"
)


def loaded_feeds(db: Path) -> dict[str, tuple[str, str, int]]:
    """Map lowercased data symbol -> (from, to, rows) for feeds holding real bars."""
    if not db.exists():
        return {}
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        rows = con.execute(
            "SELECT SYMBOL, date(DATEFROM/1000,'unixepoch'), "
            "date(DATETO/1000,'unixepoch'), ROWS FROM DATA WHERE ROWS > 0"
        ).fetchall()
    finally:
        con.close()
    return {r[0].lower(): (r[1], r[2], r[3]) for r in rows}


def link(src: Path, dst: Path) -> None:
    if dst.exists():
        return
    try:
        os.link(src, dst)
    except OSError:
        dst.symlink_to(src)


def safe_name(text: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "_", text)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("groups", type=Path, help="groups.json from sqx_config_groups.py")
    ap.add_argument("out", type=Path, help="output directory for batch folders")
    ap.add_argument(
        "--pilot",
        type=int,
        default=0,
        help="also stage a PILOT folder with N strategies from the given feed",
    )
    ap.add_argument("--pilot-feed", help="feed name for the pilot batch")
    ap.add_argument(
        "--max-complexity",
        type=int,
        help="prefer strategies at or below this complexity for the pilot",
    )
    ap.add_argument("--csv", type=Path, help="strategies.csv, for complexity lookup")
    ap.add_argument("--clean", action="store_true", help="remove existing out dir first")
    args = ap.parse_args()

    groups = json.loads(args.groups.read_text())
    feeds = loaded_feeds(SQX_DATA_DB)

    if args.clean and args.out.exists():
        shutil.rmtree(args.out)
    args.out.mkdir(parents=True, exist_ok=True)

    complexity: dict[str, int] = {}
    if args.csv and args.csv.exists():
        import csv as _csv

        for row in _csv.DictReader(args.csv.open()):
            if row["complexity"]:
                complexity[row["file"]] = int(row["complexity"])

    # smallest batch first, so the cheap feeds validate the pipeline
    ordered = sorted(groups, key=lambda g: g["count"])

    print(f"{'batch':<44} {'n':>5}  data")
    manifest = []
    for i, g in enumerate(ordered, start=1):
        feed = g["symbol"] or "unknown"
        have = feeds.get(feed.lower())
        tag = "" if have else "__NODATA"
        name = f"{i:02d}_{safe_name(feed)}_{safe_name(g['timeframe'] or '?')}{tag}"
        folder = args.out / name
        folder.mkdir(parents=True, exist_ok=True)
        for f in g["files"]:
            src = Path(f)
            link(src, folder / src.name)
        status = (
            f"{have[0]} -> {have[1]} ({have[2]:,} rows)" if have else "MISSING in SQX"
        )
        print(f"{name:<44} {g['count']:>5}  {status}")
        manifest.append(
            {
                "batch": name,
                "feed": feed,
                "timeframe": g["timeframe"],
                "strategy_range": [g["history_from"], g["history_to"]],
                "count": g["count"],
                "data_available": bool(have),
                "data_range": [have[0], have[1]] if have else None,
            }
        )

    if args.pilot and args.pilot_feed:
        candidates = [
            f
            for g in groups
            if g["symbol"] == args.pilot_feed
            for f in g["files"]
        ]
        if args.max_complexity is not None:
            simple = [
                f
                for f in candidates
                if complexity.get(f, 10**6) <= args.max_complexity
            ]
            candidates = simple or candidates
        candidates.sort(key=lambda f: (complexity.get(f, 10**6), f))
        chosen = candidates[: args.pilot]
        pilot = args.out / f"PILOT_{safe_name(args.pilot_feed)}_{len(chosen)}"
        pilot.mkdir(parents=True, exist_ok=True)
        for f in chosen:
            src = Path(f)
            link(src, pilot / src.name)
        print(f"\npilot -> {pilot}")
        for f in chosen:
            print(f"  complexity {complexity.get(f,'?'):>3}  {Path(f).name}")
        manifest.append(
            {
                "batch": pilot.name,
                "feed": args.pilot_feed,
                "count": len(chosen),
                "pilot": True,
                "files": [Path(f).name for f in chosen],
            }
        )

    (args.out / "_manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"\nwrote {args.out / '_manifest.json'}")
    blocked = sum(m["count"] for m in manifest if m.get("data_available") is False)
    total = sum(m["count"] for m in manifest if "data_available" in m)
    print(f"runnable now: {total - blocked:,} of {total:,} strategies")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
