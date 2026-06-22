# Release annotations

Why the numbers in [REPORT.md](REPORT.md) move. The data is a **dense,
per-message-isolated, layout-normalized matrix**: every message shape is measured
against every release (v0.1.0–v0.7.1), each built with only its own decoder
compiled, at the pinned toolchain (1.96.0), `lto=true, codegen-units=1`, and
**64-byte block alignment** (`-Cllvm-args=-align-all-nofallthru-blocks=6`), median
of 32 cores. See [DESIGN.md](DESIGN.md) for the system and [README.md](README.md)
for the mechanics. Each release's harness lives on its `historical-benchmark/vX.Y.Z`
branch, so any cell is rebuildable.

Two effects are controlled. **Per-message isolation** removes cross-message inliner
coupling, so one shape's benchmark can no longer perturb another's. **Block
alignment** removes the build-time code-layout lottery that otherwise dominated the
layout-sensitive operations — see "Layout normalization" below for why it was
needed and what it costs. With both controlled, the cross-release curves are
attributable to buffa's own per-shape encode/decode code, and the whole matrix is
readable at one trust threshold rather than a different one per operation.

The charts shade a **±5% band** around each message's baseline as the typical
run-to-run floor. After normalization the cross-release spread is ~4% overall, so a
line that stays inside the band is noise; movements that clear it are discussed
below. The per-operation "Measurement spread" table in [REPORT.md](REPORT.md) and
the per-benchmark spread in `runs/*.json` remain the place to check how far a given
number can be trusted.

## Headline cross-release findings (v0.1.0 → v0.7.1)

A movement counts as real if it is large *for its operation* and **persists** across
releases. Two findings stand, both now sitting on clean, flat baselines:

- **AnalyticsEvent `encode` −12% / `compute_size` −9%** — a real regression. A step
  down at v0.4.0 (encode 468→414, compute_size 1379→1262 MiB/s) that holds flat
  through v0.7.1. `compute_size` is the tightest operation and corroborates the
  `encode` figure, so the deeply nested, repeated-submessage shape genuinely lost
  ground on the owned encode/size paths — the one result worth investigating.
- **PackedTile `decode_view` +47% at v0.7.1** — flat (~175 MiB/s) from v0.1.0
  through v0.7.0, then a single-release jump to ~257 at v0.7.1, consistent with the
  packed-varint reserve work in that release. A 47% step is well clear of noise; but
  it is the latest release, so "persists" isn't confirmable yet.

Everything else is flat across the eight releases — including all of `json_encode` /
`json_decode`, which now hold steady at their fast value (LogRecord `json_encode`
~880 MiB/s at every release, vs a 19% flap before normalization). buffa's core
paths did not regress; the reassuring headline is that eight releases of `decode`,
`merge`, and the JSON paths hold steady once layout is controlled.

## Layout normalization — why, and what it costs

Before normalization, the **placement** of otherwise-identical code was the limiting
factor on resolution. The clearest case was `json_encode` for the string/scalar-heavy
shapes, which *flapped*: LogRecord ran ~880 MiB/s at some releases and ~660 at
others, in lockstep across shapes, which no real code change would do (cross-version
spread 19%).

Disassembly settled that it was layout, not code. Comparing a fast and a slow
isolated LogRecord binary, **2390 of 2393 functions are byte-identical** after
normalizing addresses; the three that differ (`__rust_alloc_zeroed`, a `raw_vec`
error path, `main`) are not in the encode path. `serialize_str` and
`format_escaped_str` — the string-escaping hot loop that dominates this shape — are
identical instruction-for-instruction, just located at different addresses. Two
experiments confirmed it is the build, not the measurement: re-measuring the same
binary one-at-a-time reproduced the gap exactly, and rebuilding the identical commit
in a different directory flipped which version was slow.

`perf stat` then pinned the *mechanism* (and corrected an earlier guess). On a slow
vs a 64-byte-aligned build of the same code (IPC 2.87 vs 3.55), the Topdown
breakdown puts the slow layout's stall in Fetch **Bandwidth** — the **µop cache
(DSB)** — at 21.7% of slots vs 11.5%, with ~2× the DSB→legacy-decoder (MITE) switch
penalty (`dsb2mite_switches.penalty_cycles`), while Fetch *Latency* and the i-cache
/ cache-line miss counters barely move. The serde serialize path is a dense tree of
many small functions (string escaping, int/float formatting), so its hot loop is
unusually sensitive to how the µop cache packs it: a placement shift tips it out of
the DSB into the slower legacy decoder. (It is *not* a cache-line effect, as an
earlier reading had it — the counters say front-end bandwidth.)

**The fix.** Building every release with 64-byte block alignment restores clean DSB
delivery and lands each build on the fast layout. Re-measuring the whole matrix this
way tightened the cross-release spread on nearly every operation and collapsed the
JSON flap:

| operation | spread, as-shipped | spread, normalized |
|-----------|------:|------:|
| json_encode | 19.2% | 2.5% |
| decode_view | 12.7% | 8.8% |
| merge | 7.2% | 5.7% |
| decode | 5.7% | 4.0% |
| json_decode | 5.3% | 4.0% |
| compute_size | 2.8% | 2.5% |
| encode | 3.3% | 3.6% |
| **all** | **5.9%** | **4.2%** |

(cross-version (max−min)/median, median across messages.) The as-shipped column is
the prior median-of-15 campaign and the normalized column median-of-32, so the
smaller per-operation tightenings partly reflect the larger sample; the
`json_encode` collapse, however, is unambiguously the alignment — its flap was
deterministic layout, not sampling, as the disassembly showed. Only `encode` is
marginally worse, within noise. Crucially the real signals survived: the
AnalyticsEvent v0.4.0 regression and the PackedTile v0.7.1 jump are unchanged —
normalization removed the layout flaps without touching the code-driven steps. That
is the test that justified adopting it: **it flattens noise, not signal.** A BOLT
pass converges to the same fast layout (even from a no-LBR profile), independently
confirming the target is real and not an artifact of the alignment flag.

**The cost — read the curves as best-achievable layout.** Block alignment measures
the layout a profile-guided optimizer, or a lucky build, would reach — not the one a
plain `cargo build` ships. For a tracker whose job is "did buffa's *code* get
faster," that is the right frame: it isolates code from placement luck. For "what
will my service see from a default release build," the as-shipped number is lower and
noisier on the JSON path. The history answers the first question.

## Why this replaced the earlier (sparse, coupled) history

An earlier version of this history built all shapes into one benchmark binary.
That made the per-shape numbers depend on which *other* shapes were present:
adding a message re-partitioned the compiler's inlining for the unchanged
decoders. It produced a convincing but false v0.7.1 regression — `MediaFrame`
`decode_view` read −13% purely because v0.7.1 added the `PackedTile` benchmark
message (proven by disassembly: removing PackedTile made MediaFrame's machine
code byte-identical to v0.7.0). Under per-message isolation that artifact is gone:
isolated `media_frame/decode_view` is flat across the whole series (≈44–48k MiB/s,
within spread). The dense isolated matrix exists so no cell can be contaminated
that way again, and so every shape has a full-history curve rather than starting
only at the release that added it to the suite.

## Caveats

These are medians of 32 cores with per-benchmark spread recorded. Reproduction
across runs rules out random run noise but not deterministic per-binary effects;
with layout now normalized and isolation removing coupling, the remaining
real-versus-artifact test is persistence across *releases*. The matrix covers the
seven portable operations (decode, merge, encode, compute_size, decode_view,
json_encode, json_decode) — the bespoke `encode_view`/`build_encode` benchmarks use
newer view-encode APIs that did not exist in older releases, so they are not part of
the dense matrix and remain only on the releases that natively support them.
