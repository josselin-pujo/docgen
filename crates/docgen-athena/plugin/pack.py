#!/usr/bin/env python3
"""Packs an Athena plugin bundle from a pack-request.json — a stdlib-only
fallback for `athena --pack-bundle` (the dist desktop binary packs only
when the app is up; this script has no such dependency).

The format mirrors what the packer emits (bundle inspected 2026-10-16):
a SQLite database of content-addressed blobs (`sha256:…`), a package row
whose `entry` is the broker's blob digest, one `filesystem` row per
packaged file (skills etc.), one `components` row per tool component and
optional deps/caps tables. Bundles written here are unsigned, like the
ones the desktop packer produces from this repo.

Usage: pack.py REQUEST.json --out BUNDLE
"""
import hashlib
import json
import sqlite3
import sys
from pathlib import Path

SCHEMA = """
CREATE TABLE package (id TEXT NOT NULL, version TEXT NOT NULL, api INTEGER NOT NULL, description TEXT NOT NULL, entry TEXT NOT NULL, license TEXT, frame TEXT);
CREATE TABLE config (key TEXT NOT NULL, value BLOB NOT NULL);
CREATE TABLE blobs (digest TEXT PRIMARY KEY, bytes BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE filesystem (path TEXT NOT NULL, digest TEXT NOT NULL, role TEXT NOT NULL);
CREATE TABLE caps_advertised (cap TEXT NOT NULL, scope TEXT);
CREATE TABLE tools (name TEXT NOT NULL, description TEXT NOT NULL);
CREATE TABLE components (tool_name TEXT NOT NULL, blob_digest TEXT NOT NULL, invoke_contract TEXT NOT NULL, wall_clock_ms INTEGER, memory_bytes INTEGER, fuel INTEGER);
CREATE TABLE deps (dep_id TEXT NOT NULL, semver_range TEXT NOT NULL, mandatory INTEGER NOT NULL);
CREATE TABLE signatures (scheme TEXT NOT NULL, signer TEXT NOT NULL, value BLOB NOT NULL);
"""


def digest(b: bytes) -> str:
    return "sha256:" + hashlib.sha256(b).hexdigest()


def pack(request_path: str, out_path: str) -> None:
    req = json.loads(Path(request_path).read_text(encoding="utf-8"))
    base = Path(request_path).parent
    pkg = req["package"]

    out = Path(out_path)
    out.unlink(missing_ok=True)  # fresh database, not an incremental update
    con = sqlite3.connect(out)
    con.executescript(SCHEMA)
    blobs: set[str] = set()

    def put(data: bytes) -> str:
        d = digest(data)
        if d not in blobs:
            con.execute("INSERT INTO blobs VALUES (?, ?)", (d, data))
            blobs.add(d)
        return d

    entry = put((base / pkg["entry"]).read_bytes())
    con.execute(
        "INSERT INTO package VALUES (?, ?, ?, ?, ?, ?, ?)",
        (pkg["id"], pkg["version"], pkg["api"], pkg["description"], entry,
         pkg.get("license"), None),
    )
    for path, meta in req.get("files", {}).items():
        con.execute(
            "INSERT INTO filesystem VALUES (?, ?, ?)",
            (path, put((base / path).read_bytes()), meta["role"]),
        )
    for cap in req.get("caps_advertised", []):
        con.execute("INSERT INTO caps_advertised VALUES (?, ?)", (cap["cap"], cap.get("scope")))
    for tool in req.get("tools", []):
        con.execute("INSERT INTO tools VALUES (?, ?)", (tool["name"], tool["description"]))
    for comp in req.get("components", []):
        payload = comp["payload"]
        d = payload if payload.startswith("sha256:") else put((base / payload).read_bytes())
        con.execute(
            "INSERT INTO components VALUES (?, ?, ?, ?, ?, ?)",
            (comp["tool_name"], d, comp["invoke_contract"], comp.get("wall_clock_ms"),
             comp.get("memory_bytes"), comp.get("fuel")),
        )
    for dep in req.get("deps", []):
        con.execute(
            "INSERT INTO deps VALUES (?, ?, ?)",
            (dep["dep_id"], dep["semver_range"], 1 if dep.get("mandatory") else 0),
        )
    con.commit()
    con.close()
    print(f"packed {out_path}")


def main() -> None:
    args = sys.argv[1:]
    if "--out" not in args or len(args) != 3:
        sys.exit("usage: pack.py REQUEST.json --out BUNDLE")
    request = args[0]
    out = args[args.index("--out") + 1]
    pack(request, out)


if __name__ == "__main__":
    main()
