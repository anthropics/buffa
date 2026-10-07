#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checker="${repo_root}/scripts/check-publish-coverage.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

"${checker}" --list >"${tmp}/valid"
"${checker}" "${tmp}/valid" >/dev/null
printf 'ok - accepts the configured publish order\n'

awk '
  $0 == "buffa-descriptor" { next }
  $0 == "buffa-types" {
    print "buffa-types"
    print "buffa-descriptor"
    found = 1
    next
  }
  { print }
  END { if (!found) exit 1 }
' "${tmp}/valid" >"${tmp}/wrong-dependency-order"
if "${checker}" "${tmp}/wrong-dependency-order" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
  printf 'error - accepted buffa-types before buffa-descriptor\n' >&2
  exit 1
fi
grep -F 'buffa-descriptor must appear before buffa-types' "${tmp}/stderr" >/dev/null
printf 'ok - rejects a local dependency published too late\n'

awk '
  $0 == "protoc-gen-buffa" { print "protoc-gen-buffa-packaging"; found_proto = 1; next }
  $0 == "protoc-gen-buffa-packaging" { print "protoc-gen-buffa"; found_packaging = 1; next }
  { print }
  END { if (!found_proto || !found_packaging) exit 1 }
' "${tmp}/valid" >"${tmp}/independent-order"
"${checker}" "${tmp}/independent-order" >/dev/null
printf 'ok - permits independent crates in either order\n'

awk '$0 != "buffa-build"' "${tmp}/valid" >"${tmp}/missing-crate"
if "${checker}" "${tmp}/missing-crate" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
  printf 'error - accepted a missing publishable crate\n' >&2
  exit 1
fi
grep -F 'buffa-build' "${tmp}/stderr" >/dev/null
printf 'ok - still rejects missing publishable crates\n'

cp "${tmp}/valid" "${tmp}/extra-crate"
printf 'buffa-yaml\n' >>"${tmp}/extra-crate"
if "${checker}" "${tmp}/extra-crate" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
  printf 'error - accepted a non-publishable crate\n' >&2
  exit 1
fi
grep -F 'buffa-yaml' "${tmp}/stderr" >/dev/null
printf 'ok - still rejects non-publishable crates\n'

cp "${tmp}/valid" "${tmp}/duplicate-crate"
printf 'buffa\n' >>"${tmp}/duplicate-crate"
if "${checker}" "${tmp}/duplicate-crate" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
  printf 'error - accepted a duplicate crate\n' >&2
  exit 1
fi
printf 'ok - still rejects duplicate crates\n'

{
  printf '# leading comment\n\n'
  cat "${tmp}/valid"
  printf '  # trailing comment\n'
} >"${tmp}/comments-and-blanks"
"${checker}" "${tmp}/comments-and-blanks" >/dev/null
printf 'ok - ignores comments and blank lines\n'
