#!/usr/bin/env bash
# Run all fuzz targets in parallel with output redirected to log files.
# Prints periodic progress summaries.
# Exits nonzero if a target fails or creates a new crash, OOM, or timeout
# artifact. Artifacts that already exist when the run starts are only reported.
#
# Usage: scripts/fuzz-all.sh [max_total_time]
#   max_total_time: seconds per target (default: 28800 = 8 hours)
#
# Logs are written to /tmp/buffa-fuzz/<target>.log
# Crashes are saved to fuzz/artifacts/<target>/

set -euo pipefail

MAX_TIME="${1:-28800}"
TARGETS=(decode_proto3 decode_proto2 decode_wkt json_roundtrip encode_proto3 wkt_json_strings)
LOG_DIR="/tmp/buffa-fuzz"
FUZZ_DIR="fuzz"
STATUS_INTERVAL=300  # seconds between progress reports

# Per-target extra libFuzzer flags.
#
# encode_proto3: uses Arbitrary<TestAllTypesProto3> — a few input bytes can
# generate structs with many heap allocations (Vecs, Strings, HashMaps, nested
# Boxes). Under ASan's allocator this causes ~2KB/iter unrecovered RSS growth
# (heap fragmentation; ASan never returns to OS). Hit the 2GB rss limit after
# ~1.6M iterations (~26 min) — see oom-f1a736fc, Mar 2026. -fork=1 spawns
# child processes that reset memory per job. Verified: oom=0 over 13 jobs.
# Trade-off: ~45% throughput loss from per-job corpus reload.
#
# The other 5 targets take bounded raw-byte/string input (max 4KB from
# libFuzzer's default -max_len), so their allocations are naturally capped
# and they run 14hr+ without fragmentation OOM.
target_extra_flags() {
    case "$1" in
        encode_proto3) echo "-fork=1 -ignore_ooms=0 -ignore_crashes=0 -ignore_timeouts=0" ;;
        *)             echo "" ;;
    esac
}

count_crash_artifacts() {
    local artifact_dir="$FUZZ_DIR/artifacts/$1"
    if [[ ! -d "$artifact_dir" ]]; then
        echo 0
        return
    fi

    find "$artifact_dir" -type f \( \
        -name 'crash-*' -o -name 'oom-*' -o -name 'timeout-*' \
    \) 2>/dev/null | wc -l | tr -d '[:space:]'
}

filter_fuzz_output() {
    local status_file="$1"
    local log="$2"
    while IFS= read -r line; do
        # Always save the most recent progress line for status reports.
        # Non-fork mode: "#NNN ACTION cov: ..."; fork mode: "#NNN: cov: ...".
        if [[ "$line" =~ ^#[0-9] ]]; then
            echo "$line" >"$status_file"
        fi
        # Log important lines only: crashes, errors, stats, summary.
        if [[ "$line" =~ SUMMARY|ERROR|CRASH|ALARM|panic|assertion|stat::|BINGO|Done[[:space:]] ]]; then
            echo "$line" >>"$log"
        fi
    done
}

run_target() {
    local target="$1"
    local log="$2"
    local status_file="$3"
    local extra_flags="$4"

    # Keep the pipeline in the wrapper's process group so cleanup can terminate
    # cargo-fuzz, the fuzz binary, and the output filter together.
    set +m
    # shellcheck disable=SC2086  # extra_flags is intentionally word-split
    cargo +nightly fuzz run --fuzz-dir "$FUZZ_DIR" "$target" \
        -- -max_total_time="$MAX_TIME" -print_final_stats=1 $extra_flags \
        2>&1 | filter_fuzz_output "$status_file" "$log"
}

# Wait for each completed target exactly once and preserve its exit code.
# With wait_all=true, block until every still-running target has exited.
record_finished_targets() {
    local wait_all="${1:-false}"
    for i in "${!PIDS[@]}"; do
        if [[ -n "${EXIT_CODES[$i]:-}" ]]; then
            continue
        fi

        local pid="${PIDS[$i]}"
        if [[ "$wait_all" == false ]] && kill -0 "$pid" 2>/dev/null; then
            continue
        fi

        local exit_code
        if wait "$pid" 2>/dev/null; then
            exit_code=0
        else
            exit_code=$?
        fi
        EXIT_CODES[i]="$exit_code"
    done
}

cleanup() {
    local exit_code="$1"
    trap - INT TERM

    # A signal may arrive after a wrapper starts but before its PID is stored.
    if [[ "$STARTING_TARGET" == true ]] && [[ -n "${!:-}" ]]; then
        local pending_pid=$!
        local last_index=$((${#PIDS[@]} - 1))
        if [[ "$last_index" -lt 0 ]] || [[ "${PIDS[$last_index]}" != "$pending_pid" ]]; then
            PIDS+=("$pending_pid")
        fi
    fi

    echo ""
    echo "Stopping all fuzz targets..."
    for i in "${!PIDS[@]}"; do
        if [[ -z "${EXIT_CODES[$i]:-}" ]]; then
            kill -TERM -- "-${PIDS[$i]}" 2>/dev/null || true
        fi
    done
    record_finished_targets true
    if declare -F print_summary >/dev/null; then
        print_summary
    fi
    exit "$exit_code"
}

mkdir -p "$LOG_DIR"

# Build all targets first (sequentially, to avoid parallel compilation issues).
echo "Building fuzz targets..."
for target in "${TARGETS[@]}"; do
    cargo +nightly fuzz build --fuzz-dir "$FUZZ_DIR" "$target" 2>&1 \
        | tail -1
done
echo ""

# Launch all targets in parallel.
# Output is filtered through a helper that:
#   - Keeps only important lines (crashes, errors, final stats) in the log
#   - Maintains a "last status" file with the most recent progress line
PIDS=()
EXIT_CODES=()
INITIAL_ARTIFACT_COUNTS=()
STARTING_TARGET=false
trap 'cleanup 130' INT
trap 'cleanup 143' TERM
for target in "${TARGETS[@]}"; do
    log="$LOG_DIR/$target.log"
    status_file="$LOG_DIR/$target.status"
    echo "Starting $target (log: $log, max_time: ${MAX_TIME}s)"
    : >"$log"
    : >"$status_file"
    extra_flags=$(target_extra_flags "$target")
    INITIAL_ARTIFACT_COUNTS+=("$(count_crash_artifacts "$target")")
    EXIT_CODES+=("")

    # Give the target wrapper and all of its descendants a dedicated process
    # group. The wrapper preserves pipefail even when wait is delayed, including
    # on Bash 3.2, and cleanup can terminate the whole group without orphans.
    record_finished_targets
    STARTING_TARGET=true
    set -m 2>/dev/null
    run_target "$target" "$log" "$status_file" "$extra_flags" &
    PIDS+=("$!")
    STARTING_TARGET=false
    set +m
done

echo ""
echo "All ${#TARGETS[@]} targets running. Logs in $LOG_DIR/"
echo "Press Ctrl-C to stop all targets."
echo ""

# Read the most recent progress line from the status file.
parse_stats() {
    local target="$1"
    local status_file="$LOG_DIR/$target.status"
    if [[ ! -f "$status_file" ]]; then
        echo "not started"
        return
    fi
    local last
    last=$(cat "$status_file" 2>/dev/null || true)
    if [[ -n "$last" ]]; then
        # Trim to the useful part: #NNN ACTION cov: X ft: Y
        echo "$last" | grep -oP '#\d+\s+\S+\s+cov: \d+\s+ft: \d+' || echo "$last"
    else
        echo "starting up..."
    fi
}

print_summary() {
    echo "── Fuzz progress $(date +%H:%M:%S) ──"
    for i in "${!TARGETS[@]}"; do
        local target="${TARGETS[$i]}"
        local status
        local exit_code="${EXIT_CODES[$i]}"
        if [[ -z "$exit_code" ]]; then
            status="running"
        elif [[ "$exit_code" -eq 0 ]]; then
            status="finished"
        else
            status="FAILED (exit $exit_code)"
        fi
        local stats
        stats=$(parse_stats "$target")
        printf "  %-20s [%s] %s\n" "$target" "$status" "$stats"

        # Check for crash artifacts.
        local artifact_dir="$FUZZ_DIR/artifacts/$target"
        local crashes
        crashes=$(count_crash_artifacts "$target")
        if [[ "$crashes" -gt 0 ]]; then
            printf "    *** %d crash artifact(s) in %s ***\n" "$crashes" "$artifact_dir"
        fi
    done
    echo ""
}

# Wait loop: periodic status reports until all targets finish.
# Check every 5 seconds whether targets are still running, but only
# print a summary every STATUS_INTERVAL seconds.
elapsed=0
while true; do
    record_finished_targets
    all_done=true
    for exit_code in "${EXIT_CODES[@]}"; do
        if [[ -z "$exit_code" ]]; then
            all_done=false
            break
        fi
    done

    if $all_done; then
        print_summary
        echo "All targets finished."
        break
    fi

    sleep 5
    elapsed=$((elapsed + 5))
    if [[ $elapsed -ge $STATUS_INTERVAL ]]; then
        print_summary
        elapsed=0
    fi
done

failed_targets=()
for i in "${!TARGETS[@]}"; do
    target="${TARGETS[$i]}"
    reasons=()

    if [[ "${EXIT_CODES[$i]}" -ne 0 ]]; then
        reasons+=("exit ${EXIT_CODES[$i]}")
    fi

    artifact_count=$(count_crash_artifacts "$target")
    new_artifact_count=$((artifact_count - INITIAL_ARTIFACT_COUNTS[i]))
    if [[ "$new_artifact_count" -gt 0 ]]; then
        reasons+=("$new_artifact_count new crash artifact(s)")
    fi

    if [[ ${#reasons[@]} -gt 0 ]]; then
        reason="${reasons[0]}"
        if [[ ${#reasons[@]} -gt 1 ]]; then
            reason+=", ${reasons[1]}"
        fi
        failed_targets+=("$target ($reason)")
    fi
done

if [[ ${#failed_targets[@]} -gt 0 ]]; then
    echo "Fuzzing failed for:"
    printf "  %s\n" "${failed_targets[@]}"
    exit 1
fi
