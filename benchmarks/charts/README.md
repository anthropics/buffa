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
