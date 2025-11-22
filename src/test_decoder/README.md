# Test Decoder Tool

Quick testing tool for Ethereum contract decoders. Just change the calldata and run!

## How to Use

1. **Edit the configuration** in `src/main.rs`:
   - **Line 23**: Choose protocol (`"aave"`, `"morpho"`, or `"uniswap"`)
   - **Line 28**: Paste your calldata (with or without `0x` prefix)
   - **Line 31**: Set chain ID (1 = Ethereum, 137 = Polygon, etc.)

2. **Run the test**:
   ```bash
   cd /Users/vani/Projects/ciganija/visualsign-parser/src
   cargo run -p test_decoder
   ```

## Example

To test an Aave supply transaction:

```rust
let protocol = "aave";
let calldata_hex = "617ba037000000000000000000000000dac17f958d2ee523a2206206994597c13d831ec7...";
let chain_id = 1;
```

Then run:
```bash
cargo run -p test_decoder
```

## Supported Protocols

- **aave**: Aave v3 Pool (supply, withdraw, borrow, repay, liquidationCall)
- **morpho**: Morpho Bundler (multicall operations)
- **uniswap**: Uniswap Universal Router (swap commands)

## Getting Calldata

From Etherscan transaction page:
1. Go to the transaction
2. Click "Click to see More"
3. Copy the "Input Data" hex string
4. Paste it into line 28 of `src/main.rs`

## Output

The tool will show:
- ✅ Decode success/failure
- 📋 Transaction summary
- 📊 Detailed parameters
- Token symbols and formatted amounts
