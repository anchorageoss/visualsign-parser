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

   **Note**: The `SimpleToken.abi.json` file in this example is generated using the command above. See TESTING.md for details on generating ABI JSON files from Solidity contracts.

2. **Embed in your application** using `include_str!` macro:
   ```rust
   const MY_CONTRACT_ABI: &str = include_str!("path/to/SimpleToken.abi.json");
   ```

3. **Register in ABI registry**:
   ```rust
   use visualsign_ethereum::embedded_abis::register_embedded_abi;
   use visualsign_ethereum::abi_registry::AbiRegistry;

   let mut registry = AbiRegistry::new();
   register_embedded_abi(&mut registry, "SimpleToken", MY_CONTRACT_ABI)?;
   registry.map_address(1, contract_address, "SimpleToken");
   ```

   **Note**: The `contract_address` must include the `0x` prefix when parsing to `alloy_primitives::Address`.

4. **Use with parser CLI** (pass ABI JSON file paths and address mappings):
   ```bash
   cargo run -p parser_cli -- \
     --chain ethereum \
     --transaction 0x... \
     --abi-json-mappings SimpleToken:SimpleToken.json:0x1234567890123456789012345678901234567890
   ```

5. **Or via Rust code** in your application

### Using the Example

#### Via Rust Code

```rust
use visualsign_ethereum::embedded_abis::register_embedded_abi;
use visualsign_ethereum::abi_registry::AbiRegistry;
use visualsign_ethereum::contracts::core::DynamicAbiVisualizer;
use visualsign_ethereum::visualizer::CalldataVisualizer;
use std::sync::Arc;

const SIMPLE_TOKEN_ABI: &str = include_str!("contracts/SimpleToken.abi.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create registry and register ABI
    let mut registry = AbiRegistry::new();
    register_embedded_abi(&mut registry, "SimpleToken", SIMPLE_TOKEN_ABI)?;

    // Get the ABI for a specific address (requires prior registration)
    let contract_address: alloy_primitives::Address =
        "0x1234567890123456789012345678901234567890".parse()?;
    registry.map_address(1, contract_address, "SimpleToken");

    // Retrieve and create visualizer
    if let Some(abi) = registry.get_abi_for_address(1, contract_address) {
        let visualizer = DynamicAbiVisualizer::new(abi);

        // Decode function call (transfer: a9059cbb)
        let calldata = hex::decode("a9059cbb0000000000000000000000001234567890123456789012345678901234567890")?;

        if let Some(field) = visualizer.visualize_calldata(&calldata, 1, None) {
            println!("Visualization: {:#?}", field);
        } else {
            println!("Could not visualize");
        }
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

## CLI Integration

The parser CLI now supports the `--abi-json-mappings` flag for mapping custom ABI JSON files to contract addresses:

### Format

```
--abi-json-mappings AbiName:FilePath:0xAddress
```

### Multiple Mappings

You can provide multiple `--abi-json-mappings` flags to register different ABIs:

```bash
cargo run -p parser_cli -- \
  --chain ethereum \
  --transaction 0x... \
  --abi-json-mappings Token:token.json:0x1111111111111111111111111111111111111111 \
  --abi-json-mappings Router:router.json:0x2222222222222222222222222222222222222222
```

### Validation

The CLI validates each ABI mapping and reports:
- Successfully mapped ABIs (logged to stderr)
- Invalid format warnings (logged to stderr)
- Final registration summary

## Supported Features

- ✅ Compile-time ABI embedding with `include_str!`
- ✅ Per-chain address mapping (register same ABI on multiple chains)
- ✅ Function selector matching (4-byte opcodes)
- ✅ Structured PreviewLayout visualization
- ✅ Multiple ABIs per binary
- ✅ CLI `--abi-json-mappings` flag for address mapping
- 📋 Optional ABI signatures (secp256k1) for validation (planned)

## Limitations

- ⚠️ No runtime parameter decoding (type-safe decoding requires compile-time generation)
- ⚠️ Parameters shown with type names, not decoded values (future enhancement)
- ⚠️ Fallback-only - doesn't override built-in visualizers (Uniswap, ERC20, etc.)
- ⚠️ Signature validation not yet implemented (will be required when specified)

## Next Steps

See the full implementation guides:
- [CLAUDE.md](../../CLAUDE.md) - Module development guidelines
- [DECODER_GUIDE.md](../../DECODER_GUIDE.md) - Writing custom decoders
