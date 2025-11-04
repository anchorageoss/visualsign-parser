#!/usr/bin/env python3
"""
Generate Jupiter Swap fixture JSON from a Solana transaction signature.

Usage:
    python3 generate_fixture.py <transaction_signature> <output_filename> [--cluster mainnet-beta]

Example:
    python3 generate_fixture.py 441ttot8CzpgsiRHvAHnNTCBwbSnPuhuy43pCjzZU9BKwBuJeW8f4TMU7FYLeqBst6WJeMEHprdQxr4thxqZSxRs route_example.json

This script:
1. Fetches the transaction from Solana RPC
2. Finds the Jupiter instruction
3. Handles v0 transactions with address table lookups
4. Extracts all accounts with their metadata
5. Generates a fixture JSON file ready for testing
"""

import sys
import json
import requests
import argparse
from typing import Dict, List, Any, Optional

JUPITER_PROGRAM_ID = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"

CLUSTER_URLS = {
    "mainnet-beta": "https://api.mainnet-beta.solana.com",
    "devnet": "https://api.devnet.solana.com",
    "testnet": "https://api.testnet.solana.com",
}


def fetch_transaction(signature: str, cluster: str = "mainnet-beta") -> Dict[str, Any]:
    """Fetch transaction data from Solana RPC."""
    url = CLUSTER_URLS.get(cluster, CLUSTER_URLS["mainnet-beta"])

    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {"encoding": "json", "maxSupportedTransactionVersion": 0}
        ]
    }

    print(f"Fetching transaction {signature} from {cluster}...")
    response = requests.post(url, json=payload)
    response.raise_for_status()

    data = response.json()
    if "error" in data:
        raise ValueError(f"RPC Error: {data['error']}")

    if not data.get("result"):
        raise ValueError(f"Transaction not found: {signature}")

    return data["result"]


def get_all_account_keys(tx_result: Dict[str, Any]) -> List[str]:
    """Combine static and loaded addresses for v0 transactions."""
    message = tx_result["transaction"]["message"]
    meta = tx_result["meta"]

    # Static account keys
    all_keys = list(message["accountKeys"])

    # Add loaded addresses if present (v0 transactions)
    if "loadedAddresses" in meta:
        loaded = meta["loadedAddresses"]
        all_keys.extend(loaded.get("writable", []))
        all_keys.extend(loaded.get("readonly", []))

    return all_keys


def find_jupiter_instruction(tx_result: Dict[str, Any], all_keys: List[str]) -> Optional[Dict[str, Any]]:
    """Find the Jupiter instruction in the transaction."""
    message = tx_result["transaction"]["message"]
    instructions = message["instructions"]

    try:
        jupiter_idx = all_keys.index(JUPITER_PROGRAM_ID)
    except ValueError:
        print(f"Warning: Jupiter program {JUPITER_PROGRAM_ID} not found in transaction")
        return None

    for inst_idx, inst in enumerate(instructions):
        if inst["programIdIndex"] == jupiter_idx:
            return {
                "index": inst_idx,
                "data": inst["data"],
                "account_indices": inst["accounts"]
            }

    return None


def get_instruction_type_from_logs(tx_result: Dict[str, Any]) -> Optional[str]:
    """Extract instruction type from transaction logs."""
    logs = tx_result.get("meta", {}).get("logMessages", [])

    for log in logs:
        if "Instruction:" in log and "JUP" in log.upper():
            # Extract instruction name (e.g., "Program log: Instruction: Route")
            parts = log.split("Instruction:")
            if len(parts) > 1:
                return parts[1].strip()

    # Fallback: look for any log mentioning instruction
    for log in logs:
        if "Instruction:" in log:
            parts = log.split("Instruction:")
            if len(parts) > 1:
                return parts[1].strip()

    return None


def generate_fixture(
    signature: str,
    output_filename: str,
    cluster: str = "mainnet-beta",
    description: Optional[str] = None,
    expected_fields: Optional[Dict[str, str]] = None
) -> None:
    """Generate fixture JSON file from transaction signature."""

    # Fetch transaction
    tx_result = fetch_transaction(signature, cluster)

    # Get all account keys (handles v0 transactions)
    all_keys = get_all_account_keys(tx_result)
    print(f"Total accounts in transaction: {len(all_keys)}")

    # Find Jupiter instruction
    jupiter_inst = find_jupiter_instruction(tx_result, all_keys)
    if not jupiter_inst:
        raise ValueError("No Jupiter instruction found in transaction")

    print(f"Found Jupiter instruction at index {jupiter_inst['index']}")
    print(f"Instruction data: {jupiter_inst['data']}")
    print(f"Number of accounts: {len(jupiter_inst['account_indices'])}")

    # Get instruction type from logs
    instruction_type = get_instruction_type_from_logs(tx_result)
    if instruction_type:
        print(f"Instruction type: {instruction_type}")

    # Extract accounts with metadata
    message = tx_result["transaction"]["message"]
    num_required_signatures = message.get("header", {}).get("numRequiredSignatures", 1)
    num_readonly_signed = message.get("header", {}).get("numReadonlySignedAccounts", 0)
    num_readonly_unsigned = message.get("header", {}).get("numReadonlyUnsignedAccounts", 0)

    accounts = []
    for idx in jupiter_inst["account_indices"]:
        pubkey = all_keys[idx]

        # Determine if signer (accounts before numRequiredSignatures are signers)
        is_signer = idx < num_required_signatures

        # Determine if writable
        # Readonly signed accounts: [numRequiredSignatures - numReadonlySignedAccounts, numRequiredSignatures)
        # Readonly unsigned accounts: determined by numReadonlyUnsignedAccounts from the end
        is_readonly_signed = (num_required_signatures - num_readonly_signed) <= idx < num_required_signatures
        is_readonly = is_readonly_signed  # Simplified; full logic is more complex for v0

        # For v0 transactions, loaded addresses have different writability rules
        # This is a simplified heuristic
        is_writable = not is_readonly

        # Try to identify known accounts
        account_desc = "Account"
        if pubkey == JUPITER_PROGRAM_ID:
            account_desc = "Jupiter program"
        elif pubkey == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA":
            account_desc = "Token program (SPL Token)"
        elif pubkey == "So11111111111111111111111111111111111111112":
            account_desc = "Wrapped SOL"
        elif pubkey == "11111111111111111111111111111111":
            account_desc = "System program"
        elif pubkey == "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr":
            account_desc = "Memo program"
        elif is_signer:
            account_desc = "User wallet"

        accounts.append({
            "pubkey": pubkey,
            "signer": is_signer,
            "writable": is_writable,
            "description": account_desc
        })

    # Generate fixture
    fixture = {
        "description": description or f"Jupiter {instruction_type or 'instruction'} transaction",
        "source": f"https://solscan.io/tx/{signature}",
        "signature": signature,
        "cluster": cluster,
        "full_transaction_note": f"This transaction has {len(message['instructions'])} instructions. The Jupiter instruction is at index {jupiter_inst['index']}.",
        "instruction_index": jupiter_inst["index"],
        "instruction_data": jupiter_inst["data"],
        "program_id": JUPITER_PROGRAM_ID,
        "accounts": accounts,
        "expected_fields": expected_fields or {
            "program_id": JUPITER_PROGRAM_ID,
            "slippage": "50"  # Default; user should update this
        }
    }

    # Write to file
    output_path = output_filename if output_filename.endswith('.json') else f"{output_filename}.json"
    with open(output_path, 'w') as f:
        json.dump(fixture, f, indent=2)

    print(f"\n✓ Fixture written to {output_path}")
    print(f"\nNext steps:")
    print(f"1. Review the fixture file and update the 'expected_fields' based on what the visualizer outputs")
    print(f"2. Update account descriptions if needed")
    print(f"3. Run the test: cargo test --lib presets::jupiter_swap::tests::fixture_tests")


def main():
    parser = argparse.ArgumentParser(
        description="Generate Jupiter Swap fixture JSON from a Solana transaction signature"
    )
    parser.add_argument(
        "signature",
        help="Transaction signature"
    )
    parser.add_argument(
        "output",
        help="Output filename (e.g., route_example.json)"
    )
    parser.add_argument(
        "--cluster",
        default="mainnet-beta",
        choices=["mainnet-beta", "devnet", "testnet"],
        help="Solana cluster (default: mainnet-beta)"
    )
    parser.add_argument(
        "--description",
        help="Description for the fixture"
    )

    args = parser.parse_args()

    try:
        generate_fixture(
            signature=args.signature,
            output_filename=args.output,
            cluster=args.cluster,
            description=args.description
        )
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
