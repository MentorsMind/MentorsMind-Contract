#!/bin/bash
set -e

# State Transition Coverage Analysis Script
# Runs the protocol state transition coverage analyzer and reports results

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="$PROJECT_ROOT/analysis_output"

echo "🔍 Running State Transition Coverage Analysis..."
echo "=================================================="

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Build and run the analyzer
cd "$PROJECT_ROOT"
cargo run --bin analyze-transitions --release 2>&1

# Check exit status
if [ $? -ne 0 ]; then
    echo "❌ Analyzer failed!"
    exit 1
fi

echo ""
echo "📊 Analysis Results:"
echo "===================="

# Display markdown report
if [ -f "$OUTPUT_DIR/coverage_report.md" ]; then
    cat "$OUTPUT_DIR/coverage_report.md"
fi

echo ""
echo "📁 Generated Files:"
ls -lh "$OUTPUT_DIR"/

# Extract overall coverage percentage for CI gates
COVERAGE_JSON="$OUTPUT_DIR/coverage_report.json"
if [ -f "$COVERAGE_JSON" ]; then
    OVERALL_COVERAGE=$(grep -o '"overall_coverage":[^,}]*' "$COVERAGE_JSON" | cut -d':' -f2 | tr -d ' ')
    COVERAGE_PERCENT=${OVERALL_COVERAGE%.*}
    
    echo ""
    echo "📈 Overall Coverage: $OVERALL_COVERAGE%"
    
    # Optional: Gate on minimum coverage
    MIN_COVERAGE=${COVERAGE_GATE:-40}
    if (( $(echo "$OVERALL_COVERAGE < $MIN_COVERAGE" | bc -l) )); then
        echo "⚠️  Coverage ($OVERALL_COVERAGE%) below threshold ($MIN_COVERAGE%)"
        echo "   Consider adding tests for missing transitions"
    else
        echo "✅ Coverage meets threshold"
    fi
fi

echo ""
echo "✨ Analysis complete! Results available in $OUTPUT_DIR"
