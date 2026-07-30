#!/usr/bin/env python3
"""Compare MT5 Strategy Tester HTML reports against the SQX backtest in each .sqx.

The .sqx fingerprint records what SQX produced when the strategy was built; the MT5
report records what the same logic did in the execution environment you actually
trade. Divergence is interpreted in two independent layers:

  signal parity - do the same trades fire? (trade count)
  economic parity - do they make the same money? (net profit, profit factor)

Signal parity failing means the port is broken and SQX robustness results do not
describe the EA you would run. Economic divergence with intact signal parity is a
cost-model difference, which is calibration rather than a broken strategy.

MT5 writes these reports as UTF-16.
"""

from __future__ import annotations

import argparse
import csv
import html
import json
import re
import unicodedata
import zipfile
from pathlib import Path

FINGERPRINT = re.compile(r"<Fingerprint\b([^>]*\bstrategyName=\"[^>]*?)/?>")
ATTR = re.compile(r'([\w]+)="([^"]*)"')


def read_html_text(path: Path) -> str:
    raw = path.read_bytes()
    for encoding in ("utf-16", "utf-16-le", "utf-8", "cp1252"):
        try:
            text = raw.decode(encoding)
        except (UnicodeDecodeError, LookupError):
            continue
        if "Strategy Tester" in text or "<html" in text.lower():
            return text
    return raw.decode("utf-8", errors="ignore")


def flatten(text: str) -> str:
    text = re.sub(r"<\s*/?\s*(td|th|tr|table|br)[^>]*>", "\x01", text, flags=re.I)
    text = re.sub(r"<[^>]+>", " ", text)
    text = html.unescape(text)
    text = unicodedata.normalize("NFKC", text)
    return re.sub(r"[ \t]+", " ", text)


def cells(flat: str) -> list[str]:
    return [c.strip() for c in flat.split("\x01") if c.strip()]


def field(parts: list[str], label: str) -> str | None:
    """MT5 reports lay out label/value in adjacent cells."""
    target = label.rstrip(":").lower()
    for i, c in enumerate(parts):
        if c.rstrip(":").lower() == target:
            for nxt in parts[i + 1 : i + 4]:
                if nxt and nxt.rstrip(":").lower() != target:
                    return nxt
    return None


def number(raw: str | None) -> float | None:
    if raw is None:
        return None
    m = re.search(r"-?[\d\u00a0,. ]+", raw)
    if not m:
        return None
    s = m.group().replace("\u00a0", "").replace(" ", "")
    if re.search(r",\d{1,2}$", s) and "." not in s:
        s = s.replace(",", ".")
    else:
        s = s.replace(",", "")
    try:
        return float(s)
    except ValueError:
        return None


def parse_mt5(path: Path) -> dict:
    parts = cells(flatten(read_html_text(path)))
    profit = number(field(parts, "Total Net Profit"))
    trades = number(field(parts, "Total Trades"))
    # "Total Deals" appears on some builds; deals are ~2x positions
    if trades is None:
        deals = number(field(parts, "Total Deals"))
        trades = deals / 2 if deals else None
    return {
        "file": path.name,
        "expert": field(parts, "Expert"),
        "symbol": field(parts, "Symbol"),
        "period": field(parts, "Period"),
        "broker": parts[1] if len(parts) > 1 else None,
        "profit": profit,
        "trades": int(trades) if trades is not None else None,
        "profit_factor": number(field(parts, "Profit Factor")),
        "drawdown": number(field(parts, "Balance Drawdown Maximal")),
    }


def parse_sqx(path: Path) -> dict | None:
    try:
        with zipfile.ZipFile(path) as zf:
            xml = zf.read("settings.xml").decode("utf-8", errors="ignore")
    except Exception:
        return None
    m = FINGERPRINT.search(xml)
    if not m:
        return None
    fp = dict(ATTR.findall(m.group(1)))
    return {
        "strategy": fp.get("strategyName"),
        "trades": int(float(fp["trades"])) if fp.get("trades") else None,
        "profit": float(fp["profit"]) if fp.get("profit") else None,
        "drawdown": float(fp["drawdown"]) if fp.get("drawdown") else None,
    }


def norm_name(text: str | None) -> str:
    """Key on symbol + version, since version numbers repeat across symbols."""
    if not text:
        return ""
    text = re.sub(r"\.(mq5|ex5|html?|sqx)$", "", text, flags=re.I)
    m = re.search(r"([A-Za-z][A-Za-z0-9]*)Strategy[_ ]+([\d.]+)", text)
    if m:
        return f"{m.group(1).lower()}|{m.group(2).rstrip('.')}"
    return re.sub(r"[^A-Za-z0-9.]", "", text).lower()


def pct(a: float | None, b: float | None) -> float | None:
    if a is None or b is None or abs(b) < 1e-9:
        return None
    return abs(a - b) / abs(b) * 100.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("html_dir", type=Path, help="folder of MT5 .html reports")
    ap.add_argument("sqx_dir", type=Path, help="folder tree containing .sqx files")
    ap.add_argument("--csv", type=Path)
    ap.add_argument(
        "--signal-tolerance",
        type=float,
        default=2.0,
        help="max %% trade-count deviation still counted as signal parity",
    )
    args = ap.parse_args()

    sqx_index: dict[str, dict] = {}
    collisions: set[str] = set()
    for p in args.sqx_dir.rglob("*"):
        if p.suffix.lower() != ".sqx":
            continue
        info = parse_sqx(p)
        if not (info and info["strategy"]):
            continue
        # The fingerprint's strategyName omits the symbol ("Strategy 2.2.126");
        # only the filename carries it ("US500Strategy 2.2.126.sqx").
        key = norm_name(p.stem)
        info["strategy"] = p.stem
        prev = sqx_index.get(key)
        if prev and (prev["trades"], prev["profit"]) != (info["trades"], info["profit"]):
            collisions.add(key)
        sqx_index.setdefault(key, info)
    if collisions:
        print(f"warning: {len(collisions)} ambiguous keys with differing results, excluded")
        for key in collisions:
            sqx_index.pop(key, None)

    reports = sorted(p for p in args.html_dir.rglob("*") if p.suffix.lower() in {".html", ".htm"})
    rows = []
    for rp in reports:
        mt5 = parse_mt5(rp)
        key = norm_name(mt5["expert"]) or norm_name(rp.stem)
        sqx = sqx_index.get(key)
        rows.append({"mt5": mt5, "sqx": sqx, "key": key})

    matched = [r for r in rows if r["sqx"] and r["mt5"]["trades"] is not None]
    print(f"MT5 reports: {len(reports)}   matched to .sqx: {len(matched)}\n")

    print(f"{'strategy':<22} {'sqx_tr':>7} {'mt5_tr':>7} {'dTr%':>7} "
          f"{'sqx_profit':>12} {'mt5_profit':>12} {'dPf%':>8}  signal")
    signal_ok = 0
    out = []
    for r in sorted(matched, key=lambda r: r["key"]):
        s, m = r["sqx"], r["mt5"]
        dtr = pct(m["trades"], s["trades"])
        dpf = pct(m["profit"], s["profit"])
        ok = dtr is not None and dtr <= args.signal_tolerance
        signal_ok += ok
        print(f"{s['strategy'] or r['key']:<22} {s['trades'] or 0:>7} {m['trades']:>7} "
              f"{(f'{dtr:.1f}%' if dtr is not None else '-'):>7} "
              f"{s['profit'] or 0:>12,.0f} {m['profit'] or 0:>12,.0f} "
              f"{(f'{dpf:.1f}%' if dpf is not None else '-'):>8}  {'OK' if ok else 'BROKEN'}")
        out.append({
            "strategy": s["strategy"], "report": m["file"], "symbol": m["symbol"],
            "period": m["period"], "sqx_trades": s["trades"], "mt5_trades": m["trades"],
            "trade_delta_pct": None if dtr is None else round(dtr, 2),
            "sqx_profit": s["profit"], "mt5_profit": m["profit"],
            "profit_delta_pct": None if dpf is None else round(dpf, 2),
            "mt5_profit_factor": m["profit_factor"],
            "signal_parity": "OK" if ok else "BROKEN",
        })

    if matched:
        deltas = sorted(r["trade_delta_pct"] for r in out if r["trade_delta_pct"] is not None)
        pdeltas = sorted(r["profit_delta_pct"] for r in out if r["profit_delta_pct"] is not None)
        mid = lambda v: v[len(v) // 2] if v else float("nan")
        print(f"\nsignal parity within {args.signal_tolerance}%: {signal_ok}/{len(matched)}")
        print(f"median trade-count deviation : {mid(deltas):.1f}%")
        print(f"median net-profit deviation  : {mid(pdeltas):.1f}%")

        def bucket(vals: list[float], label: str, edges: list[float]) -> None:
            print(f"\n{label}")
            lo = 0.0
            for hi in edges:
                n = sum(1 for v in vals if lo <= v < hi)
                print(f"  {lo:>5.0f}-{hi:<5.0f}% {n:>4}  {'#' * round(40 * n / max(len(vals), 1))}")
                lo = hi
            n = sum(1 for v in vals if v >= lo)
            print(f"   >={lo:<8.0f}% {n:>4}  {'#' * round(40 * n / max(len(vals), 1))}")

        bucket(deltas, "trade-count deviation distribution", [1, 2, 5, 10, 25])
        bucket(pdeltas, "net-profit deviation distribution", [5, 10, 20, 35, 50])

    if args.csv and out:
        args.csv.parent.mkdir(parents=True, exist_ok=True)
        with args.csv.open("w", newline="", encoding="utf-8") as fh:
            w = csv.DictWriter(fh, fieldnames=list(out[0].keys()))
            w.writeheader()
            w.writerows(out)
        print(f"\nwrote {args.csv}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
