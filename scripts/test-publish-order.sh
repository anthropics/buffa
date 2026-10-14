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

fail() {
  printf 'error - %s\n' "$1" >&2
  exit 1
}

# Run the checker on a list that it must reject, keeping stderr for the caller.
rejects() {
  local list="$1" what="$2"
  if "${checker}" "${list}" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
    fail "accepted ${what}"
  fi
}

# Run the checker on a list that it must accept.
accepts() {
  local list="$1" what="$2"
  if ! "${checker}" "${list}" >"${tmp}/stdout" 2>"${tmp}/stderr"; then
    printf 'error - rejected %s; the checker printed:\n' "${what}" >&2
    cat "${tmp}/stderr" >&2
    exit 1
  fi
}

# Require one whole line of the stderr that `rejects` captured.
expect_stderr_line() {
  if ! grep -Fx -- "$1" "${tmp}/stderr" >/dev/null; then
    printf 'error - expected this line on stderr:\n%s\nthe checker printed:\n' "$1" >&2
    cat "${tmp}/stderr" >&2
    exit 1
  fi
}

# Succeeds when crate $1 has a dependency on crate $2 that cargo publish keeps:
# every dependency except a dev-dependency without a version, which
# `cargo metadata` reports with the requirement "*". Matches by name alone,
# which is a simpler rule than the checker's own and is exact for a workspace
# whose crate names are unique.
depends_on() {
  jq -e --arg dependent "$1" --arg dependency "$2" '
    any(.packages[] | select(.name == $dependent) | .dependencies[];
      .name == $dependency and ((.kind == "dev" and .req == "*") | not))
  ' "${tmp}/metadata.json" >/dev/null
}

# Print the list with line $1 moved to the end (lines count from 1).
move_to_end() {
  sed -n "$1!p" "${tmp}/valid"
  sed -n "$1p" "${tmp}/valid"
}

"${checker}" --list >"${tmp}/valid"
accepts "${tmp}/valid" 'the configured publish order'
printf 'ok - accepts the configured publish order\n'

crates=()
while IFS= read -r crate; do
  crates+=("${crate}")
done <"${tmp}/valid"
count="${#crates[@]}"

# Swap each adjacent pair in turn. The swapped list is in dependency order
# exactly when the second crate of the pair does not depend on the first.
swaps_rejected=0
swaps_accepted=0
for ((i = 0; i + 1 < count; i++)); do
  first="${crates[i]}"
  second="${crates[i + 1]}"
  # `sed` swaps line i+1 with the line after it.
  sed "$((i + 1)){h;d;};$((i + 2)){G;}" "${tmp}/valid" >"${tmp}/swapped"
  if depends_on "${second}" "${first}"; then
    rejects "${tmp}/swapped" "${second} before its dependency ${first}"
    expect_stderr_line "  ${first} must appear before ${second}"
    swaps_rejected=$((swaps_rejected + 1))
  else
    accepts "${tmp}/swapped" "${second} before ${first}, which do not depend on each other"
    swaps_accepted=$((swaps_accepted + 1))
  fi
done
printf 'ok - adjacent swaps: %d rejected (a dependency after its dependent), %d accepted (independent crates)\n' \
  "${swaps_rejected}" "${swaps_accepted}"

# Move a dependency to the end of the list, which puts it after a dependent
# that need not be its neighbour. The pair is the first one found; a workspace
# with no dependency between publishable crates would leave the order check
# untested, so that is a failure here.
moved=''
for ((i = 0; i < count && ${#moved} == 0; i++)); do
  for ((j = i + 1; j < count; j++)); do
    if depends_on "${crates[j]}" "${crates[i]}"; then
      moved="${crates[i]}"
      dependent="${crates[j]}"
      move_to_end "$((i + 1))" >"${tmp}/moved"
      break
    fi
  done
done
[[ -n ${moved} ]] || fail 'found no dependency between publishable crates to test the order check with'
rejects "${tmp}/moved" "${moved} at the end of the list, after ${dependent}"
expect_stderr_line "  ${moved} must appear before ${dependent}"
printf 'ok - rejects a dependency listed after its dependent (%s after %s)\n' "${moved}" "${dependent}"

# A dev-dependency with a version is an ordering constraint too. Take a pair
# of listed crates whose only dependency is of that kind, when there is one.
dev_pair="$(jq -r --rawfile listed "${tmp}/valid" '
  ($listed | split("\n") | map(select(. != ""))) as $crates
  | first(
      .packages[]
      | select(.name as $n | $crates | index($n)) as $dependent
      | $dependent.dependencies[]
      | select(.kind == "dev" and .req != "*" and .path != null)
      | select(.name as $n | $crates | index($n))
      | select(.name as $n
          | any($dependent.dependencies[]; .name == $n and .kind != "dev") | not)
      | "\($dependent.name) \(.name)"
    ) // ""
' "${tmp}/metadata.json")"
if [[ -n ${dev_pair} ]]; then
  dependent="${dev_pair% *}"
  dependency="${dev_pair#* }"
  for ((i = 0; i < count; i++)); do
    if [[ ${crates[i]} == "${dependency}" ]]; then
      move_to_end "$((i + 1))" >"${tmp}/dev-moved"
    fi
  done
  rejects "${tmp}/dev-moved" "${dependency} after ${dependent}, which has a versioned dev-dependency on it"
  expect_stderr_line "  ${dependency} must appear before ${dependent}"
  printf 'ok - rejects a versioned dev-dependency listed after its dependent (%s after %s)\n' \
    "${dependency}" "${dependent}"
else
  printf 'skip - no publishable crate has a versioned dev-dependency on another\n'
fi

last="${crates[count - 1]}"
sed '$d' "${tmp}/valid" >"${tmp}/missing-crate"
rejects "${tmp}/missing-crate" "a list without the publishable crate ${last}"
expect_stderr_line "    ${last}"
printf 'ok - rejects missing publishable crates\n'

# A workspace crate with `publish = false` when there is one, and otherwise a
# name that is not in the workspace: the checker rejects both the same way.
unpublished="$(jq -r 'first(.packages[] | select(.publish != null) | .name) // "not-a-workspace-crate"' \
  "${tmp}/metadata.json")"
cp "${tmp}/valid" "${tmp}/extra-crate"
printf '%s\n' "${unpublished}" >>"${tmp}/extra-crate"
rejects "${tmp}/extra-crate" "the non-publishable crate ${unpublished}"
expect_stderr_line "    ${unpublished}"
printf 'ok - rejects non-publishable crates\n'

cp "${tmp}/valid" "${tmp}/duplicate-crate"
printf '%s\n' "${crates[0]}" >>"${tmp}/duplicate-crate"
rejects "${tmp}/duplicate-crate" "a list that names ${crates[0]} twice"
expect_stderr_line 'error: .github/publish-order.txt lists a crate more than once'
expect_stderr_line "    ${crates[0]}"
printf 'ok - rejects duplicate crates\n'

{
  printf '# leading comment\n\n'
  cat "${tmp}/valid"
  printf '  # trailing comment\n'
} >"${tmp}/comments-and-blanks"
accepts "${tmp}/comments-and-blanks" 'a list with comments and blank lines'
printf 'ok - ignores comments and blank lines\n'
