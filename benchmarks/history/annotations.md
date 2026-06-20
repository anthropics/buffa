# Release annotations

Why the numbers in [REPORT.md](REPORT.md) move. The data is now a **dense,
per-message-isolated matrix**: every message shape is measured against every
release (v0.1.0–v0.7.1), each built with only its own decoder compiled, at the
pinned toolchain (1.96.0) and `lto=true, codegen-units=1`, median of fifteen cores.
See [DESIGN.md](DESIGN.md) for the system and [README.md](README.md) for the
mechanics. Each release's harness lives on its `historical-benchmark/vX.Y.Z`
branch, so any cell is rebuildable.

Because each shape is isolated, the cross-release curves below are attributable to
buffa's own per-shape encode/decode code, not to which other messages happened to
share the benchmark binary. The charts shade a **±5% band** around each message's
baseline: that is the measured run-to-run noise floor on this hardware (median
core-to-core spread 2.6%, p90 6.6% across all 336 benchmarks), so a line that
stays inside the band never moved beyond noise. Movements that clear it are
discussed below. Spread is uneven across operations — `compute_size` is the
tightest (p90 2.7%), the binary-`encode` and JSON paths the noisiest (p90 ~9–10%)
— so the per-operation summary in [REPORT.md](REPORT.md)'s "Measurement spread"
table, and the per-benchmark spread in `runs/*.json`, are where to check how far
a given number can be trusted, rather than reading it off the chart.

## Headline cross-release findings (v0.1.0 → v0.7.1)

Read these as **directional**. As "Code layout is the dominant noise source"
below shows, clearing the ±5% band is necessary but *not sufficient* for a
movement to be real — layout artifacts can exceed 24%. The reliable test is
persistence: a change that steps to a new level and *holds* across later releases
is real; one that alternates and reverts is layout. Apply that test before
attributing any single number to buffa's code.

Improvements:

- **PackedTile `decode_view` +43%** and **MediaFrame `json_encode`'s ~27% step at
  v0.6.0** are the clearest gains — both are large and persist across the releases
  after them. (MediaFrame's full-range "+40%" mixes that real step with layout
  noise.)
- ApiResponse `decode_view` +10%, AnalyticsEvent `merge` +10%, LogRecord
  `decode_view` +6% — smaller eager-view / merge gains; directional only.
- Note: JSON encode did **not** improve broadly. The string/scalar-heavy shapes
  (LogRecord, ApiResponse) only *flap* on `json_encode` — that is code layout, not
  a gain (see below).

Regressions:

- **AnalyticsEvent `encode` −14%** and **`compute_size` −10%** — the deeply
  nested, repeated-submessage shape lost ground on the owned encode and size
  paths. This is the clearest real regression in the set and the one worth
  investigating.
- GoogleMessage1 `merge` −9%, AnalyticsEvent `json_encode` −7%, ApiResponse
  `compute_size` −6%.

Everything else is flat within the band: buffa's core binary `decode`/`encode`/
`merge` for the flat and string-heavy shapes has held steady across eight
releases, which is the reassuring headline.

## Code layout is the dominant noise source

Per-message isolation removed cross-message coupling, but a subtler effect is now
the limiting factor on resolution: the **placement** of otherwise-identical code.
The clearest case is `json_encode` for the string/scalar-heavy shapes, which
*flaps* — LogRecord and ApiResponse both run ~24% faster at v0.5.0 and v0.7.0 and
slower at the releases between, in lockstep, which no real code change would do.

Disassembling the fast (v0.5.0) and slow (v0.6.0) isolated LogRecord binaries
settled it: **2390 of 2393 functions are byte-identical** after normalising
addresses. The only three that differ are `__rust_alloc_zeroed`, a `raw_vec`
error path, and `main` — none in the encode path. `serialize_str` and
`format_escaped_str`, the string-escaping hot loop that dominates this shape, are
identical instruction-for-instruction, just located at different addresses. A
trivial source change between releases shifts a few glue functions' sizes, which
shifts every later function's address, so the *same* hot loop lands at a different
alignment / cache-line packing and runs up to 24% faster or slower **with no code
change**. It reproduces across campaigns because, for a given binary, the layout
is fixed — deterministic, but not meaningful.

What this means for reading the history:

- **Clearing the ±5% band is not sufficient.** Layout artifacts routinely exceed
  it (this one was 24%); the band catches typical noise, not this.
- **Trust persistent steps, not flaps.** A change that steps and holds is real
  (MediaFrame `json_encode` from v0.6.0); a value that alternates and reverts is
  layout (LogRecord/ApiResponse `json_encode`).
- **Weight by operation.** `compute_size`, `decode`, and `decode_view` are
  layout-stable; the `encode` and JSON paths have tight, alignment-sensitive hot
  loops and are the least trustworthy. The "Measurement spread" table in
  [REPORT.md](REPORT.md) ranks them.

Eliminating this would mean averaging over many build layouts per cell (BOLT-style)
or pinning alignment with compiler flags — neither is in place. So treat this
history as a reliable detector of **large, persistent** shifts and a poor
instrument for sub-~10% changes on the layout-sensitive operations.

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

These are medians of fifteen cores with per-benchmark spread recorded.
Reproduction across the two metal campaigns (median-of-four and median-of-fifteen)
rules out random run noise — but **not** layout artifacts, which are deterministic
per binary and reproduce just as cleanly (the `json_encode` flap above did). The
real-versus-layout test is persistence across *releases*, not reproduction across
*runs*. The matrix covers the seven portable operations (decode, merge, encode,
compute_size, decode_view, json_encode, json_decode) — the bespoke
`encode_view`/`build_encode` benchmarks use newer, view-encode APIs that did not
exist in older releases, so they are not part of the dense matrix and remain only
on the releases that natively support them.
