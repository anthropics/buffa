#!/usr/bin/env bash
# Build the `protobuf` benchmark binary several times, each with a different
# `codegen-units` setting, so the layout-sensitivity harness can measure how
# much a benchmark's number moves under pure build-layout perturbation (see
# benchmarks/history/README.md, "Layout-noise envelope").
#
# The benches build with cargo's default `bench` profile (benchmarks/buffa is
# excluded from the root workspace, so the root `lto`/`codegen-units` settings
# do NOT apply): codegen-units=16, lto=off. With 16 units and no LTO, adding
# unrelated code re-partitions functions across units and flips inline
# decisions at unit boundaries — which moves small dispatch-bound benchmarks by
# 10-20% without the measured code changing. Sweeping codegen-units is a
# stable-toolchain proxy for that layout noise (the pinned 1.95.0 toolchain has
# no `-Z randomize-layout`).
#
# Each variant binary is copied to <out-dir>/cgu<N>.bench. Run each on a quiesced
# machine, capturing its stdout (criterion needs the --bench flag):
#
#   for v in <out-dir>/cgu*.bench; do
#     "$v" --bench --measurement-time 4 > "$(basename "$v" .bench).txt"
#   done
#
# then feed the captured per-binary outputs to layout_envelope.py.
set -euo pipefail

CGUS="${CGUS:-1 2 4 8 16}"
OUT_DIR="${1:-./cgu-variants}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bench_dir="$script_dir/../buffa"

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

for cgu in $CGUS; do
    echo ">>> building protobuf bench with codegen-units=$cgu" >&2
    # `--message-format=json` so we can read the exact bench executable path;
    # changing codegen-units changes the artifact hash, so each build lands at a
    # different deps/protobuf-<hash> path that we must capture per iteration.
    exe="$(
        CARGO_PROFILE_BENCH_CODEGEN_UNITS="$cgu" \
            cargo bench --manifest-path "$bench_dir/Cargo.toml" \
            --bench protobuf --no-run --message-format=json 2>/dev/null \
            | jq -r 'select(.reason=="compiler-artifact"
                            and .target.name=="protobuf"
                            and .executable!=null) | .executable' \
            | tail -n1
    )"
    if [[ -z "$exe" || ! -x "$exe" ]]; then
        echo "error: could not locate built bench binary for codegen-units=$cgu" >&2
        exit 1
    fi
    cp "$exe" "$OUT_DIR/cgu$cgu.bench"
    echo "    -> $OUT_DIR/cgu$cgu.bench" >&2
done

echo "built $(echo "$CGUS" | wc -w) variant(s) into $OUT_DIR" >&2
