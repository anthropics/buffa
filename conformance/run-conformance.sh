#!/bin/bash
set -e

# The conformance_test_runner always runs two suites per invocation:
#   1. Binary + JSON (expect thousands of successes)
#   2. Text format (several hundred successes; expected failures listed
#      in the same --failure_list file)
#
# When CONFORMANCE_OUT is set (e.g. via docker run -v /tmp:/out -e
# CONFORMANCE_OUT=/out), each run's output is tee'd to a log file there
# for post-hoc analysis of failures.

run_suite() {
    local name="$1"
    local log="${CONFORMANCE_OUT:+$CONFORMANCE_OUT/conformance-$name.log}"
    shift
    echo "=== Conformance: $name ==="
    if [ -n "$log" ]; then
        "$@" 2>&1 | tee "$log"
    else
        "$@"
    fi
    echo ""
}

run_suite std \
    conformance_test_runner \
    --failure_list /known_failures.txt \
    --text_format_failure_list /known_failures_text.txt \
    --maximum_edition 2024 \
    /usr/local/bin/buffa-conformance

run_suite nostd \
    conformance_test_runner \
    --failure_list /known_failures_nostd.txt \
    --text_format_failure_list /known_failures_text.txt \
    --maximum_edition 2024 \
    /usr/local/bin/buffa-conformance-nostd

# Via-view mode: routes binary input through decode_view → to_owned_message.
# JSON and text I/O are skipped (views have no serde or TextFormat).
# Verifies owned/view decoder parity.
BUFFA_VIA_VIEW=1 run_suite view \
    conformance_test_runner \
    --failure_list /known_failures_view.txt \
    --maximum_edition 2024 \
    /usr/local/bin/buffa-conformance
