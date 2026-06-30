#!/usr/bin/env bash
# Build the `protobuf` benchmark binary under several LLVM code-alignment
# settings — the reproduction tool for the loop-alignment experiment recorded
# in annotations.md ("Loop alignment — closing the gap block alignment left
# open"). Use it to re-verify the alignment policy after a toolchain re-pin or
# a serde_json upgrade, either of which can change the hot loop's size and
# therefore which alignment value it needs.
#
# All variants build at the normalized profile (lto=true, codegen-units=1) so
# the only varying input is the alignment flag set:
#
#   nofallthru        — block alignment only (the pre-2026-06 policy)
#   nofallthru+loops32 — + -align-loops=32 (wrong for a 32-byte loop body:
#                        mod64 ∈ {0,32}, still a 50/50 straddle lottery)
#   nofallthru+loops64 — + -align-loops=64 (the current policy)
#   loops-only        — loop alignment without block alignment (NOT a
#                        substitute: dispatch-heavy decode paths regress)
#   none              — no alignment flags (control)
#
# Each variant is copied to <out-dir>/<label>.bench. Run them on a quiesced
# machine, e.g. via bench-on-metal compare mode (first --binary becomes the
# criterion baseline):
#
#   bench-on-metal.sh --spot \
#     --binary nofallthru.bench --binary nofallthru+loops64.bench \
#     --args "--measurement-time 8 'json_encode'"
#
# To confirm a variant's effect mechanically, check the hot loop's head
# address modulo 64 in the disassembly (fast iff mod64 + body size <= 64):
#
#   objdump -d <bin> | grep -A40 format_escaped_str_contents
set -euo pipefail

OUT_DIR="${1:-./align-variants}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bench_dir="$script_dir/../buffa"

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

NOFALL='-Cllvm-args=-align-all-nofallthru-blocks=6'
LOOPS32='-Cllvm-args=-align-loops=32'
LOOPS64='-Cllvm-args=-align-loops=64'

# label|RUSTFLAGS
variants=(
    "nofallthru|$NOFALL"
    "nofallthru+loops32|$NOFALL $LOOPS32"
    "nofallthru+loops64|$NOFALL $LOOPS64"
    "loops-only|$LOOPS64"
    "none|"
)

for v in "${variants[@]}"; do
    label="${v%%|*}"
    flags="${v#*|}"
    echo ">>> building protobuf bench: label=$label RUSTFLAGS='$flags'" >&2
    # `--message-format=json` so we can read the exact bench executable path;
    # changing RUSTFLAGS changes the artifact hash, so each build lands at a
    # different deps/protobuf-<hash> path that we must capture per iteration.
    exe="$(
        RUSTFLAGS="$flags" \
        CARGO_PROFILE_BENCH_LTO=true \
        CARGO_PROFILE_BENCH_CODEGEN_UNITS=1 \
            cargo bench --manifest-path "$bench_dir/Cargo.toml" \
            --bench protobuf --no-run --message-format=json 2>/dev/null \
            | jq -r 'select(.reason=="compiler-artifact"
                            and .target.name=="protobuf"
                            and .executable!=null) | .executable' \
            | tail -n1
    )"
    if [[ -z "$exe" || ! -x "$exe" ]]; then
        echo "error: could not locate built bench binary for label=$label" >&2
        exit 1
    fi
    cp "$exe" "$OUT_DIR/$label.bench"
    echo "    -> $OUT_DIR/$label.bench" >&2
done

echo "built ${#variants[@]} variant(s) into $OUT_DIR" >&2
