#!/bin/bash
set -euo pipefail

# Check if coverage meets minimum thresholds
# Returns non-zero exit code if any package is below threshold

echo "🎯 Checking coverage thresholds..."

# Define thresholds
CRITICAL_THRESHOLD=50
WARNING_THRESHOLD=70
GOOD_THRESHOLD=80

# Track failures
FAILED=false
WARNINGS=0

# Check each package
if [ -d "coverage-artifacts" ]; then
  for artifact_dir in coverage-artifacts/coverage-*; do
    if [ -d "$artifact_dir" ]; then
      pkg_name=$(basename "$artifact_dir" | sed 's/^coverage-//')
      stats_file="$artifact_dir/coverage-${pkg_name}-stats.json"

      if [ -f "$stats_file" ]; then
        coverage=$(jq -r '.coverage_pct' "$stats_file")

        if (( $(echo "$coverage < $CRITICAL_THRESHOLD" | bc -l) )); then
          echo "❌ $pkg_name: ${coverage}% (below critical threshold of ${CRITICAL_THRESHOLD}%)"
          FAILED=true
        elif (( $(echo "$coverage < $WARNING_THRESHOLD" | bc -l) )); then
          echo "⚠️  $pkg_name: ${coverage}% (below warning threshold of ${WARNING_THRESHOLD}%)"
          WARNINGS=$((WARNINGS + 1))
        elif (( $(echo "$coverage < $GOOD_THRESHOLD" | bc -l) )); then
          echo "🟢 $pkg_name: ${coverage}% (acceptable)"
        else
          echo "✅ $pkg_name: ${coverage}% (excellent)"
        fi
      fi
    fi
  done
fi

echo ""
if [ "$FAILED" = true ]; then
  echo "❌ Coverage check FAILED - some packages below critical threshold"
  exit 1
elif [ $WARNINGS -gt 0 ]; then
  echo "⚠️  Coverage check PASSED with $WARNINGS warning(s)"
  exit 0
else
  echo "✅ All packages meet coverage thresholds!"
  exit 0
fi
