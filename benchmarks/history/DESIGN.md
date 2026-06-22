# Benchmark history — design

## Goal

This directory tracks how buffa's encode/decode performance has changed across releases, for every message shape we benchmark, in a way that is trustworthy (a movement reflects buffa's code, not measurement noise), attributable (we can say which release and which change moved a number), and reproducible (anyone can rebuild any data point). The headline metric is throughput in MiB/s — a rate (bytes decoded or encoded per second) rather than absolute time per iteration. A rate is the right choice for two reasons: it lets shapes whose payloads differ by orders of magnitude sit on one axis, and it stays meaningful if a shape's dataset is ever regenerated with a different number of payloads (the per-byte cost is what we are comparing, not the wall time of one dataset). What a rate does *not* do is make a changed proto *shape* comparable to its former self: altering a message's fields changes what is being decoded, so it is a new benchmark series, not a continuation of the old curve. The shapes and their datasets are therefore held fixed and canonical (see below); a release introducing a new shape adds a new series, it does not perturb existing ones.

## What we learned (why this is more than `cargo bench`)

Most of the work here is not running benchmarks — it is removing sources of variance that are large enough to invent regressions that do not exist. We found these in order, each one a false signal we initially chased, and each one is now controlled:

*Machine noise*. A shared or virtualised host cannot give stable absolute throughput. Runs happen on a dedicated bare-metal box with CPU turbo disabled, the `performance` governor, and the benchmark pinned to a physical core.

*Run-position drift*. We suspected the order benchmarks ran in mattered; an interleaved re-measurement showed each version measured within ~0.1% of itself regardless of position, ruling it out.

*Build-layout noise*. The single largest trap. The default `bench` profile is `codegen-units=16, lto=off`; at sixteen codegen units the partitioning of functions across units — and therefore which calls get inlined — shifts when unrelated code is added, moving a dispatch-bound benchmark 10–20% with the measured code unchanged. The same source measured −3.3% on one build and +0.3% on a fresh one. We pin `lto=true, codegen-units=1`.

*Compiler drift*. Different toolchains generate different code. We pin one toolchain across the whole series, forced via `RUSTUP_TOOLCHAIN` so it does not depend on the working directory.

*Cross-message inliner coupling*. The subtlest. Every message's decoder compiles into one binary, so at `codegen-units=1` the inliner makes one global decision; adding a new benchmark message re-partitions inlining for the *other*, unchanged decoders. This is what made v0.7.1 look like a broad regression — it was the newly added `PackedTile` message perturbing the others, proven by disassembly (removing `PackedTile` made `MediaFrame`'s machine code byte-identical to v0.7.0). The fix is true per-message isolation: only the measured message is compiled.

The lesson worth keeping: a per-message delta below roughly the 15% build-noise envelope is not attributable to buffa's code unless the build is pinned and the message is isolated.

## The key realization

A message shape is a property of the protobuf schema, not of any buffa release. buffa v0.1.0 could always encode and decode a `MediaFrame`-shaped message — we simply never asked it to, because `MediaFrame` was not added to the benchmark suite until v0.4.0. So the schema and datasets are canonical and version-independent, and every shape can be measured against every release, bounded only by which operations a release's API actually supported. That turns a sparse history (each shape starting when it entered the suite) into a dense one (every shape, every release).

## Design

### Canonical artifacts (version-independent)

The source of truth is a `FileDescriptorSet` of all benchmark message shapes, plus the committed `.pb` datasets (one per shape). These belong to the benchmark tooling, not to any buffa release. When we introduce a new shape, we add it here once, along with its benchmark configuration, and it becomes measurable across the whole history retroactively.

### Per-message isolation via a minimal FDS

For each shape we compute the transitive type closure — walking message-typed fields, map values, oneof variants, nested types, extensions, and well-known types from the root — and emit a minimal `FileDescriptorSet` containing only that shape and its dependencies (plus `BenchmarkDataset`). The closure is computed with the current `buffa-descriptor` `DescriptorPool`; it is a pure property of the schema, so the same logic works regardless of which buffa version will consume the result.

That minimal FDS is fed to a given release's `buffa-build` through `Config::descriptor_set()`, which has existed since v0.1.0. Because only the target shape is generated, the other shapes' decoders never enter the codegen unit, and the cross-message coupling is gone at the source rather than papered over at link time (link-time tricks — DCE, disabling reflection — do not work, because the coupling is baked during compilation before dead code is removed). This was validated end to end: isolated `media_frame/decode_view` reads +1% across v0.7.0 and v0.7.1, versus −13% when coupled.

### One harness, gated by capability

A single benchmark harness is shared across all releases, restricted to the encode/decode surface that has been stable in practice — `decode_from_slice`, `encode_to_vec`, `merge_from_slice`, the view `decode_view` path, and JSON. This works because that surface happens to have changed little from v0.1.0 onward; it is an empirical property of the past releases that we verify by building, not a guarantee we can make for future releases while the API is still evolving. The gate is therefore build success: if the harness compiles against a release for a given operation, we measure it; if it does not, that cell is simply absent. Operations with signature drift (`compute_size` gained a `SizeCache`) or that genuinely did not exist yet (`encode_view`, `build_encode`) are gated to the releases that support them.

The result is a capability matrix — operation by minimum release — measured wherever it is supported, rather than a single harness pretending every release had every feature.

### Build and measurement

Every binary is built at the pinned toolchain and `lto=true, codegen-units=1`, isolated to one shape via the minimal FDS. Measurement runs on an AWS `metal` instance, to isolate from noisy neighbors and hypervisor behavior. Each benchmark is the median across several physical cores run concurrently — concurrency was measured to track the isolated single-core baseline within ~1% in this configuration, because the datasets fit private L2. The per-benchmark spread is recorded as a stability indicator.

### Data schema

Each `runs/<version>.json` records the version, its commit and date, when it was measured, the toolchain, the criterion version, the build profile, the machine and tuning, and per-benchmark metrics: `median_ns`, `throughput_mib_s`, `throughput_spread_pct`, and `samples`. These files are the source of truth; `REPORT.md` and `charts/` are generated from them.

## Caveats

The harness's source-compatibility across past versions is empirical and gated by build success, not promised for future versions. Regenerating a shape from a minimal FDS can in principle change second-order codegen context (name deconfliction, registry construction), but that does not touch the encode/decode hot path we focus on, so we accept it in exchange for isolation. A release that genuinely lacked a benchmarked operation has no data point for it, by design. There still remains a roughly ±5% reproducibility floor even on a bare-metal instance — treat smaller movements as noise unless a later release confirms the trend.

## Implementation stages

1. The canonical FDS plus the closure tool (the same closure logic also powers a forward `buffa-build` "roots" option, so a consumer can generate a minimal build for the types they actually use).
2. The capability-gated harness and the capability matrix.
3. Per-release isolated runs producing the full shape-by-release matrix, regenerating the report and charts.
4. Forward: bake per-message isolation into the committed bench harness so that every new release is clean by construction and never needs a retroactive fix.

## Status

Built and committed (PR #211). The history is now the dense, per-message-isolated matrix: every shape × every release (v0.1.0–v0.7.1), seven portable operations each, every cell built with only its own decoder compiled. Stages 2 and 3 were realized by giving each release a `historical-benchmark/vX.Y.Z` branch carrying the per-message-isolated harness (built against that release's own buffa via per-message protos and Cargo features — the per-message-proto split stood in for the closure tool of stage 1, which remains worthwhile as the forward `buffa-build` "roots" feature). The runs were re-measured on bare metal with the layout normalized — 64-byte block alignment (`-align-all-nofallthru-blocks=6`), median of 32 cores — which collapsed the residual code-layout lottery (json_encode cross-release spread 19% → 2.5%) while preserving the real code-driven steps (see annotations.md), then the report and charts were regenerated. The earlier sparse, single-binary history it replaced — where each shape started only at the release that added it, and two transitions were inliner-coupling artifacts — is fully superseded.

Still open: stage 1's reusable closure tool / `buffa-build` roots option, and stage 4 (baking per-message isolation into the committed bench harness on `main` so future releases are clean by construction). The `historical-benchmark/*` branches are the reproducibility artifact for the back-catalogue.
