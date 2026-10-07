#!/usr/bin/env bash
#
# Verify .github/publish-order.txt lists exactly the publishable crates in the
# workspace.
#
# buffa v0.9.0 shipped with buffa-remote-derive announced in the changelog but
# absent from crates.io: the publish workflow carried a hand-written list of
# steps and nobody added the new crate. Nothing failed, because nothing was
# comparing that list to the workspace. This script is that comparison, and it
# runs on every pull request so a new crate is caught when it is introduced
# rather than at release time.

#
# With --list, print the crate names in publish order instead of checking, so
# the publish workflow can drive its loop without a second copy of the parsing
# rules.

set -euo pipefail

list_only=false
if [[ ${1:-} == --list ]]; then
  list_only=true
  shift
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
order_file="${1:-${repo_root}/.github/publish-order.txt}"

if [[ ! -f ${order_file} ]]; then
  echo "error: publish order file not found: ${order_file}" >&2
  exit 2
fi

# Strip comments and blank lines, preserving order.
clean_list() {
  sed -E 's/#.*//; s/[[:space:]]+$//' "${order_file}" | grep -vE '^[[:space:]]*$'
}

if [[ ${list_only} == true ]]; then
  clean_list
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# A package is publishable unless its manifest sets `publish = false`, which
# cargo reports as a null `publish` field.
cargo metadata --no-deps --format-version 1 --manifest-path "${repo_root}/Cargo.toml" \
  >"${tmp}/metadata.json"
jq -r '.packages[] | select(.publish == null) | .name' "${tmp}/metadata.json" \
  | sort >"${tmp}/publishable"

clean_list >"${tmp}/listed"
sort "${tmp}/listed" >"${tmp}/listed.sorted"

if ! diff -q "${tmp}/publishable" "${tmp}/listed.sorted" >/dev/null; then
  # comm splits the mismatch into the two directions, which need different fixes.
  missing="$(comm -23 "${tmp}/publishable" "${tmp}/listed.sorted")"
  extra="$(comm -13 "${tmp}/publishable" "${tmp}/listed.sorted")"

  echo "error: .github/publish-order.txt does not match the workspace" >&2
  echo >&2

  if [[ -n ${missing} ]]; then
    echo "  Publishable but NOT listed — a release would silently skip these:" >&2
    echo "${missing}" | sed 's/^/    /' >&2
    echo >&2
    echo "  Add each to .github/publish-order.txt in dependency order (after the" >&2
    echo "  crates it depends on), or set 'publish = false' in its Cargo.toml if" >&2
    echo "  it is deliberately not released." >&2
    echo >&2
  fi

  if [[ -n ${extra} ]]; then
    echo "  Listed but NOT publishable — the publish step would fail on these:" >&2
    echo "${extra}" | sed 's/^/    /' >&2
    echo >&2
    echo "  Remove each from .github/publish-order.txt, or drop 'publish = false'" >&2
    echo "  from its Cargo.toml if it should be released after all." >&2
    echo >&2
  fi

  exit 1
fi

jq -r '
  .packages as $packages
  | $packages[]
  | select(.publish == null) as $dependent
  | $dependent.dependencies[]
  | select(.path != null and .kind != "dev") as $dependency
  | select(any($packages[];
      .publish == null
      and .name == $dependency.name
      and (.manifest_path | sub("/Cargo.toml$"; "")) == $dependency.path
    ))
  | [$dependency.name, $dependent.name]
  | @tsv
' "${tmp}/metadata.json" | sort -u >"${tmp}/dependencies"

if ! awk '
  NR == FNR {
    if ($0 != "") position[$0] = NR
    next
  }
  {
    dependency = $1
    dependent = $2
    if (position[dependency] >= position[dependent]) {
      printf "  %s must appear before %s\n", dependency, dependent
      invalid = 1
    }
  }
  END { exit invalid }
' "${tmp}/listed" "${tmp}/dependencies" >"${tmp}/order-errors"; then
  echo "error: .github/publish-order.txt puts a crate before its local dependency" >&2
  cat "${tmp}/order-errors" >&2
  echo "  Move each dependency above the crate that depends on it." >&2
  exit 1
fi

echo "publish coverage and dependency order OK: $(wc -l <"${tmp}/listed" | tr -d ' ') crates listed"
