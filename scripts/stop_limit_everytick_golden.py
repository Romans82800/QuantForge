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
import os
import re
import shutil
import subprocess
import sys
import time
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
        "--broker-timezone",
        str(manifest.get("broker_timezone", "ICMarkets/EST+7")),
        "--trade-timestamp-tolerance-ms",
        "60000",
        "--trade-count-relative",
        "0.10",
        "--trade-count-absolute",
        "8",
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


def _slice_tsv(src: Path, dst: Path, from_date: str, to_date: str) -> int:
    """Keep header + rows with DATE in [from_date, to_date). Dates are YYYY.MM.DD."""
    kept = 0
    with src.open(encoding="utf-8", errors="ignore") as fin, dst.open(
        "w", encoding="utf-8", newline=""
    ) as fout:
        header = fin.readline()
        fout.write(header)
        for line in fin:
            date = line.split("\t", 1)[0]
            if from_date <= date < to_date:
                fout.write(line)
                kept += 1
    print(f"sliced {src.name} -> {dst.name} ({kept} rows)", flush=True)
    return kept


def load_mt5_credentials() -> dict[str, str]:
    """Load demo login from env or gitignored `.mt5-demo.local`. Never print secrets."""
    values: dict[str, str] = {}
    local = ROOT / ".mt5-demo.local"
    if local.is_file():
        for line in local.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip().strip('"').strip("'")
    login = os.environ.get("MT5_LOGIN") or values.get("MT5_LOGIN") or values.get("Login")
    password = (
        os.environ.get("MT5_PASSWORD") or values.get("MT5_PASSWORD") or values.get("Password")
    )
    server = (
        os.environ.get("MT5_SERVER")
        or values.get("MT5_SERVER")
        or values.get("Server")
        or "ICMarketsSC-Demo"
    )
    if not login or not password:
        raise SystemExit(
            "MT5 demo credentials missing. Set MT5_LOGIN/MT5_PASSWORD/MT5_SERVER "
            "or create gitignored .mt5-demo.local (see docs/STOP_LIMIT_EVERYTICK_GOLDEN.md)."
        )
    return {"login": login, "password": password, "server": server}


def _find_quantforge() -> Path:
    target_roots = []
    if os.environ.get("CARGO_TARGET_DIR"):
        target_roots.append(Path(os.environ["CARGO_TARGET_DIR"]))
    target_roots.append(ROOT / "target")
    candidates: list[Path] = []
    for root in target_roots:
        candidates.extend(
            [
                root / "release" / "quantforge.exe",
                root / "release" / "quantforge",
                root / "debug" / "quantforge.exe",
                root / "debug" / "quantforge",
            ]
        )
    for path in candidates:
        if path.is_file():
            return path
    which = shutil.which("quantforge")
    if which:
        return Path(which)
    raise SystemExit("quantforge CLI not found — build with cargo build -p quantforge-cli --release")


def _terminal_data_dir() -> Path:
    appdata = Path(os.environ.get("APPDATA", ""))
    origin_hit = appdata / "MetaQuotes" / "Terminal"
    preferred = origin_hit / "D0E8209F77C8CF37AD8BF550E51FF075"
    if preferred.is_dir():
        return preferred
    if origin_hit.is_dir():
        for child in origin_hit.iterdir():
            origin = child / "origin.txt"
            if origin.is_file() and "MetaTrader 5" in origin.read_text(encoding="utf-8", errors="ignore"):
                if (child / "MQL5").is_dir():
                    return child
    raise SystemExit("MT5 terminal data directory not found under %APPDATA%/MetaQuotes/Terminal")


def _inject_login(tester_ini: Path, creds: dict[str, str], out_ini: Path) -> None:
    body = tester_ini.read_text(encoding="utf-8", errors="ignore")
    if "[Common]" in body:
        raise SystemExit("tester.ini already contains [Common]; refusing to overwrite login block")
    header = (
        "[Common]\n"
        f"Login={creds['login']}\n"
        f"Password={creds['password']}\n"
        f"Server={creds['server']}\n"
        "ProxyEnable=0\n"
        "KeepPrivate=1\n"
        "NewsEnable=0\n"
        "CertInstall=0\n"
        "UpdateNews=0\n\n"
    )
    out_ini.write_text(header + body, encoding="utf-8")


def _run(cmd: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
    printable = []
    for item in cmd:
        # Never echo password-bearing args (none expected; defense in depth).
        printable.append("<redacted>" if "Password=" in item else str(item))
    print("+", " ".join(printable), flush=True)
    return subprocess.run(cmd, cwd=ROOT, text=True, **kwargs)


def capture(
    out_dir: Path,
    *,
    from_date: str,
    to_date: str,
    symbol: str,
    timeout_seconds: int,
    tester_model: int,
) -> int:
    """Export + compile + Strategy Tester EveryTick capture into out_dir."""
    creds = load_mt5_credentials()
    print(
        f"using MT5 demo login={creds['login']} server={creds['server']} (password redacted)",
        flush=True,
    )
    prepare(out_dir)
    pack = Path(
        os.environ.get(
            "QUANTFORGE_DATA_PACK",
            r"C:\Users\Administrator\Documents\QuantForge\ICMarkets_EST7_2020_present",
        )
    )
    if not pack.is_dir():
        raise SystemExit(f"data pack missing: {pack}")

    h1 = pack / f"ICMarketsSC-Demo_{symbol}_H1_2020_present.tsv"
    m1 = pack / f"ICMarketsSC-Demo_{symbol}_M1_2020_present.tsv"
    broker = pack / f"{symbol}.broker.json"
    for path in (h1, m1, broker):
        if not path.is_file():
            raise SystemExit(f"required pack file missing: {path}")

    strategy = out_dir / "strategy.ir.json"
    if not strategy.is_file() and FIXTURE_IR.is_file():
        shutil.copy2(FIXTURE_IR, strategy)

    qf = _find_quantforge()
    work = ROOT / "runs" / "stop-limit-everytick-capture"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    export_dir = work / "export"
    export_dir.mkdir()
    expert = "QFStopLimitGolden"

    # Slice pack data to the same tester window so Judge and MT5 share the window.
    h1_slice = work / f"{symbol}_H1.tsv"
    m1_slice = work / f"{symbol}_M1.tsv"
    _slice_tsv(h1, h1_slice, from_date, to_date)
    _slice_tsv(m1, m1_slice, from_date, to_date)

    judge_out = out_dir / "qf_judge.json"
    if judge_out.exists():
        judge_out.unlink()
    r = _run(
        [
            str(qf),
            "judge",
            str(h1_slice),
            "--source-timezone",
            "ICMarkets/EST+7",
            "--m1",
            str(m1_slice),
            "--m1-source-timezone",
            "ICMarkets/EST+7",
            "--strategy",
            str(strategy),
            "--broker",
            str(broker),
            "--commission-per-lot-round-turn",
            "7",
            "--fallback-spread-points",
            "1",
            "--initial-balance",
            "100000",
            "--allow-execution-gaps",
            "--allow-failed-data",
            "--out",
            str(judge_out),
        ],
        capture_output=True,
    )
    sys.stdout.write(r.stdout)
    sys.stderr.write(r.stderr)
    if r.returncode:
        return r.returncode
    # Export + compile (model 4 = EveryTick real ticks; model 1 = 1-minute OHLC).
    r = _run(
        [
            str(qf),
            "export",
            "--strategy",
            str(strategy),
            "--broker",
            str(broker),
            "--out",
            str(export_dir),
            "--expert-name",
            expert,
            "--expert-directory",
            "QuantForge",
            "--timeframe",
            "H1",
            "--from-date",
            from_date,
            "--to-date",
            to_date,
            "--commission-per-lot-round-turn",
            "7",
            "--deposit",
            "100000",
            "--tester-model",
            str(tester_model),
            "--compile",
        ],
        capture_output=True,
    )
    sys.stdout.write(r.stdout)
    sys.stderr.write(r.stderr)
    if r.returncode:
        return r.returncode

    portable_candidates = []
    env_portable = os.environ.get("QUANTFORGE_MT5_PORTABLE", "").strip()
    if env_portable:
        portable_candidates.append(Path(env_portable))
    portable_candidates.extend(
        [
            Path(r"C:\Users\Administrator\Documents\Codex\2026-07-05\we\work\MT5_Backtest_A240"),
            Path(r"C:\Users\Administrator\Documents\Codex\2026-07-05\we\work\MT5_Backtest_A240_2"),
            Path(r"C:\Users\Administrator\Documents\Codex\2026-07-05\we\work\MT5_Backtest_A240_3"),
            Path(r"C:\Program Files\MetaTrader 5"),
        ]
    )
    terminal_exe = None
    portable = False
    for candidate in portable_candidates:
        exe = candidate / "terminal64.exe"
        has_config = (candidate / "Config").is_dir() or (candidate / "config").is_dir()
        is_program_files = candidate == Path(r"C:\Program Files\MetaTrader 5")
        if candidate.is_dir() and exe.is_file():
            # Prefer known ICMarkets portables (accounts.dat already logged in).
            if has_config and not is_program_files:
                terminal_exe = exe
                portable = True
                break
            if terminal_exe is None:
                terminal_exe = exe
                portable = False
    if terminal_exe is None:
        raise SystemExit("no MT5 terminal64.exe found")

    if portable:
        terminal_data = terminal_exe.parent
        print(f"using portable MT5 at {terminal_data}", flush=True)
    else:
        terminal_data = _terminal_data_dir()
        print(f"using installed MT5 data dir {terminal_data}", flush=True)

    experts = terminal_data / "MQL5" / "Experts" / "QuantForge"
    indicators = terminal_data / "MQL5" / "Indicators"
    experts.mkdir(parents=True, exist_ok=True)
    for src in export_dir.iterdir():
        if src.is_file() and src.suffix.lower() in {".mq5", ".ex5", ".set", ".evidence.json"}:
            shutil.copy2(src, experts / src.name)
    support = export_dir / "Indicators"
    if support.is_dir():
        for src in support.rglob("*"):
            if src.is_file():
                dst = indicators / src.relative_to(support)
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)

    ex5 = experts / f"{expert}.ex5"
    if not ex5.is_file():
        alt = export_dir / f"{expert}.ex5"
        if alt.is_file():
            shutil.copy2(alt, ex5)
        else:
            print(f"compiled EA missing: {ex5}", file=sys.stderr)
            return 1

    evidence = export_dir / f"{expert}.evidence.json"
    shutil.copy2(evidence, out_dir / "evidence.json")
    shutil.copy2(export_dir / f"{expert}.mq5", out_dir / "expert.mq5")

    evidence_data = json.loads(evidence.read_text(encoding="utf-8"))
    common_files = Path(os.environ["APPDATA"]) / "MetaQuotes" / "Terminal" / "Common" / "Files"
    common_files.mkdir(parents=True, exist_ok=True)

    def win_to_path(rel: str) -> Path:
        return common_files.joinpath(*rel.replace("\\", "/").split("/"))

    deals = win_to_path(evidence_data["parity_deals_file"])
    equity = win_to_path(evidence_data["parity_equity_file"])
    metadata = win_to_path(evidence_data["parity_metadata_file"])
    for path in (deals, equity, metadata):
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            path.unlink()

    subprocess.run(
        ["taskkill", "/F", "/IM", "terminal64.exe"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    subprocess.run(
        ["taskkill", "/F", "/IM", "metatester64.exe"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    time.sleep(2)

    source_ini = export_dir / f"{expert}.tester.ini"
    # Portable ICMarkets installs already store the demo account; omit password.
    # Match working worker INI style: Login+Server only, Leverage=100.
    body = source_ini.read_text(encoding="utf-8", errors="ignore")
    body = body.replace("Leverage=1:100", "Leverage=100")
    if portable:
        # Account is already saved in portable accounts.dat — do not embed password.
        header = (
            "[Common]\n"
            f"Login={creds['login']}\n"
            f"Server={creds['server']}\n"
            "CertInstall=0\n"
            "NewsEnable=0\n\n"
        )
        if "[Common]" in body:
            # Replace generated body entirely below by stripping any prior Common.
            idx = body.find("[Tester]")
            body = body[idx:] if idx >= 0 else body
        logged_ini = work / "tester_portable.ini"
        logged_ini.write_text(header + body, encoding="utf-8")
    else:
        logged_ini = work / "tester_with_login.ini"
        _inject_login(source_ini, creds, logged_ini)

    redacted = logged_ini.read_text(encoding="utf-8")
    if creds.get("password"):
        redacted = redacted.replace(creds["password"], "***")
    # Never commit account numbers — scrub Login= even when password was omitted.
    redacted = re.sub(r"(?im)^(Login=)\d+", r"\1<redacted>", redacted)
    redacted = re.sub(r"(?im)^(Password=).+$", r"\1***", redacted)
    (out_dir / "tester_config.redacted.ini").write_text(redacted, encoding="utf-8")

    mt5_out = work / "mt5-run.json"
    if mt5_out.exists():
        mt5_out.unlink()
    mt5_cmd = [
        str(qf),
        "mt5-test",
        "--tester-ini",
        str(logged_ini),
        "--evidence",
        str(evidence),
        "--out",
        str(mt5_out),
        "--timeout-seconds",
        str(timeout_seconds),
        "--terminal",
        str(terminal_exe),
        "--common-files",
        str(common_files),
    ]
    if portable:
        mt5_cmd.append("--portable")
    r = _run(mt5_cmd, capture_output=True)
    sys.stdout.write(r.stdout)
    sys.stderr.write(r.stderr)

    capture_notes = [
        "# Capture attempt notes",
        "",
        "- login: <demo account from gitignored .mt5-demo.local / env>",
        f"- server: {creds['server']}",
        f"- terminal: `{terminal_exe}`",
        f"- portable: {portable}",
        f"- symbol: {symbol}",
        f"- window: {from_date} → {to_date}",
        f"- tester_model: {tester_model} (4 = every tick based on real ticks; 1 = 1-minute OHLC)",
        f"- mt5-test exit: {r.returncode}",
        f"- deals fresh path expected: `{deals}`",
        f"- equity fresh path expected: `{equity}`",
    ]
    if r.returncode != 0:
        capture_notes.append("- status: **blocked or failed** during mt5-test")
        (out_dir / "capture_notes.md").write_text("\n".join(capture_notes) + "\n", encoding="utf-8")
        for src, name in (
            (deals, "mt5_deals.csv"),
            (equity, "mt5_equity.csv"),
            (metadata, "mt5_metadata.json"),
        ):
            if src.is_file() and src.stat().st_size > 0:
                shutil.copy2(src, out_dir / name)
        return r.returncode

    for src, name in (
        (deals, "mt5_deals.csv"),
        (equity, "mt5_equity.csv"),
        (metadata, "mt5_metadata.json"),
    ):
        if not src.is_file():
            print(f"missing tester artifact: {src}", file=sys.stderr)
            capture_notes.append(f"- missing artifact: {src}")
            (out_dir / "capture_notes.md").write_text("\n".join(capture_notes) + "\n", encoding="utf-8")
            return 1
        shutil.copy2(src, out_dir / name)

    manifest = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))
    manifest["status"] = "mt5_capture_ready"
    manifest["symbol"] = symbol
    manifest["from_date"] = from_date
    manifest["to_date"] = to_date
    manifest["tester_model_code"] = tester_model
    manifest["broker_timezone"] = "ICMarkets/EST+7"
    manifest["files"]["evidence"] = "evidence.json"
    manifest["files"]["mq5"] = "expert.mq5"
    manifest["files"]["metadata"] = "mt5_metadata.json"
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    capture_notes.append("- status: capture artifacts written")
    (out_dir / "capture_notes.md").write_text("\n".join(capture_notes) + "\n", encoding="utf-8")
    print(f"captured into {out_dir}", flush=True)
    return compare(out_dir)


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
        "--capture",
        action="store_true",
        help="login via gitignored demo creds, export/compile, Strategy Tester, compare",
    )
    parser.add_argument("--from-date", default="2024.01.01")
    parser.add_argument("--to-date", default="2024.02.01")
    parser.add_argument("--symbol", default="EURNZD")
    parser.add_argument("--timeout-seconds", type=int, default=2400)
    parser.add_argument(
        "--tester-model",
        type=int,
        default=4,
        help="MT5 tester model (4=every tick real ticks, 1=1-minute OHLC)",
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
    if args.capture:
        out = (
            args.out
            if args.out != DEFAULT_DIR
            else ROOT / "parity" / "stop_limit" / "golden_live_eurnzd"
        )
        return capture(
            out,
            from_date=args.from_date,
            to_date=args.to_date,
            symbol=args.symbol,
            timeout_seconds=args.timeout_seconds,
            tester_model=args.tester_model,
        )
    if args.compare:
        return compare(Path(args.compare))
    parser.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
