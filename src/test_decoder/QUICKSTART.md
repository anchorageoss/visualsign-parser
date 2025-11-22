# Quick Start - Test Any Transaction in 3 Steps

## Step 1: Edit `src/main.rs`

Open `/Users/vani/Projects/ciganija/visualsign-parser/src/test_decoder/src/main.rs`

### Change Line 23 - Pick Protocol
```rust
let protocol = "aave";     // ← Change this to "aave", "morpho", or "uniswap"
```

### Change Line 28 - Paste Calldata
```rust
let calldata_hex = "617ba037...";  // ← Paste your transaction calldata here
```

### Change Line 31 - Set Chain ID (optional)
```rust
let chain_id = 1;  // ← 1=Ethereum, 137=Polygon, 42161=Arbitrum, etc.
```

## Step 2: Run It

```bash
cd /Users/vani/Projects/ciganija/visualsign-parser/src
cargo run -p test_decoder
```

## Step 3: See Results

You'll get:
```
╔════════════════════════════════════════════════════════════╗
║                    ✅ DECODE SUCCESS!                      ║
╚════════════════════════════════════════════════════════════╝

📋 Label: Aave Supply
📝 Summary: Supply 110000.000000 USDT on behalf of 0xb655...

📊 Detailed Parameters:
  1. Asset: USDT (0xdac1...)
  2. Amount: 110000.000000 USDT
  3. On Behalf Of: 0xb655...
```

## Where to Get Calldata

**From Etherscan:**
1. Open transaction (e.g., https://etherscan.io/tx/0x394da...)
2. Scroll to "Input Data"
3. Click "View Input As: Original"
4. Copy the hex string (starts with `0x`)
5. Paste into line 28

**From Cast:**
```bash
cast tx 0x394da4860478e24eaf99007a617f2009ed6a4c2f3a9ac43cf4da1e8ad1db2400 --rpc-url $ETH_RPC_URL | grep input
```

## Examples

### Aave Supply (110k USDT)
```rust
let protocol = "aave";
let calldata_hex = "617ba037000000000000000000000000dac17f958d2ee523a2206206994597c13d831ec7000000000000000000000000000000000000000000000000000000199c82cc00000000000000000000000000b6559478b59836376da9937c4c697ddb21779e490000000000000000000000000000000000000000000000000000000000000000";
let chain_id = 1;
```

### Morpho Multicall
```rust
let protocol = "morpho";
let calldata_hex = "YOUR_MORPHO_CALLDATA_HERE";
let chain_id = 1;
```

### Uniswap Swap
```rust
let protocol = "uniswap";
let calldata_hex = "YOUR_UNISWAP_CALLDATA_HERE";
let chain_id = 1;
```

---

That's it! Just edit 3 lines and run. No need to touch test files or recompile the whole project.
