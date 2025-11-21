# Using Embedded ABI JSON with VisualSign Parser

This example demonstrates how to use compile-time embedded ABI JSON files with the visualsign-parser to enable transaction visualization for custom contracts.

## Why Compile-Time Embedding?

Like the `sol!` macro used throughout the parser, ABIs must be embedded at compile-time:

- **Security**: ABIs are validated during compilation, not loaded at runtime
- **Performance**: No file I/O or JSON parsing overhead at runtime
- **Determinism**: Same binary always uses the same ABIs
- **Simplicity**: No external file dependencies to manage

## Quick Start

### For Dapp Developers

To enable visualization for your custom contract:

1. **Generate ABI JSON** from your Solidity contract:
   ```bash
   solc --abi SimpleToken.sol > SimpleToken.abi.json
   ```

2. **Embed in your application** using `include_str!` macro:
   ```rust
   const MY_CONTRACT_ABI: &str = include_str!("path/to/SimpleToken.abi.json");
   ```

3. **Register in ABI registry**:
   ```rust
   use visualsign_ethereum::abi_registry::AbiRegistry;

   let mut registry = AbiRegistry::new();
   registry.register_abi("SimpleToken", MY_CONTRACT_ABI)?;
   registry.map_address(1, contract_address, "SimpleToken");
   ```

4. **Pass to parser** via CLI or gRPC

### Using the Example

#### Via CLI

```bash
# Decode a transaction to a SimpleToken contract
cargo run --example using_abijson -- \
  --chain ethereum \
  --transaction <hex_calldata> \
  --abi SimpleToken:0x<contract_address>
```

#### Via Rust Code

```rust
use visualsign_ethereum::abi_registry::AbiRegistry;
use visualsign_ethereum::contracts::core::DynamicAbiVisualizer;
use visualsign_ethereum::visualizer::CalldataVisualizer;

const SIMPLE_TOKEN_ABI: &str = include_str!("contracts/SimpleToken.abi.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse ABI
    let abi: alloy_json_abi::JsonAbi = serde_json::from_str(SIMPLE_TOKEN_ABI)?;

    // Create visualizer
    let visualizer = DynamicAbiVisualizer::new(std::sync::Arc::new(abi));

    // Decode function call
    let calldata = hex::decode("a9059cbb...")?; // Example calldata
    let visualization = visualizer.visualize_calldata(&calldata, 1, None);

    match visualization {
        Some(field) => println!("Visualization: {:?}", field),
        None => println!("Could not visualize"),
    }

    Ok(())
}
```

## How It Works

1. **ABI Parsing**: The JSON ABI is embedded at compile-time using `include_str!`
2. **Function Selection**: The 4-byte selector is used to find matching functions
3. **Visualization**: Parameters are displayed in a structured PreviewLayout

Example visualization output for `mint(address to, uint256 amount)`:
```
mint(address,uint256)
├── to: 0x1234...
└── amount: 1000000000000000000
```

## Supported Features

- ✅ Compile-time ABI embedding with `include_str!`
- ✅ Per-chain address mapping
- ✅ Function selector matching (4-byte opcodes)
- ✅ Structured PreviewLayout visualization
- ✅ Multiple ABIs per binary
- ✅ Optional ABI signatures (secp256k1) for validation

## Limitations

- ⚠️ No runtime parameter decoding (type-safe decoding requires compile-time generation)
- ⚠️ Parameters shown with type names, not decoded values (future enhancement)
- ⚠️ Fallback-only - doesn't override built-in visualizers (Uniswap, ERC20, etc.)

## Next Steps

See the full implementation guides:
- [CLAUDE.md](../../CLAUDE.md) - Module development guidelines
- [DECODER_GUIDE.md](../../DECODER_GUIDE.md) - Writing custom decoders
