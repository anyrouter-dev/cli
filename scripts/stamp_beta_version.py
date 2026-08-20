#!/usr/bin/env python3
"""Stamp Cargo.toml as the next-patch beta for a main-branch build.

The committed version stays the last stable (e.g. 0.1.8). CI on main rewrites
it in the workspace to 0.1.9-beta.<run> so the binary and GitHub prerelease
tag match, and `anyr upgrade --channel beta` sees a newer version.

  python scripts/stamp_beta_version.py --run 42
  python scripts/stamp_beta_version.py --run 42 --print   # no write
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO = ROOT / "Cargo.toml"
VER_RE = re.compile(r'^(version = ")([^"]+)(")', re.M)


def parse_core(version: str) -> tuple[int, int, int]:
    version = version.strip()
    if version.startswith("v"):
        version = version[1:]
    core = version.split("+", 1)[0].split("-", 1)[0]
    parts = core.split(".")
    major = int(parts[0])
    minor = int(parts[1]) if len(parts) > 1 else 0
    patch = int(parts[2]) if len(parts) > 2 else 0
    return major, minor, patch


def next_beta(current: str, run: int) -> str:
    if run < 1:
        raise ValueError(f"run must be >= 1, got {run}")
    major, minor, patch = parse_core(current)
    return f"{major}.{minor}.{patch + 1}-beta.{run}"


def cargo_version(text: str) -> str:
    match = VER_RE.search(text)
    if not match:
        raise ValueError("no version = \"...\" in Cargo.toml")
    return match.group(2)


def stamp_text(text: str, new: str) -> str:
    updated, n = VER_RE.subn(rf"\g<1>{new}\g<3>", text, count=1)
    if n != 1:
        raise ValueError("could not replace Cargo.toml version")
    return updated


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=int, required=True, help="GitHub run number")
    parser.add_argument(
        "--print",
        action="store_true",
        dest="print_only",
        help="print the beta version and do not write Cargo.toml",
    )
    parser.add_argument("--cargo", type=Path, default=CARGO)
    args = parser.parse_args(argv)
    text = args.cargo.read_text(encoding="utf-8")
    current = cargo_version(text)
    beta = next_beta(current, args.run)
    if not args.print_only:
        args.cargo.write_text(stamp_text(text, beta), encoding="utf-8")
    print(beta)
    return 0


if __name__ == "__main__":
    sys.exit(main())
