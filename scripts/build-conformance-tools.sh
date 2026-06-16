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

set -euo pipefail

PROTOBUF_TAG="${1:-v33.5}"
JSONCPP_TAG="${JSONCPP_TAG:-1.9.6}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${ROOT}/.local/conformance-tools"
BIN_DIR="${ROOT}/.local/bin"
PROTOS_DEST="${ROOT}/conformance/protos"
JOBS="$(nproc 2>/dev/null || echo 4)"

mkdir -p "${WORK}" "${BIN_DIR}"

if [ -x "${BIN_DIR}/conformance_test_runner" ]; then
    echo "conformance_test_runner already present at ${BIN_DIR} — skipping build."
    echo "(delete it to force a rebuild)"
else
    # ── jsoncpp (static lib + cmake package files) ──────────────────────────
    if [ ! -f "${WORK}/jsoncpp-prefix/lib/cmake/jsoncpp/jsoncppConfig.cmake" ]; then
        echo "=== Building jsoncpp ${JSONCPP_TAG} ==="
        rm -rf "${WORK}/jsoncpp"
        git clone --depth=1 --branch "${JSONCPP_TAG}" \
            https://github.com/open-source-parsers/jsoncpp.git "${WORK}/jsoncpp"
        cmake -B "${WORK}/jsoncpp/_build" -S "${WORK}/jsoncpp" \
            -DCMAKE_BUILD_TYPE=Release \
            -DBUILD_SHARED_LIBS=ON \
            -DJSONCPP_WITH_TESTS=OFF \
            -DJSONCPP_WITH_POST_BUILD_UNITTEST=OFF \
            -DCMAKE_INSTALL_PREFIX="${WORK}/jsoncpp-prefix"
        cmake --build "${WORK}/jsoncpp/_build" -j"${JOBS}"
        cmake --install "${WORK}/jsoncpp/_build"
    fi

    # ── protobuf (conformance_test_runner) ──────────────────────────────────
    if [ ! -d "${WORK}/protobuf" ]; then
        echo "=== Cloning protobuf ${PROTOBUF_TAG} ==="
        git clone --depth=1 --branch "${PROTOBUF_TAG}" \
            https://github.com/protocolbuffers/protobuf.git "${WORK}/protobuf"
    fi

    echo "=== Building conformance_test_runner (this takes a while) ==="
    cmake -B "${WORK}/protobuf/_build" -S "${WORK}/protobuf" \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_SHARED_LIBS=OFF \
        -Dprotobuf_BUILD_TESTS=OFF \
        -Dprotobuf_BUILD_CONFORMANCE=ON \
        -DCMAKE_PREFIX_PATH="${WORK}/jsoncpp-prefix"
    cmake --build "${WORK}/protobuf/_build" --target conformance_test_runner -j"${JOBS}"

    RUNNER="$(find "${WORK}/protobuf/_build" -name conformance_test_runner -type f | head -1)"
    test -n "${RUNNER}" || { echo "FAIL: conformance_test_runner not found"; exit 1; }
    cp "${RUNNER}" "${BIN_DIR}/conformance_test_runner"
    echo "Installed ${BIN_DIR}/conformance_test_runner"
fi

# ── Test protos (same set Dockerfile.tools packages into /protos) ──────────
SRC="${WORK}/protobuf"
if [ ! -d "${SRC}" ]; then
    echo "=== Cloning protobuf ${PROTOBUF_TAG} (protos only) ==="
    git clone --depth=1 --branch "${PROTOBUF_TAG}" \
        https://github.com/protocolbuffers/protobuf.git "${SRC}"
fi

echo "=== Populating ${PROTOS_DEST} ==="
mkdir -p "${PROTOS_DEST}/google/protobuf" \
         "${PROTOS_DEST}/editions/golden" \
         "${PROTOS_DEST}/conformance/test_protos"
cp "${SRC}/conformance/conformance.proto" "${PROTOS_DEST}/"
cp "${SRC}/src/google/protobuf/test_messages_proto3.proto" \
   "${SRC}/src/google/protobuf/test_messages_proto2.proto" \
   "${SRC}/src/google/protobuf/any.proto" \
   "${SRC}/src/google/protobuf/duration.proto" \
   "${SRC}/src/google/protobuf/field_mask.proto" \
   "${SRC}/src/google/protobuf/struct.proto" \
   "${SRC}/src/google/protobuf/timestamp.proto" \
   "${SRC}/src/google/protobuf/wrappers.proto" \
   "${PROTOS_DEST}/google/protobuf/"
cp "${SRC}/editions/golden/test_messages_proto2_editions.proto" \
   "${SRC}/editions/golden/test_messages_proto3_editions.proto" \
   "${PROTOS_DEST}/editions/golden/"
cp "${SRC}/conformance/test_protos/test_messages_edition2023.proto" \
   "${SRC}/conformance/test_protos/test_messages_edition_unstable.proto" \
   "${PROTOS_DEST}/conformance/test_protos/"

echo "Done. Run the suite with: task conformance-local"
