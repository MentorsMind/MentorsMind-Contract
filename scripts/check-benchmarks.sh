#!/bin/bash

# MentorsMind Contract Benchmark CI Check
# Fails if any benchmark exceeds 120M CPU instructions (>20% over 100M limit)
# Mini Changes

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
REPORT_FILE="${PROJECT_ROOT}/target/benchmark-report.json"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Thresholds (in CPU instructions)
HARD_LIMIT=100_000_000
WARNING_THRESHOLD=80_000_000
FAILURE_THRESHOLD=120_000_000

echo "=========================================="
echo "MentorsMind Benchmark CI Check"
echo "=========================================="
echo ""

# Run benchmarks
echo "Running benchmarks..."
cd "$PROJECT_ROOT"
cargo test --test benchmarks --release -- --nocapture 2>&1 | tee /tmp/benchmark_output.txt

# Parse results from test output
# Expected format: [PASS/WARN/FAIL] function_name - X.XM instructions (X.X% of limit)

echo ""
echo "=========================================="
echo "Benchmark Results Analysis"
echo "=========================================="
echo ""

FAILURES=0
WARNINGS=0
PASSES=0

# Extract benchmark results
while IFS= read -r line; do
    if [[ $line =~ \[([A-Z]+)\]\ ([^-]+)-\ ([0-9.]+)M\ instructions ]]; then
        STATUS="${BASH_REMATCH[1]}"
        FUNC_NAME="${BASH_REMATCH[2]}"
        INSTRUCTIONS="${BASH_REMATCH[3]}"

        if [ "$STATUS" = "FAIL" ]; then
            echo -e "${RED}✗ FAIL${NC} - $FUNC_NAME: ${INSTRUCTIONS}M instructions"
            ((FAILURES++))
        elif [ "$STATUS" = "WARN" ]; then
            echo -e "${YELLOW}⚠ WARN${NC} - $FUNC_NAME: ${INSTRUCTIONS}M instructions"
            ((WARNINGS++))
        else
            echo -e "${GREEN}✓ PASS${NC} - $FUNC_NAME: ${INSTRUCTIONS}M instructions"
            ((PASSES++))
        fi
    fi
done < /tmp/benchmark_output.txt

echo ""
echo "=========================================="
echo "Summary"
echo "=========================================="
echo "Passed:  $PASSES"
echo "Warned:  $WARNINGS"
echo "Failed:  $FAILURES"
echo ""

# Generate JSON report
mkdir -p "$(dirname "$REPORT_FILE")"
cat > "$REPORT_FILE" << EOF
{
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "thresholds": {
    "hard_limit": $HARD_LIMIT,
    "warning_threshold": $WARNING_THRESHOLD,
    "failure_threshold": $FAILURE_THRESHOLD
  },
  "results": {
    "passed": $PASSES,
    "warned": $WARNINGS,
    "failed": $FAILURES
  },
  "status": "$([ $FAILURES -eq 0 ] && echo 'PASS' || echo 'FAIL')"
}
EOF

echo "Report written to: $REPORT_FILE"
echo ""

# Exit with failure if any benchmarks failed
if [ $FAILURES -gt 0 ]; then
    echo -e "${RED}❌ CI CHECK FAILED${NC}"
    echo "One or more benchmarks exceeded the failure threshold (>120M instructions)"
    exit 1
fi

if [ $WARNINGS -gt 0 ]; then
    echo -e "${YELLOW}⚠️  CI CHECK PASSED WITH WARNINGS${NC}"
    echo "Some benchmarks are approaching the warning threshold (>80M instructions)"
    exit 0
fi

echo -e "${GREEN}✅ CI CHECK PASSED${NC}"
echo "All benchmarks are within acceptable limits"
exit 0
