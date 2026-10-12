#!/usr/bin/env bash
#
# Tests the cache decisions of scripts/build-conformance-tools.sh: which runs
# clone or build again, and which reuse what is there.
#
# The build script runs from a copy in a temporary directory under .local/,
# with mock `git` and `cmake` first on PATH. The mocks create the files the
# real tools would and log each clone and build; the test does not access the
# network and compiles nothing.

set -euo pipefail

SOURCE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "${SOURCE_ROOT}/.local"
TEST_ROOT="$(mktemp -d "${SOURCE_ROOT}/.local/conformance-tools-test.XXXXXX")"
trap 'rm -rf "${TEST_ROOT}"' EXIT
MOCK_BIN="${TEST_ROOT}/mock-bin"
MOCK_LOG="${TEST_ROOT}/mock-log"
mkdir -p "${TEST_ROOT}/scripts" "${TEST_ROOT}/conformance" "${MOCK_BIN}" "${MOCK_LOG}"
cp "${SOURCE_ROOT}/scripts/build-conformance-tools.sh" "${TEST_ROOT}/scripts/"

cat > "${MOCK_BIN}/git" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

test "$1" = clone
tag="$4"
url="$5"
dest="$6"
# MOCK_FAIL names one step that fails, as an interrupted run would.
if [ "${MOCK_FAIL:-}" = jsoncpp-clone ] && [[ "${url}" == *jsoncpp.git ]]; then
    exit 1
fi
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
        if [ "${MOCK_FAIL:-}" = runner-build ]; then
            exit 1
        fi
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
    libdir="$(cat "${build_dir}/.install-libdir")"
    mkdir -p "${prefix}/${libdir}/cmake/jsoncpp"
    : > "${prefix}/${libdir}/cmake/jsoncpp/jsoncppConfig.cmake"
    : > "${prefix}/${libdir}/libjsoncpp.so"
else
    build_dir=""
    prefix=""
    # The mock stands for a host where CMake installs libraries under lib64
    # unless the configure step names the directory.
    libdir="lib64"
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
            -DCMAKE_INSTALL_LIBDIR=*)
                libdir="${1#*=}"
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
        printf '%s\n' "${libdir}" > "${build_dir}/.install-libdir"
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

# Run the build with one mocked step failing; the run itself must fail.
run_failing_build() {
    if MOCK_FAIL="$3" PATH="${MOCK_BIN}:${PATH}" MOCK_LOG="${MOCK_LOG}" JSONCPP_TAG="$2" \
        "${TEST_ROOT}/scripts/build-conformance-tools.sh" "$1" > "${TEST_ROOT}/output" 2>&1; then
        printf 'expected the build to fail with the %s step failing\n' "$3" >&2
        exit 1
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

assert_file() {
    if [ ! -f "$1" ]; then
        printf 'expected the file %s to exist\n' "$1" >&2
        exit 1
    fi
}

assert_no_file() {
    if [ -e "$1" ]; then
        printf 'expected %s to be absent\n' "$1" >&2
        exit 1
    fi
}

assert_content() {
    local file="$1"
    local expected="$2"
    local actual
    actual="$(cat "${file}" 2>/dev/null || true)"
    if [ "${actual}" != "${expected}" ]; then
        printf 'expected %s to contain:\n%s\ngot:\n%s\n' "${file}" "${expected}" "${actual}" >&2
        exit 1
    fi
}

TOOLS="${TEST_ROOT}/.local/conformance-tools"

# An empty tree: clones both repositories, builds jsoncpp and the runner.
run_build v33.5 1.9.6
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 1
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 1
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 1
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 1

# The same tags again: reuses everything. The mock cmake installs under lib64
# unless the build script names the library directory, so this run also fails
# if the script looks for jsoncpp where the install did not put it.
run_build v33.5 1.9.6
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 1
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 1
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 1
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 1

# A new protobuf tag: clones protobuf again and rebuilds the runner; jsoncpp
# is reused.
run_build v34.0 1.9.6
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 2
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 1
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 2
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 1
assert_content "${TOOLS}/protobuf.tag" v34.0

# A new jsoncpp tag: rebuilds jsoncpp and the runner; protobuf is reused.
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 2
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 2
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 3
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 2
assert_content "${TOOLS}/runner.tag" "$(printf 'v34.0\n1.9.7')"

# The runner stamp is missing: rebuilds the runner only.
rm "${TOOLS}/runner.tag"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4

# The protobuf checkout is missing: clones it again and copies the protos; the
# runner, built from the same tag, is reused.
rm -rf "${TOOLS}/protobuf"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 3
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4
assert_file "${TEST_ROOT}/conformance/protos/conformance.proto"

# One test proto is missing from the checkout: clones protobuf again; the
# runner is reused.
rm "${TOOLS}/protobuf/conformance/test_protos/test_messages_edition_unstable.proto"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'protobuf.git ' 4
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4
assert_file "${TEST_ROOT}/conformance/protos/conformance/test_protos/test_messages_edition_unstable.proto"

# The jsoncpp library is missing from its prefix: rebuilds jsoncpp; the
# runner, built with the same tag, is reused.
rm "${TOOLS}/jsoncpp-prefix/lib/libjsoncpp.so"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/git-clones" 'jsoncpp.git ' 3
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 3
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 4

# The runner binary is missing: rebuilds the runner only.
rm "${TEST_ROOT}/.local/bin/conformance_test_runner"
run_build v34.0 1.9.7
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 5

# A jsoncpp rebuild that fails leaves no jsoncpp stamp, so the old tag is not
# reported for a prefix that is gone; the next run builds jsoncpp and the
# runner.
run_failing_build v34.0 1.9.8 jsoncpp-clone
assert_no_file "${TOOLS}/jsoncpp.tag"
run_build v34.0 1.9.8
assert_count "${MOCK_LOG}/cmake-builds" 'jsoncpp' 4
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 6
assert_content "${TOOLS}/jsoncpp.tag" 1.9.8

# A runner rebuild that fails leaves no runner stamp, so the runner from the
# old tag is not taken for the new one; the next run builds it.
run_failing_build v35.0 1.9.8 runner-build
assert_no_file "${TOOLS}/runner.tag"
run_build v35.0 1.9.8
assert_count "${MOCK_LOG}/cmake-builds" 'runner' 7
assert_content "${TOOLS}/runner.tag" "$(printf 'v35.0\n1.9.8')"

printf 'conformance tool cache checks passed\n'
