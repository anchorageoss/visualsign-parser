#!/usr/bin/env bash
# Generate a test fixture from a real Solana transaction
#
# This script fetches transaction data from Solana RPC and creates a fixture
# JSON file ready for testing instruction parsers.
#
# Usage:
#   ./generate-fixture.sh <signature> [instruction_index] [cluster]
#
# Examples:
#   ./generate-fixture.sh 5wHu1qwD7q4... 0
#   ./generate-fixture.sh 5wHu1qwD7q4... 1 devnet
#   ./generate-fixture.sh 5wHu1qwD7q4... 2 mainnet-beta

set -e

if [ -z "$1" ]; then
  echo "Usage: $0 <transaction_signature> [instruction_index] [cluster]"
  echo ""
  echo "Arguments:"
  echo "  transaction_signature  The transaction signature to fetch"
  echo "  instruction_index      Instruction index to extract (default: 0)"
  echo "  cluster                mainnet-beta (default) or devnet"
  echo ""
  echo "Example:"
  echo "  $0 5wHu1qwD7q4HiLxmxbhLrVP3jvVRo9F... 1 devnet"
  exit 1
fi

signature="$1"
instruction_index="${2:-0}"
cluster="${3:-mainnet-beta}"

# Validate instruction_index
if ! [[ "$instruction_index" =~ ^[0-9]+$ ]]; then
  echo "Error: instruction_index must be a non-negative integer"
  exit 1
fi

# Validate cluster
if [ "$cluster" != "mainnet-beta" ] && [ "$cluster" != "devnet" ]; then
  echo "Error: cluster must be 'mainnet-beta' or 'devnet'"
  exit 1
fi

# Set API URL based on cluster
if [ "$cluster" = "mainnet-beta" ]; then
  api_url="https://api.mainnet-beta.solana.com"
else
  api_url="https://api.devnet.solana.com"
fi

# Create temporary files
instruction_data_file=$(mktemp)
parsed_data_file=$(mktemp)

# Clean up temporary files on exit
trap 'rm -f "$instruction_data_file" "$parsed_data_file"' EXIT

echo "Fetching transaction $signature from $cluster..." >&2

# Fetch raw transaction data (JSON encoding gives base58 instruction data)
curl -s -X POST "$api_url" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "'"$signature"'",
      {"encoding":"json","maxSupportedTransactionVersion":0}
    ]
  }' > "$instruction_data_file"

# Fetch parsed transaction data (for expected field values)
curl -s -X POST "$api_url" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "'"$signature"'",
      {"encoding":"jsonParsed","maxSupportedTransactionVersion":0}
    ]
  }' > "$parsed_data_file"

# Check for errors
if jq -e '.error' "$instruction_data_file" > /dev/null 2>&1; then
  echo "Error fetching transaction:" >&2
  jq '.error' "$instruction_data_file" >&2
  exit 1
fi

# Extract account indexes for the instruction
account_indexes="$(jq < "$instruction_data_file" '.result.transaction.message.instructions['"$instruction_index"'].accounts')"

if [ "$account_indexes" = "null" ]; then
  echo "Error: Instruction at index $instruction_index not found" >&2
  echo "Available instructions:" >&2
  jq '.result.transaction.message.instructions | length' "$instruction_data_file" >&2
  exit 1
fi

# Generate accounts array with metadata from parsed response
accounts="$(jq -n --argjson indexes "$account_indexes" --slurpfile parsed "$parsed_data_file" '
  $parsed[0].result.transaction.message.accountKeys as $accountKeys |
  $indexes | map(. as $idx | $accountKeys[$idx] | {
    pubkey: .pubkey,
    signer: .signer,
    writable: .writable,
    description: "Account at index \($idx)"
  })
')"

# Generate the fixture JSON
jq --slurp \
  --argjson accounts "$accounts" \
  --arg cluster "$cluster" \
  --arg signature "$signature" \
  --argjson instruction_index "$instruction_index" \
'{
  "description": "TODO: Describe what this instruction does",
  "source": "https://solscan.io/tx/\($signature)?cluster=\($cluster)",
  "signature": $signature,
  "cluster": $cluster,
  "full_transaction_note": "TODO: Note about the full transaction context",
  "instruction_index": $instruction_index,
  "instruction_data": .[0].result.transaction.message.instructions[$instruction_index].data,
  "program_id": .[1].result.transaction.message.instructions[$instruction_index].programId,
  "accounts": $accounts,
  "expected_fields": (.[1].result.transaction.message.instructions[$instruction_index].parsed.info // {})
}' "$instruction_data_file" "$parsed_data_file"
