#!/bin/bash
# Run protoc-gen-buffa compatibility tests against multiple protoc versions.
#
# For each protoc version:
#   1. Run protoc with --buffa_out on proto2 and proto3 test files
#   2. Verify generated files exist, are non-empty, and contain Rust code
#   3. For editions-capable versions (v27+), also test an editions proto
#
# Exit code 0 if all versions pass, 1 if any fail.

set -euo pipefail

PLUGIN=/usr/local/bin/protoc-gen-buffa
PROTOS_DIR=/test/protos
PASS=0
FAIL=0
FAILURES=""

PROTOC_VERSIONS="${PROTOC_VERSIONS:-}"
if [[ -z "${PROTOC_VERSIONS//[[:space:]]/}" ]]; then
  echo "PROTOC_VERSIONS must contain at least one protoc version" >&2
  exit 2
fi

# Create editions test protos.
EDITIONS_DIR=$(mktemp -d)
trap 'rm -rf "$EDITIONS_DIR"' EXIT

# Edition 2023 — usable with protoc v27+.
cat > "${EDITIONS_DIR}/editions_2023_test.proto" <<'EOF'
edition = "2023";

package editions.test2023;

message EditionsMessage {
  string name = 1;
  int32 id = 2;
  bool active = 3;
  bytes data = 4;
  repeated string tags = 5;
}

enum EditionsEnum {
  EDITIONS_ENUM_UNSPECIFIED = 0;
  EDITIONS_ENUM_A = 1;
  EDITIONS_ENUM_B = 2;
}
EOF

# Edition 2024 — usable with protoc v33+.
cat > "${EDITIONS_DIR}/editions_2024_test.proto" <<'EOF'
edition = "2024";

package editions.test2024;

message Edition2024Message {
  string name = 1;
  int32 id = 2;
  bool active = 3;
  repeated string tags = 4;
}
EOF

# Test a single protoc version against a set of proto files.
# Args: $1=version, $2=test_name, $3..=proto files
run_test() {
  local ver="$1"
  local test_name="$2"
  shift 2

  local protoc="/protoc/${ver}/bin/protoc"
  local outdir
  outdir=$(mktemp -d)

  # Run protoc with our plugin.
  local result
  if result=$("$protoc" \
    --plugin=protoc-gen-buffa="$PLUGIN" \
    --buffa_out="$outdir" \
    --proto_path="$PROTOS_DIR" \
    --proto_path="${EDITIONS_DIR}" \
    "$@" 2>&1); then

    # Check that at least one .rs file was generated and is non-empty.
    local rs_count
    rs_count=$(find "$outdir" -name "*.rs" -size +0 | wc -l)
    if [ "$rs_count" -eq 0 ]; then
      echo "  FAIL  v${ver} :: ${test_name} — no .rs files generated"
      FAIL=$((FAIL + 1))
      FAILURES="${FAILURES}\n  v${ver} :: ${test_name} — no output"
    # Verify generated files contain Rust code (pub struct or pub mod).
    elif ! grep -rql 'pub struct\|pub mod\|pub enum' "$outdir"/*.rs 2>/dev/null; then
      echo "  FAIL  v${ver} :: ${test_name} — generated files contain no Rust types"
      FAIL=$((FAIL + 1))
      FAILURES="${FAILURES}\n  v${ver} :: ${test_name} — no Rust types in output"
    else
      echo "  PASS  v${ver} :: ${test_name} (${rs_count} file(s))"
      PASS=$((PASS + 1))
    fi
  else
    echo "  FAIL  v${ver} :: ${test_name} — protoc error:"
    echo "        ${result}"
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}\n  v${ver} :: ${test_name} — protoc error"
  fi

  rm -rf "$outdir"
}

echo "protoc-gen-buffa version compatibility tests"
echo "============================================="
echo ""

for ver in $PROTOC_VERSIONS; do
  protoc="/protoc/${ver}/bin/protoc"
  echo "--- protoc v${ver} ($("$protoc" --version)) ---"

  # Proto3: basic message with all field types.
  run_test "$ver" "proto3 (basic)" \
    "${PROTOS_DIR}/basic.proto"

  # Proto2: required fields, defaults, enums.
  run_test "$ver" "proto2 (defaults)" \
    "${PROTOS_DIR}/proto2_defaults.proto"

  # Keywords: Rust keyword collision handling.
  run_test "$ver" "keywords" \
    "${PROTOS_DIR}/keywords.proto"

  # Deep nesting.
  run_test "$ver" "nested_deep" \
    "${PROTOS_DIR}/nested_deep.proto"

  # Name collisions.
  run_test "$ver" "name_collisions" \
    "${PROTOS_DIR}/name_collisions.proto"

  # Cross-package imports (exercises dependency resolution).
  run_test "$ver" "cross_package (imports)" \
    "${PROTOS_DIR}/cross_package.proto"

  # Editions 2023 — only for protoc v27+ (experimental in v25–v26).
  major="${ver%%.*}"
  if [ "$major" -ge 27 ]; then
    run_test "$ver" "editions 2023" \
      "${EDITIONS_DIR}/editions_2023_test.proto"
  else
    echo "  SKIP  v${ver} :: editions 2023 (protoc too old)"
  fi

  # Editions 2024 — only for protoc v33+.
  if [ "$major" -ge 33 ]; then
    run_test "$ver" "editions 2024" \
      "${EDITIONS_DIR}/editions_2024_test.proto"
  else
    echo "  SKIP  v${ver} :: editions 2024 (protoc too old)"
  fi

  echo ""
done

# Summary.
echo "============================================="
echo "Results: ${PASS} passed, ${FAIL} failed"

if [ "$FAIL" -gt 0 ]; then
  echo ""
  echo "Failures:"
  printf '%b\n' "$FAILURES"
  exit 1
fi

echo ""
echo "All compatibility tests passed!"
