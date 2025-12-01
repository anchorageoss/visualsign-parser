# VisualSign Ethereum Module Guidelines

## Field Builders

The `visualsign` crate provides field builder functions that reduce boilerplate when creating payload fields. Always use these rather than constructing field structs directly.

### Available Functions

Import from `visualsign::field_builders`:

#### `create_text_field(label: &str, text: &str) -> Result<AnnotatedPayloadField>`

Creates a TextV2 field. Use for simple text display (network names, addresses, etc).

```rust
use visualsign::field_builders::create_text_field;

let field = create_text_field("Network", "Ethereum Mainnet")?;
```

#### `create_amount_field(label: &str, amount: &str, abbreviation: &str) -> Result<AnnotatedPayloadField>`

Creates an AmountV2 field with token symbol. Validates that amount is a proper signed decimal number.

```rust
use visualsign::field_builders::create_amount_field;

let field = create_amount_field("Value", "1.5", "USDC")?;
```

#### `create_number_field(label: &str, number: &str, unit: &str) -> Result<AnnotatedPayloadField>`

Creates a Number field with optional unit. Similar to amount but without requiring a symbol.

```rust
use visualsign::field_builders::create_number_field;

let field = create_number_field("Gas Limit", "21000", "units")?;
```

#### `create_address_field(label: &str, address: &str, name: Option<&str>, memo: Option<&str>, asset_label: Option<&str>, badge_text: Option<&str>) -> Result<AnnotatedPayloadField>`

Creates an AddressV2 field with optional metadata.

```rust
use visualsign::field_builders::create_address_field;

let field = create_address_field(
    "To",
    "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
    Some("Vitalik"),
    None,
    Some("ETH"),
    Some("Founder"),
)?;
```

#### `create_raw_data_field(data: &[u8], optional_fallback_string: Option<String>) -> Result<AnnotatedPayloadField>`

Creates a TextV2 field for raw bytes. Displays as hex by default.

```rust
use visualsign::field_builders::create_raw_data_field;

let field = create_raw_data_field(b"calldata", None)?;
```

### Number Validation

All amount and number fields validate the input using a regex pattern:

- Valid: `123`, `123.45`, `-123.45`, `+678.90`, `0`, `0.0`
- Invalid: `-.45`, `123.`, `abc`, `12.3.4`, `--1`

## Token Metadata

The `token_metadata` module provides canonical wallet format for token data:

```rust
use crate::token_metadata::{ChainMetadata, TokenMetadata, ErcStandard, parse_network_id};

// Parse network identifier to chain ID
let chain_id = parse_network_id("ETHEREUM_MAINNET")?; // Returns 1

// Create token metadata
let token = TokenMetadata {
    symbol: "USDC".to_string(),
    name: "USD Coin".to_string(),
    erc_standard: ErcStandard::Erc20,
    contract_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
    decimals: 6,
};

// Hash protobuf bytes
let hash = compute_metadata_hash(protobuf_bytes);
```

### Supported Networks

- `ETHEREUM_MAINNET` → chain_id: 1
- `POLYGON_MAINNET` → chain_id: 137
- `ARBITRUM_MAINNET` → chain_id: 42161
- `OPTIMISM_MAINNET` → chain_id: 10
- `BASE_MAINNET` → chain_id: 8453

## Registry

The `ContractRegistry` maps `(chain_id, Address) -> TokenMetadata` for efficient lookups:

```rust
use crate::registry::ContractRegistry;

let mut registry = ContractRegistry::new();

// Register token with metadata
registry.register_token(1, token_metadata)?;

// Get token symbol
let symbol = registry.get_token_symbol(1, address);

// Format token amount with proper decimals
let formatted = registry.format_token_amount(1, address, raw_amount);

// Load from wallet metadata
registry.load_chain_metadata(&chain_metadata)?;
```

## Context and Visualization

The `VisualizerContext` provides execution context for transaction visualization:

```rust
use crate::context::{VisualizerContext, VisualizerContextParams};

let params = VisualizerContextParams {
    chain_id,
    sender: sender_address,
    current_contract: contract_address,
    calldata,
    registry,
    visualizers,
};
let context = VisualizerContext::new(params);

// Create nested call context
let nested = context.for_nested_call(nested_contract, nested_calldata);
```

## Best Practices

1. **Always use field builders** - Don't construct SignablePayloadField structs directly
2. **Handle errors** - All field builders return `Result` types
3. **Prefer canonical types** - Use `TokenMetadata` from `token_metadata` module
4. **Use registry for lookups** - Don't duplicate token metadata storage
5. **Network ID mapping** - Always use `parse_network_id()` to convert string IDs to chain IDs
6. **Validate amounts** - Field builders validate number formats automatically
7. **Chain ID + Address as key** - Always use (chain_id, Address) tuple for token lookups

## Module Structure

```
src/
├── lib.rs                 - Main entry point, re-exports
├── chains.rs              - Chain name mappings
├── context.rs             - VisualizerContext for transaction context
├── contracts/             - Contract-specific visualizers (ERC20, Uniswap, etc)
├── fmt.rs                 - Formatting utilities (ether, gwei, etc)
├── protocols/             - Protocol-specific handlers
├── registry.rs            - ContractRegistry for metadata lookup
├── token_metadata.rs      - Canonical wallet token format
└── visualizer.rs          - VisualizerRegistry and builder
```

## Milestone 1.1 - Token and Contract Registry

- `TokenMetadata`: canonical wallet token format with symbol, name, erc_standard, contract_address, decimals
- `ChainMetadata`: grouping of tokens by network, sent from wallets as protobuf
- `parse_network_id()`: maps network identifiers to chain IDs
- `compute_metadata_hash()`: SHA256 hashing of protobuf metadata bytes
- `ContractRegistry`: (chain_id, Address) → TokenMetadata mapping for efficient lookups
- Field builders from visualsign: reusable field construction utilities

## Common Patterns

### Creating transaction fields

```rust
use visualsign::field_builders::*;

let mut fields = vec![
    create_text_field("Network", "Ethereum Mainnet")?,
    create_address_field("To", "0x...", None, None, None, None)?,
    create_amount_field("Value", "1.5", "ETH")?,
    create_number_field("Gas Limit", "21000", "")?,
];
```

### Formatting token amounts

```rust
use crate::registry::ContractRegistry;

if let Some((formatted, symbol)) = registry.format_token_amount(chain_id, token_address, raw_amount) {
    let field = create_amount_field("Amount", &formatted, &symbol)?;
    // Use field...
}
```

### Loading wallet metadata

```rust
use crate::registry::ContractRegistry;

let mut registry = ContractRegistry::new();
registry.load_chain_metadata(&wallet_metadata)?;

// Now all tokens from wallet are indexed by (chain_id, address)
```

## Solidity Protocol Decoders

All protocol decoders (Uniswap, future Aave, etc.) follow a clean, repeatable pattern using the `sol!` macro from alloy.

### Decoder Pattern

Every decoder has 4 steps:

1. **Define struct with sol!** - Type-safe parameter structure
2. **Decode or handle error** - Use `StructName::abi_decode(bytes)`
3. **Resolve tokens from registry** - Get symbols and format amounts
4. **Return TextV2 field** - Human-readable summary

### Example: Simple Decoder

```rust
fn decode_operation(
    bytes: &[u8],
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    // Step 1: Decode parameters
    let params = match OperationParams::abi_decode(bytes) {
        Ok(p) => p,
        Err(_) => {
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Operation: 0x{}", hex::encode(bytes)),
                    label: "Operation".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Failed to decode parameters".to_string(),
                },
            };
        }
    };

    // Step 2: Resolve token symbols via registry
    let token_symbol = registry
        .and_then(|r| r.get_token_symbol(chain_id, params.token))
        .unwrap_or_else(|| format!("{:?}", params.token));

    // Step 3: Format amount with decimals
    let (amount_str, _) = registry
        .and_then(|r| {
            let amount: u128 = params.amount.to_string().parse().ok()?;
            r.format_token_amount(chain_id, params.token, amount)
        })
        .unwrap_or_else(|| (params.amount.to_string(), token_symbol.clone()));

    // Step 4: Create human-readable summary
    let text = format!("Operation with {} {}", amount_str, token_symbol);

    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: text.clone(),
            label: "Operation".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text },
    }
}
```

### Defining Parameter Structs

Use the `sol!` macro to define all parameters:

```rust
sol! {
    struct SwapParams {
        address tokenIn;
        address tokenOut;
        uint256 amountIn;
        uint256 minAmountOut;
    }

    struct TransferParams {
        address to;
        uint256 amount;
        bytes data;
    }
}
```

**Benefits:**

- Automatic type-safe ABI decoding
- No manual byte parsing needed
- Compile-time correctness

### Well-Known Address Resolution

Use the registry to get well-known addresses across protocols:

```rust
use crate::registry::WellKnownAddress;

// Universal addresses (same on all chains)
let permit2_addr = registry.get_well_known_address(WellKnownAddress::Permit2, chain_id)?;

// Chain-specific addresses
let weth_addr = registry.get_well_known_address(WellKnownAddress::Weth, chain_id)?;

// Token metadata resolution
if let Some(weth_symbol) = registry.get_token_symbol(chain_id, weth_addr) {
    // Token is known in registry
}
```

### Registering Well-Known Addresses

Protocols should register their well-known addresses during initialization:

```rust
use crate::registry::WellKnownAddress;

// Universal address (same on all chains)
registry.register_universal_address(WellKnownAddress::Permit2, permit2_address);

// Chain-specific addresses
registry.register_chain_specific_address(WellKnownAddress::Weth, 1, weth_mainnet_address);
registry.register_chain_specific_address(WellKnownAddress::Weth, 137, weth_polygon_address);
```

### ASCII-Only Restrictions

Always use ASCII for terminal compatibility:

- Use `>=` instead of `≥`
- Use `<=` instead of `≤`
- Use `->` instead of `→`

### Adding New Protocols

To add any other protocol:

1. Create `src/protocols/{protocol}/` directory
2. Create a config.rs with the addresses required
3. Define Aave function structs with `sol!`
4. Create decoder functions (20-40 lines each)
5. Add to main visualizer registry

See `DECODER_GUIDE.md` for complete examples.
