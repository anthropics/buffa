#!/usr/bin/env bash
#
# Every third-party GitHub Action in .github/workflows must be pinned to a
# full commit SHA. A movable tag (@v4, @stable, @master) is resolved again
# on each run, so a retag upstream executes code this repository never reviewed.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0

while IFS= read -r file; do
  while IFS= read -r line; do
    ref="${line##*@}"
    ref="${ref%% *}"
    ref="${ref%%#*}"
    if [[ ! ${ref} =~ ^[0-9a-f]{40}$ ]]; then
      echo "error: ${file}: unpinned action: ${line}" >&2
      fail=1
    fi
  done < <(grep -E '^[[:space:]]*(- )?uses: [^ ]+@' "${file}" || true)
done < <(find "${repo_root}/.github/workflows" -type f \( -name '*.yml' -o -name '*.yaml' \))

if [[ ${fail} -ne 0 ]]; then
  echo "Pin each action to a 40-character commit SHA and keep the old tag in a trailing comment." >&2
  exit 1
fi
