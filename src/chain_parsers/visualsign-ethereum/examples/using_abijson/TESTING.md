# Testing ABI Embedding with Real Contracts

This guide shows how to test the ABI embedding feature using real contract ABIs pulled from the blockchain.

## Prerequisites

Install `cast` from the Foundry toolkit:

```bash
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

Verify installation:
```bash
cast --version
```

## Getting Real ABIs

### Method 1: Using curl + Etherscan API (Recommended)

Get a free Etherscan API key at: https://etherscan.io/apis

```bash
ETHERSCAN_API_KEY="YOUR_API_KEY"

# Get USDC ABI
curl -s "https://api.etherscan.io/api" \
  -d "module=contract" \
  -d "action=getabi" \
  -d "address=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48" \
  -d "apikey=$ETHERSCAN_API_KEY" | jq '.result' > usdc.abi.json

# Get WETH ABI
curl -s "https://api.etherscan.io/api" \
  -d "module=contract" \
  -d "action=getabi" \
  -d "address=0xc02aaa39b223fe8d0a0e8d0c9f8d0b21d0a0e8d0c" \
  -d "apikey=$ETHERSCAN_API_KEY" | jq '.result' > weth.abi.json
```

### Method 2: Using `cast` to test calldata

While `cast` may not have `abi` subcommand in all versions, you can use it to work with calldata:

```bash
# Encode function call
cast calldata "transfer(address,uint256)" \
  0x1234567890123456789012345678901234567890 \
  1000000

# Decode calldata with an ABI
cast abi-decode "transfer(address,uint256)" \
  "0xa9059cbb0000000000000000000000001234567890123456789012345678901234567890000000000000000000000000000000000000000000000000000000000f4240"
```

### Method 3: Online ABI repositories

- **Etherscan UI**: Visit `etherscan.io` → Search address → Contract tab → Copy ABI
- **4byte.directory**: https://www.4byte.directory/ (for function signatures)
- **OpenZeppelin**: Pre-made standard ABIs (ERC20, ERC721, etc.)

Example (ERC20 standard):
```bash
# Save a standard ERC20 ABI
curl -s https://raw.githubusercontent.com/OpenZeppelin/openzeppelin-contracts/master/build/contracts/ERC20.json \
  | jq '.abi' > erc20_standard.abi.json
```

## Testing Locally

### Step 1: Pull Example ABIs

```bash
cd examples/using_abijson

# Set your Etherscan API key
export ETHERSCAN_API_KEY="YOUR_API_KEY"

# Get USDC ABI
curl -s "https://api.etherscan.io/api" \
  -d "module=contract" \
  -d "action=getabi" \
  -d "address=0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48" \
  -d "apikey=$ETHERSCAN_API_KEY" | jq '.result' > contracts/USDC.abi.json

# Get USDT ABI
curl -s "https://api.etherscan.io/api" \
  -d "module=contract" \
  -d "action=getabi" \
  -d "address=0xdac17f958d2ee523a2206206994597c13d831ec7" \
  -d "apikey=$ETHERSCAN_API_KEY" | jq '.result' > contracts/USDT.abi.json

# Verify ABIs are valid
jq '.[] | select(.name == "transfer") | .inputs' contracts/USDC.abi.json
```

### Step 2: Create a Test Binary with Embedded ABIs

Create `examples/using_abijson/main.rs`:

```rust
use visualsign_ethereum::embedded_abis::register_embedded_abi;
use visualsign_ethereum::abi_registry::AbiRegistry;
use visualsign_ethereum::contracts::core::DynamicAbiVisualizer;
use visualsign_ethereum::visualizer::CalldataVisualizer;
use std::sync::Arc;

// Embed real contract ABIs
const USDC_ABI: &str = include_str!("contracts/USDC.abi.json");
const USDT_ABI: &str = include_str!("contracts/USDT.abi.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create and populate registry
    let mut registry = AbiRegistry::new();
    register_embedded_abi(&mut registry, "USDC", USDC_ABI)?;
    register_embedded_abi(&mut registry, "USDT", USDT_ABI)?;

    // Map to known addresses (Ethereum mainnet)
    let usdc_addr: alloy_primitives::Address =
        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse()?;
    let usdt_addr: alloy_primitives::Address =
        "0xdac17f958d2ee523a2206206994597c13d831ec7".parse()?;

    registry.map_address(1, usdc_addr, "USDC");
    registry.map_address(1, usdt_addr, "USDT");

    println!("Registry created with 2 ABIs:");
    println!("  - USDC: {}", usdc_addr);
    println!("  - USDT: {}", usdt_addr);
    println!();

    // Test: Decode USDC transfer
    println!("Testing USDC transfer decoding...");
    if let Some(abi) = registry.get_abi_for_address(1, usdc_addr) {
        let visualizer = DynamicAbiVisualizer::new(abi);

        // transfer(address to, uint256 amount)
        // selector: 0xa9059cbb
        let recipient: alloy_primitives::Address =
            "0x1234567890123456789012345678901234567890".parse()?;
        let amount = 1_000_000u128; // 1 USDC (6 decimals)

        // Build calldata
        let mut calldata = vec![0xa9, 0x05, 0x9c, 0xbb]; // transfer selector
        calldata.extend_from_slice(&[0u8; 32]); // Pad to 32 bytes for address
        calldata[4 + 12..4 + 32].copy_from_slice(recipient.as_slice()); // Copy address
        calldata.extend_from_slice(&amount.to_be_bytes().to_vec()); // amount (right-padded)

        if let Some(field) = visualizer.visualize_calldata(&calldata, 1, None) {
            println!("✓ Successfully visualized USDC transfer");
            println!("  Field: {:#?}", field);
        } else {
            println!("✗ Could not visualize");
        }
    }

    Ok(())
}
```

### Step 3: Run the Example

```bash
# From the project root
cargo run --example using_abijson
```

Output should show:
```
Registry created with 2 ABIs:
  - USDC: 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48
  - USDT: 0xdac17f958d2ee523a2206206994597c13d831ec7

Testing USDC transfer decoding...
✓ Successfully visualized USDC transfer
  Field: ...
```

## Testing with CLI

### Method 1: Generate Calldata with cast

```bash
# Get function selector
cast sig "transfer(address,uint256)"
# Output: 0xa9059cbb

# Generate full calldata using cast calldata
CALLDATA=$(cast calldata "transfer(address,uint256)" \
  0x1234567890123456789012345678901234567890 \
  1000000)

echo "Generated calldata: $CALLDATA"
# Output: 0xa9059cbb000000000000000000000000123456789012345678901234567890123456789000000000000000000000000000000000000000000000000000000000000f4240

# Now you can test with the parser
# Note: The CLI expects full transactions, not just calldata
# For testing, you may need to wrap this in a transaction format
```

### Method 2: Working with Function Signatures

```bash
# Get signatures for multiple USDC functions
cast sig "transfer(address,uint256)"        # 0xa9059cbb
cast sig "approve(address,uint256)"         # 0x095ea7b3
cast sig "transferFrom(address,address,uint256)"  # 0x23b872dd
cast sig "balanceOf(address)"               # 0x70a08231
```

### Method 3: Testing with SimpleToken Example

Use the built-in SimpleToken example (doesn't need external ABIs):

```bash
# Build calldata for SimpleToken.mint(address, uint256)
MINT_SELECTOR=$(cast sig "mint(address,uint256)")
echo "mint selector: $MINT_SELECTOR"

# Generate mint calldata
MINT_CALLDATA=$(cast calldata "mint(address,uint256)" \
  0x1234567890123456789012345678901234567890 \
  1000000)

echo "mint calldata: $MINT_CALLDATA"

# Test parsing
cargo test -p visualsign-ethereum --lib embedded_abis::tests
```

## Real Contract Examples

### USDC Token (0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48)
```bash
# Minimal functions: transfer, transferFrom, approve, balanceOf, allowance
cast abi 0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 | jq '.[] | select(.name | IN("transfer", "transferFrom", "approve"))'
```

### WETH (0xc02aaa39b223fe8d0a0e8d0c9f8d0b21d0a0e8d0c)
```bash
cast abi 0xc02aaa39b223fe8d0a0e8d0c9f8d0b21d0a0e8d0c | jq '.[] | select(.name | IN("deposit", "withdraw"))'
```

### Uniswap V3 SwapRouter (0xe592427a0aece92de3edee1f18e0157c05861564)
```bash
cast abi 0xe592427a0aece92de3edee1f18e0157c05861564 | jq '.[] | select(.name | IN("exactInputSingle", "exactOutputSingle"))'
```

## Validating ABI JSON

```bash
# Verify ABI is valid JSON
jq . contracts/USDC.abi.json > /dev/null && echo "Valid JSON"

# Count functions
jq '[.[] | select(.type == "function")] | length' contracts/USDC.abi.json

# List all function names
jq -r '.[].name' contracts/USDC.abi.json | sort | uniq
```

## Common Issues & Solutions

### `cast` command not found
```bash
# Make sure Foundry is in your PATH
export PATH="$HOME/.foundry/bin:$PATH"

# Or reinstall if needed
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

### Etherscan API returns empty response
```bash
# Check your API key
ETHERSCAN_API_KEY="YOUR_KEY"
curl "https://api.etherscan.io/api?module=account&action=balance&address=0x0000000000000000000000000000000000000000&apikey=$ETHERSCAN_API_KEY"

# If you get {"status":"0"}, your key is invalid
# Get a free key: https://etherscan.io/apis
```

### Invalid ABI JSON from curl
```bash
# Check the raw response
curl -s "https://api.etherscan.io/api" \
  -d "module=contract" \
  -d "action=getabi" \
  -d "address=0xINVALID" \
  -d "apikey=$ETHERSCAN_API_KEY" | jq .

# You'll see: {"status":"0","message":"Contract source code not verified"}
```

### Address format issues
Always use lowercase or checksummed addresses:
```rust
// Works - lowercase
let addr: alloy_primitives::Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse()?;

// Also works - checksummed
let addr: alloy_primitives::Address = "0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse()?;
```

### Function selector mismatch
```bash
# Double-check function signature (must match ABI exactly)
cast sig "transfer(address,uint256)"  # Correct
cast sig "transfer(address,uint)"     # Wrong - uint must be uint256

# Verify against ABI
jq '.[] | select(.name == "transfer") | .inputs' contracts/USDC.abi.json
```

## Next Steps

Once you have working ABIs:

1. **Add to version control**: Commit `*.abi.json` files to your repo
2. **Create multiple examples**: One for each protocol (Uniswap, Aave, etc.)
3. **Add to CI**: Include ABI validation in CI/CD pipeline
4. **Document formats**: Add notes about ABI version and generation date

## Testing Script

Create `fetch_abis.sh`:

```bash
#!/bin/bash
set -e

# Configuration
ETHERSCAN_API_KEY="${ETHERSCAN_API_KEY:-}"
CONTRACTS_DIR="contracts"

if [ -z "$ETHERSCAN_API_KEY" ]; then
    echo "Error: ETHERSCAN_API_KEY not set"
    echo "Get a free key at: https://etherscan.io/apis"
    exit 1
fi

mkdir -p "$CONTRACTS_DIR"

# Array of (address:name) pairs
CONTRACTS=(
    "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48:USDC"
    "0xc02aaa39b223fe8d0a0e8d0c9f8d0b21d0a0e8d0c:WETH"
    "0xdac17f958d2ee523a2206206994597c13d831ec7:USDT"
)

echo "Fetching ABIs from Etherscan..."
for contract_info in "${CONTRACTS[@]}"; do
    IFS=':' read -r address name <<< "$contract_info"
    echo "  Fetching $name ($address)..."

    response=$(curl -s "https://api.etherscan.io/api" \
      -d "module=contract" \
      -d "action=getabi" \
      -d "address=$address" \
      -d "apikey=$ETHERSCAN_API_KEY")

    # Extract ABI from response
    echo "$response" | jq '.result' > "${CONTRACTS_DIR}/${name}.abi.json"

    # Check if we got valid ABI
    if jq empty "${CONTRACTS_DIR}/${name}.abi.json" 2>/dev/null; then
        echo "    ✓ Saved to ${CONTRACTS_DIR}/${name}.abi.json"
    else
        echo "    ✗ Failed to fetch $name"
        cat "${CONTRACTS_DIR}/${name}.abi.json"
    fi
done

echo ""
echo "Verifying ABIs..."
for contract_info in "${CONTRACTS[@]}"; do
    IFS=':' read -r address name <<< "$contract_info"
    count=$(jq '[.[] | select(.type == "function")] | length' "${CONTRACTS_DIR}/${name}.abi.json" 2>/dev/null || echo "0")
    echo "  $name: $count functions"
done

echo ""
echo "Running tests..."
cargo test -p visualsign-ethereum --lib embedded_abis
```

Run it:
```bash
export ETHERSCAN_API_KEY="YOUR_API_KEY"
chmod +x fetch_abis.sh
./fetch_abis.sh
```

Quick test without fetching:
```bash
# Just run existing tests
cargo test -p visualsign-ethereum --lib embedded_abis

# Or test with cast
cast sig "mint(address,uint256)"
cast calldata "mint(address,uint256)" 0x1234567890123456789012345678901234567890 1000000
```
