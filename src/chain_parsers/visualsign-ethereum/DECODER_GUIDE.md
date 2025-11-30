# Solidity Protocol Decoder Implementation Guide

This guide shows how to add clean, maintainable decoders for any Solidity-based protocol (Uniswap, Aave, Curve, etc.) using the patterns established in this codebase.

## The Pattern: Simple, Repeatable, and Type-Safe

Every decoder follows this simple pattern:

```rust
/// Decodes OPERATION command parameters
fn decode_operation(
    bytes: &[u8],
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    // 1. Decode the struct using sol! macro
    let params = match OperationParams::abi_decode(bytes) {
        Ok(p) => p,
        Err(_) => {
            // Return error field if decoding fails
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

    // 2. Extract data from params and resolve tokens via registry
    let token_symbol = registry
        .and_then(|r| r.get_token_symbol(chain_id, params.token))
        .unwrap_or_else(|| format!("{:?}", params.token));

    let amount_u128 = params.amount.to_string().parse().unwrap_or(0);
    let (amount_str, _) = registry
        .and_then(|r| r.format_token_amount(chain_id, params.token, amount_u128))
        .unwrap_or_else(|| (params.amount.to_string(), token_symbol.clone()));

    // 3. Create human-readable text summary
    let text = format!("Perform operation with {} {}", amount_str, token_symbol);

    // 4. Return as TextV2 field
    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: text.clone(),
            label: "Operation".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text },
    }
}
```

## Step-by-Step Implementation

### Step 1: Define Struct Parameters with sol! Macro

In your main decoder file, define all the parameter structs using the `sol!` macro:

```rust
sol! {
    struct SwapParams {
        address tokenIn;
        address tokenOut;
        uint256 amountIn;
        uint256 minAmountOut;
    }

    struct ApproveLendParams {
        address token;
        address lendingPool;
        uint256 amount;
    }
}
```

**Why?** The `sol!` macro from alloy automatically generates:
- Type-safe `abi_decode()` function
- Proper ABI encoding/decoding
- Clean field access without manual byte parsing

### Step 2: Add Decoder Function

Create a `decode_*` function for each operation type. Keep it focused:

```rust
fn decode_swap(
    bytes: &[u8],
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    // Decode or return error
    let params = match SwapParams::abi_decode(bytes) {
        Ok(p) => p,
        Err(_) => return error_field("Swap"),
    };

    // Get token symbols from registry
    let token_in = registry
        .and_then(|r| r.get_token_symbol(chain_id, params.tokenIn))
        .unwrap_or_else(|| format!("{:?}", params.tokenIn));

    let token_out = registry
        .and_then(|r| r.get_token_symbol(chain_id, params.tokenOut))
        .unwrap_or_else(|| format!("{:?}", params.tokenOut));

    // Format amounts using registry decimals
    let (amount_in_str, _) = registry
        .and_then(|r| {
            let amount: u128 = params.amountIn.to_string().parse().ok()?;
            r.format_token_amount(chain_id, params.tokenIn, amount)
        })
        .unwrap_or_else(|| (params.amountIn.to_string(), token_in.clone()));

    let text = format!("Swap {} {} for {} {}",
        amount_in_str, token_in, params.minAmountOut, token_out
    );

    text_field("Swap", text)
}
```

### Step 3: Add to Match Statement

In your main decoder function, add each operation to the match statement:

```rust
match operation_type {
    OperationType::Swap => Self::decode_swap(bytes, chain_id, registry),
    OperationType::ApproveLend => Self::decode_approve_lend(bytes, chain_id, registry),
    _ => unimplemented_field(operation_type),
}
```

### Step 4: Leverage Registry for Token Resolution

The `ContractRegistry` is your key to clean code. Use these methods:

```rust
// Get token symbol
let symbol = registry.get_token_symbol(chain_id, address);

// Format amount with decimals
let (formatted, symbol) = registry.format_token_amount(
    chain_id,
    token_address,
    raw_amount  // u128
);
```

## Real-World Examples from Uniswap Router

### Simple Decoder: Wrap ETH

```rust
fn decode_wrap_eth(
    bytes: &[u8],
    _chain_id: u64,
    _registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    let params = match WrapEthParams::abi_decode(bytes) {
        Ok(p) => p,
        Err(_) => {
            return SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Wrap ETH: 0x{}", hex::encode(bytes)),
                    label: "Wrap ETH".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Failed to decode parameters".to_string(),
                },
            };
        }
    };

    let amount_str = params.amountMin.to_string();
    let text = format!("Wrap {} ETH to WETH", amount_str);

    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: text.clone(),
            label: "Wrap ETH".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text },
    }
}
```

### Complex Decoder: V3 Swap Exact In

```rust
fn decode_v3_swap_exact_in(
    bytes: &[u8],
    chain_id: u64,
    registry: Option<&ContractRegistry>,
) -> SignablePayloadField {
    // Decode parameters
    let params = match V3SwapExactInputParams::abi_decode(bytes) {
        Ok(p) => p,
        Err(_) => return error_field("V3 Swap Exact In"),
    };

    // Parse V3 path (address[20] + fee[3bytes] + address[20] + ...)
    if params.path.0.len() < 43 {
        return invalid_path_field();
    }

    let path_bytes = &params.path.0;
    let token_in = Address::from_slice(&path_bytes[0..20]);
    let fee = u32::from_be_bytes([0, path_bytes[20], path_bytes[21], path_bytes[22]]);
    let token_out = Address::from_slice(&path_bytes[23..43]);

    // Resolve tokens
    let token_in_symbol = registry
        .and_then(|r| r.get_token_symbol(chain_id, token_in))
        .unwrap_or_else(|| format!("{:?}", token_in));

    let token_out_symbol = registry
        .and_then(|r| r.get_token_symbol(chain_id, token_out))
        .unwrap_or_else(|| format!("{:?}", token_out));

    // Format amounts
    let (amount_in_str, _) = registry
        .and_then(|r| {
            let amount: u128 = params.amountIn.to_string().parse().ok()?;
            r.format_token_amount(chain_id, token_in, amount)
        })
        .unwrap_or_else(|| (params.amountIn.to_string(), token_in_symbol.clone()));

    let (amount_out_str, _) = registry
        .and_then(|r| {
            let amount: u128 = params.amountOutMinimum.to_string().parse().ok()?;
            r.format_token_amount(chain_id, token_out, amount)
        })
        .unwrap_or_else(|| (params.amountOutMinimum.to_string(), token_out_symbol.clone()));

    let fee_pct = fee as f64 / 10000.0;
    let text = format!(
        "Swap {} {} for >={} {} via V3 ({}% fee)",
        amount_in_str, token_in_symbol, amount_out_str, token_out_symbol, fee_pct
    );

    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: text.clone(),
            label: "V3 Swap Exact In".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text },
    }
}
```

## Key Principles

### 1. Type Safety First
Use the `sol!` macro to generate type-safe decoders. Avoid manual byte parsing.

### 2. Registry as Single Source of Truth
All token symbols and decimals come from `ContractRegistry`. This ensures consistency and allows wallets to customize metadata.

### 3. Graceful Error Handling
Always handle decode failures by returning a TextV2 field with the hex input. This gives users visibility into what failed.

### 4. Clean, Human-Readable Output
Format amounts with proper decimals and symbols. Make the transaction intent clear.

### 5. No ASCII Characters in Strings
Use `>=` and `<=` instead of non-ASCII characters like `≥` and `≤` for terminal compatibility.

## Reusable Utilities

### WellKnownAddresses

For contracts like WETH that don't need registry lookups:

```rust
use crate::utils::address_utils::WellKnownAddresses;

let weth_address = WellKnownAddresses::weth(chain_id)?;
let permit2_address = WellKnownAddresses::permit2();
```

### Error Fields

Create consistent error fields:

```rust
SignablePayloadField::TextV2 {
    common: SignablePayloadFieldCommon {
        fallback_text: format!("{}: 0x{}", operation_name, hex::encode(bytes)),
        label: operation_name.to_string(),
    },
    text_v2: SignablePayloadFieldTextV2 {
        text: "Failed to decode parameters".to_string(),
    },
}
```

## Adding Support for Aave

When you're ready to add Aave support, follow this pattern:

```rust
// 1. Define Aave structs using sol!
sol! {
    struct DepositParams {
        address asset;
        uint256 amount;
        address onBehalfOf;
    }

    struct BorrowParams {
        address asset;
        uint256 amount;
        uint256 interestRateMode;
        address onBehalfOf;
    }
}

// 2. Create decoder functions (same pattern as Uniswap)
fn decode_deposit(bytes: &[u8], chain_id: u64, registry: Option<&ContractRegistry>) -> SignablePayloadField {
    // ... follows the same pattern ...
}

// 3. Add to main match statement
match aave_operation {
    AaveOp::Deposit => decode_deposit(bytes, chain_id, registry),
    AaveOp::Borrow => decode_borrow(bytes, chain_id, registry),
    // ...
}
```

## Summary

The pattern is simple and scales:
1. Define structs with `sol!`
2. Create decoder function (20-40 lines)
3. Add to match statement
4. Test with real transaction data

This approach has been used successfully for Uniswap's 19 command types. It will work for any Solidity protocol.
