# Jupiter Swap Transaction Test Fixtures

This directory contains real transaction data from Solana explorers for validating Jupiter Swap instruction field extraction.

## General Testing Philosophy

**See [/TESTING.md](/TESTING.md) for the complete testing guide**, including:
- Fixture philosophy and test principles
- Step-by-step guide for creating fixtures
- Critical testing rules (never modify fixture data!)
- Test infrastructure and helper functions

This README contains only Jupiter Swap-specific notes and examples.

## Jupiter Swap Program Details

- **Program ID**: `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4` (Jupiter v6)
- **Crate**: `jupiter-swap-api-client = "0.2.0"` (for types)
- **Instruction Types**: Discriminator-based (8-byte prefixes)
  - Route: `[0xe5, 0x17, 0xcb, 0x97, 0x7a, 0xe3, 0xad, 0x2a]`
  - ExactOutRoute: `[0x4b, 0xd7, 0xdf, 0xa8, 0x0c, 0xd0, 0xb6, 0x2a]`
  - SharedAccountsRoute: `[0x3a, 0xf2, 0xaa, 0xae, 0x2f, 0xb6, 0xd4, 0x2a]`

## Covered Instructions

Current fixtures test these Jupiter instructions:
- [ ] Route - Standard swap with in_amount specified
- [ ] ExactOutRoute - Swap with exact out_amount specified
- [ ] SharedAccountsRoute - Swap using shared accounts optimization

## Jupiter-Specific Notes

### Field Name Conventions

Jupiter field names should match common swap terminology:

| Our Field Name | Description |
|----------------|-------------|
| `instruction` | Instruction type (e.g., "Route", "Exact Out Route") |
| `in_amount` | Input token amount |
| `out_amount` | Output token amount (or minimum for slippage) |
| `slippage_bps` | Slippage tolerance in basis points |
| `in_token` | Input token symbol (if available) |
| `out_token` | Output token symbol (if available) |

### Data Layout

Jupiter instructions use:
- First 8 bytes: Discriminator (identifies instruction type)
- Last 16 bytes: Amount data (in_amount as u64 LE, out_amount as u64 LE)
- Middle bytes: Routing and account information

## Running Jupiter Swap Tests

```bash
# Run all Jupiter Swap fixture tests
cargo test --package visualsign-solana --lib presets::jupiter_swap::tests::fixture_tests -- --nocapture

# Run a specific fixture test
cargo test --package visualsign-solana --lib presets::jupiter_swap::tests::test_route_real_transaction -- --nocapture
```

## Creating New Jupiter Swap Fixtures

Follow the general process in [/TESTING.md](/TESTING.md) with these Jupiter specifics:

1. **Find transaction** on Solscan or Solana Explorer
   - Look for Jupiter v6 transactions: `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4`

2. **Fetch with JSON encoding** to get base58 instruction data:
```bash
curl -X POST https://api.mainnet-beta.solana.com \
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

3. **Extract instruction data**
   - Look for the Jupiter program ID in accountKeys
   - Find the instruction with that programIdIndex
   - The `.data` field is base58-encoded

4. **Identify instruction type**
   - Decode the first 8 bytes to match against discriminators
   - Route: `e517cb977ae3ad2a` (hex)
   - ExactOutRoute: `4bd7dfa80cd0b62a` (hex)
   - SharedAccountsRoute: `3af2aaae2fb6d42a` (hex)

5. **Create fixture JSON** with expected fields

6. **Test and validate**

## Example: Finding a Jupiter Swap Transaction

1. Go to https://solscan.io/
2. Search for Jupiter v6 program: `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4`
3. Click on "Transactions" tab
4. Pick a recent swap transaction
5. Extract the instruction data following the process above

## Contributing

When adding new Jupiter Swap instruction fixtures:
1. ✓ Use real mainnet or devnet transactions
2. ✓ Include transaction context in `full_transaction_note` if there are multiple instructions
3. ✓ Verify the discriminator matches the instruction type
4. ✓ Add clear descriptions explaining what the swap is doing
5. ✓ Verify all expected fields pass validation
6. ✗ **Never** modify instruction_data to make tests pass
