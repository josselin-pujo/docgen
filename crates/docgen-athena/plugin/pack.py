#!/usr/bin/env python3
"""Packs an Athena plugin bundle from a pack-request.json — a stdlib-only
fallback for `athena --pack-bundle` (the dist desktop binary packs only
when the app is up; this script has no such dependency).

The format mirrors what the packer emits: a SQLite database sealed at
`user_version = 1` (the version gate — the host refuses any other value,
forward included; packing without it produces a bundle that is refused
at install), content-addressed blobs (`sha256:…`), a package row whose
`entry` is the broker's blob digest, one `filesystem` row per packaged
file (skills etc.; `src` relocates the payload on disk), one `components`
row per tool component, and optional deps/caps tables. Bundles written
here are unsigned, like the ones the desktop packer produces from this
repo.

Not implemented — refuses loudly rather than packing a lie: `config`
(CBOR encoding). Unknown request keys are refused too.

Usage: pack.py REQUEST.json --out BUNDLE
"""
import hashlib
import json
import re
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

BASE_IDS = {"frontend", "system", "prompt", "llm", "tools", "coordinator"}
RESERVED_LOOPS = {"reference", "dream"}
TOP_KEYS = {"package", "files", "tools", "components", "deps", "caps_advertised", "config"}
PKG_KEYS = {"id", "version", "api", "description", "entry", "license", "frame"}
ID_RE = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
TOOL_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_-]*$")
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")


def refuse(why: str) -> None:
    sys.exit(f"pack.py: refusing to pack — {why}")


def validate(req: dict) -> None:
    """The birth refusals a conforming packer raises at pack time."""
    unknown = set(req) - TOP_KEYS
    if unknown:
        refuse(f"unknown request keys {sorted(unknown)}")
    if "config" in req:
        refuse("`config` (CBOR) is not implemented by this fallback — use the desktop packer")
    pkg = req["package"]
    unknown = set(pkg) - PKG_KEYS
    if unknown:
        refuse(f"unknown package keys {sorted(unknown)}")
    pid = pkg["id"]
    if len(pid) > 63 or not ID_RE.match(pid) or pid in BASE_IDS | RESERVED_LOOPS:
        refuse(f"package id {pid!r} (lowercase kebab, ≤63 chars, no base-harness or reserved loop ids)")
    if not VERSION_RE.match(pkg["version"]):
        refuse(f"version {pkg['version']!r} is not MAJOR.MINOR.PATCH")
    tool_names = set()
    for tool in req.get("tools", []):
        name = tool["name"]
        if not TOOL_RE.match(name):
            refuse(f"tool name {name!r} violates the identifier grammar")
        if name in tool_names:
            refuse(f"duplicate tool {name!r}")
        tool_names.add(name)
    component_tools = set()
    for comp in req.get("components", []):
        tool = comp["tool_name"]
        if tool not in tool_names:
            refuse(f"component cites undeclared tool {tool!r}")
        if tool in component_tools:
            refuse(f"two components for tool {tool!r} (one per tool)")
        component_tools.add(tool)
        memory = comp.get("memory_bytes")
        if not isinstance(memory, int) or memory <= 0 or memory % 65536:
            refuse(f"component {tool!r}: memory_bytes must be a positive multiple of 64 KiB")
    for dep in req.get("deps", []):
        if dep["dep_id"] == pid:
            refuse("self-dependency")
    seen: dict[str, str] = {}
    for path in req.get("files", {}):
        parts = path.split("/")
        if path.startswith("/") or any(p in ("", "..") for p in parts) or any(p.startswith(".") for p in parts):
            refuse(f"filesystem path {path!r} (relative, /-separated, no dot segments)")
        low = path.lower()
        if low in seen:
            refuse(f"case-colliding fs paths on NTFS: {seen[low]!r} and {path!r}")
        seen[low] = path


def digest(b: bytes) -> str:
    return "sha256:" + hashlib.sha256(b).hexdigest()


def pack(request_path: str, out_path: str) -> None:
    req = json.loads(Path(request_path).read_text(encoding="utf-8"))
    validate(req)
    base = Path(request_path).parent
    pkg = req["package"]

    out = Path(out_path)
    out.unlink(missing_ok=True)  # fresh database, not an incremental update
    con = sqlite3.connect(out)
    con.executescript(SCHEMA)
    # The format's version gate: exactly 1. The host refuses any other
    # value (forward included) — a bundle stamped 0 is "badly packed".
    con.execute("PRAGMA user_version = 1")
    blobs: set[str] = set()

    def put(data: bytes) -> str:
        d = digest(data)
        if d not in blobs:
            con.execute("INSERT INTO blobs VALUES (?, ?)", (d, data))
            blobs.add(d)
        return d

    entry = put((base / pkg["entry"]).read_bytes())
    frame = put((base / pkg["frame"]).read_bytes()) if pkg.get("frame") else None
    con.execute(
        "INSERT INTO package VALUES (?, ?, ?, ?, ?, ?, ?)",
        (pkg["id"], pkg["version"], pkg["api"], pkg["description"], entry,
         pkg.get("license"), frame),
    )
    for path, meta in req.get("files", {}).items():
        # `src` RELOCATES the payload on disk; the map path is the bundle's.
        con.execute(
            "INSERT INTO filesystem VALUES (?, ?, ?)",
            (path, put((base / meta.get("src", path)).read_bytes()), meta["role"]),
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
    # Read back through a second connection: the seal is a pack-time fact.
    check = sqlite3.connect(out_path)
    version = check.execute("PRAGMA user_version").fetchone()[0]
    check.close()
    if version != 1:
        sys.exit(f"pack.py: {out_path} came out with user_version={version}, expected 1")
    print(f"packed {out_path} ({out.stat().st_size} bytes, user_version=1)")


def main() -> None:
    args = sys.argv[1:]
    if "--out" not in args or len(args) != 3:
        sys.exit("usage: pack.py REQUEST.json --out BUNDLE")
    request = args[0]
    out = args[args.index("--out") + 1]
    pack(request, out)


if __name__ == "__main__":
    main()
