#!/bin/bash
set -euo pipefail

# Parse lcov file and extract coverage statistics
# Usage: parse-coverage.sh <lcov_file> <package_name>

LCOV_FILE=$1
PACKAGE_NAME=$2

if [ ! -f "$LCOV_FILE" ]; then
  echo "⚠️  Coverage file not found: $LCOV_FILE"
  echo "{\"package\": \"$PACKAGE_NAME\", \"error\": \"no_coverage_file\"}" > "coverage-${PACKAGE_NAME}-stats.json"
  exit 0
fi

echo "📊 Parsing coverage for $PACKAGE_NAME..."

# Parse lcov file
TOTAL_LINES=0
COVERED_LINES=0

while IFS= read -r line; do
  if [[ $line =~ ^LF:([0-9]+) ]]; then
    TOTAL_LINES=$((TOTAL_LINES + ${BASH_REMATCH[1]}))
  elif [[ $line =~ ^LH:([0-9]+) ]]; then
    COVERED_LINES=$((COVERED_LINES + ${BASH_REMATCH[1]}))
  fi
done < "$LCOV_FILE"

if [ $TOTAL_LINES -eq 0 ]; then
  COVERAGE_PCT=0
else
  COVERAGE_PCT=$(awk "BEGIN {printf \"%.2f\", ($COVERED_LINES / $TOTAL_LINES) * 100}")
fi

echo "  Total lines: $TOTAL_LINES"
echo "  Covered lines: $COVERED_LINES"
echo "  Coverage: ${COVERAGE_PCT}%"

# Determine status emoji
if (( $(echo "$COVERAGE_PCT >= 80" | bc -l) )); then
  STATUS="✅"
  STATUS_TEXT="excellent"
elif (( $(echo "$COVERAGE_PCT >= 70" | bc -l) )); then
  STATUS="🟢"
  STATUS_TEXT="good"
elif (( $(echo "$COVERAGE_PCT >= 50" | bc -l) )); then
  STATUS="🟡"
  STATUS_TEXT="needs_improvement"
else
  STATUS="🔴"
  STATUS_TEXT="critical"
fi

# Create JSON output
cat > "coverage-${PACKAGE_NAME}-stats.json" <<EOF
{
  "package": "$PACKAGE_NAME",
  "total_lines": $TOTAL_LINES,
  "covered_lines": $COVERED_LINES,
  "coverage_pct": $COVERAGE_PCT,
  "status": "$STATUS",
  "status_text": "$STATUS_TEXT"
}
EOF

# Output for GitHub Actions
echo "total_lines=$TOTAL_LINES" >> "$GITHUB_OUTPUT"
echo "covered_lines=$COVERED_LINES" >> "$GITHUB_OUTPUT"
echo "coverage_pct=$COVERAGE_PCT" >> "$GITHUB_OUTPUT"
echo "status=$STATUS" >> "$GITHUB_OUTPUT"

echo "$STATUS Coverage: ${COVERAGE_PCT}%"
