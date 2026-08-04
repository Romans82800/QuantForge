#!/usr/bin/env python3
"""Certify QuantForge M1 replay against MT5's canonical bid/ask feed.

Each case is deliberately two phase: MT5 Model=1 first captures the exact
broker bid/ask M1 stream and its execution results; QuantForge then replays that
quote sidecar before strict, one-to-one reconciliation. No synthetic-spread or
trade-count tolerance is permitted in certification mode.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACK = ROOT / "ICMarkets_EST7_2020_present"
SYMBOL = "AUDUSD"
H1 = PACK / f"ICMarketsSC-Demo_{SYMBOL}_H1_2020_present.tsv"
M1 = PACK / f"ICMarketsSC-Demo_{SYMBOL}_M1_2020_present.tsv"
BROKER = PACK / f"{SYMBOL}.broker.json"
TZ = "ICMarkets/EST+7"
COMMISSION = 7.0
RUN_ROOT = ROOT / "runs" / "family-mt5-parity" / SYMBOL
TIMEFRAME = "H1"
TIME_STOP_OVERRIDE: int | None = None
QF = ROOT / "target/release/quantforge"
WINE = Path("/Applications/MetaTrader 5.app/Contents/SharedSupport/wine/bin/wine")
WINEPREFIX = Path.home() / "Library/Application Support/net.metaquotes.wine.metatrader5"
MT5 = WINEPREFIX / "drive_c/Program Files/MetaTrader 5"
EXPERTS = MT5 / "MQL5/Experts/QuantForge"
INDICATORS_DST = MT5 / "MQL5/Indicators"
INDICATORS_SRC = ROOT / "mql5/QuantForge/Indicators"
COMMON = (
    WINEPREFIX
    / "drive_c/users"
    / os.environ.get("USER", "danielagbonkpolor")
    / "AppData/Roaming/MetaQuotes/Terminal/Common/Files"
)

FAMILIES = [
    "trend_pullback",
    "momentum_burst",
    "donchian_breakout",
    "mean_reversion_band",
    "zscore_reversion",
    "session_orb",
    "impulse_candle",
    "vol_squeeze_break",
    "supply_demand_reclaim",
    "sweep_reclaim",
]
MODES = ["market", "pending"]


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run(cmd, cwd=ROOT, text=True, **kwargs)


def kill_mt5() -> None:
    env = {**os.environ, "WINEPREFIX": str(WINEPREFIX), "WINEDEBUG": "-all"}
    subprocess.run(
        [str(WINE), "taskkill", "/F", "/IM", "terminal64.exe"],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    subprocess.run(
        [str(WINE), "taskkill", "/F", "/IM", "metatester64.exe"],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1)


def ensure_tools() -> None:
    # Export templates are compiled into the CLI binary. Always rebuild so a
    # parity run cannot silently exercise a stale EA after template changes.
    r = run(["cargo", "build", "-p", "quantforge-cli", "--release"])
    if r.returncode:
        sys.exit(r.returncode)
    r = run(
        [
            "cargo",
            "build",
            "-p",
            "quantforge-discover",
            "--example",
            "emit_family_strategy",
            "--release",
        ]
    )
    if r.returncode:
        sys.exit(r.returncode)
    EXPERTS.mkdir(parents=True, exist_ok=True)


def emit_strategy(family: str, mode: str, out: Path, sequence: int) -> None:
    emit = ROOT / "target/release/examples/emit_family_strategy"
    r = run(
        [
            str(emit),
            "--family",
            family,
            "--mode",
            mode,
            "--sequence",
            str(sequence),
            "--out",
            str(out),
        ]
    )
    if r.returncode:
        raise RuntimeError(f"emit failed for {family}/{mode}")
    if TIME_STOP_OVERRIDE is not None:
        strategy = json.loads(out.read_text())
        strategy["manage"]["time_stop_bars"] = TIME_STOP_OVERRIDE
        out.write_text(json.dumps(strategy, indent=2) + "\n")


def slice_tsv(src: Path, dst: Path, from_date: str, to_date: str) -> None:
    """Keep header + rows with DATE in [from_date, to_date). Dates are YYYY.MM.DD."""
    with src.open() as fin, dst.open("w") as fout:
        header = fin.readline()
        fout.write(header)
        for line in fin:
            date = line.split("\t", 1)[0]
            if from_date <= date < to_date:
                fout.write(line)


def write_bid_m1_from_quotes(quotes: Path, destination: Path, point: float) -> None:
    """Materialize the capture EA's bid bars as canonical MT5 M1 TSV.

    The timestamp integer deliberately remains broker-wall epoch here; the QF
    loader and the quote loader both apply the same source timezone afterward.
    """
    with quotes.open(newline="") as source, destination.open("w", newline="") as target:
        reader = csv.DictReader(source)
        target.write("<DATE>\t<TIME>\t<OPEN>\t<HIGH>\t<LOW>\t<CLOSE>\t<TICKVOL>\t<VOL>\t<SPREAD>\n")
        rows = 0
        for row in reader:
            timestamp = dt.datetime.utcfromtimestamp(int(row["timestamp_ms"]) / 1000)
            spread = max(
                0,
                round((float(row["ask_close"]) - float(row["bid_close"])) / point),
            )
            target.write(
                "\t".join(
                    [
                        timestamp.strftime("%Y.%m.%d"),
                        timestamp.strftime("%H:%M:%S"),
                        row["bid_open"],
                        row["bid_high"],
                        row["bid_low"],
                        row["bid_close"],
                        row.get("tick_count") or "0",
                        "0",
                        str(spread),
                    ]
                )
                + "\n"
            )
            rows += 1
    if rows < 2:
        raise RuntimeError("MT5 quote capture produced fewer than two M1 bid bars")


def aggregate_bid_decision_bars(m1: Path, destination: Path, timeframe: str) -> None:
    """Build the decision timeframe from canonical bid M1 broker-wall bars."""
    minutes = {"M15": 15, "H1": 60}.get(timeframe)
    if minutes is None:
        raise ValueError(f"unsupported canonical decision timeframe: {timeframe}")

    def bucket(row: dict[str, str]) -> dt.datetime:
        value = dt.datetime.strptime(
            f'{row["<DATE>"]} {row["<TIME>"]}', "%Y.%m.%d %H:%M:%S"
        )
        floored_minute = (value.minute // minutes) * minutes if minutes < 60 else 0
        return value.replace(minute=floored_minute, second=0, microsecond=0)

    with m1.open(newline="") as source:
        rows = list(csv.DictReader(source, delimiter="\t"))
    if not rows:
        raise RuntimeError("canonical M1 input is empty")

    groups: list[tuple[dt.datetime, list[dict[str, str]]]] = []
    for row in rows:
        key = bucket(row)
        if not groups or groups[-1][0] != key:
            groups.append((key, []))
        groups[-1][1].append(row)

    with destination.open("w", newline="") as target:
        target.write(
            "<DATE>\t<TIME>\t<OPEN>\t<HIGH>\t<LOW>\t<CLOSE>\t<TICKVOL>\t<VOL>\t<SPREAD>\n"
        )
        for stamp, bars in groups:
            target.write(
                "\t".join(
                    [
                        stamp.strftime("%Y.%m.%d"),
                        stamp.strftime("%H:%M:%S"),
                        bars[0]["<OPEN>"],
                        str(max(float(bar["<HIGH>"]) for bar in bars)),
                        str(min(float(bar["<LOW>"]) for bar in bars)),
                        bars[-1]["<CLOSE>"],
                        str(sum(int(float(bar["<TICKVOL>"])) for bar in bars)),
                        str(sum(int(float(bar["<VOL>"])) for bar in bars)),
                        bars[-1]["<SPREAD>"],
                    ]
                )
                + "\n"
            )


def count_trades_from_scout(path: Path) -> int:
    data = json.loads(path.read_text())
    return int(data["result"]["metrics"]["trade_count"])


def count_trades_from_parity(path: Path) -> tuple[int, int, int, bool]:
    data = json.loads(path.read_text())
    ref = int(data["reference"]["metrics"]["trade_count"])
    ext = int(data["external"]["metrics"]["trade_count"])
    delta = int(data["report"]["trade_count_delta"])
    passed = bool(data["report"]["passed"])
    return ref, ext, delta, passed


def acceptable(ref: int, ext: int) -> bool:
    if ref == 0 and ext == 0:
        return True
    allowed = max(8, int((max(ref, 1) * 0.10) + 0.999))
    return abs(ext - ref) <= allowed


def find_sequence_with_trades(
    family: str, mode: str, work: Path, decision_data: Path, from_date: str, to_date: str
) -> int:
    """Pick first seed sequence with enough Scout trades for a meaningful compare."""
    for sequence in range(0, 24):
        strat = work / f"probe_{sequence}.ir.json"
        emit_strategy(family, mode, strat, sequence)
        scout_out = work / f"probe_{sequence}_scout.json"
        if scout_out.exists():
            scout_out.unlink()
        r = run(
            [
                str(QF),
                "scout",
                str(decision_data),
                "--source-timezone",
                TZ,
                "--strategy",
                str(strat),
                "--broker",
                str(BROKER),
                "--commission-per-lot-round-turn",
                str(COMMISSION),
                "--fallback-spread-points",
                "1",
                "--initial-balance",
                "100000",
                "--allow-failed-data",
                "--out",
                str(scout_out),
            ],
            capture_output=True,
        )
        if r.returncode:
            print(r.stdout)
            print(r.stderr)
            continue
        trades = count_trades_from_scout(scout_out)
        print(f"  probe seq={sequence} scout_trades={trades}", flush=True)
        if trades >= 30:
            return sequence
    raise RuntimeError(f"no seed with >=30 trades for {family}/{mode}")


def run_case(
    family: str,
    mode: str,
    from_date: str,
    to_date: str,
    sequence: int | None,
    force: bool = False,
) -> dict:
    tag = f"{family}_{mode}"
    work = RUN_ROOT / tag
    summary_file = work / "summary.json"
    if summary_file.exists() and not force:
        prior = json.loads(summary_file.read_text())
        if prior.get("acceptable"):
            print(f"==> SKIP {family}/{mode} (already PASS)", flush=True)
            return prior
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)

    h1 = work / f"{SYMBOL}_H1.tsv"
    m1 = work / f"{SYMBOL}_M1.tsv"
    slice_tsv(H1, h1, from_date, to_date)
    slice_tsv(M1, m1, from_date, to_date)
    if TIMEFRAME == "H1":
        decision_data = h1
    elif TIMEFRAME == "M15":
        decision_data = work / f"{SYMBOL}_M15.tsv"
        aggregate_bid_decision_bars(m1, decision_data, TIMEFRAME)
    else:
        decision_data = m1

    if sequence is None:
        sequence = find_sequence_with_trades(family, mode, work, decision_data, from_date, to_date)

    strategy = work / "strategy.ir.json"
    emit_strategy(family, mode, strategy, sequence)

    expert = f"{SYMBOL}_Fam_{family[:10]}_{mode[:3]}_{sequence}"
    # MT5 expert names must be alphanumeric/underscore only.
    expert = "".join(ch if ch.isalnum() or ch == "_" else "_" for ch in expert)

    scout = work / "scout.json"
    judge = work / "judge.json"
    export_dir = work / "export"
    export_dir.mkdir()

    # Scout remains useful as a quick selected-timeframe diagnostic. The Judge
    # is intentionally deferred until MT5 has captured the canonical quotes.
    r = run(
        [
            str(QF),
            "scout",
            str(decision_data),
            "--source-timezone",
            TZ,
            "--strategy",
            str(strategy),
            "--broker",
            str(BROKER),
            "--commission-per-lot-round-turn",
            str(COMMISSION),
            "--fallback-spread-points",
            "1",
            "--initial-balance",
            "100000",
            "--allow-failed-data",
            "--out",
            str(scout),
        ],
        capture_output=True,
    )
    if r.returncode:
        print(r.stdout)
        print(r.stderr)
        raise RuntimeError(f"scout failed for {tag}")

    scout_trades = count_trades_from_scout(scout)
    print(f"  scout={scout_trades}; canonical Judge waits for MT5 quote capture", flush=True)

    # Export + compile into MT5 Experts tree so tester finds the EA.
    experts_export = EXPERTS  # compile in-place under Wine C:
    # Write into a temp under Experts then compile there.
    case_export = EXPERTS  # files named uniquely
    # Clean prior artifacts with same expert name
    for ext in (".mq5", ".ex5", ".set", ".tester.ini", ".evidence.json", ".compile.json", ".compile.log"):
        p = EXPERTS / f"{expert}{ext}"
        if p.exists():
            p.unlink()

    r = run(
        [
            str(QF),
            "export",
            "--strategy",
            str(strategy),
            "--broker",
            str(BROKER),
            "--out",
            str(export_dir),
            "--expert-name",
            expert,
            "--expert-directory",
            "QuantForge",
            "--timeframe",
            TIMEFRAME,
            "--from-date",
            from_date,
            "--to-date",
            to_date,
            "--commission-per-lot-round-turn",
            str(COMMISSION),
            "--deposit",
            "100000",
            "--tester-model",
            # Canonical parity uses MT5's one-minute OHLC model. Model 4 is
            # reserved for an explicit real-tick audit and must never be
            # substituted here; model 2 is open-prices-only and cannot claim
            # M1 chronology parity.
            "1",
            "--compile",
        ],
        capture_output=True,
    )
    if r.returncode:
        print(r.stdout)
        print(r.stderr)
        raise RuntimeError(f"export/compile failed for {tag}")

    # Install compiled EA + support into MT5 Experts/Indicators tree.
    for src in export_dir.iterdir():
        if not src.is_file():
            continue
        name = src.name
        if name.endswith(
            (".mq5", ".ex5", ".set", ".tester.ini", ".evidence.json")
        ):
            shutil.copy2(src, EXPERTS / name)
    support = export_dir / "Indicators"
    if support.is_dir():
        for src in support.rglob("*"):
            if src.is_file():
                rel = src.relative_to(support)
                dst = INDICATORS_DST / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)

    ex5 = EXPERTS / f"{expert}.ex5"
    if not ex5.is_file():
        raise RuntimeError(f"compiled EA missing after install for {tag}: {ex5}")

    evidence = export_dir / f"{expert}.evidence.json"
    evidence_data = json.loads(evidence.read_text())
    deals_rel = evidence_data["parity_deals_file"]
    equity_rel = evidence_data["parity_equity_file"]
    meta_rel = evidence_data["parity_metadata_file"]
    quote_rel = evidence_data["parity_quote_file"]

    def win_to_path(rel: str) -> Path:
        return COMMON.joinpath(*rel.replace("\\", "/").split("/"))

    deals = win_to_path(deals_rel)
    equity = win_to_path(equity_rel)
    metadata = win_to_path(meta_rel)
    quotes = win_to_path(quote_rel)
    for p in (deals, equity, metadata, quotes):
        if p.exists():
            p.unlink()

    kill_mt5()
    mt5_out = work / "mt5-run.json"
    if mt5_out.exists():
        mt5_out.unlink()
    r = run(
        [
            str(QF),
            "mt5-test",
            "--tester-ini",
            str(EXPERTS / f"{expert}.tester.ini"),
            "--evidence",
            str(evidence),
            "--out",
            str(mt5_out),
            "--timeout-seconds",
            "600",
        ],
        capture_output=True,
    )
    if r.returncode:
        print(r.stdout)
        print(r.stderr)
        raise RuntimeError(f"mt5-test failed for {tag}")

    # Copy artifacts into work dir
    for src, name in (
        (deals, "deals.csv"),
        (equity, "equity.csv"),
        (metadata, "metadata.csv"),
        (quotes, "quotes.csv"),
    ):
        shutil.copy2(src, work / name)

    point = float(json.loads(BROKER.read_text())["point"])
    write_bid_m1_from_quotes(work / "quotes.csv", m1, point)
    # Every decision timeframe must be built from the exact same captured bid
    # minutes. Reusing the older midpoint H1 export changes indicators and can
    # introduce a weekend bar-count discrepancy in time-based exits.
    if TIMEFRAME == "M1":
        decision_data = m1
    else:
        aggregate_bid_decision_bars(m1, decision_data, TIMEFRAME)

    # Phase two: QF replays the exact bid/ask minutes emitted by the same MT5
    # test above. The legacy fallback spread is deliberately absent.
    if judge.exists():
        judge.unlink()
    r = run(
        [
            str(QF),
            "judge",
            str(decision_data),
            "--source-timezone",
            TZ,
            "--m1",
            str(m1),
            "--m1-source-timezone",
            TZ,
            "--quote-path",
            str(work / "quotes.csv"),
            "--quote-source-timezone",
            TZ,
            "--strategy",
            str(strategy),
            "--broker",
            str(BROKER),
            "--commission-per-lot-round-turn",
            str(COMMISSION),
            "--initial-balance",
            "100000",
            "--allow-execution-gaps",
            "--allow-failed-data",
            "--out",
            str(judge),
        ],
        capture_output=True,
    )
    if r.returncode:
        print(r.stdout)
        print(r.stderr)
        raise RuntimeError(f"canonical quote Judge failed for {tag}")
    judge_trades = count_trades_from_scout(judge)
    print(f"  canonical judge={judge_trades}", flush=True)

    parity_out = work / "parity_judge_vs_mt5.json"
    if parity_out.exists():
        parity_out.unlink()
    r = run(
        [
            str(QF),
            "parity",
            "--scout-result",
            str(judge),
            "--evidence",
            str(evidence),
            "--mq5",
            str(export_dir / f"{expert}.mq5"),
            "--mt5-deals",
            str(work / "deals.csv"),
            "--mt5-equity",
            str(work / "equity.csv"),
            "--mt5-metadata",
            str(work / "metadata.csv"),
            "--quote-path",
            str(work / "quotes.csv"),
            "--broker-timezone",
            TZ,
            "--strict-one-to-one",
            "true",
            "--out",
            str(parity_out),
        ],
        capture_output=True,
    )
    print(r.stdout)
    if r.stderr:
        print(r.stderr)
    if r.returncode:
        raise RuntimeError(f"parity command failed for {tag}")

    ref, ext, delta, passed_flag = count_trades_from_parity(parity_out)
    ok = passed_flag
    result = {
        "family": family,
        "mode": mode,
        "sequence": sequence,
        "scout_trades": scout_trades,
        "judge_trades": ref,
        "mt5_trades": ext,
        "delta": delta,
        "trade_count_passed": passed_flag,
        "acceptable": ok,
        "work": str(work),
    }
    (work / "summary.json").write_text(json.dumps(result, indent=2))
    status = "PASS" if ok else "FAIL"
    print(
        f"==> {status} {family}/{mode} judge={ref} mt5={ext} delta={delta}",
        flush=True,
    )
    return result


def main() -> int:
    global PACK, SYMBOL, H1, M1, BROKER, COMMISSION, RUN_ROOT, TIMEFRAME, TIME_STOP_OVERRIDE
    parser = argparse.ArgumentParser()
    parser.add_argument("--from-date", default="2024.01.01")
    parser.add_argument("--to-date", default="2024.07.01")
    parser.add_argument("--family", action="append", default=[])
    parser.add_argument("--mode", action="append", default=[])
    parser.add_argument("--sequence", type=int, default=None)
    parser.add_argument(
        "--symbol",
        default="AUDUSD",
        help="symbol in the IC Markets pack (for example AUDUSD, US500 or BTCUSD)",
    )
    parser.add_argument(
        "--pack-dir",
        type=Path,
        default=PACK,
        help="directory containing ICMarketsSC-Demo_<SYMBOL>_{H1,M1}_2020_present.tsv and broker JSON",
    )
    parser.add_argument(
        "--commission",
        type=float,
        default=None,
        help="round-turn commission per lot; defaults to captured IC Markets terms (0 for BTCUSD/US500, 7 for FX/metals)",
    )
    parser.add_argument("--continue", dest="cont", action="store_true")
    parser.add_argument("--force", action="store_true", help="rerun even if prior PASS")
    parser.add_argument(
        "--timeframe",
        choices=("H1", "M15", "M1"),
        default="H1",
        help="decision and MT5 tester timeframe; M1 compares the direct M1 QF replay with an M1 EA",
    )
    parser.add_argument(
        "--time-stop-bars",
        type=int,
        default=None,
        help="optional deterministic management override for a focused parity case",
    )
    args = parser.parse_args()

    SYMBOL = args.symbol.upper()
    PACK = args.pack_dir.expanduser().resolve()
    H1 = PACK / f"ICMarketsSC-Demo_{SYMBOL}_H1_2020_present.tsv"
    M1 = PACK / f"ICMarketsSC-Demo_{SYMBOL}_M1_2020_present.tsv"
    BROKER = PACK / f"{SYMBOL}.broker.json"
    COMMISSION = args.commission if args.commission is not None else (0.0 if SYMBOL in {"BTCUSD", "US500"} else 7.0)
    TIMEFRAME = args.timeframe
    TIME_STOP_OVERRIDE = args.time_stop_bars
    RUN_ROOT = ROOT / "runs" / "family-mt5-parity" / TIMEFRAME / SYMBOL
    for required in (H1, M1, BROKER):
        if not required.is_file():
            parser.error(f"missing asset input: {required}")

    families = args.family or FAMILIES
    modes = args.mode or MODES
    ensure_tools()

    results = []
    failures = []
    for family in families:
        for mode in modes:
            print(f"\n===== {family} / {mode} =====", flush=True)
            try:
                result = run_case(
                    family,
                    mode,
                    args.from_date,
                    args.to_date,
                    args.sequence,
                    force=args.force,
                )
                results.append(result)
                if not result["acceptable"]:
                    failures.append(result)
                    if not args.cont:
                        break
            except Exception as exc:  # noqa: BLE001
                print(f"ERROR {family}/{mode}: {exc}", flush=True)
                failures.append({"family": family, "mode": mode, "error": str(exc)})
                if not args.cont:
                    break
        else:
            continue
        break

    summary_path = RUN_ROOT / "SUMMARY.json"
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    summary_path.write_text(json.dumps({"results": results, "failures": failures}, indent=2))
    print("\nSUMMARY", summary_path)
    for row in results:
        mark = "PASS" if row.get("acceptable") else "FAIL"
        print(
            f"  {mark} {row['family']}/{row['mode']}: judge={row['judge_trades']} mt5={row['mt5_trades']} delta={row['delta']}"
        )
    for row in failures:
        if "error" in row:
            print(f"  ERROR {row['family']}/{row['mode']}: {row['error']}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
