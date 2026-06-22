#!/usr/bin/env python3
"""Parse a criterion bench run into one release's history JSON.

The input is the captured stdout of a criterion benchmark binary run with
`--bench` (see benchmarks/history/README.md for how the runs are produced).
Each benchmark prints a block like:

    buffa/api_response/decode
                            time:   [8.1753 us 8.2557 us 8.3250 us]
                            thrpt:  [738.31 MiB/s 744.51 MiB/s 751.83 MiB/s]

We keep the midpoint of each triplet (criterion's point estimate): time is
normalised to nanoseconds and throughput to MiB/s, so numbers stay comparable
across releases even when a tag's dataset size drifts. When the run also carries
the `=== sysinfo ... ===` header that the metal-box runner emits, the machine
details (CPU, kernel, tuning) are harvested from it.

Usage:
    parse_criterion.py --version v0.3.0 --stdout run.txt \
        --commit <sha> --commit-date 2026-04-01T15:25:42-07:00 \
        --measured-at 2026-06-19T04:30:00Z \
        --instance-type c7i.metal-24xl --toolchain default \
        --criterion 0.5.1 --out runs/v0.3.0.json
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from pathlib import Path

# Time units -> nanoseconds.
TIME_TO_NS = {
    "ps": 1e-3,
    "ns": 1.0,
    "us": 1e3,
    "µs": 1e3,  # U+00B5 micro sign (what criterion emits)
    "μs": 1e3,  # U+03BC greek mu, just in case
    "ms": 1e6,
    "s": 1e9,
}

# Byte-throughput units -> MiB/s.
THRPT_TO_MIBPS = {
    "B/s": 1.0 / (1024 * 1024),
    "KiB/s": 1.0 / 1024,
    "MiB/s": 1.0,
    "GiB/s": 1024.0,
    "TiB/s": 1024.0 * 1024.0,
}

_NUM = r"[0-9]+(?:\.[0-9]+)?"
# A triplet line: "time:   [lo mid hi]" with unit after each number.
TIME_RE = re.compile(rf"time:\s*\[\s*{_NUM}\s+(\S+)\s+({_NUM})\s+(\S+)\s+{_NUM}\s+(\S+)\s*\]")
THRPT_RE = re.compile(rf"thrpt:\s*\[\s*{_NUM}\s+(\S+)\s+({_NUM})\s+(\S+)\s+{_NUM}\s+(\S+)\s*\]")

# A benchmark id: "buffa/<group>/<bench>". Matched against the leading token of
# an unindented line, so it must be a full-string match (no trailing metric).
ID_RE = re.compile(r"^[A-Za-z0-9_]+(?:/[A-Za-z0-9_]+)+$")

_warned_units: set[str] = set()


def _warn_unit(kind: str, unit: str) -> None:
    if unit not in _warned_units:
        _warned_units.add(unit)
        print(f"warning: unrecognized {kind} unit {unit!r}; affected benchmarks skipped",
              file=sys.stderr)


def _time_ns(value: str, unit: str) -> float | None:
    mult = TIME_TO_NS.get(unit)
    if mult is None:
        _warn_unit("time", unit)
        return None
    return round(float(value) * mult, 4)


def _thrpt_mibps(value: str, unit: str) -> float | None:
    mult = THRPT_TO_MIBPS.get(unit)
    if mult is None:
        _warn_unit("throughput", unit)
        return None
    return round(float(value) * mult, 4)


def parse_benchmarks(text: str) -> dict[str, dict]:
    """Map benchmark id -> {median_ns, throughput_mib_s} from criterion text.

    Criterion's CLI formatter only prints a benchmark id on its own line when
    the id is longer than 23 characters; for shorter ids it left-pads the id to
    23 columns and prints `time:` on the *same* line (e.g.
    `buffa/log_record/merge  time:   [...]`). Several buffa ids sit at or under
    that threshold, so an unindented line is treated as the leading id token
    followed by an optional inline metric — we must not skip the rest of the
    line. The `thrpt:` line is always on its own indented line.
    """
    benchmarks: dict[str, dict] = {}
    current: str | None = None
    # Unindented non-id lines from criterion / the runner: progress, plot
    # backend, outlier notes, and the sysinfo/run banners.
    noise = ("Benchmarking", "Found", "Testing", "Gnuplot")
    for line in text.splitlines():
        if line[:1] not in (" ", "\t"):
            head = line.split(None, 1)[0] if line.strip() else ""
            if ID_RE.match(head) and not head.startswith(noise):
                current = head
            else:
                current = None
                continue
            # Fall through: this same line may also carry an inline "time:".
        if current is None:
            continue
        tm = TIME_RE.search(line)
        if tm:
            # groups: (lo_unit, mid_val, mid_unit, hi_unit) — use mid.
            ns = _time_ns(tm.group(2), tm.group(3))
            if ns is not None:
                benchmarks.setdefault(current, {})["median_ns"] = ns
            continue
        pm = THRPT_RE.search(line)
        if pm:
            thr = _thrpt_mibps(pm.group(2), pm.group(3))
            if thr is not None:
                benchmarks.setdefault(current, {})["throughput_mib_s"] = thr
    return benchmarks


def parse_sysinfo(text: str) -> dict:
    """Harvest machine details from the metal-runner's sysinfo header."""
    info: dict = {}
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("Linux ") and "kernel" not in info:
            # `uname -a` is "Linux <nodename> <release> <version...>"; drop the
            # nodename (an ephemeral runner hostname) and keep the kernel info.
            parts = s.split()
            # Keep "Linux <release> <version...>"; drop parts[1] (the nodename).
            info["kernel"] = " ".join([parts[0]] + parts[2:]) if len(parts) > 2 else parts[0]
        elif s.startswith("Model name:"):
            info["cpu"] = s.split(":", 1)[1].strip()
        elif s.startswith("turbo_disabled="):
            for kv in s.split():
                if "=" in kv:
                    k, v = kv.split("=", 1)
                    info[k] = v
    return info


def aggregate(captures: list[dict[str, dict]]) -> dict[str, dict]:
    """Merge several captures of the same release (one per core) into one set of
    per-benchmark stats: the median across captures, plus the spread as a
    stability indicator. A single capture yields that value with zero spread."""
    by_bench: dict[str, dict[str, list[float]]] = {}
    for cap in captures:
        for bench, stats in cap.items():
            slot = by_bench.setdefault(bench, {"median_ns": [], "throughput_mib_s": []})
            for k in ("median_ns", "throughput_mib_s"):
                if stats.get(k) is not None:
                    slot[k].append(stats[k])
    merged: dict[str, dict] = {}
    for bench, slot in by_bench.items():
        thr = slot["throughput_mib_s"]
        ns = slot["median_ns"]
        entry: dict = {}
        if ns:
            entry["median_ns"] = round(statistics.median(ns), 4)
        if thr:
            med = statistics.median(thr)
            entry["throughput_mib_s"] = round(med, 4)
            entry["samples"] = len(thr)
            # Spread = (max-min)/median %, a direction-agnostic stability measure.
            entry["throughput_spread_pct"] = (
                round((max(thr) - min(thr)) / med * 100, 2) if len(thr) > 1 and med else 0.0
            )
        merged[bench] = entry
    return merged


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True, help="release tag, e.g. v0.3.0")
    ap.add_argument("--stdout", required=True, type=Path, action="append",
                    help="captured criterion stdout (repeatable: one per core, merged to a median)")
    ap.add_argument("--commit", default="", help="full commit sha of the tag")
    ap.add_argument("--commit-date", default="", help="committer date (ISO 8601)")
    ap.add_argument("--measured-at", default="", help="when this run was measured (ISO 8601)")
    ap.add_argument("--instance-type", default="", help="AWS instance type")
    ap.add_argument("--toolchain", default="", help="rust toolchain used")
    ap.add_argument("--criterion", default="", help="criterion version")
    ap.add_argument("--profile", default="", help="build profile, e.g. 'lto=true, codegen-units=1'")
    ap.add_argument("--out", type=Path, help="output JSON path (default: stdout)")
    args = ap.parse_args(argv)

    texts = [p.read_text(encoding="utf-8", errors="replace") for p in args.stdout]
    captures = [parse_benchmarks(t) for t in texts]
    benchmarks = aggregate(captures)
    if not benchmarks:
        print(f"error: no benchmarks parsed from {args.stdout}", file=sys.stderr)
        return 1

    # Machine/sysinfo are identical across cores; harvest from the first capture.
    machine = {"instance_type": args.instance_type}
    machine.update(parse_sysinfo(texts[0]))

    run = {
        "version": args.version,
        "commit": args.commit,
        "commit_date": args.commit_date,
        "measured_at": args.measured_at,
        "toolchain": args.toolchain,
        "criterion": args.criterion,
        "build_profile": args.profile,
        "machine": machine,
        "benchmarks": dict(sorted(benchmarks.items())),
    }
    out = json.dumps(run, indent=2, ensure_ascii=False) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(out, encoding="utf-8")
        print(f"wrote {args.out} ({len(benchmarks)} benchmarks)", file=sys.stderr)
    else:
        sys.stdout.write(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
