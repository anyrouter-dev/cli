#!/usr/bin/env python3
"""Measure anyr binary size + startup, or fold CI JSON into a markdown report."""
from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path


def human_bytes(n: int) -> str:
    if n < 1024:
        return f"{n} B"
    x = float(n)
    for unit in ("KiB", "MiB", "GiB"):
        x /= 1024.0
        if x < 1024:
            return f"{x:.1f} {unit}"
    return f"{x:.1f} TiB"


def time_cmd(cmd: list[str], n: int = 21) -> dict:
    samples: list[float] = []
    for i in range(n):
        t0 = time.perf_counter()
        proc = subprocess.run(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        ms = (time.perf_counter() - t0) * 1000.0
        if proc.returncode != 0 and i == 0:
            raise SystemExit(f"command failed ({proc.returncode}): {' '.join(cmd)}")
        samples.append(ms)
    samples.sort()
    p95_idx = min(len(samples) - 1, max(0, int(round(0.95 * (len(samples) - 1)))))
    return {
        "n": n,
        "min_ms": round(samples[0], 2),
        "median_ms": round(statistics.median(samples), 2),
        "p95_ms": round(samples[p95_idx], 2),
        "mean_ms": round(statistics.mean(samples), 2),
    }


def measure(args: argparse.Namespace) -> None:
    path = Path(args.bin).expanduser()
    if not path.is_absolute():
        path = Path.cwd() / path
    path = path.resolve()
    if not path.is_file():
        raise SystemExit(f"binary not found: {path}")
    bin_cmd = str(path)
    size = path.stat().st_size
    version = (
        subprocess.check_output([bin_cmd, "--version"], text=True).strip()
        if args.kind == "native"
        else args.version or "wasm"
    )
    record = {
        "asset": args.asset,
        "kind": args.kind,
        "path": str(path),
        "bytes": size,
        "size": human_bytes(size),
        "version": version,
        "target": args.target or "",
        "os": args.os or "",
    }
    if args.kind == "native":
        record["startup_version"] = time_cmd([bin_cmd, "--version"], n=args.iters)
        record["startup_help"] = time_cmd([bin_cmd, "--help"], n=args.iters)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(record, indent=2))


def load_records(paths: list[str]) -> list[dict]:
    rows = []
    for p in paths:
        path = Path(p)
        if path.is_dir():
            for child in sorted(path.rglob("*.json")):
                rows.append(json.loads(child.read_text(encoding="utf-8")))
        elif path.is_file():
            rows.append(json.loads(path.read_text(encoding="utf-8")))
    rows.sort(key=lambda r: r.get("asset") or r.get("path") or "")
    return rows


def to_markdown(rows: list[dict], title: str) -> str:
    lines = [
        f"## {title}",
        "",
        "Startup is wall time for a cold `anyr --version` / `anyr --help` "
        "(median of 21 runs). Size is the stripped release binary, or the "
        "`.wasm` for the browser demo.",
        "",
        "| Asset | Kind | Size | `--version` median | `--help` median |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for row in rows:
        ver = row.get("startup_version") or {}
        help_ = row.get("startup_help") or {}
        ver_s = f"{ver['median_ms']} ms" if "median_ms" in ver else "—"
        help_s = f"{help_['median_ms']} ms" if "median_ms" in help_ else "—"
        lines.append(
            f"| `{row.get('asset', '?')}` | {row.get('kind', '?')} | "
            f"{row.get('size', '?')} | {ver_s} | {help_s} |"
        )
    lines.append("")
    lines.append("<details><summary>raw timings</summary>")
    lines.append("")
    lines.append("```json")
    lines.append(json.dumps(rows, indent=2))
    lines.append("```")
    lines.append("")
    lines.append("</details>")
    lines.append("")
    return "\n".join(lines)


def report(args: argparse.Namespace) -> None:
    rows = load_records(args.inputs)
    if not rows:
        raise SystemExit("no bench JSON found")
    md = to_markdown(rows, args.title)
    Path(args.out).write_text(md, encoding="utf-8")
    json_out = Path(args.json_out) if args.json_out else Path(args.out).with_suffix(".json")
    json_out.write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    sys.stdout.write(md)


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    m = sub.add_parser("measure")
    m.add_argument("--bin", required=True)
    m.add_argument("--asset", required=True)
    m.add_argument("--out", required=True)
    m.add_argument("--kind", default="native", choices=("native", "wasm"))
    m.add_argument("--target", default="")
    m.add_argument("--os", default="")
    m.add_argument("--version", default="")
    m.add_argument("--iters", type=int, default=21)
    m.set_defaults(func=measure)

    r = sub.add_parser("report")
    r.add_argument("inputs", nargs="+")
    r.add_argument("--out", required=True)
    r.add_argument("--json-out", default="")
    r.add_argument("--title", default="anyr size and startup")
    r.set_defaults(func=report)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
