# Benchmark history

This directory tracks the performance of buffa's own benchmarks across releases,
so a regression or improvement is visible and attributable to a specific version.
It complements `benchmarks/charts/`, which compares buffa against other libraries
at a single point in time; this directory compares buffa against *its own past*.

## What is measured

For every release we build that tag's own `protobuf` benchmark binary and run it,
then record one data point per benchmark. The numbers are therefore what each
release actually delivered — not a re-measurement of old code under a modern
harness. The headline metric is **throughput in MiB/s** (higher is better),
because it stays comparable across releases even when a tag changed the size of
its benchmark dataset. Median nanoseconds per iteration are stored alongside.

The benchmark set grows over time: the four core message types (`ApiResponse`,
`LogRecord`, `AnalyticsEvent`, `GoogleMessage1`) are present from v0.1.0,
`MediaFrame` joins at v0.4.0, and `PackedTile` later still. Each benchmark's
series simply starts at the release that introduced it.

## How the numbers are produced

To keep absolute numbers stable, runs are done on a quiesced machine: a dedicated
host with CPU turbo disabled, the `performance` frequency governor, and the
benchmark pinned to one core. A shared or virtualised machine cannot give
trustworthy absolute throughput, so do not regenerate these files on a laptop or
a busy CI runner and commit the result — the drift would masquerade as a
regression.

## Comparability caveats

- **This is "as each release measured itself," not a controlled experiment.** The
  benchmark harness and datasets evolved alongside the library. Throughput
  normalises for dataset size, but a change in the benchmark loop body between two
  releases can move a number without the library itself changing. When a delta
  looks surprising, check whether that benchmark's source changed at that tag
  before attributing it to the library.
- **There is a reproducibility floor of roughly ±5%** even on a quiesced machine,
  from residual scheduler and thermal effects. Treat sub-5% movements as noise
  unless a later release confirms the trend.
- **The compiler is held constant.** No release tag pins a Rust toolchain, so
  every binary in the current series was built with the same compiler (recorded
  as `"toolchain": "default"` in each run file). That removes the compiler as a
  variable — a movement reflects buffa's own code, not a rustc change. If a
  future release pins a toolchain, record it and watch for compiler-driven shifts.

## Files

- `runs/<version>.json` — one file per release: the version, its commit and date,
  when it was measured, the machine and tuning, the toolchain, and per-benchmark
  `median_ns` + `throughput_mib_s`. These are the source of truth, hand-auditable
  and diffable.
- `REPORT.md` — generated tables of throughput per release (with the delta against
  the previous release) plus the biggest movers across the tracked range.
- `charts/<op>.svg` — generated throughput-over-releases line charts, one per
  operation, with a line per message type.
- `annotations.md` — per-release notes on what changed and why a number moved,
  cross-referenced with the [CHANGELOG](../../CHANGELOG.md). This is the
  hand-written half: the data says *what* moved, the annotations say *why*.
- `parse_criterion.py` — turns a release's captured criterion output into one
  `runs/<version>.json`.
- `generate.py` — renders `REPORT.md` and `charts/` from `runs/`.

## Regenerating the report

After editing or adding any `runs/*.json`, regenerate the rendered output:

```bash
python3 benchmarks/history/generate.py     # or: task bench-history-report
```

## Adding a new release

1. Build the release tag's bench binary: from a checkout of the tag,
   `cd benchmarks/buffa && cargo bench --bench protobuf --no-run`.
2. Run it on a quiesced machine, capturing stdout — criterion needs the `--bench`
   flag: `<binary> --bench --measurement-time 4 > <version>.txt`.
3. Parse it into a run file:
   ```bash
   python3 benchmarks/history/parse_criterion.py \
     --version <version> --stdout <version>.txt \
     --commit $(git rev-parse <version>) \
     --commit-date "$(git log -1 --format=%cI <version>)" \
     --measured-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
     --out benchmarks/history/runs/<version>.json
   ```
4. Regenerate the report (above) and add an `annotations.md` entry explaining any
   notable movement.
