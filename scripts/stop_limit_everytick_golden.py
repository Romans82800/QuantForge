#!/usr/bin/env python3
"""Prepare / compare StopLimit EveryTick golden directories.

--prepare  writes the stub layout for a later MT5 capture.
--compare  runs a real numeric pass/fail once deals + reference metrics exist:
  1. Prefer `quantforge parity` when evidence/mq5/metadata are present.
  2. Otherwise compare inline metrics (judge/reference vs MT5 deals/equity or
     sidecar `*_metrics.json`) with mt5-parity-v2 default tolerances.
--write-fixture  materializes a self-contained numeric fixture that --compare passes.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_TAG = "golden_stub"
DEFAULT_DIR = ROOT / "parity" / "stop_limit" / DEFAULT_TAG
FIXTURE_DIR = ROOT / "parity" / "stop_limit" / "golden_numeric_fixture"
FIXTURE_IR = ROOT / "fixtures" / "stop_limit_pending_strategy.json"

# Match crates/quantforge-parity ParityTolerances::default()
DEFAULT_TOLERANCES = {
    "trade_count_relative": 0.10,
    "trade_count_absolute": 3,
    "net_profit_relative": 0.15,
    "max_drawdown_relative": 0.15,
    "max_equity_divergence_percent": 5.0,
}


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
            "reference_metrics": "reference_metrics.json",
            "external_metrics": "external_metrics.json",
            "notes": "notes.md",
        },
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    if FIXTURE_IR.exists():
        shutil.copy2(FIXTURE_IR, out_dir / "strategy.ir.json")
    notes = f"""# Stop-limit EveryTick golden notes

1. Export `strategy.ir.json` to MQ5 (desktop or `quantforge export`).
2. Strategy Tester: model **Every tick based on real ticks**, bound symbol/broker.
3. Save deals + equity CSVs as `mt5_deals.csv` / `mt5_equity.csv`.
4. Run QuantForge Judge on the same window → `qf_judge.json`
   (or drop matching `reference_metrics.json` / `external_metrics.json`).
5. `python scripts/stop_limit_everytick_golden.py --compare {out_dir.as_posix()}`

External blocker without MT5: leave `status` as awaiting_mt5_capture.
"""
    (out_dir / "notes.md").write_text(notes, encoding="utf-8")
    for name in ("mt5_deals.csv", "mt5_equity.csv"):
        path = out_dir / name
        if not path.exists():
            path.write_text("# awaiting MT5 capture\n", encoding="utf-8")
    print(f"prepared {out_dir}")


def write_fixture(out_dir: Path) -> None:
    """Self-contained numeric golden that --compare can pass without MT5."""
    out_dir.mkdir(parents=True, exist_ok=True)
    prepare(out_dir)
    metrics = {
        "trade_count": 4,
        "net_profit": 1250.0,
        "max_drawdown": 320.0,
        "initial_balance": 100_000.0,
        "ending_balance": 101_250.0,
        "equity": [
            {"timestamp_ms": 1_600_000_000_000, "equity": 100_000.0},
            {"timestamp_ms": 1_600_003_600_000, "equity": 100_400.0},
            {"timestamp_ms": 1_600_007_200_000, "equity": 99_900.0},
            {"timestamp_ms": 1_600_010_800_000, "equity": 101_250.0},
        ],
    }
    (out_dir / "reference_metrics.json").write_text(
        json.dumps(metrics, indent=2) + "\n", encoding="utf-8"
    )
    (out_dir / "external_metrics.json").write_text(
        json.dumps(metrics, indent=2) + "\n", encoding="utf-8"
    )
    # Minimal deal/equity rows so readiness checks pass.
    deals = (
        "Time,Deal,Symbol,Type,Direction,Volume,Price,Order,Commission,Swap,Profit,Balance,Comment\n"
        "2020.10.01 10:00:00,1,EURNZD,buy,in,1.00,1.70000,1,0,0,0,100000,open\n"
        "2020.10.01 12:00:00,2,EURNZD,sell,out,1.00,1.70200,1,0,0,200,100200,tp\n"
        "2020.10.02 10:00:00,3,EURNZD,buy,in,1.00,1.70100,2,0,0,0,100200,open\n"
        "2020.10.02 14:00:00,4,EURNZD,sell,out,1.00,1.70500,2,0,0,400,100600,tp\n"
        "2020.10.03 10:00:00,5,EURNZD,sell,in,1.00,1.70600,3,0,0,0,100600,open\n"
        "2020.10.03 15:00:00,6,EURNZD,buy,out,1.00,1.70300,3,0,0,300,100900,tp\n"
        "2020.10.04 10:00:00,7,EURNZD,buy,in,1.00,1.70400,4,0,0,0,100900,open\n"
        "2020.10.04 16:00:00,8,EURNZD,sell,out,1.00,1.70750,4,0,0,350,101250,tp\n"
    )
    (out_dir / "mt5_deals.csv").write_text(deals, encoding="utf-8")
    equity = (
        "Time,Equity\n"
        "2020.10.01 10:00:00,100000\n"
        "2020.10.01 12:00:00,100400\n"
        "2020.10.02 14:00:00,99900\n"
        "2020.10.04 16:00:00,101250\n"
    )
    (out_dir / "mt5_equity.csv").write_text(equity, encoding="utf-8")
    judge = {
        "result": {
            "metrics": {
                "trade_count": metrics["trade_count"],
                "net_profit": metrics["net_profit"],
                "max_drawdown": metrics["max_drawdown"],
                "initial_balance": metrics["initial_balance"],
                "ending_balance": metrics["ending_balance"],
            },
            "equity": metrics["equity"],
            "trades": [],
        },
        "engine": "ohlc-scout-fixture",
    }
    (out_dir / "qf_judge.json").write_text(json.dumps(judge, indent=2) + "\n", encoding="utf-8")
    manifest = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))
    manifest["status"] = "numeric_fixture_ready"
    manifest["tag"] = out_dir.name
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (out_dir / "notes.md").write_text(
        "# Numeric fixture\n\nSynthetic matching metrics for CI `--compare` pass without MT5.\n",
        encoding="utf-8",
    )
    print(f"wrote numeric fixture {out_dir}")


def _is_placeholder(path: Path) -> bool:
    if not path.exists() or path.stat().st_size < 16:
        return True
    text = path.read_text(encoding="utf-8", errors="ignore").lstrip()
    return text.startswith("#") or "awaiting" in text[:80].lower()


def _relative_delta(delta: float, reference: float) -> float:
    denom = abs(reference)
    if denom < 1.0e-12:
        return 0.0 if abs(delta) < 1.0e-12 else math.inf
    return abs(delta) / denom


def _load_metrics_blob(path: Path) -> dict[str, Any] | None:
    if not path.exists() or _is_placeholder(path):
        return None
    data = json.loads(path.read_text(encoding="utf-8"))
    if "result" in data and isinstance(data["result"], dict):
        metrics = data["result"].get("metrics", data["result"])
        equity = data["result"].get("equity", data.get("equity", []))
    elif "metrics" in data:
        metrics = data["metrics"]
        equity = data.get("equity", [])
    else:
        metrics = data
        equity = data.get("equity", [])
    return {
        "trade_count": int(metrics.get("trade_count", 0)),
        "net_profit": float(metrics.get("net_profit", 0.0)),
        "max_drawdown": float(metrics.get("max_drawdown", 0.0)),
        "initial_balance": float(metrics.get("initial_balance", 100_000.0)),
        "ending_balance": float(
            metrics.get("ending_balance", metrics.get("initial_balance", 100_000.0))
        ),
        "equity": equity if isinstance(equity, list) else [],
    }


def _metrics_from_deals_equity(deals: Path, equity: Path, initial_balance: float) -> dict[str, Any]:
    profits: list[float] = []
    with deals.open(newline="", encoding="utf-8", errors="ignore") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            keys = {k.lower(): k for k in row}
            profit_key = keys.get("profit")
            direction_key = keys.get("direction") or keys.get("entry")
            if not profit_key:
                continue
            try:
                profit = float(row[profit_key] or 0.0)
            except ValueError:
                continue
            direction = (row.get(direction_key, "") if direction_key else "").lower()
            # Count closed-out deals with non-zero profit, else any non-zero profit row.
            if direction in {"out", "out closing", "close"} or (
                not direction and abs(profit) > 1.0e-12
            ):
                if abs(profit) > 1.0e-12:
                    profits.append(profit)
    trade_count = len(profits)
    net_profit = sum(profits)

    equity_points: list[float] = []
    with equity.open(newline="", encoding="utf-8", errors="ignore") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            keys = {k.lower(): k for k in row}
            eq_key = keys.get("equity") or keys.get("balance")
            if not eq_key:
                continue
            try:
                equity_points.append(float(row[eq_key]))
            except ValueError:
                continue
    if not equity_points:
        equity_points = [initial_balance, initial_balance + net_profit]
    peak = equity_points[0]
    max_dd = 0.0
    for value in equity_points:
        peak = max(peak, value)
        max_dd = max(max_dd, peak - value)
    return {
        "trade_count": trade_count,
        "net_profit": net_profit,
        "max_drawdown": max_dd,
        "initial_balance": initial_balance,
        "ending_balance": equity_points[-1],
        "equity": [{"equity": value} for value in equity_points],
    }


def _equity_path_divergence(ref: dict[str, Any], ext: dict[str, Any], samples: int = 256) -> float:
    ref_eq = [float(p.get("equity", 0.0)) for p in ref.get("equity", []) if isinstance(p, dict)]
    ext_eq = [float(p.get("equity", 0.0)) for p in ext.get("equity", []) if isinstance(p, dict)]
    if not ref_eq or not ext_eq:
        return 0.0
    n = min(samples, len(ref_eq), len(ext_eq))
    if n <= 0:
        return 0.0
    worst = 0.0
    for i in range(n):
        ri = int(i * (len(ref_eq) - 1) / max(n - 1, 1))
        ei = int(i * (len(ext_eq) - 1) / max(n - 1, 1))
        worst = max(worst, abs(ref_eq[ri] - ext_eq[ei]))
    return worst


def compare_metrics(
    reference: dict[str, Any],
    external: dict[str, Any],
    tolerances: dict[str, float] | None = None,
) -> dict[str, Any]:
    tol = {**DEFAULT_TOLERANCES, **(tolerances or {})}
    trade_count_delta = external["trade_count"] - reference["trade_count"]
    allowed = max(
        int(tol["trade_count_absolute"]),
        int(math.ceil(reference["trade_count"] * tol["trade_count_relative"])),
    )
    trade_count_passed = abs(trade_count_delta) <= allowed

    net_profit_delta = external["net_profit"] - reference["net_profit"]
    net_profit_delta_relative = _relative_delta(net_profit_delta, reference["net_profit"])
    net_profit_passed = net_profit_delta_relative <= tol["net_profit_relative"]

    max_drawdown_delta = external["max_drawdown"] - reference["max_drawdown"]
    max_drawdown_delta_relative = _relative_delta(max_drawdown_delta, reference["max_drawdown"])
    max_drawdown_passed = max_drawdown_delta_relative <= tol["max_drawdown_relative"]

    path_div = _equity_path_divergence(reference, external)
    initial = max(reference.get("initial_balance", 0.0), 1.0e-12)
    path_div_pct = path_div / initial * 100.0
    equity_path_passed = path_div_pct <= tol["max_equity_divergence_percent"]

    passed = (
        trade_count_passed
        and net_profit_passed
        and max_drawdown_passed
        and equity_path_passed
    )
    return {
        "protocol_version": "mt5-parity-v2",
        "mode": "inline_metrics",
        "passed": passed,
        "trade_count_delta": trade_count_delta,
        "allowed_trade_count_delta": allowed,
        "trade_count_passed": trade_count_passed,
        "net_profit_delta": net_profit_delta,
        "net_profit_delta_relative": net_profit_delta_relative,
        "net_profit_passed": net_profit_passed,
        "max_drawdown_delta": max_drawdown_delta,
        "max_drawdown_delta_relative": max_drawdown_delta_relative,
        "max_drawdown_passed": max_drawdown_passed,
        "max_equity_path_divergence": path_div,
        "max_equity_path_divergence_percent": path_div_pct,
        "equity_path_passed": equity_path_passed,
        "tolerances": tol,
        "reference": {
            "trade_count": reference["trade_count"],
            "net_profit": reference["net_profit"],
            "max_drawdown": reference["max_drawdown"],
        },
        "external": {
            "trade_count": external["trade_count"],
            "net_profit": external["net_profit"],
            "max_drawdown": external["max_drawdown"],
        },
    }


def _try_quantforge_parity(out_dir: Path, manifest: dict[str, Any]) -> int | None:
    """Return exit code if full parity inputs exist and quantforge is runnable."""
    files = manifest.get("files", {})
    evidence = out_dir / files.get("evidence", "evidence.json")
    mq5 = out_dir / files.get("mq5", "expert.mq5")
    metadata = out_dir / files.get("metadata", "mt5_metadata.json")
    judge = out_dir / files.get("judge", "qf_judge.json")
    deals = out_dir / files.get("deals", "mt5_deals.csv")
    equity = out_dir / files.get("equity", "mt5_equity.csv")
    needed = [evidence, mq5, metadata, judge, deals, equity]
    if any(_is_placeholder(path) or not path.exists() for path in needed):
        return None
    report_out = out_dir / "parity_report.json"
    if report_out.exists():
        report_out.unlink()
    cmd = [
        "quantforge",
        "parity",
        "--scout-result",
        str(judge),
        "--evidence",
        str(evidence),
        "--mq5",
        str(mq5),
        "--mt5-deals",
        str(deals),
        "--mt5-equity",
        str(equity),
        "--mt5-metadata",
        str(metadata),
        "--out",
        str(report_out),
    ]
    try:
        completed = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except FileNotFoundError:
        print("quantforge CLI not on PATH; falling back to inline metrics", file=sys.stderr)
        return None
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    if completed.returncode != 0:
        return completed.returncode
    if report_out.exists():
        report = json.loads(report_out.read_text(encoding="utf-8"))
        passed = bool(report.get("passed", report.get("report", {}).get("passed")))
        print("quantforge parity:", "PASS" if passed else "FAIL")
        return 0 if passed else 1
    return completed.returncode


def compare(out_dir: Path) -> int:
    manifest_path = out_dir / "manifest.json"
    if not manifest_path.exists():
        print(f"missing manifest: {manifest_path}", file=sys.stderr)
        return 2
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    files = manifest.get("files", {})
    deals = out_dir / files.get("deals", "mt5_deals.csv")
    equity = out_dir / files.get("equity", "mt5_equity.csv")
    judge = out_dir / files.get("judge", "qf_judge.json")
    ref_sidecar = out_dir / files.get("reference_metrics", "reference_metrics.json")
    ext_sidecar = out_dir / files.get("external_metrics", "external_metrics.json")

    parity_code = _try_quantforge_parity(out_dir, manifest)
    if parity_code is not None:
        return parity_code

    reference = _load_metrics_blob(ref_sidecar) or _load_metrics_blob(judge)
    external = _load_metrics_blob(ext_sidecar)
    if external is None and not _is_placeholder(deals) and not _is_placeholder(equity):
        initial = float((reference or {}).get("initial_balance", 100_000.0))
        external = _metrics_from_deals_equity(deals, equity, initial)

    missing: list[str] = []
    if reference is None:
        missing.append("reference_metrics.json|qf_judge.json")
    if external is None:
        missing.append("external_metrics.json|mt5_deals.csv+mt5_equity.csv")
    if missing:
        print(
            "golden incomplete — capture still required for: " + ", ".join(missing),
            file=sys.stderr,
        )
        print("status:", manifest.get("status", "unknown"))
        return 1

    report = compare_metrics(reference, external)
    report_path = out_dir / "compare_report.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    print("wrote", report_path)
    print("PASS" if report["passed"] else "FAIL")
    return 0 if report["passed"] else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true", help="create stub golden directory")
    parser.add_argument("--compare", metavar="DIR", help="numeric pass/fail vs golden metrics")
    parser.add_argument(
        "--write-fixture",
        action="store_true",
        help=f"write self-contained numeric fixture at {FIXTURE_DIR}",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=DEFAULT_DIR,
        help=f"output directory (default {DEFAULT_DIR})",
    )
    args = parser.parse_args()
    if args.write_fixture:
        write_fixture(FIXTURE_DIR if args.out == DEFAULT_DIR else args.out)
        return 0
    if args.prepare:
        prepare(args.out)
        return 0
    if args.compare:
        return compare(Path(args.compare))
    parser.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
