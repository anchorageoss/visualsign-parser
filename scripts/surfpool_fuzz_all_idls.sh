#!/usr/bin/env bash
# Run surfpool_real_idl_roundtrip against every embedded IDL.
#
# Usage:
#   ./scripts/surfpool_fuzz_all_idls.sh
#   PROPTEST_CASES=512 ./scripts/surfpool_fuzz_all_idls.sh
#
# Required:
#   surfpool binary installed (cargo install surfpool --git https://github.com/txtx/surfpool)
#   SOLANA_RPC_URL env var set (used by SurfpoolConfig::default() as the fork URL)
#
# Each IDL is tested in a fresh process so surfpool is restarted per IDL.
# Set PROPTEST_CASES to control how many cases are run per IDL (default 256).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IDL_DIR="$REPO_ROOT/src/solana_test_utils/idls"

# IDL filename → real program address (from solana_parser's IDL_DB)
declare -A IDL_PROGRAMS
IDL_PROGRAMS["ape_pro.json"]="JSW99DKmxNyREQM14SQLDykeBvEUG63TeohrvmofEiw"
IDL_PROGRAMS["cndy.json"]="cndyAnrLdpjq1Ssp1z8xxDsB8dxe7u4HL5Nxi2K5WXZ"
IDL_PROGRAMS["drift.json"]="dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH"
IDL_PROGRAMS["jupiter.json"]="JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB"
IDL_PROGRAMS["jupiter_agg_v6.json"]="JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
IDL_PROGRAMS["jupiter_limit.json"]="j1o2qRpjcyUwEvwtcfhEQefh773ZgjxcVRry7LDqg5X"
IDL_PROGRAMS["kamino.json"]="6LtLpnUFNByNXLyCoK9wA2MykKAmQNZKBdY8s47dehDc"
IDL_PROGRAMS["lifinity.json"]="2wT8Yq49kHgDzXuPxZSaeLaH1qbmGXtEyPy64bL7aD3c"
IDL_PROGRAMS["meteora.json"]="LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
IDL_PROGRAMS["openbook.json"]="opnb2LAfJYbRMAHHvqjCwQxanZn7ReEHp1k81EohpZb"
IDL_PROGRAMS["orca.json"]="whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc"
IDL_PROGRAMS["raydium.json"]="CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C"
IDL_PROGRAMS["stabble.json"]="swapNyd8XiQwJ6ianp9snpu4brUqFxadzvHebnAXjJZ"

PASS=0
FAIL=0
FAILED_IDLS=()

# Header
printf "\n%-30s %-46s %s\n" "IDL" "PROGRAM_ID" "RESULT"
printf "%s\n" "$(printf '─%.0s' {1..85})"

for idl_file in "${!IDL_PROGRAMS[@]}"; do
    program_id="${IDL_PROGRAMS[$idl_file]}"
    idl_path="$IDL_DIR/$idl_file"

    if [[ ! -f "$idl_path" ]]; then
        printf "%-30s %-46s %s\n" "$idl_file" "$program_id" "SKIP (file missing)"
        continue
    fi

    printf "%-30s %-46s " "$idl_file" "$program_id"

    if IDL_FILE="$idl_path" \
       PROGRAM_ID="$program_id" \
       PROPTEST_CASES="${PROPTEST_CASES:-256}" \
       cargo test \
           --manifest-path "$REPO_ROOT/src/Cargo.toml" \
           -p solana_test_utils \
           --test surfpool_fuzz \
           surfpool_real_idl_roundtrip \
           -- --ignored --nocapture 2>/dev/null; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        FAIL=$((FAIL + 1))
        FAILED_IDLS+=("$idl_file")
    fi
done

printf "%s\n" "$(printf '─%.0s' {1..85})"
printf "Results: %d passed, %d failed\n\n" "$PASS" "$FAIL"

if [[ ${#FAILED_IDLS[@]} -gt 0 ]]; then
    echo "Failed IDLs:"
    for f in "${FAILED_IDLS[@]}"; do
        echo "  - $f"
    done
    echo ""
fi

[[ $FAIL -eq 0 ]]
