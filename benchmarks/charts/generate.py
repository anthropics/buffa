#!/usr/bin/env python3
"""Generate SVG bar charts from benchmark output files.

Usage:
    # Parse benchmark output and generate charts + README tables:
    python3 benchmarks/charts/generate.py benchmarks/results/

    # Or via task:
    task bench-charts

The results directory should contain output files from the benchmark
containers:

    buffa.json  — cargo-criterion JSON from bench-buffa
    prost.json  — cargo-criterion JSON from bench-prost
    google.json — cargo-criterion JSON from bench-google
    go.txt      — Go testing.B output from bench-go

cargo-criterion JSON contains one JSON object per line. Each
"benchmark-complete" message includes the benchmark id, throughput
(bytes per iteration), and timing statistics (ns per iteration).

Go output is parsed from standard "BenchmarkX/Msg-N ... MB/s" lines.
"""

from __future__ import annotations

import json
import math
import re
import sys
from dataclasses import dataclass
from pathlib import Path

# ── Colors ──────────────────────────────────────────────────────────────

COLORS = {
    "buffa": "#4C78A8",
    "buffa (view)": "#72B7B2",
    "prost": "#F58518",
    "prost (bytes)": "#EEAE62",
    "protobuf-v4": "#E45756",
    "Go": "#54A24B",
}

MESSAGES = ["ApiResponse", "LogRecord", "AnalyticsEvent", "GoogleMessage1", "MediaFrame"]

# Map from snake_case benchmark names to display names.
MSG_DISPLAY = {
    "api_response": "ApiResponse",
    "log_record": "LogRecord",
    "analytics_event": "AnalyticsEvent",
    "google_message1_proto3": "GoogleMessage1",
    "media_frame": "MediaFrame",
}

# ── Parsers ─────────────────────────────────────────────────────────────

def parse_criterion_json(text: str) -> dict[str, float]:
    """Parse cargo-criterion JSON output into {bench_id: median_MiB_s}.

    Each line is a JSON object. We look for "benchmark-complete" messages
    and compute throughput from throughput.per_iteration / typical.estimate.
    """
    results: dict[str, float] = {}

    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue

        if msg.get("reason") != "benchmark-complete":
            continue

        bench_id = msg["id"]
        throughput = msg.get("throughput")
        typical = msg.get("typical")

        if not throughput or not typical:
            continue

        bytes_per_iter = throughput[0]["per_iteration"]
        ns_per_iter = typical["estimate"]

        # bytes/ns = bytes/s * 1e-9, so MiB/s = bytes_per_iter / ns_per_iter * 1e9 / 1048576
        mib_per_s = bytes_per_iter / ns_per_iter * 1e9 / 1_048_576
        results[bench_id] = mib_per_s

    return results


def parse_criterion_verbose(text: str) -> dict[str, float]:
    """Parse criterion verbose (human-readable) output as fallback.

    Used when cargo-criterion is not available. Extracts benchmark names
    from "Benchmarking <name>" lines and median throughput from the first
    "thrpt:" line after each name.
    """
    results: dict[str, float] = {}
    current: str | None = None

    for line in text.splitlines():
        m = re.match(r"^(?:Benchmarking\s+)?(\S+/\S+/\S+)", line)
        if m:
            current = m.group(1).rstrip(":")

        if current and current not in results:
            m = re.search(
                r"thrpt:\s+\[[\d.]+ [A-Za-z/]+\s+([\d.]+) ([A-Za-z/]+)\s+[\d.]+ [A-Za-z/]+\]",
                line,
            )
            if m:
                val = float(m.group(1))
                if m.group(2) == "GiB/s":
                    val *= 1024
                results[current] = val

    return results


def parse_criterion(path: Path) -> dict[str, float]:
    """Auto-detect format and parse criterion output."""
    text = path.read_text()
    # If the first non-empty line starts with '{', it's JSON.
    for line in text.splitlines():
        if line.strip():
            if line.strip().startswith("{"):
                return parse_criterion_json(text)
            return parse_criterion_verbose(text)
    return {}


def parse_go(text: str) -> dict[str, float]:
    """Parse Go benchmark output into {bench_name: MiB_s}.

    Expects lines like:
        BenchmarkBinaryDecode/ApiResponse-64  200000  8234 ns/op  746.12 MB/s
    """
    results: dict[str, float] = {}
    for line in text.splitlines():
        m = re.match(
            r"^(Benchmark\w+/\w+)-\d+\s+\d+\s+[\d.]+ ns/op\s+([\d.]+) MB/s",
            line,
        )
        if m:
            # Go uses decimal MB/s; convert to MiB/s.
            results[m.group(1)] = float(m.group(2)) * 1_000_000 / 1_048_576
    return results


# ── Data extraction ─────────────────────────────────────────────────────

def _get(data: dict[str, float], prefix: str, msg_snake: str, op: str) -> float | None:
    key = f"{prefix}/{msg_snake}/{op}"
    return data.get(key)


def _get_go(data: dict[str, float], op: str, msg_display: str) -> float | None:
    key = f"Benchmark{op}/{msg_display}"
    return data.get(key)


def build_tables(
    buffa: dict[str, float],
    prost: dict[str, float],
    prost_bytes: dict[str, float],
    google: dict[str, float],
    go: dict[str, float],
) -> dict[str, dict[str, dict[str, float | None]]]:
    """Build structured data for each chart.

    Returns: {chart_name: {series_name: {MessageDisplay: value}}}
    """
    tables: dict[str, dict[str, dict[str, float | None]]] = {}

    for chart, series_defs in [
        ("binary-decode", [
            ("buffa",         lambda ms, md: _get(buffa, "buffa", ms, "decode")),
            ("buffa (view)",  lambda ms, md: _get(buffa, "buffa", ms, "decode_view")),
            ("prost",         lambda ms, md: _get(prost, "prost", ms, "decode")),
            ("prost (bytes)", lambda ms, md: _get(prost_bytes, "prost-bytes", ms, "decode")),
            ("protobuf-v4",   lambda ms, md: _get(google, "google", ms, "decode")),
            ("Go",            lambda ms, md: _get_go(go, "BinaryDecode", md)),
        ]),
        ("binary-encode", [
            ("buffa",         lambda ms, md: _get(buffa, "buffa", ms, "encode")),
            ("buffa (view)",  lambda ms, md: _get(buffa, "buffa", ms, "encode_view")),
            ("prost",         lambda ms, md: _get(prost, "prost", ms, "encode")),
            ("prost (bytes)", lambda ms, md: _get(prost_bytes, "prost-bytes", ms, "encode")),
            ("protobuf-v4",   lambda ms, md: _get(google, "google", ms, "encode")),
            ("Go",            lambda ms, md: _get_go(go, "BinaryEncode", md)),
        ]),
        ("build-encode", [
            ("buffa",        lambda ms, md: _get(buffa, "buffa", ms, "build_encode")),
            ("buffa (view)", lambda ms, md: _get(buffa, "buffa", ms, "build_encode_view")),
        ]),
        ("json-encode", [
            ("buffa",        lambda ms, md: _get(buffa, "buffa", ms, "json_encode")),
            ("prost",        lambda ms, md: _get(prost, "prost", ms, "json_encode")),
            ("Go",           lambda ms, md: _get_go(go, "JsonEncode", md)),
        ]),
        ("json-decode", [
            ("buffa",        lambda ms, md: _get(buffa, "buffa", ms, "json_decode")),
            ("prost",        lambda ms, md: _get(prost, "prost", ms, "json_decode")),
            ("Go",           lambda ms, md: _get_go(go, "JsonDecode", md)),
        ]),
    ]:
        table: dict[str, dict[str, float | None]] = {}
        for series_name, getter in series_defs:
            row: dict[str, float | None] = {}
            for msg_snake, msg_display in MSG_DISPLAY.items():
                row[msg_display] = getter(msg_snake, msg_display)
            table[series_name] = row
        tables[chart] = table

    return tables


def messages_with_data(table: dict[str, dict[str, float | None]]) -> list[str]:
    """Subset of MESSAGES that have at least one non-None value in this table."""
    return [m for m in MESSAGES if any(table[s].get(m) for s in table)]


# ── SVG chart generation ───────────────────────────────────────────────

@dataclass
class Series:
    name: str
    color: str
    data: dict[str, float]


def _format_axis(v: float, unit: str) -> str:
    """Axis tick labels. Plain integers with commas for small values; 'k' suffix
    only kicks in at ≥10,000 (so 1,200 isn't rendered as "1.2k"). When the
    chart uses GiB/s, show one decimal place."""
    if unit == "GiB/s":
        return f"{v:.1f}"
    if v >= 10_000:
        return f"{v / 1000:.0f}k"
    return f"{v:,.0f}"


def _format_bar(v: float, unit: str) -> str:
    """Inline value label next to each bar."""
    if unit == "GiB/s":
        return f"{v:.2f}"
    return f"{int(v):,}"


def _nice_max(values: list[float]) -> float:
    raw_max = max(values)
    magnitude = 10 ** math.floor(math.log10(raw_max))
    return math.ceil(raw_max / magnitude) * magnitude


# Threshold at which we switch MiB/s → GiB/s for a chart: values ≥10 GiB/s
# produce unwieldy MiB/s axis labels ("70k MiB/s" → "68.4 GiB/s" reads better).
_MIB_TO_GIB_CUTOFF = 10 * 1024


def generate_chart(title: str, unit: str, messages: list[str],
                   series_list: list[Series]) -> str:
    bar_h = 22
    bar_gap = 4
    group_gap = 20
    label_w = 130
    chart_left = label_w + 10
    chart_w = 580
    legend_h = 40
    title_h = 30
    top_margin = title_h + legend_h + 10
    bottom_margin = 35

    n_bars = len(series_list)
    group_h = n_bars * (bar_h + bar_gap) - bar_gap + group_gap
    total_chart_h = len(messages) * group_h - group_gap
    svg_h = top_margin + total_chart_h + bottom_margin
    svg_w = chart_left + chart_w + 80

    all_vals = [v for s in series_list for v in s.data.values() if v]
    # Auto-rescale MiB/s → GiB/s on the chart when the max value is large
    # enough to make integer-MiB axis labels unreadable.
    scale_factor = 1.0
    if unit == "MiB/s" and max(all_vals) >= _MIB_TO_GIB_CUTOFF:
        unit = "GiB/s"
        scale_factor = 1 / 1024
    all_vals = [v * scale_factor for v in all_vals]
    max_val = _nice_max(all_vals)
    scale = chart_w / max_val

    n_grid = 5
    grid_step = max_val / n_grid

    lines: list[str] = []
    a = lines.append

    a(f'<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"'
      f' viewBox="0 0 {svg_w} {svg_h}">')
    a('  <style>')
    a('    text { font-family: -apple-system, BlinkMacSystemFont, '
      '"Segoe UI", Helvetica, Arial, sans-serif; }')
    a('    .title { font-size: 16px; font-weight: 600; fill: #24292f; }')
    a('    .label { font-size: 12px; fill: #24292f; }')
    a('    .value { font-size: 11px; fill: #57606a; }')
    a('    .axis-label { font-size: 11px; fill: #57606a; }')
    a('    .legend-text { font-size: 12px; fill: #24292f; }')
    a('    .grid { stroke: #d0d7de; stroke-width: 0.5; }')
    a('  </style>')
    a('  <rect width="100%" height="100%" fill="white"/>')

    a(f'  <text x="{svg_w / 2}" y="{title_h - 5}" text-anchor="middle"'
      f' class="title">{title}</text>')

    lx = chart_left
    for s in series_list:
        a(f'  <rect x="{lx}" y="{title_h + 5}" width="14" height="14"'
          f' rx="2" fill="{s.color}"/>')
        a(f'  <text x="{lx + 18}" y="{title_h + 16}"'
          f' class="legend-text">{s.name}</text>')
        lx += len(s.name) * 7.5 + 32

    for i in range(n_grid + 1):
        val = grid_step * i
        x = chart_left + val * scale
        a(f'  <line x1="{x:.1f}" y1="{top_margin}"'
          f' x2="{x:.1f}" y2="{top_margin + total_chart_h}" class="grid"/>')
        a(f'  <text x="{x:.1f}" y="{top_margin + total_chart_h + 15}"'
          f' text-anchor="middle" class="axis-label">'
          f'{_format_axis(val, unit)}</text>')

    a(f'  <text x="{chart_left + chart_w / 2}"'
      f' y="{svg_h - 5}" text-anchor="middle" class="axis-label">{unit}</text>')

    for gi, msg in enumerate(messages):
        gy = top_margin + gi * group_h
        label_y = gy + (n_bars * (bar_h + bar_gap) - bar_gap) / 2 + 4
        a(f'  <text x="{label_w}" y="{label_y:.1f}" text-anchor="end"'
          f' class="label">{msg}</text>')

        for si, s in enumerate(series_list):
            val = s.data.get(msg, 0) * scale_factor
            by = gy + si * (bar_h + bar_gap)
            bw = max(val * scale, 1)
            a(f'  <rect x="{chart_left}" y="{by:.1f}" width="{bw:.1f}"'
              f' height="{bar_h}" rx="2" fill="{s.color}"/>')
            a(f'  <text x="{chart_left + bw + 4:.1f}" y="{by + bar_h / 2 + 4:.1f}"'
              f' class="value">{_format_bar(val, unit)}</text>')

    a('</svg>')
    return '\n'.join(lines)


# ── README table generation ────────────────────────────────────────────

def _pct(val: float | None, baseline: float | None) -> str:
    """Format value with percentage diff vs baseline."""
    if val is None:
        return "\u2014"
    v = int(round(val))
    if baseline is None or baseline == val:
        return f"{v:,}"
    diff = (val - baseline) / baseline * 100
    sign = "+" if diff > 0 else "\u2212"
    return f"{v:,} ({sign}{abs(diff):.0f}%)"


def generate_readme_tables(tables: dict[str, dict[str, dict[str, float | None]]]) -> str:
    """Generate markdown table snippets for the README."""
    sections: list[str] = []

    chart_meta = {
        "binary-decode": ("Binary decode", "buffa"),
        "binary-encode": ("Binary encode", "buffa"),
        "build-encode": ("Build + binary encode", "buffa"),
        "json-encode": ("JSON encode", "buffa"),
        "json-decode": ("JSON decode", "buffa"),
    }

    for chart_name, table in tables.items():
        heading, baseline_name = chart_meta[chart_name]
        series_names = list(table.keys())
        header = "| Message | " + " | ".join(series_names) + " |"
        sep = "|---------|" + "|".join("------:" for _ in series_names) + "|"

        rows: list[str] = []
        for msg in messages_with_data(table):
            baseline = table[baseline_name].get(msg)
            cells = [_pct(table[s].get(msg), baseline) for s in series_names]
            rows.append(f"| {msg} | " + " | ".join(cells) + " |")

        sections.append(f"### {heading}\n\n{header}\n{sep}\n" + "\n".join(rows))

    return "\n\n".join(sections)


# ── Main ────────────────────────────────────────────────────────────────

def main() -> None:
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <results-dir>", file=sys.stderr)
        print(f"  e.g.: {sys.argv[0]} benchmarks/results/", file=sys.stderr)
        sys.exit(1)

    results_dir = Path(sys.argv[1])
    charts_dir = Path(__file__).parent

    # Parse output files. Try .json first, fall back to .txt for criterion.
    def load_criterion(name: str) -> dict[str, float]:
        json_path = results_dir / f"{name}.json"
        txt_path = results_dir / f"{name}.txt"
        if json_path.exists():
            return parse_criterion(json_path)
        if txt_path.exists():
            return parse_criterion(txt_path)
        print(f"  warning: no results for {name} ({json_path} / {txt_path})",
              file=sys.stderr)
        return {}

    buffa = load_criterion("buffa")
    prost = load_criterion("prost")
    prost_bytes = load_criterion("prost-bytes")
    google = load_criterion("google")

    go_path = results_dir / "go.txt"
    go = parse_go(go_path.read_text()) if go_path.exists() else {}

    print(f"Parsed: {len(buffa)} buffa, {len(prost)} prost, "
          f"{len(prost_bytes)} prost-bytes, {len(google)} google, "
          f"{len(go)} Go benchmarks")

    # Build structured tables.
    tables = build_tables(buffa, prost, prost_bytes, google, go)

    # Generate SVGs.
    chart_titles = {
        "binary-decode": "Binary Decode Throughput",
        "binary-encode": "Binary Encode Throughput",
        "build-encode": "Build + Binary Encode Throughput (from borrowed source data)",
        "json-encode": "JSON Encode Throughput",
        "json-decode": "JSON Decode Throughput",
    }

    # Per-message SVGs: one file per (chart, message) so each can use its own
    # throughput scale. MediaFrame's ~70 GiB/s view decode would otherwise
    # compress the other four messages' bars into a few pixels.
    # Reverse-map display name → snake-case filename stem.
    snake_for: dict[str, str] = {v: k for k, v in MSG_DISPLAY.items()}
    for chart_name, table in tables.items():
        title_base = chart_titles[chart_name]
        for msg in MESSAGES:
            series_list = [
                Series(name=name, color=COLORS[name],
                       data={msg: vals[msg]} if vals.get(msg) is not None else {})
                for name, vals in table.items()
            ]
            # Drop series that have no value for this message (e.g. google/go
            # for MediaFrame) so the chart doesn't render empty bars.
            series_list = [s for s in series_list if s.data]
            if not series_list:
                continue
            svg = generate_chart(f"{title_base} — {msg}", "MiB/s",
                                 [msg], series_list)
            path = charts_dir / f"{chart_name}-{snake_for[msg]}.svg"
            path.write_text(svg + "\n")
            print(f"  wrote {path}")

    # Print README-ready tables.
    readme = generate_readme_tables(tables)
    readme_path = charts_dir / "tables.md"
    readme_path.write_text(readme + "\n")
    print(f"  wrote {readme_path} (copy to README.md Performance section)")


if __name__ == "__main__":
    main()
