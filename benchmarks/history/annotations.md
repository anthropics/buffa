# Release annotations

Why the numbers in [REPORT.md](REPORT.md) move. The data is now a **dense,
per-message-isolated matrix**: every message shape is measured against every
release (v0.1.0–v0.7.1), each built with only its own decoder compiled, at the
pinned toolchain (1.96.0) and `lto=true, codegen-units=1`, median of four cores.
See [DESIGN.md](DESIGN.md) for the system and [README.md](README.md) for the
mechanics. Each release's harness lives on its `historical-benchmark/vX.Y.Z`
branch, so any cell is rebuildable.

Because each shape is isolated, the cross-release curves below are attributable to
buffa's own per-shape encode/decode code, not to which other messages happened to
share the benchmark binary. Movements within the per-benchmark spread (recorded in
each `runs/*.json`, typically 1–6%) are noise.

## Headline cross-release findings (v0.1.0 → v0.7.1)

Improvements that clear the spread:
- **PackedTile `decode_view` +42%** and **MediaFrame `json_encode` +40%** — the
  largest gains. JSON encoding improved broadly across shapes over the series, and
  the packed-tile view-decode path got substantially faster.
- ApiResponse `decode_view` +10%, LogRecord `decode_view` +5% — eager-view decode
  improved for the flat/string-heavy shapes.

Regressions that clear the spread:
- **AnalyticsEvent `encode` −15%** and **`compute_size` −11%** — the deeply
  nested, repeated-submessage shape lost ground on the owned encode and size
  paths. This is the clearest real regression in the set and the one worth
  investigating.
- GoogleMessage1 `merge` −8%, AnalyticsEvent `json_encode` −8%, ApiResponse
  `compute_size` −6%.

Everything else is flat within spread: buffa's core binary `decode`/`encode`/
`merge` for the flat and string-heavy shapes has held steady across eight
releases, which is the reassuring headline.

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

These are single-run medians-of-four with spread recorded; a delta below the
per-benchmark spread is noise, and the ±5% bare-metal reproducibility floor still
applies. The matrix covers the seven portable operations (decode, merge, encode,
compute_size, decode_view, json_encode, json_decode) — the bespoke
`encode_view`/`build_encode` benchmarks use newer, view-encode APIs that did not
exist in older releases, so they are not part of the dense matrix and remain only
on the releases that natively support them.
