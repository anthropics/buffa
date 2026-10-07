#!/usr/bin/env bash

set -euo pipefail

SOURCE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "${SOURCE_ROOT}/.local"
TEST_ROOT="$(mktemp -d "${SOURCE_ROOT}/.local/conformance-tools-test.XXXXXX")"
MOCK_BIN="${TEST_ROOT}/mock-bin"
MOCK_LOG="${TEST_ROOT}/mock-log"
mkdir -p "${TEST_ROOT}/scripts" "${TEST_ROOT}/conformance" "${MOCK_BIN}" "${MOCK_LOG}"
cp "${SOURCE_ROOT}/scripts/build-conformance-tools.sh" "${TEST_ROOT}/scripts/"
trap 'rm -rf "${TEST_ROOT}"' EXIT

cat > "${MOCK_BIN}/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

test "$1" = clone
tag="$4"
url="$5"
dest="$6"
mkdir -p "${dest}"
printf '%s %s\n' "${url##*/}" "${tag}" >> "${MOCK_LOG}/git-clones"
if [[ "${url}" == *protobuf.git ]]; then
    for path in \
        conformance/conformance.proto \
        src/google/protobuf/test_messages_proto3.proto \
        src/google/protobuf/test_messages_proto2.proto \
        src/google/protobuf/any.proto \
        src/google/protobuf/duration.proto \
        src/google/protobuf/field_mask.proto \
        src/google/protobuf/struct.proto \
        src/google/protobuf/timestamp.proto \
        src/google/protobuf/wrappers.proto \
        editions/golden/test_messages_proto2_editions.proto \
        editions/golden/test_messages_proto3_editions.proto \
        conformance/test_protos/test_messages_edition2023.proto \
        conformance/test_protos/test_messages_edition_unstable.proto; do
        mkdir -p "${dest}/$(dirname "${path}")"
        : > "${dest}/${path}"
    done
fi
EOF

cat > "${MOCK_BIN}/cmake" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [ "$1" = --build ]; then
    build_dir="$2"
    if [[ "${build_dir}" == */protobuf/_build ]]; then
        printf 'runner\n' >> "${MOCK_LOG}/cmake-builds"
        mkdir -p "${build_dir}"
        printf '#!/usr/bin/env bash\nexit 0\n' > "${build_dir}/conformance_test_runner"
        chmod +x "${build_dir}/conformance_test_runner"
    else
        printf 'jsoncpp\n' >> "${MOCK_LOG}/cmake-builds"
    fi
elif [ "$1" = --install ]; then
    build_dir="$2"
    prefix="$(cat "${build_dir}/.install-prefix")"
    mkdir -p "${prefix}/lib/cmake/jsoncpp"
    : > "${prefix}/lib/cmake/jsoncpp/jsoncppConfig.cmake"
    : > "${prefix}/lib/libjsoncpp.so"
else
    build_dir=""
    prefix=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            -B)
                build_dir="$2"
                shift 2
                ;;
            -DCMAKE_INSTALL_PREFIX=*)
                prefix="${1#*=}"
                shift
                ;;
            *)
                shift
                ;;
        esac
    done
    mkdir -p "${build_dir}"
    if [ -n "${prefix}" ]; then
        printf '%s\n' "${prefix}" > "${build_dir}/.install-prefix"
    fi
fi
EOF

chmod +x "${MOCK_BIN}/git" "${MOCK_BIN}/cmake"

run_build() {
    if ! PATH="${MOCK_BIN}:${PATH}" MOCK_LOG="${MOCK_LOG}" JSONCPP_TAG="$2" \
        "${TEST_ROOT}/scripts/build-conformance-tools.sh" "$1" > "${TEST_ROOT}/output" 2>&1; then
        cat "${TEST_ROOT}/output" >&2
        return 1
    fi
}

assert_count() {
    local file="$1"
    local pattern="$2"
    local expected="$3"
    local actual
    actual="$(grep -c "^${pattern}" "${file}" || true)"
    if [ "${actual}" != "${expected}" ]; then
        printf 'expected %s matching %s lines in %s, got %s\n' "${expected}" "${pattern}" "${file}" "${actual}" >&2
        exit 1
    fi
}

run_build v33.5 1.9.6
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 1
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 1
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 1
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 1

run_build v33.5 1.9.6
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 1
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 1
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 1
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 1

run_build v34.0 1.9.6
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 2
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 1
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 2
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 1
test "$(cat "${TEST_ROOT}/.local/conformance-tools/protobuf.tag")" = v34.0

run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 2
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 2
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 3
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 2
test "$(cat "${TEST_ROOT}/.local/conformance-tools/runner-tags")" = "$(printf 'v34.0\n1.9.7')"

rm "${TEST_ROOT}/.local/conformance-tools/runner-tags"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4

rm -rf "${TEST_ROOT}/.local/conformance-tools/protobuf"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 3
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4
test -f "${TEST_ROOT}/conformance/protos/conformance.proto"

rm "${TEST_ROOT}/.local/conformance-tools/protobuf/conformance/test_protos/test_messages_edition_unstable.proto"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 4
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4
test -f "${TEST_ROOT}/conformance/protos/conformance/test_protos/test_messages_edition_unstable.proto"

rm "${TEST_ROOT}/.local/conformance-tools/jsoncpp-prefix/lib/libjsoncpp.so"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 3
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 3
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4

rm "${TEST_ROOT}/.local/bin/conformance_test_runner"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 5

printf 'conformance tool cache checks passed\n'
