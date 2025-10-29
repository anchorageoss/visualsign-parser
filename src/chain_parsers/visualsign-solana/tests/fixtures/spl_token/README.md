# SPL Token Transaction Test Fixtures

This directory contains real transaction data from Solana explorers for validating field extraction.

## Fixture Philosophy

**These are UNIT tests for SPL Token instruction parsing**, not integration tests for full transactions.

- Each fixture tests ONE specific SPL Token instruction
- The fixture references the full transaction URL for context
- But only includes data for the specific instruction being validated
- This keeps tests focused, fast, and easy to debug

**For integration testing** of full multi-instruction transactions, use higher-level tests in the parent visualsign-parser project.

## How to Create a New Fixture

### Step 1: Get Transaction Signature from Explorer
Find a transaction on Solscan, Solana Explorer, or similar (e.g., `https://solscan.io/tx/<SIGNATURE>?cluster=devnet`)

**Note**: Many transactions have multiple instructions. You'll extract just the SPL Token instruction you want to test.

### Step 2: Fetch Raw Transaction Data via RPC

```bash
curl -X POST https://api.devnet.solana.com \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "<TRANSACTION_SIGNATURE>",
      {
        "encoding":"json",
        "maxSupportedTransactionVersion":0
      }
    ]
  }' | python3 -m json.tool > transaction.json
```

### Step 3: Extract Instruction Data

From the response JSON, locate the specific instruction you want to test:
- `result.transaction.message.instructions[INDEX]` contains the instruction
- `.data` field has the **base58-encoded** instruction data (when using `encoding: "json"`)
- `.accounts` array has account indices
- `.programIdIndex` maps to `result.transaction.message.accountKeys[INDEX]`

**Important**: Solana RPC uses different encodings:
- `encoding: "json"` → instruction data is base58 encoded
- `encoding: "base64"` → entire transaction is base64 encoded
- Our fixtures use base58 (from JSON encoding)

### Step 4: Create Fixture JSON

Create a file like `<instruction_type>_example.json`:

```json
{
  "description": "Human-readable description of what this instruction does",
  "source": "URL to the transaction on explorer",
  "signature": "Transaction signature",
  "cluster": "devnet or mainnet-beta",
  "instruction_index": 0,
  "instruction_data": "<base64 encoded data from .data field>",
  "program_id": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
  "accounts": [
    {
      "pubkey": "account public key",
      "signer": false,
      "writable": true,
      "description": "What this account represents (e.g., 'Token Mint')"
    }
  ],
  "expected_fields": {
    "instruction_name": "Expected value",
    "amount": "Expected amount",
    "token_mint": "Expected mint address"
  }
}
```

### Step 5: Verify Expected Fields

Use the explorer's parsed view or the `jsonParsed` encoding to verify expected values:

```bash
curl -X POST https://api.devnet.solana.com \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "<TRANSACTION_SIGNATURE>",
      {"encoding":"jsonParsed","maxSupportedTransactionVersion":0}
    ]
  }'
```

The `parsed.info` object will show you the expected field values.

### Step 6: Run the Test

```bash
cargo test --package visualsign-solana --lib presets::spl_token::tests::fixture_tests -- --nocapture
```

The test will print:
- Extracted fields from our parser
- Validation results comparing extracted vs expected

## Important Notes

- **DO NOT modify the instruction_data** to make tests pass
- The fixture data represents the REAL transaction - it's the source of truth
- If tests fail, fix the parser code, not the fixture
- Use the explorer's parsed view to verify what the expected values should be
