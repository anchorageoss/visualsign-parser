#!/bin/bash
set -euo pipefail

# This script detects which Rust packages have changed and which packages depend on them
# It outputs JSON arrays for use in GitHub Actions matrix

echo "🔍 Detecting changed packages..."

# Determine base ref
if [ "${GITHUB_BASE_REF:-}" != "" ]; then
  BASE_REF="origin/${GITHUB_BASE_REF}"
else
  BASE_REF="HEAD~1"
fi

# Get changed files
CHANGED_FILES=$(git diff --name-only "$BASE_REF"...HEAD | grep -E '^src/.*\.(rs|toml)$' || true)

if [ -z "$CHANGED_FILES" ]; then
  echo "No Rust files changed"
  echo "packages=[]" >> "$GITHUB_OUTPUT"
  echo "dependents=[]" >> "$GITHUB_OUTPUT"
  echo "all_affected=[]" >> "$GITHUB_OUTPUT"
  exit 0
fi

echo "Changed files:"
echo "$CHANGED_FILES"

# Extract package names from changed files
declare -A CHANGED_PACKAGES
declare -A ALL_AFFECTED_PACKAGES

for file in $CHANGED_FILES; do
  # Extract package directory (e.g., src/visualsign or src/chain_parsers/visualsign-solana)
  if [[ $file =~ ^src/([^/]+)(/([^/]+))?/ ]]; then
    if [ "${BASH_REMATCH[3]}" != "" ]; then
      # Path like src/chain_parsers/visualsign-solana/...
      pkg="${BASH_REMATCH[3]}"
    else
      # Path like src/visualsign/... or src/parser/app/...
      if [[ $file =~ ^src/parser/([^/]+)/ ]]; then
        pkg="parser_${BASH_REMATCH[1]}"
      else
        pkg="${BASH_REMATCH[1]}"
      fi
    fi

    # Normalize package name (replace - with _)
    pkg_normalized=$(echo "$pkg" | tr '-' '_')
    CHANGED_PACKAGES[$pkg_normalized]=1
    ALL_AFFECTED_PACKAGES[$pkg_normalized]=1
  fi
done

echo ""
echo "📦 Directly changed packages:"
for pkg in "${!CHANGED_PACKAGES[@]}"; do
  echo "  - $pkg"
done

# Function to find packages that depend on a given package
find_dependents() {
  local target_pkg=$1
  local dependents=()

  # Parse workspace Cargo.toml to get all packages
  cd src

  # Get all workspace members
  while IFS= read -r member; do
    if [ -f "$member/Cargo.toml" ]; then
      # Check if this package depends on the target
      if grep -q "^${target_pkg} = " "$member/Cargo.toml" 2>/dev/null || \
         grep -q "path = \".*${target_pkg}\"" "$member/Cargo.toml" 2>/dev/null; then
        # Extract package name from path
        pkg_name=$(basename "$member")
        dependents+=("$pkg_name")
      fi
    fi
  done < <(cargo metadata --format-version=1 --no-deps | jq -r '.packages[] | select(.source == null) | .manifest_path' | xargs -I {} dirname {} | sed 's|^.*/src/||')

  cd ..

  # Return dependents
  printf '%s\n' "${dependents[@]}"
}

# Find all dependents
echo ""
echo "🔗 Finding dependent packages..."

for pkg in "${!CHANGED_PACKAGES[@]}"; do
  # Convert package name format (e.g., visualsign_ethereum -> visualsign-ethereum)
  pkg_with_dashes=$(echo "$pkg" | tr '_' '-')

  while IFS= read -r dependent; do
    if [ -n "$dependent" ]; then
      dependent_normalized=$(echo "$dependent" | tr '-' '_')
      if [ ! "${CHANGED_PACKAGES[$dependent_normalized]:-}" ]; then
        ALL_AFFECTED_PACKAGES[$dependent_normalized]=1
        echo "  - $dependent (depends on $pkg)"
      fi
    fi
  done < <(find_dependents "$pkg_with_dashes")
done

# Convert to JSON arrays
CHANGED_JSON=$(printf '%s\n' "${!CHANGED_PACKAGES[@]}" | jq -R -s -c 'split("\n") | map(select(length > 0))')
ALL_AFFECTED_JSON=$(printf '%s\n' "${!ALL_AFFECTED_PACKAGES[@]}" | jq -R -s -c 'split("\n") | map(select(length > 0))')

# Calculate dependents (all_affected - changed)
DEPENDENTS=()
for pkg in "${!ALL_AFFECTED_PACKAGES[@]}"; do
  if [ ! "${CHANGED_PACKAGES[$pkg]:-}" ]; then
    DEPENDENTS+=("$pkg")
  fi
done

DEPENDENTS_JSON=$(printf '%s\n' "${DEPENDENTS[@]}" | jq -R -s -c 'split("\n") | map(select(length > 0))')

echo ""
echo "📊 Summary:"
echo "  Directly changed: $(echo "$CHANGED_JSON" | jq 'length')"
echo "  Dependents affected: $(echo "$DEPENDENTS_JSON" | jq 'length')"
echo "  Total packages to test: $(echo "$ALL_AFFECTED_JSON" | jq 'length')"

# Output for GitHub Actions
echo "packages=$CHANGED_JSON" >> "$GITHUB_OUTPUT"
echo "dependents=$DEPENDENTS_JSON" >> "$GITHUB_OUTPUT"
echo "all_affected=$ALL_AFFECTED_JSON" >> "$GITHUB_OUTPUT"

echo ""
echo "✅ Detection complete"
