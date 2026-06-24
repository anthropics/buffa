#!/usr/bin/env python3
"""Aggregate a bare-metal cross-impl run into per-implementation median + spread.

Input is the concatenated stdout of `cross_metal_run.sh`: one block per
(implementation, pass), delimited by `===BLOCK=<impl>_<tag>===` markers, each
containing that pass's criterion stdout (Rust impls) or `go test -bench` output
(Go). For each implementation this takes the median throughput per benchmark
across the passes (plus the spread = (max-min)/median as a stability indicator),
then writes the per-impl inputs `generate.py` expects (`<impl>.json` in criterion
"verbose" form, `go.txt` in Go form) and a `measurement-spread.md` table.

    cross_aggregate.py <run-stdout.txt> <results-dir>

`generate.py <results-dir>` then renders tables.md + charts. Stdlib only.
"""
import re, sys, statistics
from pathlib import Path
from collections import defaultdict


def parse_criterion_verbose(text):
    """criterion human-readable stdout -> {bench_id: throughput MiB/s} (median)."""
    results = {}
    current = None
    for line in text.splitlines():
        m = re.match(r"^(?:Benchmarking\s+)?(\S+/\S+/\S+)", line)
        if m:
            current = m.group(1).rstrip(":")
        if current and current not in results:
            m = re.search(r"thrpt:\s+\[[\d.]+ [A-Za-z/]+\s+([\d.]+) ([A-Za-z/]+)\s+[\d.]+ [A-Za-z/]+\]", line)
            if m:
                val = float(m.group(1))
                if m.group(2) == "GiB/s":
                    val *= 1024
                results[current] = val
    return results


def parse_go(text):
    """`go test -bench` output -> {bench_name: throughput MiB/s}."""
    results = {}
    for line in text.splitlines():
        m = re.match(r"^(Benchmark\w+/\w+)(?:-\d+)?\s+\d+\s+[\d.]+ ns/op\s+([\d.]+) MB/s", line)
        if m:
            results[m.group(1)] = float(m.group(2)) * 1_000_000 / 1_048_576
    return results


def main():
    full = Path(sys.argv[1]).read_text(errors="replace")
    outdir = Path(sys.argv[2])
    outdir.mkdir(parents=True, exist_ok=True)

    # Split into blocks on the ===BLOCK=<tag>=== markers.
    blocks = {}
    cur, buf = None, []
    for line in full.splitlines():
        m = re.match(r"^===BLOCK=(.+?)===$", line)
        if m:
            if cur is not None:
                blocks.setdefault(cur, []).append("\n".join(buf))
            cur, buf = m.group(1), []
        elif line.strip() == "===END===":
            if cur is not None:
                blocks.setdefault(cur, []).append("\n".join(buf))
                cur = None
        else:
            buf.append(line)
    if cur is not None:
        blocks.setdefault(cur, []).append("\n".join(buf))

    # Tag is "<impl>_<pass-tag>"; prost-bytes contains a hyphen so test longest first.
    IMPLS = ["buffa", "prost-bytes", "prost", "google", "go"]
    per_impl = defaultdict(lambda: defaultdict(list))  # impl -> bench_id -> [values]
    passes = defaultdict(set)
    for tag, texts in blocks.items():
        impl = next((i for i in IMPLS if tag.startswith(i + "_")), None)
        if not impl:
            continue
        parsed = parse_go(texts[0]) if impl == "go" else parse_criterion_verbose(texts[0])
        if parsed:
            passes[impl].add(tag[len(impl) + 1:])
        for bid, v in parsed.items():
            per_impl[impl][bid].append(v)

    agg = {}  # impl -> bench_id -> (median, spread_pct)
    for impl, benches in per_impl.items():
        agg[impl] = {}
        for bid, vals in benches.items():
            med = statistics.median(vals)
            spread = (max(vals) - min(vals)) / med * 100 if len(vals) > 1 and med else 0.0
            agg[impl][bid] = (med, spread)

    # Synthesize generate.py inputs: criterion-verbose per Rust impl, Go form for go.
    def verbose(med):
        return f"                        thrpt:  [{med:.4f} MiB/s {med:.4f} MiB/s {med:.4f} MiB/s]"

    for impl in ("buffa", "prost", "prost-bytes", "google"):
        lines = []
        for bid, (med, _) in sorted(agg.get(impl, {}).items()):
            if bid.startswith("reflect/"):
                continue  # buffa's reflection benchmarks go to reflect.json (below)
            lines.append(bid)
            lines.append(verbose(med))
        (outdir / f"{impl}.json").write_text("\n".join(lines) + "\n")
    # buffa's reflection benchmarks (generated codec vs reflect vs DynamicMessage)
    # are a separate input that generate.py loads as "reflect".
    rlines = []
    for bid, (med, _) in sorted(agg.get("buffa", {}).items()):
        if bid.startswith("reflect/"):
            rlines.append(bid)
            rlines.append(verbose(med))
    (outdir / "reflect.json").write_text("\n".join(rlines) + "\n")
    glines = []
    for bid, (med, _) in sorted(agg.get("go", {}).items()):
        mbps = med * 1_048_576 / 1_000_000  # MiB/s -> decimal MB/s for parse_go
        glines.append(f"{bid}-64  1  1.0 ns/op  {mbps:.2f} MB/s")
    (outdir / "go.txt").write_text("\n".join(glines) + "\n")

    npass = max((len(p) for p in passes.values()), default=0)
    DISP = {"buffa": "buffa", "prost": "prost", "prost-bytes": "prost (bytes)",
            "google": "protobuf-v4", "go": "Go"}
    md = ["## Measurement spread",
          "",
          f"Per-benchmark spread = (max−min)/median throughput across the "
          f"{npass} sequential one-at-a-time passes. Generated by cross_aggregate.py.",
          "",
          "| Implementation | benchmarks | median | p90 | max |",
          "|---|--:|--:|--:|--:|"]
    for impl in ("buffa", "prost", "prost-bytes", "google", "go"):
        sp = sorted(s for bid, (_, s) in agg.get(impl, {}).items() if not bid.startswith("reflect/"))
        if not sp:
            continue
        p50 = statistics.median(sp)
        p90 = sp[min(len(sp) - 1, int(len(sp) * 0.9))]
        md.append(f"| {DISP[impl]} | {len(sp)} | {p50:.1f}% | {p90:.1f}% | {sp[-1]:.1f}% |")
    (Path(__file__).resolve().parent / "measurement-spread.md").write_text("\n".join(md) + "\n")
    print(f"passes per impl: {{{', '.join(f'{k}: {len(v)}' for k, v in passes.items())}}}")
    print(f"benchmarks per impl: {{{', '.join(f'{k}: {len(v)}' for k, v in agg.items())}}}")


if __name__ == "__main__":
    main()
