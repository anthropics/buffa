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

Two things are pinned so a cross-release delta reflects buffa's code, not the
measurement.

**The machine.** Runs are done on a quiesced host: CPU turbo disabled, the
`performance` frequency governor, and the benchmark pinned to one core. A shared
or virtualised machine cannot give trustworthy absolute throughput, so do not
regenerate these files on a laptop or a busy CI runner and commit the result —
the drift would masquerade as a regression.

**The build profile.** Every binary is built with **`lto=true,
codegen-units=1`** — the same optimized profile a consumer building buffa in
release gets, and the one that is reproducible across releases. (At cargo's
default `bench` profile, `codegen-units=16, lto=off`, the binary's *layout* is
unstable: adding unrelated code re-partitions functions across the 16 units and a
benchmark can swing 10-20% with no code change — see the layout-noise envelope
below. A single codegen unit removes that partitioning, and LTO matches the
shipped profile.) Because `benchmarks/buffa` is excluded from the root workspace,
the root's profile does not reach it, so the profile is applied via
`CARGO_PROFILE_BENCH_LTO=true CARGO_PROFILE_BENCH_CODEGEN_UNITS=1` at build time;
each run file records it in `build_profile`.

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
- **Build-layout noise is controlled by the profile, not eliminated.** Building at
  `codegen-units=1` removes the codegen-unit-partitioning instability that
  dominates the default `bench` profile (measured there at p50 5.8% / p90 15% /
  max 24% across builds — large enough to invent a regression, which is exactly
  what happened to the first v0.7.1 data set; see `annotations.md`). A single unit
  has nothing to re-partition, so the series is far more reproducible. The
  layout-noise harness below still exists to *verify* the floor on a quiesced box;
  a surprising delta should clear the measured envelope before being attributed to
  the library.
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
- `build-cgu-variants.sh` — builds the bench binary at several `codegen-units`
  settings for the layout-noise harness.
- `layout_envelope.py` — computes the per-benchmark layout-noise envelope from
  labelled criterion captures of those variants (`test_layout_envelope.py`
  covers it; run `python3 -m unittest` from this directory).

## Layout-noise envelope

To measure how much a benchmark moves under pure build perturbation (so a
cross-release delta can be told apart from a code change), build the *same*
source at several `codegen-units` settings — each is a distinct, deterministic
layout — and compare. The pinned stable toolchain has no `-Z randomize-layout`,
so a `codegen-units` sweep is the layout-perturbation proxy; it also tells you
which setting is most stable for the series (lower units → less partition
churn; `codegen-units=1` is the most reproducible cross-release).

```bash
# 1. Build the variants (default sweep: codegen-units 1 2 4 8 16).
task bench-layout-variants -- /tmp/cgu        # or CGUS="1 16" task bench-layout-variants -- /tmp/cgu

# 2. Run them on a quiesced box, capturing each binary's stdout. On metal:
bench-on-metal.sh --spot \
  --binary /tmp/cgu/cgu1.bench --binary /tmp/cgu/cgu16.bench \
  --args "--measurement-time 4"
# (save each binary's output as cgu1.txt, cgu16.txt, …)

# 3. Compute the envelope.
task bench-layout-envelope -- --run cgu1=cgu1.txt --run cgu16=cgu16.txt
```

The report ranks benchmarks by their range across layouts and prints the suite
p50 / p90 / max. Read a release-over-release delta against the max (or p90)
envelope: at or below it, the movement is layout noise.

## Regenerating the report

After editing or adding any `runs/*.json`, regenerate the rendered output:

```bash
python3 benchmarks/history/generate.py     # or: task bench-history-report
```

## Adding a new release

1. Build the release tag's bench binary at the pinned profile: from a checkout of
   the tag,
   `cd benchmarks/buffa && CARGO_PROFILE_BENCH_LTO=true CARGO_PROFILE_BENCH_CODEGEN_UNITS=1 cargo bench --bench protobuf --no-run`.
2. Run it on a quiesced machine, capturing stdout — criterion needs the `--bench`
   flag: `<binary> --bench --measurement-time 4 > <version>.txt`.
3. Parse it into a run file (record the profile so the data is self-documenting):

   ```bash
   python3 benchmarks/history/parse_criterion.py \
     --version <version> --stdout <version>.txt \
     --commit $(git rev-parse <version>) \
     --commit-date "$(git log -1 --format=%cI <version>)" \
     --measured-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
     --profile "lto=true, codegen-units=1" \
     --out benchmarks/history/runs/<version>.json
   ```

4. Regenerate the report (above) and add an `annotations.md` entry explaining any
   notable movement.
