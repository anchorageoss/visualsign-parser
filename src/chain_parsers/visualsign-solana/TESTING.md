# Testing Guide for visualsign-solana

This document establishes the testing philosophy and practices for all program presets in the visualsign-solana crate.

## Test Philosophy

### Unit Tests for Instruction Parsing

All program preset tests (SPL Token, System Program, Stake Pool, etc.) follow the same principles:

- **Each fixture tests ONE specific instruction** in isolation
- **These are UNIT tests**, not integration tests for full multi-instruction transactions
- **Fixtures are the source of truth** - they contain real transaction data from actual on-chain transactions
- **No network dependencies** - test fixtures are manually extracted and committed to git
- **Field names match explorer output** - makes manual verification against Solscan/explorers easy

### Integration Testing

For testing full multi-instruction transactions and end-to-end workflows, use higher-level integration tests in the parent visualsign-parser project, not at this crate level.

## Creating Test Fixtures

### Overview

Test fixtures validate that our instruction parsers correctly extract and display parameters from real Solana transactions. Each fixture:

1. References a real transaction from an explorer (Solscan, Solana Explorer, etc.)
2. Contains the raw instruction data for a specific instruction within that transaction
3. Specifies expected field values to validate parser output

### Step-by-Step Guide

#### Step 1: Find a Transaction

Find a transaction on an explorer that contains the instruction type you want to test:
- Solscan: `https://solscan.io/tx/<SIGNATURE>?cluster=<CLUSTER>`
- Solana Explorer: `https://explorer.solana.com/tx/<SIGNATURE>?cluster=<CLUSTER>`

**Note**: Most transactions contain multiple instructions. You'll extract just the one instruction you want to test.

#### Step 2: Fetch Raw Transaction Data

Use Solana RPC to get the raw transaction data:

```bash
# For devnet
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

# For mainnet
curl -X POST https://api.mainnet-beta.solana.com \
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

#### Step 3: Extract Instruction Data

From the RPC response, locate the specific instruction you want to test:

```json
{
  "result": {
    "transaction": {
      "message": {
        "accountKeys": ["pubkey1", "pubkey2", ...],
        "instructions": [
          {
            "programIdIndex": 5,
            "accounts": [0, 1, 2],
            "data": "6YCQpfSgHpSj"
          }
        ]
      }
    }
  }
}
```

Key fields:
- `instructions[INDEX].data` - **base58-encoded** instruction data (when using `encoding: "json"`)
- `instructions[INDEX].accounts` - array of account indices that map to `accountKeys`
- `instructions[INDEX].programIdIndex` - index into `accountKeys` for the program ID

**Important: Encoding Formats**

Solana RPC uses different encodings depending on the request:
- `encoding: "json"` → instruction data is **base58 encoded** ✓ (we use this)
- `encoding: "base64"` → entire transaction is base64 encoded

When decoding fixtures in tests, use `bs58::decode()` for the instruction data.

#### Step 4: Get Expected Field Values

Use `jsonParsed` encoding to see what the instruction parameters should be:

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

Look for the `parsed.info` object in the response - this contains the expected field values. Use field names that match what the explorer shows (e.g., "mint", "account", "authority" for SPL Token).

#### Step 5: Create Fixture JSON

Create a fixture file in `tests/fixtures/<program_name>/<instruction_type>_example.json`:

```json
{
  "description": "Brief description of what this instruction does",
  "source": "https://solscan.io/tx/<SIGNATURE>?cluster=<CLUSTER>",
  "signature": "<TRANSACTION_SIGNATURE>",
  "cluster": "devnet",
  "full_transaction_note": "Optional: This transaction has X instructions: [0] InstructionA, [1] InstructionB. We're testing instruction [N].",
  "instruction_index": 1,
  "instruction_data": "<base58-encoded instruction data from .data field>",
  "program_id": "<PROGRAM_ID>",
  "accounts": [
    {
      "pubkey": "<ACCOUNT_PUBKEY>",
      "signer": false,
      "writable": true,
      "description": "Human-readable description of this account's role"
    }
  ],
  "expected_fields": {
    "fieldname1": "expected value 1",
    "fieldname2": "expected value 2"
  }
}
```

**Fixture Fields Explained:**

- `description` - Brief human-readable explanation of the instruction
- `source` - URL to the transaction on an explorer for manual verification
- `signature` - Transaction signature (for traceability)
- `cluster` - "devnet" or "mainnet-beta"
- `full_transaction_note` - (Optional) Context about the full transaction if it has multiple instructions
- `instruction_index` - Which instruction this was in the original transaction (for reference)
- `instruction_data` - Base58-encoded instruction data from RPC response
- `program_id` - The program ID for this instruction
- `accounts` - Array of accounts with their metadata
- `expected_fields` - Key-value pairs of field names and expected values to validate

#### Step 6: Write Test Code

Create a test function that:

1. Loads the fixture using a helper function
2. Decodes the base58 instruction data
3. Creates a Solana `Instruction` struct
4. Calls your program's visualization function
5. Validates the extracted fields match expected values

Example test structure:

```rust
#[test]
fn test_instruction_from_real_transaction() {
    let fixture = load_fixture("instruction_name_example");

    // Create Instruction from fixture
    let instruction = create_instruction_from_fixture(&fixture);

    // Visualize the instruction
    let preview_layout = visualize_instruction(&instruction);

    // Validate fields
    validate_fields(&preview_layout, &fixture.expected_fields);
}
```

See `src/presets/spl_token/tests.rs` for a complete reference implementation.

#### Step 7: Run the Tests

```bash
# Run all tests for a specific program
cargo test --package visualsign-solana --lib presets::<program_name>::tests -- --nocapture

# Run a specific fixture test
cargo test --package visualsign-solana --lib presets::<program_name>::tests::test_<instruction>_real_transaction -- --nocapture
```

The `--nocapture` flag will show detailed output including:
- Transaction details and source URL
- Extracted fields from the parser
- Validation results (✓ for matches, ✗ for mismatches)

## Critical Testing Rules

### 1. NEVER Modify Fixture Data to Pass Tests

**❌ WRONG:**
```json
// Test failing? Let me "fix" the instruction_data...
{
  "instruction_data": "SomeValueICalculated"
}
```

**✓ RIGHT:**
- Fixture data represents REAL on-chain transactions
- If tests fail, fix the parser code, not the fixture
- Use explorer's parsed view to verify what expected values should be

### 2. Fixtures Are the Source of Truth

The fixture data comes from actual on-chain transactions. If your parser disagrees with the fixture:
1. First verify the fixture is correct by checking the explorer
2. Then fix the parser logic
3. Only update a fixture if you initially extracted it incorrectly

### 3. Use Base58 Decoding for Instruction Data

```rust
// ✓ RIGHT - instruction data from JSON RPC is base58 encoded
let data = bs58::decode(&fixture.instruction_data)
    .into_vec()
    .unwrap();

// ❌ WRONG - don't use base64
let data = base64::decode(&fixture.instruction_data).unwrap();
```

### 4. Match Explorer Field Names

Field names in `expected_fields` should match what users see in Solscan or Solana Explorer:

```rust
// ✓ RIGHT - matches explorer output
"mint": "5pdHyGbtCmZdJ7ye71nzeke8kcQ4ngJNPHqoDvE5L2WT"

// ❌ WRONG - doesn't match explorer
"token_mint": "5pdHyGbtCmZdJ7ye71nzeke8kcQ4ngJNPHqoDvE5L2WT"
```

This makes manual verification much easier.

## Test Infrastructure

### Fixture Test Helpers

Each program preset should implement these helper functions in its `tests.rs`:

```rust
// Load fixture from JSON file
fn load_fixture(name: &str) -> TestFixture {
    let fixture_path = format!(
        "{}/tests/fixtures/<program_name>/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    let fixture_content = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", fixture_path, e));
    serde_json::from_str(&fixture_content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {}: {}", fixture_path, e))
}

// Create Instruction from fixture data
fn create_instruction_from_fixture(fixture: &TestFixture) -> Instruction {
    let program_id = Pubkey::from_str(&fixture.program_id).unwrap();
    let accounts: Vec<AccountMeta> = fixture
        .accounts
        .iter()
        .map(|acc| {
            let pubkey = Pubkey::from_str(&acc.pubkey).unwrap();
            AccountMeta {
                pubkey,
                is_signer: acc.signer,
                is_writable: acc.writable,
            }
        })
        .collect();

    // Instruction data from JSON RPC is base58 encoded
    let data = bs58::decode(&fixture.instruction_data)
        .into_vec()
        .unwrap();

    Instruction {
        program_id,
        accounts,
        data,
    }
}

// Validate extracted fields against expected values
fn validate_fields(preview_layout: &PreviewLayout, expected_fields: &serde_json::Map<String, Value>) {
    for (key, expected_value) in expected_fields {
        let expected_str = expected_value.as_str().unwrap();

        if let Some(expanded) = &preview_layout.expanded {
            let found = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field {
                    let label_matches = common.label.to_lowercase().replace(" ", "_") == key.to_lowercase();
                    let value_matches = text_v2.text == expected_str;
                    if label_matches {
                        if value_matches {
                            println!("✓ {}: {} (matches)", key, expected_str);
                        } else {
                            println!("✗ {}: expected '{}', got '{}'", key, expected_str, text_v2.text);
                        }
                        return value_matches;
                    }
                    false
                } else {
                    false
                }
            });

            if !found {
                println!("✗ {}: field not found in output", key);
            }
        }
    }
}
```

### Fixture Struct

```rust
#[derive(Debug, serde::Deserialize)]
struct TestFixture {
    description: String,
    source: String,
    signature: String,
    cluster: String,
    #[serde(default)]
    full_transaction_note: Option<String>,
    instruction_index: usize,
    instruction_data: String,
    program_id: String,
    accounts: Vec<TestAccount>,
    expected_fields: serde_json::Map<String, Value>,
}

#[derive(Debug, serde::Deserialize)]
struct TestAccount {
    pubkey: String,
    signer: bool,
    writable: bool,
    description: String,
}
```

## Directory Structure

```
visualsign-solana/
├── TESTING.md                          # This file
├── src/
│   └── presets/
│       ├── spl_token/
│       │   ├── mod.rs                  # Implementation
│       │   └── tests.rs                # Tests
│       ├── system/
│       │   ├── mod.rs
│       │   └── tests.rs
│       └── stake_pool/
│           ├── mod.rs
│           └── tests.rs
└── tests/
    └── fixtures/
        ├── spl_token/
        │   ├── README.md               # Program-specific notes
        │   ├── mint_to_example.json
        │   ├── transfer_example.json
        │   └── ...
        ├── system/
        │   ├── README.md
        │   ├── transfer_example.json
        │   └── ...
        └── stake_pool/
            ├── README.md
            └── ...
```

## Example: Complete Test Flow

Let's walk through testing an SPL Token MintTo instruction:

1. **Find transaction**: https://solscan.io/tx/35XirCzssnAVUB2FbLrf8vYUYmTq5omepqyR8tr5Y6eJ6yurs3LcRfzGxzn92wU3w5vBvM8BfodXsscz7nin8SbC?cluster=devnet

2. **Fetch raw data**:
```bash
curl -X POST https://api.devnet.solana.com \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "35XirCzssnAVUB2FbLrf8vYUYmTq5omepqyR8tr5Y6eJ6yurs3LcRfzGxzn92wU3w5vBvM8BfodXsscz7nin8SbC",
      {"encoding":"json","maxSupportedTransactionVersion":0}
    ]
  }'
```

3. **Extract instruction [1]** (the MintTo instruction):
- instruction_data: `"6YCQpfSgHpSj"` (base58)
- accounts: [mint, destination, authority]

4. **Get expected values** using jsonParsed encoding:
```bash
curl -X POST https://api.devnet.solana.com \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "35XirCzssnAVUB2FbLrf8vYUYmTq5omepqyR8tr5Y6eJ6yurs3LcRfzGxzn92wU3w5vBvM8BfodXsscz7nin8SbC",
      {"encoding":"jsonParsed","maxSupportedTransactionVersion":0}
    ]
  }'
```

Response shows: `amount: "1230000000"`, `mint: "5pdH..."`, etc.

5. **Create fixture** at `tests/fixtures/spl_token/mint_to_example.json`

6. **Run test**:
```bash
cargo test --package visualsign-solana --lib presets::spl_token::tests::test_mint_to_real_transaction -- --nocapture
```

7. **Validate output**:
```
✓ amount: 1230000000 (matches)
✓ mint: 5pdHyGbtCmZdJ7ye71nzeke8kcQ4ngJNPHqoDvE5L2WT (matches)
✓ account: D7ZapNPiycy1imiL7deUiBXiqUGMisUuAmvAwSdxUKQT (matches)
✓ mintauthority: 9AM41swmGH1iq3L1oNnV8T385BwzVUeNUMuGqKJbiDMm (matches)
```

## Benefits of This Approach

1. **Real-world validation** - Tests use actual on-chain transaction data
2. **No network dependencies** - Fixtures are committed to git, tests run offline
3. **Fast and focused** - Each test validates one instruction in isolation
4. **Easy to debug** - Clear output shows what fields match or mismatch
5. **Regression prevention** - Fixtures ensure parser changes don't break existing functionality
6. **Documentation** - Fixtures serve as examples of real instruction usage

## Using Coverage to Find Missing Fixtures

Coverage reports help identify which instruction types need test fixtures.

**Quick start:**
```bash
# Generate HTML report and open in browser
cargo llvm-cov --package visualsign-solana --lib --open
```

Then navigate to `src/presets/<program_name>/mod.rs` to see which instruction match arms are uncovered (red).

**Coverage-driven workflow:**

1. Run coverage (see [project TESTING.md](/TESTING.md))
2. Find uncovered match arms in `src/presets/spl_token/mod.rs` (or other program presets)
3. Each uncovered instruction type → create a new fixture following the guide above
4. Re-run coverage to verify the new fixture covers the code

## Adopting This Pattern for New Programs

When adding a new program preset (e.g., Metaplex, Raydium, etc.):

1. Create `tests/fixtures/<program_name>/` directory
2. Add a program-specific README.md with any special notes
3. Implement the test helper functions in `<program_name>/tests.rs`
4. Create fixture JSON files for key instruction types
5. Write fixture tests that validate field extraction
6. Reference this TESTING.md for the overall philosophy and process
