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
# JSON output and text I/O are skipped (covered by the std and view-json runs).
# Verifies owned/view decoder parity.
BUFFA_VIA_VIEW=1 run_suite view \
    conformance_test_runner \
    --failure_list /known_failures_view.txt \
    --maximum_edition 2024 \
    /usr/local/bin/buffa-conformance

# View-JSON mode: serves binary input + JSON output requests via
# decode_view → serde_json::to_string(&view). Exercises the generated view
# Serialize impls (and WKT view Serialize impls in buffa-types) against the
# conformance reference assertions, independently of the owned encoder.
# JSON input, binary output, and text format are skipped.
BUFFA_VIEW_JSON=1 run_suite view-json \
    conformance_test_runner \
    --failure_list /known_failures_view_json.txt \
    --maximum_edition 2024 \
    /usr/local/bin/buffa-conformance
