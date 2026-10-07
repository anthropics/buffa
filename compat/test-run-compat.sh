#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
runner="${script_dir}/run-compat.sh"
expected_message="PROTOC_VERSIONS must contain at least one protoc version"

assert_rejected() {
  local versions="$1"
  local output
  if output=$(PROTOC_VERSIONS="$versions" bash "$runner" 2>&1); then
    printf 'expected rejection for version list %q\n' "$versions" >&2
    return 1
  fi
  if [[ "$output" != *"$expected_message"* ]]; then
    printf 'unexpected output for version list %q:\n%s\n' "$versions" "$output" >&2
    return 1
  fi
}

invalid_lists=("" " " "    " $'\t' $'\n' $' \t\n ')
for versions in "${invalid_lists[@]}"; do
  assert_rejected "$versions"
done

if output=$(env -u PROTOC_VERSIONS bash "$runner" 2>&1); then
  printf 'expected rejection when PROTOC_VERSIONS is unset\n' >&2
  exit 1
fi
if [[ "$output" != *"$expected_message"* ]]; then
  printf 'unexpected output when PROTOC_VERSIONS is unset:\n%s\n' "$output" >&2
  exit 1
fi

printf 'empty and whitespace-only version lists are rejected\n'
