#!/usr/bin/env python3
"""Drive StrategyQuant X Retester via the local BrowserToken HTTP API.

Requires the SQX GUI to already be running (sqcli cannot attach while it is).
The Electron UI embeds a BrowserToken in user/settings/settings.xml; requests to
http://127.0.0.1:8080 must send that token as a BrowserToken header.

Typical pilot flow:
  1. clear source + target databanks
  2. loadFilesToDatabank from a staging folder
  3. start Retester
  4. export Results to CSV
"""

from __future__ import annotations

import argparse
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

DEFAULT_BASE = "http://127.0.0.1:8080"
SETTINGS = Path(
    "/Applications/StrategyQuantXB144_arm.app/Contents/Resources/user/settings/settings.xml"
)


def read_token(settings: Path = SETTINGS) -> str:
    text = settings.read_text(encoding="utf-8", errors="ignore")
    m = re.search(r"<BrowserToken>(\d+)</BrowserToken>", text)
    if not m:
        raise SystemExit(f"no BrowserToken in {settings}")
    return m.group(1)


class SqxApi:
    def __init__(self, base: str = DEFAULT_BASE, token: str | None = None):
        self.base = base.rstrip("/")
        self.token = token or read_token()

    def call(self, path: str, q: dict | None = None, data: dict | None = None):
        headers = {"BrowserToken": self.token}
        url = self.base + path
        if q:
            url += "?" + urllib.parse.urlencode(q, doseq=True)
        body = None
        method = "GET"
        if data is not None:
            body = urllib.parse.urlencode(data, doseq=True).encode()
            headers["Content-Type"] = "application/x-www-form-urlencoded"
            method = "POST"
        req = urllib.request.Request(url, data=body, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=300) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                return json.loads(raw)
            except Exception:
                raise SystemExit(f"HTTP {e.code} {path}: {raw[:300]!r}") from e
        try:
            return json.loads(raw)
        except Exception:
            return {"raw": raw.decode("utf-8", "ignore")}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("retest", help="load folder -> start Retester -> export CSV")
    p.add_argument("--project", default="Retester")
    p.add_argument("--source", default="k", help="input databank name")
    p.add_argument("--target", default="Results", help="output databank name")
    p.add_argument("--folder", type=Path, required=True, help="folder of .sqx files")
    p.add_argument("--export", type=Path, required=True, help="CSV output path")
    p.add_argument("--wait", type=float, default=120.0)

    sub.add_parser("token", help="print the current BrowserToken")
    s = sub.add_parser("status", help="list databank strategy counts")
    s.add_argument("--project", default="Retester")

    args = ap.parse_args()
    api = SqxApi()

    if args.cmd == "token":
        print(api.token)
        return 0

    if args.cmd == "status":
        for name in ("k", "Results", "pilot"):
            body = api.call(
                "/project/databankCount",
                q={"projectName": args.project, "databankName": name},
            )
            print(name, body)
        return 0

    folder = args.folder.resolve()
    if not folder.is_dir():
        raise SystemExit(f"folder not found: {folder}")

    print("clear", args.source, api.call(
        "/project/removeAllReports",
        q={"projectName": args.project, "databankName": args.source},
    ))
    print("clear", args.target, api.call(
        "/project/removeAllReports",
        q={"projectName": args.project, "databankName": args.target},
    ))
    print("load", api.call(
        "/project/loadFilesToDatabank",
        q={
            "projectName": args.project,
            "databankName": args.source,
            "folder": str(folder),
            "clear": "true",
        },
    ))
    time.sleep(1.5)
    print("source", api.call(
        "/project/databankCount",
        q={"projectName": args.project, "databankName": args.source},
    ))
    print("start", api.call("/project/start", q={"projectName": args.project}))

    n_src = api.call(
        "/project/databankCount",
        q={"projectName": args.project, "databankName": args.source},
    ).get("count", 0)
    deadline = time.time() + args.wait
    while time.time() < deadline:
        n = api.call(
            "/project/databankCount",
            q={"projectName": args.project, "databankName": args.target},
        ).get("count", 0)
        print(f"target count={n}/{n_src}")
        if n_src and n >= n_src:
            break
        time.sleep(0.5)

    args.export.parent.mkdir(parents=True, exist_ok=True)
    if args.export.exists():
        args.export.unlink()
    print("export", api.call(
        "/resultsDatabankActions/exportDatabankToCSV",
        q={
            "projectName": args.project,
            "databankName": args.target,
            "file": str(args.export.resolve()),
            "path": str(args.export.resolve()),
        },
    ))
    time.sleep(1.5)
    print("wrote", args.export, "bytes", args.export.stat().st_size if args.export.exists() else 0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
