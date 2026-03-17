#!/usr/bin/env bash
# fuzz_all_idls.sh — run IDL fuzz tests against every embedded Solana IDL.
#
# The embedded IDLs live in the solana_parser git dependency:
#
#   ape_pro.json          6 instructions   4 types   (Ape Pro)
#   cndy.json             7 instructions   4 types   (Metaplex Candy Machine)
#   collision.json        1 instruction    2 types   (test fixture: duplicate type names)
#   cyclic.json           1 instruction    2 types   (test fixture: cyclic type references)
#   drift.json          199 instructions  81 types   (Drift Protocol V2)
#   jupiter.json         34 instructions   8 types   (Jupiter Swap)
#   jupiter_agg_v6.json  14 instructions   9 types   (Jupiter Aggregator V6)
#   jupiter_limit.json    8 instructions  12 types   (Jupiter Limit)
#   kamino.json          36 instructions  51 types   (Kamino)
#   lifinity.json         3 instructions   4 types   (Lifinity Swap V2)
#   meteora.json         64 instructions  38 types   (Meteora)
#   openbook.json        29 instructions  32 types   (Openbook)
#   orca.json            49 instructions  11 types   (Orca Whirlpool)
#   raydium.json         10 instructions   5 types   (Raydium)
#   stabble.json         17 instructions   8 types   (Stabble)
#
# For each IDL the script runs two test functions from fuzz_idl_parsing.rs:
#
#   real_idl_never_panics
#     — 50/50 valid/random discriminator mix; on Ok asserts correct dispatch.
#
#   real_idl_valid_data_always_parses_ok
#     — generates borsh-correct bytes for every instruction; asserts is_ok().
#
# Usage:
#   ./scripts/fuzz_all_idls.sh
#   PROPTEST_CASES=1000 ./scripts/fuzz_all_idls.sh
#   ./scripts/fuzz_all_idls.sh /path/to/extra.json ...   # append extra IDLs
#
# Requirements: cargo, python3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_TOML="$SCRIPT_DIR/../src/Cargo.toml"
CASES="${PROPTEST_CASES:-256}"

# ── Locate the solana_parser IDL directory via cargo metadata ─────────────────

IDL_DIR="$(python3 - "$WORKSPACE_TOML" <<'PY'
import json, os, subprocess, sys

manifest = sys.argv[1]
result = subprocess.run(
    ["cargo", "metadata", "--manifest-path", manifest, "--format-version", "1"],
    capture_output=True, text=True, check=True,
)
data = json.loads(result.stdout)
for pkg in data["packages"]:
    if pkg["name"] == "solana_parser":
        idl_dir = os.path.join(os.path.dirname(pkg["manifest_path"]), "src", "solana", "idls")
        if os.path.isdir(idl_dir):
            print(idl_dir)
            sys.exit(0)
print("error: solana_parser IDL directory not found", file=sys.stderr)
sys.exit(1)
PY
)"

# ── Collect IDL files: embedded + any extras passed as arguments ──────────────

IDL_FILES=("$IDL_DIR"/*.json)
for extra in "${@}"; do
    IDL_FILES+=("$extra")
done

# ── Build once so the loop doesn't pay compilation cost each iteration ─────────

echo "Building test binary..."
cargo test \
    --manifest-path "$WORKSPACE_TOML" \
    -p visualsign-solana \
    --test fuzz_idl_parsing \
    --no-run \
    2>&1 | grep -E "^(   Compiling|    Finished|error)" || true
echo ""

# ── Run tests for each IDL ────────────────────────────────────────────────────

PASS=0
FAIL=0
FAILED_IDLS=()

printf "%-30s %13s %7s  %s\n" "IDL" "Instructions" "Types" "Result"
printf "%-30s %13s %7s  %s\n" "───────────────────────────" "────────────" "─────" "──────"

for idl_file in "${IDL_FILES[@]}"; do
    name="$(basename "$idl_file" .json)"

    # Get instruction/type counts
    read -r inst_count type_count < <(python3 -c "
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    print(len(d.get('instructions', [])), len(d.get('types', [])))
except Exception:
    print(0, 0)
" "$idl_file")

    printf "%-30s %13s %7s  " "$name" "$inst_count" "$type_count"

    # Run both real_idl_* tests for this IDL.
    output=$(IDL_FILE="$idl_file" PROPTEST_CASES="$CASES" \
        cargo test \
            --manifest-path "$WORKSPACE_TOML" \
            -p visualsign-solana \
            --test fuzz_idl_parsing \
            real_idl \
            --quiet \
            2>&1)

    # Extract "N passed; M failed" directly from cargo's summary line.
    summary=$(echo "$output" | grep -oE "[0-9]+ passed; [0-9]+ failed" | head -1)

    if [ -z "$summary" ]; then
        echo "FAIL (no test result)"
        FAIL=$(( FAIL + 1 ))
        FAILED_IDLS+=("$name ($idl_file)")
    else
        failed_count=$(echo "$summary" | grep -oE "^[0-9]+ failed" | grep -oE "^[0-9]+" || \
                       echo "$summary" | grep -oE "[0-9]+ failed" | grep -oE "[0-9]+")
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
    echo ""
    echo "Re-run a single IDL with full output:"
    echo "  IDL_FILE=<path> cargo test --manifest-path src/Cargo.toml -p visualsign-solana --test fuzz_idl_parsing real_idl"
    exit 1
fi
