#!/usr/bin/env bash
# Run the full protobuf conformance suite without Docker.
#
# Native equivalent of conformance/run-conformance.sh + conformance/Dockerfile:
# builds the std and no_std conformance binaries with the host cargo, then
# drives them through conformance_test_runner (built by
# scripts/build-conformance-tools.sh) for the same seven runs as the Docker
# image. The runner talks to the testee over stdin/stdout pipes, so no
# container plumbing is needed.
#
# Prerequisites:
#   - task conformance-tools-local   (one-time: builds the runner, fetches protos)
#   - protoc v30+ on PATH or $PROTOC (e.g. task install-protoc)
#
# Env:
#   CONFORMANCE_OUT  — directory to tee per-run logs into (optional)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUNNER="${ROOT}/.local/bin/conformance_test_runner"
CONF="${ROOT}/conformance"

if [ ! -x "${RUNNER}" ]; then
    echo "conformance_test_runner not found at ${RUNNER}."
    echo "Run: task conformance-tools-local"
    exit 1
fi
if [ ! -f "${CONF}/protos/conformance.proto" ]; then
    echo "conformance/protos/ not populated. Run: task conformance-tools-local"
    exit 1
fi

echo "=== Building conformance binaries (std + no_std) ==="
cargo build --release --manifest-path "${CONF}/Cargo.toml"
cargo build --release --manifest-path "${CONF}/Cargo.toml" \
    --no-default-features --target-dir "${CONF}/target-nostd"

STD_BIN="${CONF}/target/release/conformance"
NOSTD_BIN="${CONF}/target-nostd/release/conformance"

run_suite() {
    local name="$1"
    local log="${CONFORMANCE_OUT:+${CONFORMANCE_OUT}/conformance-${name}.log}"
    shift
    echo "=== Conformance: ${name} ==="
    if [ -n "${log}" ]; then
        "$@" 2>&1 | tee "${log}"
    else
        "$@"
    fi
    echo ""
}

run_suite std \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures.txt" \
    --text_format_failure_list "${CONF}/known_failures_text.txt" \
    --maximum_edition 2024 \
    "${STD_BIN}"

run_suite nostd \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures_nostd.txt" \
    --text_format_failure_list "${CONF}/known_failures_text.txt" \
    --maximum_edition 2024 \
    "${NOSTD_BIN}"

BUFFA_VIA_VIEW=1 run_suite view \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures_view.txt" \
    --maximum_edition 2024 \
    "${STD_BIN}"

BUFFA_VIA_LAZY=1 run_suite lazy \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures_lazy.txt" \
    --maximum_edition 2024 \
    "${STD_BIN}"

BUFFA_VIEW_JSON=1 run_suite view-json \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures_view_json.txt" \
    --maximum_edition 2024 \
    "${STD_BIN}"

BUFFA_VIA_REFLECT=1 run_suite reflect \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures_reflect.txt" \
    --maximum_edition 2024 \
    "${STD_BIN}"

BUFFA_VIA_VTABLE=1 run_suite vtable \
    "${RUNNER}" \
    --failure_list "${CONF}/known_failures_view_vtable.txt" \
    --maximum_edition 2024 \
    "${STD_BIN}"

echo "All seven conformance runs completed."
