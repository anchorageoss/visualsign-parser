#!/bin/bash
set -euo pipefail

# Get coverage from base branch (from cache, artifacts, or generate fresh)
# Usage: get-base-coverage.sh <base_branch>

BASE_BRANCH=$1

echo "📥 Fetching base branch coverage for $BASE_BRANCH..."

# Try to get coverage from GitHub artifacts (previous runs)
mkdir -p base-coverage

# Check if we can use the GitHub CLI to download artifacts
if command -v gh &> /dev/null && [ -n "${GITHUB_TOKEN:-}" ]; then
  echo "Attempting to download base coverage from previous runs..."

  # Get the latest successful run on base branch
  RUN_ID=$(gh run list \
    --branch "$BASE_BRANCH" \
    --workflow "coverage.yml" \
    --status success \
    --limit 1 \
    --json databaseId \
    --jq '.[0].databaseId' 2>/dev/null || echo "")

  if [ -n "$RUN_ID" ]; then
    echo "Found run ID: $RUN_ID"

    # Download all coverage artifacts
    gh run download "$RUN_ID" --dir base-coverage 2>/dev/null || true

    if [ "$(ls -A base-coverage 2>/dev/null)" ]; then
      echo "✅ Base coverage downloaded from artifacts"
      exit 0
    fi
  fi
fi

echo "⚠️  Could not fetch base coverage from artifacts"
echo "Base coverage comparison will be skipped"

# Create empty marker
touch base-coverage/.no-base-coverage
