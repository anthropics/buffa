#!/usr/bin/env bash
#
# Tests for scripts/check-publish-coverage.sh.
#
# Every case is built from .github/publish-order.txt and `cargo metadata`, so
# the cases hold when a crate is added, removed, or made publishable.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checker="${repo_root}/scripts/check-publish-coverage.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

cargo metadata --no-deps --format-version 1 --manifest-path "${repo_root}/Cargo.toml" \
  >"${tmp}/metadata.json"

# Run the checker on a list that it must reject, keeping stderr for the caller.
rejects() {
  local list="$1" what="$2"
  if "${checker}" "${list}" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
    printf 'error - accepted %s\n' "${what}" >&2
    exit 1
  fi
}

# Succeeds when crate $1 has a dependency on crate $2 that is not a
# dev-dependency. Matches by name alone, which is a simpler rule than the
# checker's own and is exact for a workspace whose crate names are unique.
depends_on() {
  jq -e --arg dependent "$1" --arg dependency "$2" '
    any(.packages[] | select(.name == $dependent) | .dependencies[];
      .name == $dependency and .kind != "dev")
  ' "${tmp}/metadata.json" >/dev/null
}

"${checker}" --list >"${tmp}/valid"
"${checker}" "${tmp}/valid" >/dev/null
printf 'ok - accepts the configured publish order\n'

# Swap each adjacent pair in turn. The swapped list is in dependency order
# exactly when the second crate of the pair does not depend on the first.
crates=()
while IFS= read -r crate; do
  crates+=("${crate}")
done <"${tmp}/valid"
swaps_rejected=0
swaps_accepted=0
for ((i = 0; i + 1 < ${#crates[@]}; i++)); do
  first="${crates[i]}"
  second="${crates[i + 1]}"
  # `sed` swaps line i+1 with the line after it (lines count from 1).
  sed "$((i + 1)){h;d;};$((i + 2)){G;}" "${tmp}/valid" >"${tmp}/swapped"
  if depends_on "${second}" "${first}"; then
    rejects "${tmp}/swapped" "${second} before its dependency ${first}"
    grep -Fx "  ${first} must appear before ${second}" "${tmp}/stderr" >/dev/null
    swaps_rejected=$((swaps_rejected + 1))
  else
    if ! "${checker}" "${tmp}/swapped" >/dev/null 2>"${tmp}/stderr"; then
      printf 'error - rejected %s before %s, which do not depend on each other\n' \
        "${second}" "${first}" >&2
      cat "${tmp}/stderr" >&2
      exit 1
    fi
    swaps_accepted=$((swaps_accepted + 1))
  fi
done
printf 'ok - rejects a dependency listed after its dependent (%d of %d adjacent swaps)\n' \
  "${swaps_rejected}" "$((swaps_rejected + swaps_accepted))"
printf 'ok - permits independent crates in either order (%d of %d adjacent swaps)\n' \
  "${swaps_accepted}" "$((swaps_rejected + swaps_accepted))"

last="${crates[${#crates[@]} - 1]}"
sed '$d' "${tmp}/valid" >"${tmp}/missing-crate"
rejects "${tmp}/missing-crate" 'a list with a publishable crate missing'
grep -Fx "    ${last}" "${tmp}/stderr" >/dev/null
printf 'ok - still rejects missing publishable crates\n'

# A workspace crate with `publish = false` when there is one, and otherwise a
# name that is not in the workspace: the checker rejects both the same way.
unpublished="$(jq -r 'first(.packages[] | select(.publish != null) | .name) // "not-a-workspace-crate"' \
  "${tmp}/metadata.json")"
cp "${tmp}/valid" "${tmp}/extra-crate"
printf '%s\n' "${unpublished}" >>"${tmp}/extra-crate"
rejects "${tmp}/extra-crate" "the non-publishable crate ${unpublished}"
grep -Fx "    ${unpublished}" "${tmp}/stderr" >/dev/null
printf 'ok - still rejects non-publishable crates\n'

cp "${tmp}/valid" "${tmp}/duplicate-crate"
printf '%s\n' "${crates[0]}" >>"${tmp}/duplicate-crate"
rejects "${tmp}/duplicate-crate" 'a duplicate crate'
printf 'ok - still rejects duplicate crates\n'

{
  printf '# leading comment\n\n'
  cat "${tmp}/valid"
  printf '  # trailing comment\n'
} >"${tmp}/comments-and-blanks"
"${checker}" "${tmp}/comments-and-blanks" >/dev/null
printf 'ok - ignores comments and blank lines\n'
