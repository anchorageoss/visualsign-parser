#!/usr/bin/env bash
# gen_near_token_seeds.sh — resolve NEAR Intents token seeds against the
# omni-bridge contract, the authoritative on-chain registry Near-One/omni-bridge
# itself uses to route transfers.
#
# For each entry below, this:
#   1. Resolves a foreign (chain, address) pair -- or a chain's native asset --
#      to its NEAR token account via `omni.bridge.near`'s `get_token_id` /
#      `get_native_token_id` view methods.
#   2. Reads that NEAR account's own `ft_metadata` (symbol, decimals).
#   3. Prints a Rust tuple line in the shape of
#      `chain_parsers/visualsign-near/src/presets/intents/tokens.rs`'s `SEEDS`
#      table.
#
# This is a one-time/periodic generator, not a build step: its output is meant
# to be read, spot-checked, and pasted into `SEEDS` by hand -- nothing here
# writes to the Rust source directly, matching that table's own doc comment
# ("each entry MUST be verified... before being added").
#
# A token id that isn't registered on the bridge (ERR_TOKEN_NOT_REGISTERED)
# is reported on stderr and skipped, not guessed at.
#
# Usage:
#   ./scripts/gen_near_token_seeds.sh
#
# Env overrides:
#   NEAR_RPC_URL       default: https://rpc.mainnet.near.org
#   OMNI_BRIDGE_ACCOUNT default: omni.bridge.near
#   RPC_DELAY_SECONDS  default: 1 (sleep between RPC calls; this hits a public,
#                      shared-rate-limit endpoint, so don't drop this to 0)
#
# Requirements: curl, jq, base64

set -euo pipefail

NEAR_RPC_URL="${NEAR_RPC_URL:-https://rpc.mainnet.near.org}"
OMNI_BRIDGE_ACCOUNT="${OMNI_BRIDGE_ACCOUNT:-omni.bridge.near}"
RPC_DELAY_SECONDS="${RPC_DELAY_SECONDS:-1}"

# ── Input list: what to resolve ────────────────────────────────────────────
#
# Foreign tokens as `chain:address` (the OmniAddress string form the bridge
# contract's `FromStr`/`Deserialize` impls accept), plus a separate list of
# chains whose *native* asset (ETH, etc.) should be resolved via
# `get_native_token_id` instead of `get_token_id`.
#
# This output is a starting point, not the authority. Two things must be
# checked by hand before anything reaches the `SEEDS` table:
#
#   1. `get_native_token_id` does not always answer with the id real intents
#      carry. It answers `Eth` with the legacy rainbow-bridge
#      `eth.bridge.near`, and `Base`/`Arb`/`Pol` with `.omdep.near` deposit
#      contracts, while observed traffic uses `<chain>.omft.near`. Cross-check
#      every id against `ft_metadata` and against a real envelope.
#
#   2. The reported `symbol` is not unique across chains. Four assets report
#      `ETH` and four report `USDC`. `SEEDS` requires origin-qualified symbols
#      (`ETH.base`, `USDC.sol`) so two assets never render alike; see the
#      table's own docs.

FOREIGN_TOKENS=(
    "eth:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"   # USDC on Ethereum
    "eth:0xdac17f958d2ee523a2206206994597c13d831ec7"   # USDT on Ethereum
    "sol:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" # USDC on Solana
    "sol:Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" # USDT on Solana
)

# Base and Arbitrum stablecoins are deliberately absent: `get_token_id`
# returns nothing for them, so this script cannot source their ids, and the
# `SEEDS` table leaves them out rather than seeding an unsourced id.

NATIVE_CHAINS=(
    "Eth"
    "Sol"
    "Base"
    "Arb"
    "Pol"
    "Btc"
)

# ── RPC helpers ─────────────────────────────────────────────────────────────

# Call a NEAR view method and print the raw JSON-RPC response. Sleeps
# afterward so callers never need to remember to throttle themselves.
near_view_raw() {
    local account="$1" method="$2" args_json="$3"
    local args_b64
    args_b64=$(printf '%s' "$args_json" | base64 -w0)
    curl -s -X POST "$NEAR_RPC_URL" -H 'Content-Type: application/json' -d "{
        \"jsonrpc\": \"2.0\",
        \"id\": \"gen_near_token_seeds\",
        \"method\": \"query\",
        \"params\": {
            \"request_type\": \"call_function\",
            \"finality\": \"final\",
            \"account_id\": \"$account\",
            \"method_name\": \"$method\",
            \"args_base64\": \"$args_b64\"
        }
    }"
    sleep "$RPC_DELAY_SECONDS"
}

# Call a NEAR view method that returns a JSON string, and print that string
# decoded (quotes stripped). Prints nothing and returns nonzero on a
# contract-side panic (e.g. ERR_TOKEN_NOT_REGISTERED).
near_view_string() {
    local account="$1" method="$2" args_json="$3"
    local response result_bytes
    response=$(near_view_raw "$account" "$method" "$args_json")
    if echo "$response" | jq -e '.result.error' >/dev/null 2>&1; then
        return 1
    fi
    result_bytes=$(echo "$response" | jq -r '.result.result | implode')
    # implode of a JSON string's UTF-8 bytes yields the quoted JSON literal;
    # jq -r strips one layer of quoting for us already only for scalar output,
    # so unwrap the surrounding quotes explicitly.
    echo "$result_bytes" | sed -e 's/^"//' -e 's/"$//'
}

# ── Resolution ───────────────────────────────────────────────────────────────

# Given a resolved NEAR token account, emit a `SEEDS` tuple line by reading
# ft_metadata. Skips (with a stderr note) if ft_metadata is missing symbol or
# decimals.
emit_seed_line() {
    local token_account="$1" label="$2"
    local metadata symbol decimals
    metadata=$(near_view_raw "$token_account" "ft_metadata" '{}' | jq -r '.result.result | implode' 2>/dev/null || true)
    if [ -z "$metadata" ] || [ "$metadata" = "null" ]; then
        echo "SKIP $label ($token_account): ft_metadata call failed" >&2
        return
    fi
    symbol=$(echo "$metadata" | jq -r '.symbol')
    decimals=$(echo "$metadata" | jq -r '.decimals')
    if [ -z "$symbol" ] || [ "$symbol" = "null" ] || [ -z "$decimals" ] || [ "$decimals" = "null" ]; then
        echo "SKIP $label ($token_account): ft_metadata missing symbol/decimals" >&2
        return
    fi
    printf '    ("nep141:%s", "%s", %s), // %s\n' "$token_account" "$symbol" "$decimals" "$label"
}

echo "# Generated $(date -u +%Y-%m-%d 2>/dev/null || echo unknown) via $OMNI_BRIDGE_ACCOUNT on $NEAR_RPC_URL." >&2
echo "# Review before pasting into tokens.rs's SEEDS table." >&2
echo "" >&2

for entry in "${FOREIGN_TOKENS[@]}"; do
    token_account=$(near_view_string "$OMNI_BRIDGE_ACCOUNT" "get_token_id" "{\"address\":\"$entry\"}") || {
        echo "SKIP $entry: not registered on $OMNI_BRIDGE_ACCOUNT (ERR_TOKEN_NOT_REGISTERED)" >&2
        continue
    }
    emit_seed_line "$token_account" "$entry"
done

for chain in "${NATIVE_CHAINS[@]}"; do
    token_account=$(near_view_string "$OMNI_BRIDGE_ACCOUNT" "get_native_token_id" "{\"chain\":\"$chain\"}") || {
        echo "SKIP native:$chain: not registered on $OMNI_BRIDGE_ACCOUNT" >&2
        continue
    }
    emit_seed_line "$token_account" "native:$chain"
done
