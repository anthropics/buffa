#!/usr/bin/env bash
# Compile each generated crate in isolation so workspace feature unification
# cannot supply a missing std feature through another workspace member.
set -euo pipefail
if (( $# > 1 )); then
    printf '%s\n' 'usage: check-arbitrary-features.sh [target]' >&2
    exit 2
fi
repo_root="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# Rustup and Cargo discover toolchains and configuration from the working
# directory, even when each manifest path is absolute.
cd "$repo_root"
check_args=(--locked --no-default-features)
if (( $# == 1 )); then
    check_args+=(--target "$1")
fi
for package in buffa-types buffa-descriptor buffa-proto-options; do
    for features in arbitrary arbitrary,json; do
        cargo check --manifest-path "$repo_root/$package/Cargo.toml" "${check_args[@]}" --features "$features"
    done
done
