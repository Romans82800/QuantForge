#!/usr/bin/env python3
"""Install qf-import-market CSV outputs into ICMarkets_EST7_2020_present as TSVs."""

from __future__ import annotations

import csv
import json
import shutil
import sys
from pathlib import Path

PACK_PREFIX = "ICMarketsSC-Demo_"
PACK_SUFFIX = "_2020_present"

# Guess broker template for newly added symbols.
TEMPLATE_MAP = {
    "EURUSD": "GBPUSD",
    "EURCAD": "GBPUSD",
    "EURAUD": "GBPUSD",
    "GBPNZD": "EURNZD",
    "CHFJPY": "USDJPY",
    "BTCUSD": "XAUUSD",
    "US100": "US500",
    "XTIUSD": "XAUUSD",
}


def csv_to_tsv(src: Path, dst: Path) -> None:
    with src.open(newline="", encoding="utf-8") as handle:
        rows = list(csv.reader(handle))
    if not rows:
        raise SystemExit(f"empty file: {src}")
    with dst.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerows(rows)


def resolve_symbol_stem(staging: Path, symbol: str) -> str | None:
    exact = staging / f"{symbol}_M1.csv"
    if exact.exists():
        return symbol
    matches = sorted(staging.glob(f"{symbol}*_M1.csv"))
    if not matches:
        return None
    return matches[0].name[: -len("_M1.csv")]


def install_symbol(staging: Path, pack: Path, symbol: str, stem: str) -> None:
    m1_csv = staging / f"{stem}_M1.csv"
    h1_csv = staging / f"{stem}_H1.csv"
    m1_meta = staging / f"{stem}_M1.metadata.csv"
    h1_meta = staging / f"{stem}_H1.metadata.csv"
    for path in (m1_csv, h1_csv, m1_meta, h1_meta):
        if not path.exists():
            raise SystemExit(f"missing {path}")

    m1_tsv = pack / f"{PACK_PREFIX}{symbol}_M1{PACK_SUFFIX}.tsv"
    h1_tsv = pack / f"{PACK_PREFIX}{symbol}_H1{PACK_SUFFIX}.tsv"
    csv_to_tsv(m1_csv, m1_tsv)
    csv_to_tsv(h1_csv, h1_tsv)
    shutil.copy2(m1_meta, m1_tsv.with_suffix(".metadata.csv"))
    shutil.copy2(h1_meta, h1_tsv.with_suffix(".metadata.csv"))

    # Stamp symbol into metadata if import used MARKET / wrong stem.
    for meta_path in (
        m1_tsv.with_suffix(".metadata.csv"),
        h1_tsv.with_suffix(".metadata.csv"),
    ):
        text = meta_path.read_text(encoding="utf-8")
        lines = []
        for line in text.splitlines():
            if line.startswith("symbol,"):
                lines.append(f"symbol,{symbol}")
            else:
                lines.append(line)
        meta_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"installed {symbol}: {h1_tsv.name} + {m1_tsv.name}")


def ensure_broker(pack: Path, symbol: str) -> None:
    dest = pack / f"{symbol}.broker.json"
    if dest.exists():
        return
    template_symbol = TEMPLATE_MAP.get(symbol, "GBPUSD")
    if not (pack / f"{template_symbol}.broker.json").exists():
        template_symbol = "GBPUSD"
    src = pack / f"{template_symbol}.broker.json"
    data = json.loads(src.read_text(encoding="utf-8"))
    data["symbol"] = symbol
    data["profile_name"] = (
        f"Raw Trading Ltd · ICMarketsSC-Demo · {symbol} · MT5 build 5834 (imported)"
    )
    fx = {
        "EURUSD": ("EUR", "USD", "USD"),
        "EURCAD": ("EUR", "CAD", "CAD"),
        "EURAUD": ("EUR", "AUD", "AUD"),
        "GBPNZD": ("GBP", "NZD", "NZD"),
        "CHFJPY": ("CHF", "JPY", "JPY"),
        "BTCUSD": ("BTC", "USD", "USD"),
        "US100": ("USD", "USD", "USD"),
        "XTIUSD": ("XTI", "USD", "USD"),
    }
    if symbol in fx:
        base, profit, margin = fx[symbol]
        data["base_currency"] = base
        data["profit_currency"] = profit
        data["margin_currency"] = margin
    if symbol.endswith("JPY") and symbol not in {"BTCUSD", "US100", "XTIUSD", "US500", "XAUUSD"}:
        data["digits"] = 3
        data["point"] = 0.001
        data["tick_size"] = 0.001
    elif symbol not in {"BTCUSD", "US100", "XTIUSD", "US500", "XAUUSD"}:
        data["digits"] = 5
        data["point"] = 0.00001
        data["tick_size"] = 0.00001
    dest.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    print(f"wrote broker stub {dest.name} from {template_symbol}")


def discover_symbols(staging: Path) -> dict[str, str]:
    """Map pack symbol -> staging stem."""
    mapping: dict[str, str] = {}
    for m1 in sorted(staging.glob("*_M1.csv")):
        stem = m1.name[: -len("_M1.csv")]
        symbol = stem.split("_")[0]
        meta = staging / f"{stem}_M1.metadata.csv"
        if meta.exists():
            for line in meta.read_text(encoding="utf-8").splitlines():
                if line.startswith("source_file,") and "_TickData" in line:
                    name = Path(line.split(",", 1)[1]).name
                    if name.endswith("_TickData.csv"):
                        symbol = name[: -len("_TickData.csv")]
                    break
                if line.startswith("symbol,") and line.split(",", 1)[1] not in {"", "MARKET"}:
                    symbol = line.split(",", 1)[1]
        if symbol == "MARKET":
            continue
        mapping[symbol] = stem
    return mapping


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(
            "usage: install_icmarkets_pack.py <staging_import_dir> <ICMarkets_EST7_2020_present>"
        )
    staging = Path(sys.argv[1])
    pack = Path(sys.argv[2])
    pack.mkdir(parents=True, exist_ok=True)
    mapping = discover_symbols(staging)
    if not mapping:
        raise SystemExit(f"no M1 csv files in {staging}")
    for symbol, stem in sorted(mapping.items()):
        install_symbol(staging, pack, symbol, stem)
        ensure_broker(pack, symbol)
    print(f"done · {len(mapping)} symbols")


if __name__ == "__main__":
    main()
