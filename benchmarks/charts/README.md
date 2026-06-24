# Cross-implementation benchmarks

How buffa compares to other protobuf implementations — prost, prost with
`bytes`, Google's official protobuf v4 (`protobuf-v4`), and Go's `google.golang.org/protobuf`
— on the canonical benchmark messages, at a single point in time. (For buffa
against *its own past*, see [`../history/`](../history/).)

`tables.md` and `charts/*.svg` are the rendered results; `measurement-spread.md`
records how stable each number is.

## Two ways to run

**Docker — for contributors (`task bench-cross`).** Each implementation builds and
runs in its own container, so a contributor can reproduce the comparison anywhere
without installing five toolchains. This is the right tool for a local regression
check. It is **not** how the published numbers are produced: a laptop or CI host is
shared and virtualised, so its absolute throughput drifts and is not trustworthy.

**Bare metal — for the published numbers.** The committed `tables.md` is produced on
a quiesced bare-metal host with the same strategy as the per-release history, so the
numbers are stable and the implementations are compared on an equal footing.

## Bare-metal methodology

- **One quiesced machine.** A dedicated bare-metal host with CPU turbo disabled and
  the `performance` governor. Each benchmark runs **one at a time** — a single
  instance pinned to one isolated physical core, with nothing else running — so there
  is no cross-instance contention. Each implementation is run for **five sequential
  passes** and the per-benchmark number is the **median** across them, with the
  spread recorded in `measurement-spread.md`.
- **Pinned toolchains, held constant across all implementations.** rust **1.96.0**
  (the same pin as the per-release history), go **1.23**, and protoc **33.1**.
  protoc 33.1 specifically because Google's `protobuf` v4 crate version-checks protoc
  exactly; the other implementations only need ≥ 3.15 for proto3 `optional`.
- **The release profile, applied to every Rust implementation.** Each Rust bench
  crate builds at **`lto=true, codegen-units=1`** plus **64-byte block alignment**
  (`-Cllvm-args=-align-all-nofallthru-blocks=6`) — the same release-and-layout-normalized
  profile the history uses. This matters for fairness: the bench crates are excluded
  from the root workspace, so before this was fixed they silently built at cargo's
  default `codegen-units=16, lto=off`, which understated *every* Rust implementation
  (a disassembly showed 317 vs 7 un-inlined `decode_varint` call sites in the buffa
  binary). The empty `[workspace]` + `[profile.bench]` table in each bench crate's
  `Cargo.toml` makes the profile take effect; alignment is applied at build time.

## Caveats

- **Best-achievable layout, not as-shipped.** Block alignment removes the build-time
  code-layout lottery (so a rebuild reproduces the numbers), at the cost of measuring
  the layout a profile-guided build would reach rather than what a plain `cargo build`
  ships. The right frame for "which implementation is faster," the same choice the
  history makes — see [`../history/annotations.md`](../history/annotations.md).
- **Each implementation runs as its combined benchmark binary** (all messages in one
  binary), so buffa's numbers here sit ~10% below the per-message-isolated history.
  That cross-message inliner coupling is constant across the comparison, so it does
  not bias one implementation against another; the history isolates per message
  because *there* the coupling varies across releases.
- **protobuf-v4 wraps upb (C); the Rust profile flags only partly reach it.** The
  bench profile's `lto=true`, `codegen-units=1`, and block-alignment apply to Rust
  LLVM IR; `upb.c` is compiled separately by `cc` and linked without cross-language
  LTO (the upstream `build.rs` has a `// TODO: enable lto` for this). The `cc` crate
  inherits cargo's `-O3` but does not auto-define `NDEBUG`, and the upstream
  `build.rs` doesn't either, so the metal script and Dockerfile set `CFLAGS=-DNDEBUG`
  explicitly — without it every `UPB_ASSERT` is a live `assert()` and every
  `UPB_ASSUME` is `assert()` instead of `__builtin_unreachable()`.

## Why protobuf-v4 encode is ~3× slower

The encode harness is identical across implementations (pre-decode outside the timed
loop, fresh allocation per iteration, `black_box` the result), and `serialize()` is
the only public encode API on the `protobuf` v4 crate, so the gap is what every Rust
consumer of that crate sees today. Two architectural costs account for it:

- **`serialize()` allocates a fresh upb arena, encodes back-to-front into it, then
  copies the arena buffer into a `Vec<u8>`.** `upb_Encode` has no capacity hint
  (`upb_ByteSize` exists but is implemented as a second full encode), so it starts
  from a small block and `encode_growbuffer` doubles on overflow — each growth is a
  `memmove` of the bytes written so far. The Rust wrapper then `.to_vec()`s the final
  arena slice. For a message dominated by a large `bytes` field (MediaFrame), `perf`
  shows ~55–60% of self-time in libc `memcpy`/`memmove` plus arena alloc/free, and
  the result is roughly three times the copy work of buffa's and prost's
  `Vec::with_capacity(encoded_len())` followed by a single in-place write. There is
  no `serialize_into(&mut Vec)` or arena-reuse path in the public API; the
  lower-level `upb::wire::encode` lives under `protobuf::__internal`. This boundary
  copy — moving bytes from C-managed arena memory into a Rust-owned `Vec` — is
  exactly the FFI tax a pure-Rust implementation avoids by construction.
- **upb's encoder is table-driven, where buffa's and prost's are monomorphized
  generated code.** For scalar-heavy messages (ApiResponse, GoogleMessage1), `perf`
  shows ~75% of time inside `upb_Encode`'s mini-table walk (`encode_message` /
  `encode_scalar` / `encode_array`) versus the fully-inlined per-field code that
  Rust codegen produces. That is a deliberate upb design choice (one encoder for all
  message shapes, small binary), not a benchmark artefact.

Decode is a different story: upb's decoder is competitive with the generated Rust
decoders on most shapes, because the arena already holds the parsed message and
there is no C→Rust output copy on the hot path.

## Files

- `tables.md`, `charts/*.svg` — generated comparison tables and per-message charts.
- `measurement-spread.md` — generated per-implementation spread (stability).
- `generate.py` — renders `tables.md` + `charts/` from a results directory.
- `cross_aggregate.py` — turns a bare-metal run's output into per-impl median +
  spread (the inputs `generate.py` consumes, plus `measurement-spread.md`).
- `cross_metal_run.sh` — the bare-metal build-and-run script (below).

## Reproducing the published numbers

On a quiesced bare-metal host with the toolchains above on `PATH`, from the repo root:

```bash
benchmarks/charts/cross_metal_run.sh > /tmp/cross.out
python3 benchmarks/charts/cross_aggregate.py /tmp/cross.out benchmarks/results
python3 benchmarks/charts/generate.py benchmarks/results
```

Cloud provisioning and teardown of the metal host are intentionally kept out of the
repository.
