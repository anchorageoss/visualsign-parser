# Morpho Bundler - Implementation Status

## Overview

This document outlines the implementation status of the Morpho Bundler `multicall` command visualization. Based on the `BundlerV3` contract, we catalog:

- ✅ Implemented commands
- ⏳ Commands needing implementation
- 📋 Known special cases and encoding requirements

## Reference

- **Contract**: [BundlerV3.sol on GitHub](https://github.com/morpho-org/morpho-blue-bundlers/blob/main/src/BundlerV3.sol)
- **Configuration**: `src/protocols/morpho/config.rs`
- **Implementation**: `src/protocols/morpho/contracts/bundler.rs`
- **Tests**: All tests passing (5/5 ✓)

---

## Implemented Commands (✅)

The `multicall` function takes an array of `Call` structs. The action to be performed is determined by the `selector` field within each `Call` struct.

### 0xd505accf - `permit(address owner, address spender, uint256 value, uint256 deadline, bytes signature)`

**Status**: ✅ Fully Implemented
**Visualization**: Shows token, amount, spender, and expiration.
**Special Case**: The `signature` is a dynamic `bytes` array. The decoder handles this by reading the offset and length.

### 0xd96ca0b9 - `erc20TransferFrom(address token, address from, uint256 amount)`

**Status**: ✅ Fully Implemented
**Visualization**: Shows token, amount, and the source address.
**Notes**: This is a wrapper for a standard `transferFrom` call, executed by the bundler contract.

### 0x6ef5eeae - `erc4626Deposit(address vault, uint256 assets, uint256 minShares, address receiver)`

**Status**: ✅ Fully Implemented
**Visualization**: Shows vault address, assets deposited, minimum shares expected, and the receiver.
**Notes**: Resolves vault symbol if available in the `ContractRegistry`.

---

## Commands Requiring Implementation (⏳)

The following operations are part of the Morpho protocol but are not yet implemented in the visualizer.

### Bundler Actions

- ⏳ `erc4626Redeem`
- ⏳ `erc4626Withdraw`
- ⏳ `wethWithdraw`
- ⏳ `wethWithdrawTo`
- ⏳ `transfer`
- ⏳ `pull`

### Morpho Blue Actions

- ⏳ `blueSupply`
- ⏳ `blueWithdraw`
- ⏳ `blueBorrow`
- ⏳ `blueRepay`
- ⏳ `blueAddCollateral`

---

## Implementation Priority Matrix

### Tier 1 (High Priority - Core Functionality)

- [ ] `blueSupply` - Essential for interacting with Morpho Blue markets.
- [ ] `blueBorrow` - Core user action on Morpho Blue.
- [ ] `blueRepay` - Completes the borrowing lifecycle.
- [ ] `blueWithdraw` - Allows users to retrieve their supplied assets.

### Tier 2 (Medium Priority - Vault and Collateral)

- [ ] `erc4626Redeem` / `erc4626Withdraw` - Common vault interactions.
- [ ] `blueAddCollateral` - Important for managing loan health.

### Tier 3 (Lower Priority - Utility Functions)

- [ ] `wethWithdraw` / `wethWithdrawTo` - WETH handling.
- [ ] `transfer` / `pull` - Basic token movements.

---

## Key Technical Findings

### `sol!` Macro for Decoding

The implementation relies heavily on the `alloy-sol-types` `sol!` macro for generating type-safe Rust structs from Solidity definitions. This simplifies decoding and reduces boilerplate.

### Nested Dynamic Calls

The core of the bundler is the `multicall(Call[] calldata calls)` function. The visualizer must first decode this outer call, then iterate through the `calls` array. For each `Call` in the array, it must:

1. Read the `selector`.
2. Match the `selector` to a known function.
3. Decode the `data` field using the corresponding function's ABI.

### Contract Registry for Context

The `ContractRegistry` is crucial for providing context, such as token symbols and decimals. All visualizers should query the registry to enrich the output.

---

## How to Add a New Command

To add support for a new command (e.g., `blueSupply`):

1.  **`contracts/bundler.rs`**:

    - Add the function signature and any required structs to the `sol!` macro block.
      ```rust
      // ... existing sol! macro
      function blueSupply(address market, uint256 assets, address onBehalf, bytes data) external;
      // ...
      ```
    - Add a new `const` for the selector.
      ```rust
      // ...
      const BLUE_SUPPLY_SELECTOR: [u8; 4] = selector!("blueSupply(address,uint256,address,bytes)");
      // ...
      ```
    - Add a new match arm in `decode_nested_call`.
      ```rust
      // ...
      match selector {
          // ...
          BLUE_SUPPLY_SELECTOR => self.decode_blue_supply(registry, &call.data),
          _ => Ok(unhandled_field(call)),
      }
      // ...
      ```
    - Implement the `decode_blue_supply` function. This function should decode the parameters and return a `SignablePayloadField`.
      ```rust
      fn decode_blue_supply(...) -> Result<SignablePayloadField, ...> {
          // 1. Decode data using sol! struct: blueSupplyCall::decode_single(&data, true)?
          // 2. Look up token symbols in registry.
          // 3. Format amounts.
          // 4. Return a TextV2 or PreviewLayout field.
      }
      ```

2.  **`tests` in `contracts/bundler.rs`**:

    - Add a unit test for the new `decode_blue_supply` function with sample data.
    - If possible, add the new command to the `test_visualize_multicall_real_transaction` test or create a new integration test.

3.  **`IMPLEMENTATION_STATUS.md` (This file)**:
    - Move the command from the "⏳" section to the "✅" section.
    - Update the status and add implementation notes.

---

_Document Version 1.0_
_Last Updated: 2025-11-16_
_Status: Initial implementation with three core bundler commands._
