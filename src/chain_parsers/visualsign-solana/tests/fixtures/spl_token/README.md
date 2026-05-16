# SPL Token Transaction Test Fixtures

This directory contains real transaction data from Solana explorers for validating SPL Token instruction field extraction.

## General Testing Philosophy

**See [/TESTING.md](/TESTING.md) for the complete testing guide**, including:
- Fixture philosophy and test principles
- Step-by-step guide for creating fixtures
- Critical testing rules (never modify fixture data!)
- Test infrastructure and helper functions

This README contains only SPL Token-specific notes and examples.

## SPL Token Program Details

- **Program ID**: `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`
- **Crate**: `spl-token = "7.0.0"`
- **Instruction Enum**: `spl_token::instruction::TokenInstruction`

## Covered Instructions

Current fixtures test these SPL Token instructions:
- [x] MintTo - Minting tokens to an account
- [ ] Transfer - Transferring tokens between accounts
- [ ] TransferChecked - Transfer with explicit decimals check
- [ ] Burn - Burning tokens from an account
- [ ] BurnChecked - Burn with explicit decimals check
- [ ] Approve - Delegating token authority
- [ ] ApproveChecked - Approve with explicit decimals check
- [ ] SetAuthority - Changing account authorities
- [ ] InitializeMint - Creating a new token mint
- [ ] InitializeAccount - Creating a new token account

## SPL Token-Specific Notes

### Field Name Conventions

SPL Token field names should match what Solscan's `jsonParsed` format returns:

| Our Field Name | jsonParsed Field | Description |
|----------------|------------------|-------------|
| `mint` | `mint` | Token mint address |
| `account` | `account` or `destination` | Token account address |
| `amount` | `amount` | Token amount (as string, in base units) |
| `mintAuthority` | `mintAuthority` | Mint authority address |
| `source` | `source` | Source token account |
| `destination` | `destination` | Destination token account |
| `owner` | `owner` | Account owner/signer |
| `delegate` | `delegate` | Delegated authority |

### Account Layouts

Common account ordering for SPL Token instructions:

**MintTo / MintToChecked**:
- [0] mint
- [1] destination account
- [2] mint authority

**Transfer / TransferChecked**:
- [0] source account
- [1] destination account (or mint for TransferChecked)
- [2] owner/authority

**Burn / BurnChecked**:
- [0] account to burn from (or mint for BurnChecked)
- [1] mint (for BurnChecked)
- [2] owner

**SetAuthority**:
- [0] account whose authority is being set
- [1] current authority

**Approve / ApproveChecked**:
- [0] source account
- [1] delegate
- [2] owner

See the [SPL Token documentation](https://spl.solana.com/token) for complete details.

## Running SPL Token Tests

```bash
# Run all SPL Token fixture tests
cargo test --package visualsign-solana --lib presets::spl_token::tests::fixture_tests -- --nocapture

# Run a specific fixture test
cargo test --package visualsign-solana --lib presets::spl_token::tests::test_mint_to_real_transaction -- --nocapture
```

## Creating New SPL Token Fixtures

Follow the general process in [/TESTING.md](/TESTING.md) with these SPL Token specifics:

1. **Find transaction** on Solscan or Solana Explorer
2. **Fetch with JSON encoding** to get base58 instruction data:
```bash
curl -X POST https://api.devnet.solana.com \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "<SIGNATURE>",
      {"encoding":"json","maxSupportedTransactionVersion":0}
    ]
  }' | python3 -m json.tool > transaction.json
```

3. **Get expected fields** using jsonParsed:
```bash
curl -X POST https://api.devnet.solana.com \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"getTransaction",
    "params":[
      "<SIGNATURE>",
      {"encoding":"jsonParsed","maxSupportedTransactionVersion":0}
    ]
  }' | python3 -m json.tool > transaction_parsed.json
```

Look for the instruction in the response and find the `parsed.info` object - these are your `expected_fields`.

4. **Create fixture JSON** using the field names from `parsed.info`

5. **Test and validate** - run the test and ensure all fields match ✓

## Example Fixture

See `mint_to_example.json` for a complete working example with:
- Real devnet transaction
- Base58-encoded instruction data
- Proper account metadata
- Expected fields matching Solscan output

## Contributing

When adding new SPL Token instruction fixtures:
1. ✓ Use real devnet or mainnet transactions
2. ✓ Match field names to Solscan's jsonParsed output
3. ✓ Include `full_transaction_note` if the transaction has multiple instructions
4. ✓ Add account descriptions explaining each account's role
5. ✓ Verify all expected fields pass validation
6. ✗ **Never** modify instruction_data to make tests pass
