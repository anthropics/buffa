#!/usr/bin/env python3
"""Quantify the build-layout-noise envelope of buffa's benchmark binary.

The per-release history (REPORT.md) attributes a movement to "buffa's own
code". That attribution is only sound above the binary's *layout noise*: the
benches build with cargo's default `bench` profile (codegen-units=16, lto=off,
because benchmarks/buffa is excluded from the root workspace), and at 16
codegen units the partitioning of functions into units — and therefore which
calls get inlined and where code lands — shifts when unrelated code is added.
A small dispatch-bound benchmark can move 10-20% from that alone.

This tool measures that envelope. Feed it several captures of the *same*
source built at different codegen-units settings (see build-cgu-variants.sh);
each `codegen-units` value is a distinct, deterministic layout, so the spread
across them is a proxy for how much pure layout perturbation moves each
benchmark. A cross-release delta smaller than this envelope is not
attributable to a code change.

Usage:
    layout_envelope.py --run cgu1=cgu1.txt --run cgu16=cgu16.txt [...] \
        [--metric throughput_mib_s|median_ns] [--out envelope.json]

Reuses parse_criterion.parse_benchmarks; stdlib only.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from parse_criterion import parse_benchmarks  # noqa: E402


def percentile(values: list[float], q: float) -> float:
    """Linear-interpolated percentile (q in [0, 100]). Empty -> 0.0."""
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    ordered = sorted(values)
    rank = (q / 100.0) * (len(ordered) - 1)
    lo = int(rank)
    hi = min(lo + 1, len(ordered) - 1)
    frac = rank - lo
    return ordered[lo] + (ordered[hi] - ordered[lo]) * frac


def compute_envelope(
    runs: dict[str, dict[str, dict]], metric: str
) -> dict[str, dict]:
    """Per-benchmark spread of `metric` across the labelled runs.

    Only benchmarks present in at least two runs are included — a single
    sample has no spread to report. `range_pct` is (max - min) / median * 100,
    a layout-perturbation magnitude that is unit-free and direction-agnostic
    (it measures instability, not whether higher is better).
    """
    labels = list(runs)
    benches = sorted(
        {b for r in runs.values() for b in r}
    )
    out: dict[str, dict] = {}
    for bench in benches:
        values = {
            label: runs[label][bench][metric]
            for label in labels
            if bench in runs[label] and metric in runs[label][bench]
        }
        if len(values) < 2:
            continue
        vs = list(values.values())
        lo, hi = min(vs), max(vs)
        median = statistics.median(vs)
        range_pct = (hi - lo) / median * 100 if median else 0.0
        out[bench] = {
            "values": values,
            "min": lo,
            "max": hi,
            "median": round(median, 4),
            "range_pct": round(range_pct, 2),
        }
    return out


def summarize(envelope: dict[str, dict]) -> dict:
    """Distribution of per-benchmark range_pct across the suite."""
    ranges = [e["range_pct"] for e in envelope.values()]
    return {
        "benchmarks": len(ranges),
        "p50_range_pct": round(percentile(ranges, 50), 2),
        "p90_range_pct": round(percentile(ranges, 90), 2),
        "max_range_pct": round(max(ranges), 2) if ranges else 0.0,
    }


def render_markdown(
    envelope: dict[str, dict], summary: dict, labels: list[str], metric: str
) -> str:
    arrow = "lower=better" if metric == "median_ns" else "higher=better"
    lines = [
        "# Benchmark layout-noise envelope",
        "",
        f"Metric: `{metric}` ({arrow}). Each column is the same source built at a",
        "different `codegen-units` value; the spread across columns is pure",
        "build-layout perturbation, not a code change.",
        "",
        f"**Envelope across {summary['benchmarks']} benchmarks:** "
        f"p50 {summary['p50_range_pct']}%, p90 {summary['p90_range_pct']}%, "
        f"max {summary['max_range_pct']}%.",
        "",
        "Cross-release deltas at or below this envelope are layout noise, not",
        "attributable to buffa's code.",
        "",
    ]
    header = ["benchmark", *labels, "range %"]
    lines.append("| " + " | ".join(header) + " |")
    lines.append("|" + "|".join(["---"] * len(header)) + "|")
    for bench, e in sorted(
        envelope.items(), key=lambda kv: kv[1]["range_pct"], reverse=True
    ):
        cells = [bench]
        for label in labels:
            v = e["values"].get(label)
            cells.append(f"{v:g}" if v is not None else "—")
        cells.append(f"{e['range_pct']:.2f}")
        lines.append("| " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def _parse_run_arg(spec: str) -> tuple[str, Path]:
    if "=" not in spec:
        raise argparse.ArgumentTypeError(
            f"--run expects LABEL=PATH, got {spec!r}"
        )
    label, path = spec.split("=", 1)
    if not label:
        raise argparse.ArgumentTypeError(f"empty label in {spec!r}")
    return label, Path(path)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--run", action="append", type=_parse_run_arg, default=[], metavar="LABEL=PATH",
        help="a labelled criterion stdout capture; repeat for each build",
    )
    ap.add_argument(
        "--metric", default="throughput_mib_s",
        choices=["throughput_mib_s", "median_ns"],
    )
    ap.add_argument("--out", type=Path, help="write the envelope as JSON here")
    args = ap.parse_args(argv)

    if len(args.run) < 2:
        print("error: need at least two --run captures to measure a spread",
              file=sys.stderr)
        return 2

    runs: dict[str, dict[str, dict]] = {}
    for label, path in args.run:
        if label in runs:
            print(f"error: duplicate run label {label!r}", file=sys.stderr)
            return 2
        text = path.read_text(encoding="utf-8", errors="replace")
        benches = parse_benchmarks(text)
        if not benches:
            print(f"error: no benchmarks parsed from {path}", file=sys.stderr)
            return 1
        runs[label] = benches

    envelope = compute_envelope(runs, args.metric)
    if not envelope:
        print("error: no benchmark appears in two or more runs", file=sys.stderr)
        return 1
    summary = summarize(envelope)

    if args.out:
        payload = {"metric": args.metric, "summary": summary, "benchmarks": envelope}
        args.out.write_text(
            json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        print(f"wrote {args.out}", file=sys.stderr)
    sys.stdout.write(render_markdown(envelope, summary, list(runs), args.metric))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
