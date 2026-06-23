#!/bin/sh
# Summarize the googleapis stress test results.

set -eu

echo "=== Google Cloud APIs codegen stress test ==="
echo ""

status=0

# Count generated files.
file_count=$(find /results/gen -name '*.rs' 2>/dev/null | wc -l)
total_lines=$(find /results/gen -name '*.rs' -exec cat {} + 2>/dev/null | wc -l)
total_size=$(du -sh /results/gen 2>/dev/null | cut -f1)

echo "Generated files: $file_count"
echo "Total lines:     $total_lines"
echo "Total size:      $total_size"

# Count compiled files (lib.rs includes minus excluded).
if [ -f /results/lib.rs ]; then
    compiled=$(grep -c 'include!' /results/lib.rs || true)
else
    compiled="?"
    status=1
fi
echo "Compiled files:  $compiled"
echo ""

# Show generation results.
echo "=== Generation ==="
if grep -q "Generate exit code: 0" /results/generate.log; then
    echo "PASS — code generation completed successfully"
else
    echo "FAIL — code generation errors detected"
    status=1
fi
echo ""

# Show any errors from the generation log.
if grep -qi "error\|panic\|failed" /results/generate.log; then
    echo "=== GENERATION ERRORS ==="
    grep -i "error\|panic\|failed" /results/generate.log
    echo ""
fi

# Show compilation results.
echo "=== Compilation ==="
if grep -q "Compile exit code: 0" /results/compile.log; then
    echo "PASS — generated code compiles successfully"
else
    echo "FAIL — compilation errors detected"
    status=1
    echo ""
    # Show the last 50 lines of errors (skip warnings).
    grep -E "^error" /results/compile.log | tail -50 || true
fi
echo ""

# If /out is mounted, copy results there. Tolerate missing artifacts so a
# failed-generation run still reaches the final exit with the right status.
if [ -d /out ]; then
    echo "Copying results to /out ..."
    cp -r /results/gen /out/gen 2>/dev/null || true
    cp /results/generate.log /out/generate.log 2>/dev/null || true
    cp /results/compile.log /out/compile.log 2>/dev/null || true
    cp /results/lib.rs /out/lib.rs 2>/dev/null || true
    echo "Done."
fi

exit "$status"
