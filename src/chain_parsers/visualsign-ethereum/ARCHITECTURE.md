# VisualSign Ethereum Module Architecture

## Overview

The visualsign-ethereum module provides transaction visualization for Ethereum and EVM-compatible chains. It follows a layered architecture that separates generic contract standards from protocol-specific implementations.

## Directory Structure

```
src/
├── lib.rs                          - Main entry point, transaction parsing
├── chains.rs                       - Chain ID to name mappings
├── context.rs                      - VisualizerContext for transaction context
├── fmt.rs                          - Formatting utilities (ether, gwei, etc)
├── registry.rs                     - ContractRegistry for address-to-type mapping
├── token_metadata.rs               - Canonical wallet token format
├── visualizer.rs                   - VisualizerRegistry and builder pattern
│
├── contracts/                      - Generic contract standards
│   ├── mod.rs                      - Re-exports all contract modules
│   └── core/                       - Core contract standards
│       ├── mod.rs
│       ├── erc20.rs                - ERC20 token standard visualizer
│       └── fallback.rs             - Catch-all hex visualizer for unknown contracts
│
└── protocols/                      - Protocol-specific implementations
    ├── mod.rs                      - register_all() function
    └── uniswap/                    - Uniswap DEX protocol
        ├── mod.rs                  - Protocol registration
        ├── config.rs               - Contract addresses and chain deployments
        └── contracts/              - Uniswap-specific contract visualizers
            ├── mod.rs
            └── universal_router.rs - Universal Router (V2/V3/V4) visualizer
```

## Key Concepts

### Contracts vs Protocols

**Contracts** (`src/contracts/`):
- Generic, cross-protocol contract standards
- Implemented by many different projects
- Examples: ERC20, ERC721, ERC1155
- Organized by category:
  - **core/** - Fundamental token standards (ERC20, ERC721)
  - **staking/** - Generic staking patterns (future)
  - **governance/** - Generic governance patterns (future)

**Protocols** (`src/protocols/`):
- Specific DeFi/Web3 protocols with custom business logic
- Each protocol is a collection of related contracts
- Examples: Uniswap, Aave, Compound
- Each protocol contains:
  - **config.rs** - Contract addresses, chain deployments, metadata
  - **contracts/** - Protocol-specific contract visualizers
  - **mod.rs** - Registration function

### Example: Uniswap Protocol

```
protocols/uniswap/
├── config.rs                    # Addresses for all chains (Mainnet, Arbitrum, etc)
├── contracts/
│   ├── universal_router.rs      # Handles Universal Router calls
│   ├── v3_router.rs            # (future) V3-specific router
│   └── v2_router.rs            # (future) V2-specific router
└── mod.rs                       # register() function
```

The `config.rs` file defines:
- **Contract type markers** (type-safe unit structs implementing `ContractType`)
- Contract addresses per chain
- Helper methods to query deployments

## Type-Safe Contract Identifiers

The module uses the `ContractType` trait to ensure compile-time uniqueness of contract types:

```rust
/// Define a contract type marker (in protocols/uniswap/config.rs)
pub struct UniswapUniversalRouter;
impl ContractType for UniswapUniversalRouter {}

// If someone copies this and forgets to rename:
pub struct UniswapUniversalRouter; // ❌ Compile error: duplicate type!
```

This ensures compile-time uniqueness and automatic type ID generation from type names.

## Registration System

The module uses a dual-registry pattern:

### 1. ContractRegistry (Address → Type)
Maps `(chain_id, address)` to contract type string:
```rust
// Type-safe registration (preferred)
registry.register_contract_typed::<UniswapUniversalRouter>(1, vec![address]);

// String-based registration (backward compatibility)
registry.register_contract(1, "CustomContract", vec![address]);
```

### 2. EthereumVisualizerRegistry (Type → Visualizer)
Maps contract type to visualizer implementation:
```rust
// Example: "UniswapUniversalRouter" → UniswapUniversalRouterVisualizer
visualizer_reg.register(Box::new(UniswapUniversalRouterVisualizer::new()));
```

### Registration Flow

```rust
// protocols/uniswap/mod.rs
pub fn register(
    contract_reg: &mut ContractRegistry,
    visualizer_reg: &mut EthereumVisualizerRegistryBuilder,
) {
    use config::UniswapUniversalRouter;

    let address = UniswapConfig::universal_router_address();

    // 1. Register Universal Router on all supported chains (type-safe)
    for &chain_id in UniswapConfig::universal_router_chains() {
        contract_reg.register_contract_typed::<UniswapUniversalRouter>(
            chain_id,
            vec![address],
        );
    }

    // 2. Register visualizers (future)
    // visualizer_reg.register(Box::new(UniswapUniversalRouterVisualizer::new()));
}

// protocols/mod.rs
pub fn register_all(
    contract_reg: &mut ContractRegistry,
    visualizer_reg: &mut EthereumVisualizerRegistryBuilder,
) {
    uniswap::register(contract_reg, visualizer_reg);
    // Future: aave::register(contract_reg, visualizer_reg);
    // Future: compound::register(contract_reg, visualizer_reg);
}
```

## Visualization Pipeline

1. **Transaction Parsing** ([lib.rs:89](src/lib.rs#L89))
   - Parse RLP-encoded transaction
   - Extract chain_id, to, value, input data

2. **Contract Type Lookup** ([lib.rs:198](src/lib.rs#L198))
   - Query `ContractRegistry` with (chain_id, to_address)
   - Get contract type string (e.g., "Uniswap_UniversalRouter")

3. **Visualizer Dispatch** (future enhancement)
   - Query `EthereumVisualizerRegistry` with contract type
   - Invoke visualizer's `visualize()` method

4. **Fallback Visualization** ([lib.rs:389](src/lib.rs#L389))
   - If no specific visualizer handles the call
   - Use `FallbackVisualizer` to display raw hex

## Scope and Limitations

### Calldata Decoding vs Transaction Simulation

This module **decodes transaction calldata** to show user intent. It does **not simulate transaction execution** to show results or state changes.

#### What We Can Decode (Calldata Analysis):
✅ Function calls and parameters (e.g., `execute(commands, inputs, deadline)`)
✅ **Outgoing amounts** - Exact amounts user is sending (e.g., "240 SETH", "60 SETH")
✅ **Minimum expected outputs** - Slippage protection (e.g., ">=0.0035 WETH")
✅ Token symbols from registry (e.g., "SETH", "WETH" instead of addresses)
✅ Pool fee tiers (e.g., "0.3% fee" indicates which V3 pool tier)
✅ Recipients and addresses for transfers and payments
✅ Deadline timestamps
✅ Command sequences showing transaction flow (e.g., swap → pay fee → unwrap)

**Example output:**
```
Command 1: Swap 240 SETH for >=0.00357 WETH via V3 (0.3% fee)
Command 2: Swap 60 SETH for >=0.000895 WETH via V3 (1% fee)
Command 3: Pay 0.25% of WETH to 0x000000fee13a103a10d593b9ae06b3e05f2e7e1c
Command 4: Unwrap >=0.00446920 WETH to ETH
```

#### What Requires Simulation (Out of Scope):

❌ **Actual received amounts** - Exact output after execution (vs minimum expected)
  - We show: ">=0.00357 WETH" (from calldata)
  - Simulation shows: "0.003573913782539750 WETH received" (actual result)
  - Requires: EVM execution to compute exact amounts after slippage

❌ **Pool address resolution** - Which specific pool contract handles each swap
  - We show: "via V3 (0.3% fee)" (fee tier from calldata)
  - Simulation shows: "via pool 0xd6e420f6...34cd" (actual pool address)
  - Requires: RPC queries to find pools for token pairs + fee tier

❌ **Balance changes in external contracts** - State deltas in pools, routers, etc.
  - We show: User intent (swap X for Y, pay fee, unwrap)
  - Simulation shows: "Pool 0xd6e420f6: WETH -0.0036, SETH +240"
  - Requires: State tracking during execution for all touched contracts

❌ **Multi-hop routing** - Intermediate tokens in complex swap paths
  - Current: Single-hop decoding (token A → token B)
  - Future enhancement: Parse multi-hop paths from calldata (no simulation needed)

❌ **Gas estimation** - Actual gas consumed
  - Requires: EVM execution

**Why these are out of scope:**

1. **Architectural separation**: Visualizers decode calldata (signing time), not execution results (runtime)
2. **No RPC dependency**: This module is pure calldata → human-readable transformation
3. **Deterministic behavior**: Decoding doesn't depend on chain state or external data
4. **Performance**: No network calls or heavy computation required

**Tools that provide simulation:**
- [Tenderly](https://tenderly.co) - Full EVM simulation with state tracking
- [Foundry's cast](https://book.getfoundry.sh/cast/) - Local simulation
- Block explorers with internal transaction tracing

This module's goal is to make **what the user is signing** clear, not to predict execution outcomes.

## Adding New Protocols

To add a new protocol (e.g., Aave):

1. **Create protocol directory**:
   ```bash
   mkdir -p src/protocols/aave/contracts
   ```

2. **Create config.rs with type-safe contract markers**:
   ```rust
   // src/protocols/aave/config.rs
   use alloy_primitives::Address;
   use crate::registry::ContractType;

   /// Contract type marker for Aave Lending Pool
   #[derive(Debug, Clone, Copy)]
   pub struct AaveLendingPool;
   impl ContractType for AaveLendingPool {}

   /// Aave protocol configuration
   pub struct AaveConfig;

   impl AaveConfig {
       pub fn lending_pool_address() -> Address {
           "0x7d2768dE32b0b80b7a3454c06BdAc94A69DDc7A9".parse().unwrap()
       }

       pub fn lending_pool_chains() -> &'static [u64] {
           &[1, 137, 42161, 10, 8453] // Mainnet, Polygon, Arbitrum, etc.
       }
   }
   ```

3. **Create contract visualizers**:
   ```rust
   // src/protocols/aave/contracts/lending_pool.rs
   pub struct AaveLendingPoolVisualizer {}
   ```

4. **Create registration function**:
   ```rust
   // src/protocols/aave/mod.rs
   pub fn register(
       contract_reg: &mut ContractRegistry,
       visualizer_reg: &mut EthereumVisualizerRegistryBuilder,
   ) {
       use config::AaveLendingPool;

       let address = AaveConfig::lending_pool_address();

       // Register using type-safe method
       for &chain_id in AaveConfig::lending_pool_chains() {
           contract_reg.register_contract_typed::<AaveLendingPool>(
               chain_id,
               vec![address],
           );
       }

       // Register visualizers (future)
       // visualizer_reg.register(Box::new(AaveLendingPoolVisualizer::new()));
   }
   ```

5. **Register in protocols/mod.rs**:
   ```rust
   pub mod aave;

   pub fn register_all(...) {
       uniswap::register(contract_reg, visualizer_reg);
       aave::register(contract_reg, visualizer_reg);
   }
   ```

## Fallback Mechanism

The `FallbackVisualizer` ([contracts/core/fallback.rs](src/contracts/core/fallback.rs)) provides a catch-all for unknown contract calls:

- Returns raw calldata as hex: `0x1234567890abcdef`
- Label: "Contract Call Data"
- Similar to Solana's unknown program handler

This ensures all transactions can be visualized, even without specific protocol support.

## Configuration Pattern

Each protocol uses a simple configuration struct with static methods:

```rust
use alloy_primitives::Address;
use crate::registry::ContractType;

/// Contract type marker (compile-time unique)
#[derive(Debug, Clone, Copy)]
pub struct UniswapUniversalRouter;
impl ContractType for UniswapUniversalRouter {}

/// Protocol configuration
pub struct UniswapConfig;

impl UniswapConfig {
    /// Returns the Universal Router address (same across chains)
    pub fn universal_router_address() -> Address {
        "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD".parse().unwrap()
    }

    /// Returns supported chain IDs
    pub fn universal_router_chains() -> &'static [u64] {
        &[1, 10, 137, 8453, 42161]
    }
}
```

## Future Enhancements

### 1. Visualizer Trait Implementation
Currently, protocol visualizers (like `UniswapV4Visualizer`) use ad-hoc methods. They should implement the `ContractVisualizer` trait:

```rust
impl ContractVisualizer for UniswapUniversalRouterVisualizer {
    fn contract_type(&self) -> &str {
        UNISWAP_UNIVERSAL_ROUTER
    }

    fn visualize(&self, context: &VisualizerContext)
        -> Result<Option<Vec<AnnotatedPayloadField>>, VisualSignError>
    {
        // Decode and visualize Universal Router calls
    }
}
```

### 2. Registry Architecture Refactor
See [lib.rs:116-164](src/lib.rs#L116) for detailed TODO about moving registries from converter ownership to context-based passing.

### 3. Protocol Version Support
Each protocol should support multiple versions:
```
protocols/uniswap/contracts/
├── v2_router.rs
├── v3_router.rs
└── universal_router.rs
```

### 4. Cross-Protocol Standards
Some patterns span multiple protocols:
```
contracts/
├── core/          # ERC standards
├── staking/       # Generic staking (not protocol-specific)
└── governance/    # Generic governance contracts
```
