#!/usr/bin/env bash
# Run all fuzz targets in parallel with output redirected to log files.
# Prints periodic progress summaries.
# Exits nonzero if a target fails. libFuzzer already exits nonzero on a
# crash/OOM/timeout/leak (non-fork mode inherently; the one -fork=1 target via
# -ignore_*=0), so the exit code is the primary signal. Scanning for new
# artifacts is a backstop for the case where cargo-fuzz masks that status.
# Artifacts already present when the run starts are reported but do not fail
# it; note fuzz/artifacts/ is shared, so a concurrent manual `cargo fuzz run`
# can trip the backstop.
#
# On a signal, every target's process group is sent TERM and waited for, then
# the script exits 128+signum (130 Ctrl-C, 143 TERM, 129 HUP, 131 QUIT). It
# yields no pass/fail verdict, even if the summary already reported a crash.
#
# Per-target logs keep everything a target printed before its first progress
# line, then only crash/error/stat lines. A target that dies during startup
# therefore has its cause logged rather than an empty file.
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
INTERRUPTED=false    # set by cleanup, so the summary can say so

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

    # leak-<sha> (LSan) counts too: it is a real finding. slow-unit-<sha>
    # deliberately does not — libFuzzer writes it for a unit over
    # -report_slow_units and then keeps going and exits 0, so counting it
    # would fail runs that libFuzzer considers clean.
    find "$artifact_dir" -type f \( \
        -name 'crash-*' -o -name 'oom-*' -o -name 'timeout-*' -o -name 'leak-*' \
    \) 2>/dev/null | wc -l | tr -d '[:space:]'
}

filter_fuzz_output() {
    local status_file="$1"
    local log="$2"
    # Everything before the first progress line is startup output, and is kept
    # verbatim: a target that dies during startup (missing nightly toolchain,
    # link failure) emits nothing matching the filter below, which would leave
    # a nonzero exit with an empty log and no way to tell why.
    local started=false
    while IFS= read -r line; do
        # Always save the most recent progress line for status reports.
        # Non-fork mode: "#NNN ACTION cov: ..."; fork mode: "#NNN: cov: ...".
        if [[ "$line" =~ ^#[0-9] ]]; then
            started=true
            echo "$line" >"$status_file"
        fi
        # Then log important lines only: crashes, errors, stats, summary.
        if [[ "$started" == false ]] ||
            [[ "$line" =~ SUMMARY|ERROR|CRASH|ALARM|panic|assertion|stat::|BINGO|Done[[:space:]] ]]; then
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

# Signal the process group of every target that has not been reaped yet.
signal_live_groups() {
    local sig="$1"
    for i in "${!PIDS[@]}"; do
        if [[ -z "${EXIT_CODES[$i]:-}" ]]; then
            kill "-$sig" -- "-${PIDS[$i]}" 2>/dev/null || true
        fi
    done
}

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
        local exit_code="${EXIT_CODES[$i]:-}"
        if [[ "$i" -ge "${#PIDS[@]}" ]]; then
            status="not launched"
        elif [[ -z "$exit_code" ]]; then
            status="running"
        elif [[ "$exit_code" -eq 0 ]]; then
            status="finished"
        elif [[ "$INTERRUPTED" == true ]] && [[ "$exit_code" == 143 ]]; then
            # Died from the TERM we sent, rather than failing on its own — a
            # target that had already failed keeps its real status. Only 143:
            # targets are in their own process groups, so a terminal Ctrl-C
            # never reaches them, and we send nothing but TERM.
            status="interrupted"
        else
            status="FAILED (exit $exit_code)"
        fi
        local stats
        stats=$(parse_stats "$target")
        printf "  %-20s [%s] %s\n" "$target" "$status" "$stats"

        # Report crash artifacts, distinguishing the ones this run produced
        # from any that were already on disk: only new ones fail the run.
        local artifact_dir="$FUZZ_DIR/artifacts/$target"
        local crashes new_crashes
        crashes=$(count_crash_artifacts "$target")
        new_crashes=$((crashes - ${INITIAL_ARTIFACT_COUNTS[$i]:-0}))
        if [[ "$new_crashes" -gt 0 ]]; then
            printf "    *** %d new crash artifact(s) (%d total) in %s ***\n" \
                "$new_crashes" "$crashes" "$artifact_dir"
        elif [[ "$crashes" -gt 0 ]]; then
            printf "    (%d pre-existing crash artifact(s) in %s)\n" "$crashes" "$artifact_dir"
        fi
    done
    echo ""
}

cleanup() {
    local exit_code="$1"
    trap - INT TERM HUP QUIT
    INTERRUPTED=true

    # A signal may arrive after a wrapper starts but before its PID is stored.
    if [[ "$STARTING_TARGET" == true ]] && [[ -n "${!:-}" ]]; then
        local pending_pid=$!
        local last_index=$((${#PIDS[@]} - 1))
        if [[ "$last_index" -lt 0 ]] || [[ "${PIDS[$last_index]}" != "$pending_pid" ]]; then
            PIDS+=("$pending_pid")
        fi
    fi

    # Signal before announcing. On SIGHUP the terminal is already gone, so
    # these echoes fail with EIO and `set -e` would abort the handler before
    # it stopped anything — orphaning every target for the rest of the budget,
    # which is the exact case the HUP trap exists for.
    signal_live_groups TERM
    echo "" || true
    echo "Stopping all fuzz targets..." || true
    record_finished_targets true
    print_summary
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
# Each target gets its own process group, so a terminal hangup no longer
# reaches the fuzz processes on its own — trap HUP/QUIT too, or an ssh drop
# would orphan six fuzzers for the rest of the time budget. Exit codes are
# the usual 128+signum.
trap 'cleanup 130' INT
trap 'cleanup 143' TERM
trap 'cleanup 129' HUP
trap 'cleanup 131' QUIT
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
    # group: the wrapper's pipefail status survives a deferred wait, and cleanup
    # can terminate the whole group without orphaning cargo-fuzz or the filter.
    record_finished_targets
    STARTING_TARGET=true
    set -m
    run_target "$target" "$log" "$status_file" "$extra_flags" &
    PIDS+=("$!")
    STARTING_TARGET=false
    set +m
done

echo ""
echo "All ${#TARGETS[@]} targets running. Logs in $LOG_DIR/"
echo "Press Ctrl-C to stop all targets."
echo ""

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
        reason=$(printf '%s, ' "${reasons[@]}")
        failed_targets+=("$target (${reason%, })")
    fi
done

if [[ ${#failed_targets[@]} -gt 0 ]]; then
    echo "Fuzzing failed for:"
    printf "  %s\n" "${failed_targets[@]}"
    exit 1
fi
