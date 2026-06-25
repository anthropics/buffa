#!/usr/bin/env bash
#
# Bare-metal cross-implementation benchmark run (the strategy used for the
# *published* comparison tables; see README.md "Bare-metal methodology").
#
# Builds each implementation's benchmark at the release profile + 64-byte block
# alignment, then runs each one ONE AT A TIME (a single instance pinned to one
# isolated core, nothing else running) for several sequential passes, and prints
# every pass's output delimited by ===BLOCK=<impl>_passN=== markers. Pipe that
# into cross_aggregate.py to get per-impl median + spread, then generate.py.
#
# Run from the repository root on a quiesced bare-metal host (turbo disabled,
# `performance` governor). Requires the pinned toolchains on PATH:
#   - rustc/cargo via rustup, toolchain 1.96.0 (set RUSTUP_TOOLCHAIN below)
#   - go 1.23.x  (for the Go implementation)
#   - protoc 33.1 (Google's protobuf v4 crate version-checks protoc exactly;
#                  the other impls only need >= 3.15 for proto3 optional)
#   - protoc-gen-go v1.36.6 on PATH (for the Go codegen)
#
#   benchmarks/charts/cross_metal_run.sh > /tmp/cross.out
#   python3 benchmarks/charts/cross_aggregate.py /tmp/cross.out benchmarks/results
#   python3 benchmarks/charts/generate.py benchmarks/results
#
# Cloud provisioning/teardown of the metal host is intentionally NOT in the repo.
set -uo pipefail

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.96.0}"
# Release profile (the bench crates also pin this in [profile.bench]) + the
# 64-byte block alignment that removes the build-time code-layout lottery.
export CARGO_PROFILE_BENCH_LTO=true
export CARGO_PROFILE_BENCH_CODEGEN_UNITS=1
export RUSTFLAGS="${RUSTFLAGS:--Cllvm-args=-align-all-nofallthru-blocks=6}"
# protobuf-v4 wraps upb (C); the cc crate does not auto-define NDEBUG, and the
# upstream build.rs doesn't either, so without this every UPB_ASSERT is a live
# assert() and every UPB_ASSUME is assert() instead of __builtin_unreachable().
export CFLAGS="${CFLAGS:-} -DNDEBUG"
PASSES="${PASSES:-5}"          # sequential one-at-a-time passes per impl
WARMUP="${WARMUP:-1}"
MEASURE="${MEASURE:-3}"
# The published number is the median across PASSES, so the per-pass criterion
# CI width is not what bounds precision — the cross-pass spread is. With the
# block-aligned build there is no layout lottery to average over either, so 50
# samples (down from criterion's default 100), `--nresamples 1000` (down from
# criterion's default 100,000 bootstrap CI resamples — pure waste since
# cross_aggregate.py only reads the median, never the CI bounds), `--noplot`,
# and `--discard-baseline` together cut the wall-clock to roughly the
# warm-up + measurement time without moving the published median outside the
# existing ~2-5% spread floor.
SAMPLES="${SAMPLES:-50}"
NRESAMPLES="${NRESAMPLES:-1000}"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"   # repo root
cd "$ROOT"
WORK="$(mktemp -d)"

echo "== build (rustc $(rustc --version | awk '{print $2}'), protoc $(protoc --version | awk '{print $2}')) =="
declare -A BIN
for c in buffa prost prost-bytes google; do
  if cargo bench --manifest-path "benchmarks/$c/Cargo.toml" --no-run >"$WORK/build-$c.log" 2>&1; then
    BIN[$c]="$(ls -t benchmarks/$c/target/release/deps/protobuf-* 2>/dev/null | grep -v '\.d$' | head -1)"
    echo "  $c OK"
  else echo "  BUILD FAIL $c"; tail -6 "$WORK/build-$c.log"; fi
done
# buffa also has a reflection benchmark (generated codec vs DynamicMessage); a
# plain `cargo bench --no-run` built it above. Run it alongside buffa's protobuf
# bench so tables.md's Reflection sections are populated.
BIN_REFLECT="$(ls -t benchmarks/buffa/target/release/deps/reflect-* 2>/dev/null | grep -v '\.d$' | head -1)"
( cd benchmarks/go && mkdir -p gen/bench gen/benchmarks gen/proto3 \
  && protoc --go_out=gen/bench --go_opt=paths=source_relative --go_opt=Mbench_messages.proto=github.com/anthropics/buffa/benchmarks/go/gen/bench -I ../proto ../proto/bench_messages.proto \
  && protoc --go_out=gen/benchmarks --go_opt=paths=source_relative --go_opt=Mbenchmarks.proto=github.com/anthropics/buffa/benchmarks/go/gen/benchmarks -I ../proto ../proto/benchmarks.proto \
  && protoc --go_out=gen/proto3 --go_opt=paths=source_relative --go_opt=Mbenchmark_message1_proto3.proto=github.com/anthropics/buffa/benchmarks/go/gen/proto3 -I ../proto ../proto/benchmark_message1_proto3.proto \
  && go mod tidy && go test -bench=. -benchtime=1x ./... ) >"$WORK/build-go.log" 2>&1 && echo "  go OK" || { echo "  BUILD FAIL go"; tail -6 "$WORK/build-go.log"; }

# Pin to the leader of one physical core (avoid SMT siblings + core 0).
CORE="$(lscpu -p=CPU,CORE | grep -v '^#' | awk -F, '!s[$2]++{print $1}' | sed -n '2p')"
echo "== run one-at-a-time on core $CORE, $PASSES passes (warmup ${WARMUP}s, measure ${MEASURE}s) =="
for rep in $(seq 1 "$PASSES"); do
  for c in buffa prost prost-bytes google; do
    [ -n "${BIN[$c]:-}" ] || continue
    d="$WORK/${c}_pass${rep}"; mkdir -p "$d"
    ( cd "$d" && chrt -f 99 taskset -c "$CORE" "$ROOT/${BIN[$c]}" --bench --noplot --discard-baseline --sample-size "$SAMPLES" --nresamples "$NRESAMPLES" --warm-up-time "$WARMUP" --measurement-time "$MEASURE" >out.txt 2>&1 )
    if [ "$c" = buffa ] && [ -n "${BIN_REFLECT:-}" ]; then
      ( cd "$d" && chrt -f 99 taskset -c "$CORE" "$ROOT/$BIN_REFLECT" --bench --noplot --discard-baseline --sample-size "$SAMPLES" --nresamples "$NRESAMPLES" --warm-up-time "$WARMUP" --measurement-time "$MEASURE" >>out.txt 2>&1 )
    fi
  done
  d="$WORK/go_pass${rep}"; mkdir -p "$d"
  ( cd benchmarks/go && chrt -f 99 taskset -c "$CORE" go test -bench=. -benchmem -benchtime="${MEASURE}s" ./... >"$d/out.txt" 2>&1 )
  echo "  pass $rep done" >&2
done

for d in "$WORK"/buffa_pass* "$WORK"/prost_pass* "$WORK"/prost-bytes_pass* "$WORK"/google_pass* "$WORK"/go_pass*; do
  [ -d "$d" ] || continue
  echo "===BLOCK=$(basename "$d" | sed 's/_pass/_/')==="
  cat "$d/out.txt"
done
echo "===END==="
rm -rf "$WORK"
