# Release annotations

Why the numbers in [REPORT.md](REPORT.md) moved. The data says *what* changed;
this file says *why*, cross-referenced with the [CHANGELOG](../../CHANGELOG.md).
Each entry lists the changes in that release most likely to affect benchmark
throughput, then records what was actually observed.

"Observed" reads REPORT.md's per-release deltas. Movements within the ±5%
reproducibility floor (see [README](README.md#comparability-caveats)) are treated
as noise unless they form a consistent pattern across many benchmarks.

## v0.7.1 — 2026-06-10

Net-flat versus v0.7.0 (median −0.2%), and — importantly — **none of the
per-message deltas across this boundary are real code changes.** The run files
show `media_frame/decode_view` −11.6%, `GoogleMessage1` decode/merge/decode_view
+10 to +14%, and `log_record/decode_view` ~−5%, all clearing their spread and
reproducing across runs. They are nonetheless artifacts of **cross-message
inliner coupling**, because v0.7.1 added a new benchmark message (`PackedTile`,
#174) and every message's decoder is compiled into one binary.

Proof (disassembly, clean profile, by the v0.7.1-regression investigation):
`MediaFrameView::decode_view` is 605 instructions in v0.7.0 and 666 in v0.7.1
(it stops inlining `decode_varint_slice` / `decode_varint_slow` / `borrow_str`) —
the −11.6%. Remove `PackedTile` from the v0.7.1 bench and it recompiles to 605
instructions, **byte-identical to v0.7.0** after symbol normalisation. MediaFrame's
generated view code and buffa's decode runtime are byte-identical between the tags,
so nothing in the decoder changed; adding `PackedTile` shifted rustc's global
inlining decisions for the *other* messages' unchanged decoders. The same
mechanism drives the GoogleMessage1 and log_record deltas.

`codegen-units=1` does not immunise this — it **maximises** it: one codegen unit
means all decoders share the inliner's global view, so adding any message
perturbs them all. (This is separate from, and on top of, the earlier finding
that the *default* `bench` profile — cgu=16, lto=off, single sample — added pure
layout noise: that's why a still-earlier measurement fingered
`GoogleMessage1/decode_view` at −17.5%, which is now +10%. Both effects are build
artifacts, not code.)

**Consequence for this history:** a release that adds or removes a benchmark
message moves the numbers of *unchanged* messages. The affected transitions are
v0.3.0→v0.4.0 (added `MediaFrame`) and v0.7.0→v0.7.1 (added `PackedTile`); their
per-message deltas are coupling artifacts, not code. A consumer who decodes only
`MediaFrame` would not link `PackedTile`, so they would see the v0.7.0 codegen.
The clean fix is per-message isolation — build each message's decoder in its own
binary — which is also more representative of real usage; see
[README](README.md#comparability-caveats). (An earlier packed-varint
over-reservation theory was also unsupported — the messages it would affect have
no packed varint fields — though it surfaced a separate, real allocation fix.)

## v0.7.0 — 2026-05-28

Likely perf-relevant changes:
- `reflect()` borrows the source instead of a bridge round-trip (reflection
  benchmarks are not in this set).
- Custom string/bytes types can take the raw payload and inline / take ownership
  zero-copy on the decode path.

Observed: essentially flat versus v0.6.0 (all core ops within ±5%). No
regression or improvement attributable to this release in this benchmark set.

## v0.6.0 — 2026-05-15

Likely perf-relevant changes:
- Map-field codegen emits ~40-50 inline lines per map field instead of a generic
  call path.
- Wire-type guard refactored across ~1,100 generated sites.
- Compile-time string literals remove a runtime allocation on some paths.

Observed: recovered the v0.5.0 encode regression — binary encode +12-13%
(ApiResponse, LogRecord, GoogleMessage1), view encode +11-16%, and view decode
improved (GoogleMessage1 +16%, ApiResponse +11%). The inlined map/wire-type
codegen is the most likely cause of the encode recovery.

## v0.5.0 — 2026-05-05

Likely perf-relevant changes:
- `unbox_oneof()` inlines non-recursive oneof variants, removing an allocation
  per construction.
- Zero-copy JSON serialization without `to_owned_message()`.
- `Any::clone()` becomes a refcount bump (not in this set).

Observed: two opposing effects. JSON encode jumped sharply (LogRecord +26%,
GoogleMessage1 +17%, MediaFrame +33%, AnalyticsEvent +10%) and GoogleMessage1
compute_size +8% — but the binary and view *encode* paths regressed (binary
encode ApiResponse −13%, LogRecord −9%, GoogleMessage1 −13%; view encode −6 to
−15%). v0.6.0 recovered the encode regression, so v0.5.0 looks like a JSON-encode
win that briefly cost the binary-encode path. Worth confirming which v0.5.0
change caused the binary-encode dip.

## v0.4.0 — 2026-04-27

Likely perf-relevant changes:
- `Bytes`-backed zero-copy decode: a field backed by a shared buffer is a
  refcount bump rather than an allocation + memcpy. Introduces the `MediaFrame`
  benchmark and the `*/encode_view`, `*/build_encode*` benchmarks.

Observed: GoogleMessage1 encode +9% (continuing v0.3.0's encode gains), but
AnalyticsEvent encode −10% and compute_size −8%, and ApiResponse compute_size
−6%. Mixed; the new view/build-encode benchmarks start their series here.

## v0.3.0 — 2026-04-01

Likely perf-relevant changes:
- The CHANGELOG [0.3.0] is dominated by features (extensions, text format, the
  `buffa-descriptor` crate) with no explicitly perf-targeted entry. The
  improvement below is therefore most plausibly a side effect of generated-code
  changes ("generated code emits `Self`", codegen restructuring) rather than a
  documented optimization. **Flagged to investigate** which codegen change moved
  it. All releases were built with the same toolchain (see below), so this is not
  a compiler effect.

Observed: the standout improvement release. GoogleMessage1 decode +16%, merge
+16%, encode +8%; ApiResponse decode +7%, encode +15%; LogRecord encode +12%.
The gains concentrate on GoogleMessage1 (a deeply nested message) and the encode
path generally, and they hold through later releases.

## v0.2.0 — 2026-03-16

Likely perf-relevant changes: none obviously perf-affecting.

Observed: flat versus v0.1.0 across every benchmark (all within ±3%), as
expected.

## v0.1.0 — 2026-03-07

Initial tracked release — the baseline for every series. The CHANGELOG's own
benchmark section reports binary encode 26-44% faster than prost 0.13 and JSON
decode 12-60% faster at this release.
