# Benchmark history

This directory tracks the performance of buffa's own benchmarks across releases,
so a regression or improvement is visible and attributable to a specific version.
It complements `benchmarks/charts/`, which compares buffa against other libraries
at a single point in time; this directory compares buffa against *its own past*.

## What is measured

For every release we build that tag's own `protobuf` benchmark source and run it
under **one fixed toolchain and build profile, held constant across the whole
series**. The numbers therefore isolate buffa's own code changes from compiler and
build-config changes — this is a controlled re-measurement of each release's code,
not "whatever that tag happened to ship with." The headline metric is **throughput
in MiB/s** (higher is better), because it stays comparable across releases even
when a tag changed the size of its benchmark dataset. Median nanoseconds per
iteration are stored alongside, and each number is the **median across several
cores** with its spread recorded (see below).

The matrix is **dense**: every message shape is measured against every release
(v0.1.0–v0.7.1), not just from the release that first added it to the suite. A
shape is a property of the protobuf schema, not of any buffa version — buffa
v0.1.0 could always decode a `MediaFrame`, we just never asked it to — so the
canonical shapes and datasets are fed to each release's own codegen and every
shape gets a full-history curve. Each release's per-message-isolated harness lives
on its `historical-benchmark/vX.Y.Z` branch.

## How the numbers are produced

Two things are pinned so a cross-release delta reflects buffa's code, not the
measurement.

**The machine.** Runs are done on a quiesced host: CPU turbo disabled, the
`performance` frequency governor, and each benchmark instance pinned to its own
physical core (SMT siblings avoided). Several copies of a release run at once on
distinct cores and the per-benchmark number is the **median across them**, which
is robust to the occasional noisy core; the spread is recorded per benchmark.
Concurrency was validated to track the isolated single-core baseline within ~1%
on this box (the working sets fit private L2). A shared or virtualised machine
cannot give trustworthy absolute throughput, so do not regenerate these files on
a laptop or a busy CI runner and commit the result — the drift would masquerade
as a regression.

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

**Layout normalization.** On top of the profile, every binary is built with
**64-byte block alignment** (`RUSTFLAGS="-Cllvm-args=-align-all-nofallthru-blocks=6"`).
Without it, final function and loop placement is a lottery: rebuilding the identical
source in a different directory can move a hot loop ~20% with byte-identical machine
code, worst on the serde JSON path (a µop-cache / DSB effect, measured with `perf
stat` — see `annotations.md`). Aligning hot block heads to the cache line lands every
build on the fast layout, which collapsed the cross-release spread (`json_encode` 19%
→ 2.5%, overall 5.9% → 4.2%) while leaving the real code-driven steps untouched. The
trade-off is that the curves show a *best-achievable* layout — the one a
profile-guided build (or BOLT) would reach — not what a plain `cargo build` ships;
that is the right frame for "did buffa's code get faster," and the wrong one for
"what will my service see." See the caveat below.

## Comparability caveats

- **The harness and datasets evolved with the library.** Toolchain, profile, and
  method are held constant, but the benchmark loop body and a tag's dataset can
  still differ between releases. Throughput normalises for dataset size, but a
  change in the benchmark loop body between two
  releases can move a number without the library itself changing. When a delta
  looks surprising, check whether that benchmark's source changed at that tag
  before attributing it to the library.
- **There is a reproducibility floor of roughly ±5%** even on a quiesced machine,
  from residual scheduler and thermal effects. With 32 self-concurrent cores per
  release the measured core-to-core spread across all 336 benchmarks is p50 ~3.6% /
  p90 ~9.4% (a few points higher than at lower concurrency, but the per-release
  *median* stays robust); this floor is systematic, not sampling noise. The charts
  shade a ±5% band around each message's baseline against the per-release medians,
  whose cross-release spread is ~4% after layout normalization: treat movement that
  stays inside the band as noise unless a later release confirms the trend.
- **Build-layout noise is controlled by the profile, not eliminated.** Building at
  `codegen-units=1` removes the codegen-unit-partitioning instability that
  dominates the default `bench` profile (measured there at p50 5.8% / p90 15% /
  max 24% across builds — large enough to invent a regression, which is exactly
  what happened to the first v0.7.1 data set; see `annotations.md`). A single unit
  has nothing to re-partition, so the series is far more reproducible. The
  layout-noise harness below still exists to *verify* the floor on a quiesced box;
  a surprising delta should clear the measured envelope before being attributed to
  the library.
- **Layout is normalized, so the curves are best-achievable, not as-shipped.**
  `codegen-units=1` removes the *partitioning* instability above, but final function
  placement still shifts with trivial inputs — a one-line source change between
  releases, or even rebuilding the identical commit in a different directory, can move
  a hot loop ~20% with byte-identical machine code (proven by disassembly; the serde
  JSON path is worst, a µop-cache / DSB effect — see `annotations.md`). Rather than
  live with that, every binary is built with 64-byte block alignment (above), which
  lands each build on the fast layout: it collapsed the cross-release spread
  (`json_encode` 19% → 2.5%, overall 5.9% → 4.2%) while leaving the real code-driven
  steps (the AnalyticsEvent v0.4.0 regression, the PackedTile v0.7.1 jump) untouched,
  so `json_encode` / `json_decode` are trustworthy again and charted like every other
  operation. The trade-off is the frame: these are the layout a profile-guided build
  would reach, not the lower, noisier number a default `cargo build` ships.
- **Cross-message inliner coupling — resolved by per-message isolation.** If all
  message decoders share one binary, rustc's inlining is a global decision and
  adding a message reshuffles inlining for the *unchanged* decoders (worst at
  `codegen-units=1`, since there is one unit). That produced a false v0.7.1
  regression in an earlier sparse history — `MediaFrameView::decode_view` read
  −11.6% purely because v0.7.1 added the `PackedTile` benchmark message, proven by
  disassembly (remove `PackedTile`, the machine code is byte-identical to v0.7.0).
  The current matrix removes this at the source: **each shape is built with only
  its own decoder compiled** (its own feature/proto, `--bench <shape>`), so no
  other message can perturb it. Isolated, `media_frame/decode_view` is flat across
  the whole series. This is also why every shape can span the full history.
- **The compiler is held constant.** Every binary is built with one explicitly
  pinned toolchain (recorded in each run file's `toolchain`), forced via
  `RUSTUP_TOOLCHAIN` so it does not depend on the working directory's
  `rust-toolchain.toml`. That removes the compiler as a variable — a movement
  reflects buffa's code, not a rustc change. The pin is the **latest stable at
  the time of the run** (currently `1.96.0`), chosen for longevity rather than
  the minimum: it only has to be ≥ the highest MSRV across the tracked releases
  (1.87 today), and pinning to latest stable keeps the whole series buildable
  until stable advances past a future release's MSRV — roughly a year out under a
  stable-minus-12-months MSRV policy. Re-pin and **regenerate the entire series**
  (not just the new release) when that happens, so every row shares one compiler.

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

# 2. Run each variant on a quiesced machine, capturing its stdout — criterion
#    needs the --bench flag:
for v in /tmp/cgu/cgu*.bench; do
  "$v" --bench --measurement-time 4 > "$(basename "$v" .bench).txt"
done
# (yields cgu1.txt, cgu16.txt, …)

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

All releases share one toolchain and profile, so adding a release means matching them, not picking new ones. If the new release's MSRV exceeds the pinned toolchain, re-pin to a newer stable and regenerate the *whole* series instead.

1. **Create the reproducible root branch.** From the release tag, branch and push `historical-benchmark/vX.Y.Z` (the convention recorded in `CONTRIBUTING.md`). Releases cut from `main` already carry the per-message-isolated harness; the back-catalogue (v0.1.0–v0.7.1) had it retrofitted onto these branches. This branch is what makes any cell rebuildable later.

2. **Build each shape in isolation** from that branch, at the pinned toolchain and profile — only the target shape's decoder is compiled, so no other shape can perturb it via the compiler's inlining:

   ```bash
   cd benchmarks/buffa
   for m in api_response log_record analytics_event google_message1 media_frame packed_tile; do
     RUSTUP_TOOLCHAIN=1.96.0 CARGO_PROFILE_BENCH_LTO=true CARGO_PROFILE_BENCH_CODEGEN_UNITS=1 \
       RUSTFLAGS="-Cllvm-args=-align-all-nofallthru-blocks=6" \
       cargo bench --no-default-features --features "iso,$m" --bench "$m" --no-run
   done
   ```

   The `RUSTFLAGS` block-alignment flag is required, not optional — it is what
   normalizes the layout (above). Omitting it reintroduces the per-build lottery and
   the numbers will not be comparable to the rest of the series.

   (`task bench-iso -- <message>` is the convenience wrapper for one shape.)

3. **Run each isolated binary** on a quiesced machine, capturing stdout per shape — criterion needs the `--bench` flag. For a stable median, run each binary on several pinned physical cores and capture each: `<binary> --bench --measurement-time 4 > <version>.<msg>.<core>.txt`.

4. **Parse all the captures into one run file.** The parser takes the median across every capture that carries a given benchmark id and records the spread, so pass each capture with a repeated `--stdout` flag:

   ```bash
   stdout_args=$(printf -- '--stdout %s ' <version>.*.txt)
   python3 benchmarks/history/parse_criterion.py \
     --version <version> $stdout_args \
     --commit $(git rev-parse <version>) \
     --commit-date "$(git log -1 --format=%cI <version>)" \
     --measured-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
     --toolchain 1.96.0 \
     --profile "lto=true, codegen-units=1, per-message-isolated, 64-byte block-aligned (-align-all-nofallthru-blocks=6)" \
     --out benchmarks/history/runs/<version>.json
   ```

5. Regenerate the report (above) and extend `annotations.md` for any notable movement.
