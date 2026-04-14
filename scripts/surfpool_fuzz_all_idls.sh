#!/usr/bin/env bash
# surfpool_fuzz_all_idls.sh — run surfpool integration tests against every embedded IDL.
#
# Requires: cargo, surfpool binary
# Optional: HELIUS_API_KEY (for faster RPC), PROPTEST_CASES (default: 32)
#
# Usage:
#   ./scripts/surfpool_fuzz_all_idls.sh
#   HELIUS_API_KEY=<key> PROPTEST_CASES=64 ./scripts/surfpool_fuzz_all_idls.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_TOML="$SCRIPT_DIR/../src/Cargo.toml"
CASES="${PROPTEST_CASES:-32}"

# Build tools
cargo build --manifest-path "$WORKSPACE_TOML" -p idl-meta --quiet

IDL_META="cargo run --manifest-path $WORKSPACE_TOML -p idl-meta --quiet --"
IDL_DIR="$($IDL_META locate-idls --manifest-path "$WORKSPACE_TOML")"

IDL_FILES=("$IDL_DIR"/*.json)

# Build test binary once
echo "Building surfpool fuzz test binary..."
cargo test \
    --manifest-path "$WORKSPACE_TOML" \
    -p visualsign-solana \
    --test surfpool_fuzz \
    --no-run \
    2>&1 | grep -E "^(   Compiling|    Finished|error)" || true
echo ""

PASS=0
FAIL=0
FAILED_IDLS=()

printf "%-30s %13s %7s  %s\n" "IDL" "Instructions" "Types" "Result"
printf "%-30s %13s %7s  %s\n" "───────────────────────────" "────────────" "─────" "──────"

for idl_file in "${IDL_FILES[@]}"; do
    name="$(basename "$idl_file" .json)"
    read -r inst_count type_count < <($IDL_META counts "$idl_file")

    printf "%-30s %13s %7s  " "$name" "$inst_count" "$type_count"

    output=$(IDL_FILE="$idl_file" PROPTEST_CASES="$CASES" \
        cargo test \
            --manifest-path "$WORKSPACE_TOML" \
            -p visualsign-solana \
            --test surfpool_fuzz \
            -- --ignored --quiet \
            2>&1)

    summary=$(echo "$output" | grep -oE "[0-9]+ passed; [0-9]+ failed" | head -1)

    if [ -z "$summary" ]; then
        echo "FAIL (no test result)"
        FAIL=$(( FAIL + 1 ))
        FAILED_IDLS+=("$name ($idl_file)")
    else
        failed_count=$(echo "$summary" | grep -oE "[0-9]+ failed" | grep -oE "[0-9]+")
        if [ "${failed_count:-0}" -gt 0 ]; then
            echo "FAIL ($summary)"
            FAIL=$(( FAIL + 1 ))
            FAILED_IDLS+=("$name ($idl_file)")
        else
            echo "PASS ($summary)"
            PASS=$(( PASS + 1 ))
        fi
    fi
done

echo ""
echo "Results: $PASS passed, $FAIL failed  (PROPTEST_CASES=$CASES)"

if (( FAIL > 0 )); then
    echo ""
    echo "Failed:"
    for entry in "${FAILED_IDLS[@]}"; do
        echo "  $entry"
    done
    exit 1
fi
