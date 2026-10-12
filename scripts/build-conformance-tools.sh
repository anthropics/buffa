#!/usr/bin/env bash
# Build the protobuf conformance_test_runner from source — no Docker — and
# populate conformance/protos/ with the test .proto files.
#
# Native equivalent of conformance/Dockerfile.tools: clones protobuf at the
# pinned tag, pre-installs jsoncpp (the conformance cmake target requires
# the `jsoncpp_lib` cmake package), builds the runner with the host
# toolchain (dynamically linked — fine for local runs, unlike the image
# which must be `-static`), and installs it into .local/bin/.
#
# protoc is NOT built here (it adds ~half the build time); the editions
# test protos need protoc v27+ on PATH or $PROTOC — see `task install-protoc`.
#
# Usage: scripts/build-conformance-tools.sh [protobuf-tag]
#   protobuf-tag defaults to v33.5 (keep in sync with TOOLS_IMAGE).
#   JSONCPP_TAG in the environment selects the jsoncpp tag (default 1.9.6).
#
# Each piece records the tag it was built from in a stamp file under
# .local/conformance-tools/: jsoncpp.tag, protobuf.tag, and runner.tag (the
# protobuf and jsoncpp tags the runner was built with). A piece is rebuilt
# when its stamp is missing or names another tag, or when a file it should
# have produced is missing. A tree built before the stamps existed has none,
# so the first run on it rebuilds everything once.

set -euo pipefail

PROTOBUF_TAG="${1:-v33.5}"
JSONCPP_TAG="${JSONCPP_TAG:-1.9.6}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${ROOT}/.local/conformance-tools"
BIN_DIR="${ROOT}/.local/bin"
PROTOS_DEST="${ROOT}/conformance/protos"
JOBS="$(nproc 2>/dev/null || echo 4)"

mkdir -p "${WORK}" "${BIN_DIR}"

# The test protos (the same set Dockerfile.tools packages into /protos), one
# entry per file: its path in the protobuf checkout, then its directory under
# conformance/protos/.
PROTO_FILES=(
    "conformance/conformance.proto ."
    "src/google/protobuf/test_messages_proto3.proto google/protobuf"
    "src/google/protobuf/test_messages_proto2.proto google/protobuf"
    "src/google/protobuf/any.proto google/protobuf"
    "src/google/protobuf/duration.proto google/protobuf"
    "src/google/protobuf/field_mask.proto google/protobuf"
    "src/google/protobuf/struct.proto google/protobuf"
    "src/google/protobuf/timestamp.proto google/protobuf"
    "src/google/protobuf/wrappers.proto google/protobuf"
    "editions/golden/test_messages_proto2_editions.proto editions/golden"
    "editions/golden/test_messages_proto3_editions.proto editions/golden"
    "conformance/test_protos/test_messages_edition2023.proto conformance/test_protos"
    "conformance/test_protos/test_messages_edition_unstable.proto conformance/test_protos"
)

# The jsoncpp configure step sets CMAKE_INSTALL_LIBDIR=lib, so these paths hold
# on hosts where CMake would otherwise install libraries under lib64.
JSONCPP_CONFIG="${WORK}/jsoncpp-prefix/lib/cmake/jsoncpp/jsoncppConfig.cmake"
JSONCPP_STAMP="${WORK}/jsoncpp.tag"
jsoncpp_library_present() {
    local library
    for library in "${WORK}/jsoncpp-prefix"/lib/libjsoncpp.*; do
        [ -f "${library}" ] && return 0
    done
    return 1
}
if [ ! -f "${JSONCPP_CONFIG}" ] \
    || [ ! -f "${JSONCPP_STAMP}" ] \
    || ! jsoncpp_library_present \
    || [ "$(cat "${JSONCPP_STAMP}")" != "${JSONCPP_TAG}" ]; then
    rm -rf "${WORK}/jsoncpp" "${WORK}/jsoncpp-prefix" "${JSONCPP_STAMP}"
    echo "=== Building jsoncpp ${JSONCPP_TAG} ==="
    git clone --depth=1 --branch "${JSONCPP_TAG}" \
        https://github.com/open-source-parsers/jsoncpp.git "${WORK}/jsoncpp"
    cmake -B "${WORK}/jsoncpp/_build" -S "${WORK}/jsoncpp" \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_SHARED_LIBS=ON \
        -DJSONCPP_WITH_TESTS=OFF \
        -DJSONCPP_WITH_POST_BUILD_UNITTEST=OFF \
        -DCMAKE_INSTALL_LIBDIR=lib \
        -DCMAKE_INSTALL_PREFIX="${WORK}/jsoncpp-prefix"
    cmake --build "${WORK}/jsoncpp/_build" -j"${JOBS}"
    cmake --install "${WORK}/jsoncpp/_build"
    printf '%s\n' "${JSONCPP_TAG}" > "${JSONCPP_STAMP}"
fi

PROTOBUF_STAMP="${WORK}/protobuf.tag"
protobuf_sources_present() {
    local entry
    for entry in "${PROTO_FILES[@]}"; do
        [ -f "${WORK}/protobuf/${entry% *}" ] || return 1
    done
}
if [ ! -d "${WORK}/protobuf" ] \
    || [ ! -f "${PROTOBUF_STAMP}" ] \
    || ! protobuf_sources_present \
    || [ "$(cat "${PROTOBUF_STAMP}")" != "${PROTOBUF_TAG}" ]; then
    rm -rf "${WORK}/protobuf" "${PROTOBUF_STAMP}"
    echo "=== Cloning protobuf ${PROTOBUF_TAG} ==="
    git clone --depth=1 --branch "${PROTOBUF_TAG}" \
        https://github.com/protocolbuffers/protobuf.git "${WORK}/protobuf"
    printf '%s\n' "${PROTOBUF_TAG}" > "${PROTOBUF_STAMP}"
fi

RUNNER="${BIN_DIR}/conformance_test_runner"
RUNNER_STAMP="${WORK}/runner.tag"
EXPECTED_RUNNER_STAMP="$(printf '%s\n%s' "${PROTOBUF_TAG}" "${JSONCPP_TAG}")"
if [ -x "${RUNNER}" ] \
    && [ -f "${RUNNER_STAMP}" ] \
    && [ "$(cat "${RUNNER_STAMP}")" = "${EXPECTED_RUNNER_STAMP}" ]; then
    echo "conformance_test_runner already present at ${BIN_DIR} — skipping build."
    echo "(delete it to force a rebuild)"
else
    # The stamp goes first, so a build or copy that stops part-way leaves a
    # runner that the next run rebuilds.
    rm -f "${RUNNER_STAMP}"
    echo "=== Building conformance_test_runner (this takes a while) ==="
    cmake -B "${WORK}/protobuf/_build" -S "${WORK}/protobuf" \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_SHARED_LIBS=OFF \
        -Dprotobuf_BUILD_TESTS=OFF \
        -Dprotobuf_BUILD_CONFORMANCE=ON \
        -DCMAKE_PREFIX_PATH="${WORK}/jsoncpp-prefix"
    cmake --build "${WORK}/protobuf/_build" --target conformance_test_runner -j"${JOBS}"

    BUILT_RUNNER="$(find "${WORK}/protobuf/_build" -name conformance_test_runner -type f | head -1)"
    test -n "${BUILT_RUNNER}" || { echo "FAIL: conformance_test_runner not found"; exit 1; }
    cp "${BUILT_RUNNER}" "${RUNNER}"
    printf '%s\n' "${EXPECTED_RUNNER_STAMP}" > "${RUNNER_STAMP}"
    echo "Installed ${RUNNER}"
fi

echo "=== Populating ${PROTOS_DEST} ==="
for entry in "${PROTO_FILES[@]}"; do
    mkdir -p "${PROTOS_DEST}/${entry#* }"
    cp "${WORK}/protobuf/${entry% *}" "${PROTOS_DEST}/${entry#* }/"
done

echo "Done. Run the suite with: task conformance-local"
