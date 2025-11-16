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

**Benefits:**
- ✅ Compile-time uniqueness - can't have duplicate type names
- ✅ No manual string maintenance
- ✅ Type-safe at API boundaries
- ✅ Automatic type ID generation from type name

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

Benefits:
- ✅ Single source of truth for contract addresses
- ✅ Easy to add new chains
- ✅ Compile-time type safety with `ContractType` trait
- ✅ Simple, stateless design
- ✅ Easy to test

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

## Testing

Each module should include tests:

- **Config tests**: Verify addresses are registered correctly
- **Visualizer tests**: Test calldata decoding and field generation
- **Integration tests**: End-to-end transaction visualization

Example from [protocols/uniswap/mod.rs](src/protocols/uniswap/mod.rs#L48):
```rust
#[test]
fn test_register_uniswap_contracts() {
    let mut contract_reg = ContractRegistry::new();
    let mut visualizer_reg = EthereumVisualizerRegistryBuilder::new();

    register(&mut contract_reg, &mut visualizer_reg);

    let addr = "0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD".parse().unwrap();

    for chain_id in [1, 10, 137, 8453, 42161] {
        let contract_type = contract_reg.get_contract_type(chain_id, addr);
        assert_eq!(contract_type.unwrap(), UNISWAP_UNIVERSAL_ROUTER);
    }
}
```

## References

- [CLAUDE.md](CLAUDE.md) - Development guidelines and best practices
- [visualsign crate](../../visualsign/) - Field builders and core types
- [Registry TODO](src/lib.rs#L116) - Future registry architecture improvements
