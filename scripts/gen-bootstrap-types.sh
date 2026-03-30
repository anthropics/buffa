#!/usr/bin/env bash
# Regenerate buffa-descriptor/src/generated/ (bootstrap descriptor types).
#
# The source protos are vendored in buffa-descriptor/protos/ (pinned to a
# specific protobuf release) so the generated output does not depend on
# which protoc is installed locally — only the protoc binary is needed,
# not its bundled includes.
#
# Usage: scripts/gen-bootstrap-types.sh
# Env:   PROTOC=/path/to/protoc   (default: from PATH)

set -euo pipefail

# Minimum protoc version. The vendored descriptor.proto uses option syntax
# (edition_defaults, retention, declaration) that older protoc rejects.
# This matches the repo's general floor (CONTRIBUTING.md Prerequisites).
readonly PROTOC_MIN=27

PROTOC="${PROTOC:-$(command -v protoc || true)}"
if [ -z "$PROTOC" ] || [ ! -x "$PROTOC" ]; then
    echo "error: protoc not found. Install it or set PROTOC=/path/to/protoc." >&2
    exit 1
fi

# protoc --version output: "libprotoc X.Y" (or "libprotoc X.Y.Z")
ver_str="$("$PROTOC" --version)"
ver_major="$(echo "$ver_str" | sed -n 's/^libprotoc \([0-9]*\).*/\1/p')"
if [ -z "$ver_major" ] || [ "$ver_major" -lt "$PROTOC_MIN" ]; then
    echo "error: protoc v${PROTOC_MIN}+ required, found: ${ver_str}" >&2
    echo "       Run 'task install-protoc' then re-run with PROTOC=.local/bin/protoc" >&2
    exit 1
fi

echo "protoc: $PROTOC ($ver_str)"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

DESC=/tmp/buffa-descriptor-set.pb
"$PROTOC" --descriptor_set_out="$DESC" --include_imports --include_source_info \
    -I "$ROOT/buffa-descriptor/protos" \
    google/protobuf/descriptor.proto \
    google/protobuf/compiler/plugin.proto

cd "$ROOT"
cargo run -p buffa-codegen --bin gen_descriptor_types -- \
    "$DESC" buffa-descriptor/src/generated
