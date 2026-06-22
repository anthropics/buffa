#!/usr/bin/env python3
"""Render buffa's per-release benchmark history into a report and charts.

Reads the per-release JSON files under benchmarks/history/runs/ (one per
release, produced by parse_criterion.py) and writes:

    benchmarks/history/REPORT.md      tables of throughput per release, with
                                      the delta against the previous release,
                                      plus the biggest movers across the range.
    benchmarks/history/charts/*.svg   one throughput-over-releases line chart
                                      per operation, with a line per message.

Throughput (MiB/s) is the headline metric because it stays comparable across
releases even when a tag's dataset size drifts; the report also lists median
nanoseconds for the latest release. Run:

    python3 benchmarks/history/generate.py            # from repo root
    python3 benchmarks/history/generate.py <history-dir>

Stdlib only — the charts are hand-rolled SVG, matching benchmarks/charts/.
"""

from __future__ import annotations

import json
import math
import re
import sys
from pathlib import Path
from xml.sax.saxutils import escape

MINUS = "−"  # U+2212 MINUS SIGN, matching benchmarks/charts/tables.md.

# Half-width (percent) of the shaded "noise floor" band drawn around each
# message's 100% baseline on the charts. A line that stays inside the band never
# moved beyond the bare-metal run-to-run reproducibility floor, so its change is
# not consequential. Sized to the measured median spread of the run series.
NOISE_BAND_PCT = 5.0

# Stable display order + colours for the message types (a line each per chart).
MESSAGES = [
    ("api_response", "ApiResponse", "#4C78A8"),
    ("log_record", "LogRecord", "#F58518"),
    ("analytics_event", "AnalyticsEvent", "#54A24B"),
    ("google_message1_proto3", "GoogleMessage1", "#E45756"),
    ("media_frame", "MediaFrame", "#72B7B2"),
    ("packed_tile", "PackedTile", "#B279A2"),
]
MSG_DISPLAY = {snake: disp for snake, disp, _ in MESSAGES}
MSG_COLOR = {snake: color for snake, _, color in MESSAGES}
MSG_ORDER = {snake: i for i, (snake, _, _) in enumerate(MESSAGES)}

# Display names + order for the operations (one chart + one table each).
OPS = [
    ("decode", "Binary decode"),
    ("merge", "Merge into existing"),
    ("encode", "Binary encode"),
    ("compute_size", "Compute size"),
    ("decode_view", "View decode"),
    ("encode_view", "View encode"),
    ("build_encode", "Build + encode"),
    ("build_encode_view", "Build + encode (view)"),
    ("json_encode", "JSON encode"),
    ("json_decode", "JSON decode"),
]
OP_DISPLAY = dict(OPS)


def semver_key(version: str) -> tuple[int, ...]:
    nums = re.findall(r"\d+", version)
    return tuple(int(n) for n in nums)


def split_id(bench_id: str) -> tuple[str, str] | None:
    # "buffa/<message>/<op>" — op may itself contain underscores (kept whole).
    parts = bench_id.split("/")
    if len(parts) != 3 or parts[0] != "buffa":
        return None
    return parts[1], parts[2]


def load_runs(history_dir: Path) -> list[dict]:
    runs = []
    for p in (history_dir / "runs").glob("*.json"):
        try:
            runs.append(json.loads(p.read_text(encoding="utf-8")))
        except (json.JSONDecodeError, OSError) as e:
            raise SystemExit(f"bad run JSON {p}: {e}") from e
    if not runs:
        raise SystemExit(f"no run JSON files under {history_dir / 'runs'}")
    runs.sort(key=lambda r: semver_key(r["version"]))
    return runs


def throughput_matrix(runs: list[dict]) -> dict[tuple[str, str], dict[str, float]]:
    """(message, op) -> {version: throughput_mib_s}."""
    matrix: dict[tuple[str, str], dict[str, float]] = {}
    for run in runs:
        version = run["version"]
        for bench_id, stats in run["benchmarks"].items():
            mo = split_id(bench_id)
            if mo is None:
                continue
            thr = stats.get("throughput_mib_s")
            if thr is None:
                continue
            matrix.setdefault(mo, {})[version] = thr
    return matrix


# ── Formatting ──────────────────────────────────────────────────────────


def fmt_thr(v: float) -> str:
    return f"{v:,.0f}"


def fmt_delta(cur: float, prev: float | None) -> str:
    if prev is None or prev == 0:
        return ""
    pct = (cur - prev) / prev * 100
    sign = MINUS if pct < 0 else "+"
    return f" ({sign}{abs(pct):.0f}%)"


# ── Report ──────────────────────────────────────────────────────────────


def render_report(runs: list[dict], matrix: dict[tuple[str, str], dict[str, float]]) -> str:
    versions = [r["version"] for r in runs]
    latest = runs[-1]
    machine = latest.get("machine", {})

    out: list[str] = []
    w = out.append
    w("# buffa benchmark history")
    w("")
    w("Throughput of buffa's own protobuf benchmarks across releases, measured on a")
    w("dedicated bare-metal box (turbo off, `performance` governor, per-core pinned).")
    w("Each release's source is built at one fixed toolchain and profile, held")
    w("constant across the series, so a delta reflects buffa's code rather than a")
    w("compiler or build-config change. The headline metric is **throughput in")
    w("MiB/s**, the median across cores, comparable across releases even when a tag's")
    w("dataset changed size. See [README.md](README.md) for methodology and caveats.")
    w("")
    w("<!-- GENERATED by benchmarks/history/generate.py — do not edit by hand. -->")
    w("")
    cpu = machine.get("cpu", "?")
    w(f"- Releases: {', '.join(versions)}")
    w(f"- Machine: {machine.get('instance_type', '?')} — {cpu}")
    w(f"- Tuning: turbo_disabled={machine.get('turbo_disabled', '?')}, "
      f"governor={machine.get('governor', '?')}, pin_core={machine.get('pin_core', '?')}")
    w(f"- Build profile: {latest.get('build_profile') or '?'}")
    samples = max((b.get("samples", 1) for b in latest["benchmarks"].values()), default=1)
    if samples > 1:
        w(f"- Samples: median of {samples} cores per release (per-benchmark spread in run files)")
    w(f"- Criterion: {latest.get('criterion', '?')} · latest measured at "
      f"{latest.get('measured_at', '?')}")
    w("")

    # Biggest movers across each benchmark's tracked range (first → latest).
    movers: list[tuple[float, str, str, float, float, str, str]] = []
    for (msg, op), by_ver in matrix.items():
        present = [(v, by_ver[v]) for v in versions if v in by_ver]
        if len(present) < 2:
            continue
        (v0, t0), (v1, t1) = present[0], present[-1]
        if t0 == 0:
            continue
        pct = (t1 - t0) / t0 * 100
        movers.append((pct, msg, op, t0, t1, v0, v1))

    improvements = sorted((m for m in movers if m[0] > 0), key=lambda m: -m[0])[:8]
    regressions = sorted((m for m in movers if m[0] < 0), key=lambda m: m[0])[:8]

    w("## Biggest movers (first tracked release → latest)")
    w("")
    w("| Benchmark | First | Latest | Change | Range |")
    w("|-----------|------:|-------:|-------:|-------|")
    for pct, msg, op, t0, t1, v0, v1 in improvements + regressions:
        sign = MINUS if pct < 0 else "+"
        label = f"{MSG_DISPLAY.get(msg, msg)} / {op}"
        w(f"| {label} | {fmt_thr(t0)} | {fmt_thr(t1)} | {sign}{abs(pct):.0f}% | {v0}→{v1} |")
    w("")
    w("All throughput values are MiB/s; higher is better.")
    w("")

    # Per-operation tables.
    w("## Throughput by operation (MiB/s)")
    w("")
    for op, op_disp in OPS:
        rows = sorted(
            ((msg, by_ver) for (msg, o), by_ver in matrix.items() if o == op),
            key=lambda r: MSG_ORDER.get(r[0], 99),
        )
        if not rows:
            continue
        w(f"### {op_disp}")
        w("")
        w(f"![{op_disp}](charts/{op}.svg)")
        w("")
        w("| Message | " + " | ".join(versions) + " |")
        w("|---------|" + "|".join(["------:"] * len(versions)) + "|")
        for msg, by_ver in rows:
            cells = []
            prev: float | None = None
            for v in versions:
                if v in by_ver:
                    cur = by_ver[v]
                    # Delta is vs the previous release that *has* this benchmark,
                    # which for a benchmark introduced mid-history is the column
                    # to the left except across its introduction gap.
                    cells.append(fmt_thr(cur) + fmt_delta(cur, prev))
                    prev = cur
                else:
                    cells.append("—")
            w(f"| {MSG_DISPLAY.get(msg, msg)} | " + " | ".join(cells) + " |")
        w("")

    # Measurement spread per operation — how noisy each op's numbers are, so the
    # tables and charts above are read with the right caution. The charts shade a
    # ±5% noise floor; an operation whose spread routinely exceeds it needs a
    # larger move before it is meaningful. Full per-benchmark spread (and sample
    # count) is in runs/*.json; this is the per-op summary over all messages and
    # releases.
    op_spreads: dict[str, list[float]] = {}
    for run in runs:
        for bench_id, stats in run["benchmarks"].items():
            mo = split_id(bench_id)
            sp = stats.get("throughput_spread_pct")
            if mo is not None and sp is not None:
                op_spreads.setdefault(mo[1], []).append(sp)

    def pctile(xs: list[float], q: float) -> float:
        s = sorted(xs)
        return s[min(len(s) - 1, int(q * len(s)))]

    if op_spreads:
        w("## Measurement spread (core-to-core)")
        w("")
        w("Spread of the per-benchmark median across cores, summarised per operation")
        w("over all messages and releases. A delta in the tables above smaller than")
        w("the operation's spread here is noise, not signal.")
        w("")
        w("| Operation | Median spread | p90 spread | Max |")
        w("|-----------|--------------:|-----------:|----:|")
        for op, op_disp in OPS:
            xs = op_spreads.get(op)
            if not xs:
                continue
            w(f"| {op_disp} | {pctile(xs, 0.5):.1f}% | {pctile(xs, 0.9):.1f}% | {max(xs):.1f}% |")
        w("")

    while out and out[-1] == "":
        out.pop()  # avoid a trailing blank line (markdownlint MD012)
    return "\n".join(out) + "\n"


# ── Charts (hand-rolled SVG line charts) ────────────────────────────────


def _axis_bounds(lo: float, hi: float) -> tuple[float, float, float]:
    """Return (axis_min, axis_max, step) spanning [lo, hi], rounded to 10s and
    always including the 100% baseline."""
    lo = min(lo, 100.0)
    hi = max(hi, 100.0)
    axis_min = math.floor(lo / 10) * 10
    axis_max = math.ceil(hi / 10) * 10
    if axis_max == axis_min:
        axis_max = axis_min + 10
    # Aim for ≤7 gridlines at a 5/10/20/... step.
    span = axis_max - axis_min
    candidates = (5, 10, 20, 25, 50, 100, 200, 250, 500, 1000)
    step = candidates[-1]
    for candidate in candidates:
        if span / candidate <= 7:
            step = candidate
            break
    return float(axis_min), float(axis_max), float(step)


def render_chart(op: str, op_disp: str, versions: list[str],
                 matrix: dict[tuple[str, str], dict[str, float]]) -> str | None:
    # Each message is normalised to its own first tracked release (= 100%), so a
    # 10% regression looks the same whether the message runs at 100 MiB/s or
    # 250,000 MiB/s. Absolute throughput lives in REPORT.md's tables; the chart's
    # job is to make per-release trends and regressions legible across messages
    # whose absolute throughput spans three orders of magnitude.
    series = []
    for msg, disp, color in MESSAGES:
        by_ver = matrix.get((msg, op))
        if not by_ver:
            continue
        pts = [(i, by_ver[v]) for i, v in enumerate(versions) if v in by_ver]
        if pts:
            base = pts[0][1]
            if base <= 0:
                continue
            norm = [(i, v / base * 100.0) for i, v in pts]
            series.append((disp, color, norm))
    if not series:
        return None

    plot_w, plot_h = 560, 320
    left, top = 56, 44
    right_legend = 160
    svg_w = left + plot_w + right_legend
    svg_h = top + plot_h + 56

    all_vals = [v for _, _, pts in series for _, v in pts]
    axis_min, axis_max, step = _axis_bounds(min(all_vals), max(all_vals))
    n = len(versions)
    x_step = plot_w / max(n - 1, 1)

    def px(i: int) -> float:
        return left + i * x_step

    def py(v: float) -> float:
        return top + plot_h - (v - axis_min) / (axis_max - axis_min) * plot_h

    L: list[str] = []
    a = L.append
    a(f'<svg xmlns="http://www.w3.org/2000/svg" width="{svg_w}" height="{svg_h}"'
      f' viewBox="0 0 {svg_w} {svg_h}">')
    a('  <style>')
    a('    text { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI",'
      ' Helvetica, Arial, sans-serif; }')
    a('    .title { font-size: 15px; font-weight: 600; fill: #24292f; }')
    a('    .axis-label { font-size: 11px; fill: #57606a; }')
    a('    .legend-text { font-size: 12px; fill: #24292f; }')
    a('    .grid { stroke: #d0d7de; stroke-width: 0.5; }')
    a('    .baseline { stroke: #8c959f; stroke-width: 1; stroke-dasharray: 4 3; }')
    a('    .noise-band { fill: #b1bac4; opacity: 0.18; }')
    a('    .footnote { font-size: 11px; fill: #57606a; }')
    a('  </style>')
    a('  <rect width="100%" height="100%" fill="white"/>')
    # Shaded ±NOISE_BAND_PCT band around the 100% baseline (the reproducibility
    # floor), drawn behind the gridlines and series so sub-floor wiggles read as
    # noise. Clamped to the plot area in case the band exceeds the axis range.
    band_top = max(top, py(100.0 + NOISE_BAND_PCT))
    band_bot = min(top + plot_h, py(100.0 - NOISE_BAND_PCT))
    if band_bot > band_top:
        a(f'  <rect x="{left}" y="{band_top:.1f}" width="{plot_w}"'
          f' height="{band_bot - band_top:.1f}" class="noise-band"/>')
    a(f'  <text x="{left + plot_w / 2}" y="26" text-anchor="middle" class="title">'
      f'{escape(op_disp)} — throughput vs each message’s first release (%)</text>')

    n_ticks = round((axis_max - axis_min) / step)
    for k in range(n_ticks + 1):
        val = axis_min + k * step
        y = py(val)
        a(f'  <line x1="{left}" y1="{y:.1f}" x2="{left + plot_w}" y2="{y:.1f}" class="grid"/>')
        a(f'  <text x="{left - 8}" y="{y + 4:.1f}" text-anchor="end" class="axis-label">'
          f'{val:.0f}%</text>')
    # The 100% reference is drawn unconditionally — with an odd axis_min and a
    # large step it may fall between gridlines, and it must always be visible
    # because every series is indexed to it. Label it only when it isn't already
    # a gridline tick, to avoid overlapping that tick's label.
    yb = py(100.0)
    a(f'  <line x1="{left}" y1="{yb:.1f}" x2="{left + plot_w}" y2="{yb:.1f}" class="baseline"/>')
    if (100.0 - axis_min) % step != 0:
        a(f'  <text x="{left - 8}" y="{yb + 4:.1f}" text-anchor="end" class="axis-label">100%</text>')

    for i, v in enumerate(versions):
        x = px(i)
        a(f'  <text x="{x:.1f}" y="{top + plot_h + 18}" text-anchor="middle"'
          f' class="axis-label">{escape(v)}</text>')

    for disp, color, pts in series:
        d = " ".join(f"{'M' if k == 0 else 'L'}{px(i):.1f},{py(v):.1f}"
                     for k, (i, v) in enumerate(pts))
        a(f'  <path d="{d}" fill="none" stroke="{color}" stroke-width="2"/>')
        for i, v in pts:
            a(f'  <circle cx="{px(i):.1f}" cy="{py(v):.1f}" r="3" fill="{color}"/>')

    ly = top + 6
    lx = left + plot_w + 16
    for disp, color, _ in series:
        a(f'  <rect x="{lx}" y="{ly}" width="12" height="12" rx="2" fill="{color}"/>')
        a(f'  <text x="{lx + 18}" y="{ly + 11}" class="legend-text">{escape(disp)}</text>')
        ly += 22

    # Footnote explaining the shaded band, below the plot where the lines can't
    # obscure it (rather than a label sitting inside the band among the series).
    a(f'  <text x="{left}" y="{top + plot_h + 42:.1f}" class="footnote">'
      f'Shaded band: ±{NOISE_BAND_PCT:.0f}% run-to-run noise floor — a line staying'
      f' inside it moved by less than the noise floor.</text>')
    a('</svg>')
    return "\n".join(L) + "\n"


def main(argv: list[str]) -> int:
    history_dir = Path(argv[0]) if argv else Path(__file__).resolve().parent
    runs = load_runs(history_dir)
    matrix = throughput_matrix(runs)
    versions = [r["version"] for r in runs]

    (history_dir / "REPORT.md").write_text(render_report(runs, matrix), encoding="utf-8")
    charts_dir = history_dir / "charts"
    charts_dir.mkdir(exist_ok=True)
    n_charts = 0
    for op, op_disp in OPS:
        svg = render_chart(op, op_disp, versions, matrix)
        if svg is not None:
            (charts_dir / f"{op}.svg").write_text(svg, encoding="utf-8")
            n_charts += 1
    print(f"wrote REPORT.md and {n_charts} charts for {len(runs)} releases", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
