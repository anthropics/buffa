#!/usr/bin/env bash
# Regenerate buffa-proto-options/src/generated/ from
# buffa-proto-options/protos/buffa/ext/options.proto.
#
# descriptor.proto, which options.proto imports, is read from
# buffa-descriptor/protos/ (pinned), so the output does not depend on the
# includes bundled with the local protoc.
#
# Usage: scripts/gen-option-types.sh
# Env:   PROTOC=/path/to/protoc   (default: from PATH)

set -euo pipefail

# Minimum protoc version, as in gen-bootstrap-types.sh. The generated code
# embeds descriptor.proto as protoc serialized it, and a protoc that predates
# option retention serializes it differently from the one CI runs.
readonly PROTOC_MIN=27

PROTOC="$(command -v -- "${PROTOC:-protoc}" || true)"
if [ -z "$PROTOC" ] || [ ! -x "$PROTOC" ]; then
    echo "error: protoc not found. Install it or set PROTOC=/path/to/protoc." >&2
    exit 1
fi
# The script changes directory before it runs protoc.
[[ $PROTOC == /* ]] || PROTOC="$PWD/$PROTOC"

# protoc --version output: "libprotoc X.Y" (or "libprotoc X.Y.Z")
ver_str="$("$PROTOC" --version)"
ver_major="$(echo "$ver_str" | sed -n 's/^libprotoc \([0-9]*\).*/\1/p')"
if [ -z "$ver_major" ] || [ "$ver_major" -lt "$PROTOC_MIN" ]; then
    echo "error: protoc v${PROTOC_MIN}+ required, found: ${ver_str}" >&2
    echo "       Run 'task install-protoc' then re-run with PROTOC=.local/bin/protoc" >&2
    exit 1
fi

echo "protoc: $PROTOC ($ver_str)" >&2

ROOT="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

desc_dir="$(mktemp -d)"
trap 'rm -rf "$desc_dir"' EXIT

"$PROTOC" --descriptor_set_out="$desc_dir/options.pb" --include_imports --include_source_info \
    -I buffa-proto-options/protos -I buffa-descriptor/protos \
    buffa/ext/options.proto

cargo run -p buffa-codegen --bin gen_option_types -- \
    "$desc_dir/options.pb" buffa-proto-options/src/generated
