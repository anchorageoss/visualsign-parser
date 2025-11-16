# Extending VisualSign: A Guide to Implementing Custom Protocol Decoders

## 1. Introduction

Welcome! This workshop will guide you through the process of adding a new protocol decoder to the VisualSign parser. VisualSign's power comes from its ability to translate complex, hexadecimal transaction data into a human-readable format. By the end of this session, you will have the knowledge and tools to extend VisualSign to support any EVM-based protocol.

Our case study will be the **Morpho Bundler**, a contract that batches multiple operations into a single transaction.

## 2. Before You Code: The Art of the "One-Shot" Spec

The most common pitfall in software development is a poorly defined task. A vague request leads to a cycle of exploration, rework, and refinement. To avoid this, we advocate for creating a "one-shot" specification before writing a single line of code.

The goal of a "one-shot" spec is to provide a developer with all the necessary information to complete a feature correctly in a single pass. It acts as a mission brief, a blueprint, and a definition of done, all in one.

### Anatomy of a "One-Shot" Spec

A perfect spec for a new protocol decoder should contain these seven elements:

1.  **High-Level Goal**: A single, clear sentence defining the objective.
2.  **Project Context & Precedent**: Point to existing code that should be used as a template. This ensures architectural consistency.
3.  **Target Contract & Transaction**: Provide the specific contract address, the relevant Solidity ABI, and a real-world raw transaction hex to serve as the primary test case.
4.  **Core Logic Requirements**: Break down the decoding steps. How is the transaction identified? How are nested data structures handled?
5.  **Detailed Output Specification**: A visual mock-up of the desired final output. This is the most critical part, as it removes all ambiguity about what "done" looks like.
6.  **File Structure & Naming**: Specify the exact directory and file names to be created.
7.  **Documentation Requirements**: State the need for documentation, like an `IMPLEMENTATION_STATUS.md` file, and provide a template.

## 3. Case Study: The Morpho Bundler Mission Brief

Here is the "one-shot" spec we could have used to implement the Morpho decoder. Each protocol may have unique quirks but if you're using something without too much assembly and quirks, this might work most of the way to get you started.

---

**Subject: Implement VisualSign Decoder for Morpho BundlerV3**

**1. High-Level Goal**
Create a visualizer that decodes a `multicall` to the Morpho Bundler, displaying each of the nested operations in a human-readable format.

**2. Project Context & Precedent**
Please follow the existing architectural patterns outlined in `src/chain_parsers/visualsign-ethereum/DECODER_GUIDE.md`. The implementation should mirror the structure of the Uniswap protocol found in `src/chain_parsers/visualsign-ethereum/src/protocols/uniswap/`.

**3. Target Contract & Transaction**
*   **Contract Address**: `0x6566194141eefa99Af43Bb5Aa71460Ca2Dc90245`
*   **Example Raw Transaction**: `0x02f9...da44c0` Full unsigned hex
*   **Relevant ABIs**:
    ```solidity
    // For use with the sol! macro
    struct Call { address target; bytes data; /* ... */ }
    function multicall(Call[] calldata calls) external;
    // Nested call ABIs
    function permit(address owner, address spender, uint256 value, uint256 deadline, bytes signature) external;
    function erc20TransferFrom(address token, address from, uint256 amount) external;
    function erc4626Deposit(address vault, uint256 assets, uint256 minShares, address receiver) external;
    ```

**4. Core Logic Requirements**
*   The visualizer should trigger for transactions sent to the BundlerV3 address with the `multicall` selector (`0x374f435d`).
*   The main `visualize_multicall` function should decode the `Call[]` array.
*   It should then loop through each `Call` and use a `match` statement on the first 4 bytes of `call.data` (the selector) to delegate to a specific decoding function.
*   Use the `ContractRegistry` to resolve token addresses to symbols and format amounts using the correct decimals.

**5. Detailed Output Specification**
The final output should be structured as follows, using `ListLayout` and `PreviewLayout`:
```
Morpho Bundler
  Title: Morpho Bundler Multicall
  Detail: 3 operation(s)
  📖 Expanded View:
  ├─ Permit: Permit 1.000000 USDC to 0x4a6c... (expires: ...)
  ├─ Transfer From: Transfer 1.000000 USDC from 0x4a6c...
  └─ Vault Deposit: Deposit 1000000 assets into 0xbeef... vault
```
*Note: The expanded view for each item should show all parameters, and if a token symbol is not found, display only the address.*

**6. File Structure & Naming**
Create the following structure:
`src/chain_parsers/visualsign-ethereum/src/protocols/morpho/`
├── `mod.rs`
├── `config.rs`
└── `contracts/`
    ├── `mod.rs`
    └── `bundler.rs`

**7. Documentation**
After implementation, create an `IMPLEMENTATION_STATUS.md` file inside `protocols/morpho/`, following the format of `protocols/uniswap/IMPLEMENTATION_STATUS.md`.

---

## 4. Step-by-Step Implementation Guide

Now, let's walk through how to turn that spec into working code.

### Step 1: Set Up the File Structure

As specified, create the directory and empty files. This provides the skeleton for our module.

```sh
mkdir -p src/chain_parsers/visualsign-ethereum/src/protocols/morpho/contracts
touch src/chain_parsers/visualsign-ethereum/src/protocols/morpho/{mod.rs,config.rs}
touch src/chain_parsers/visualsign-ethereum/src/protocols/morpho/contracts/{mod.rs,bundler.rs}
```

### Step 2: Configure the Protocol (`config.rs`)

Define a new `struct` for the contract and implement the `ContractType` trait. Then, create a config struct to hold the address and registration logic.

```rust
// In src/chain_parsers/visualsign-ethereum/src/protocols/morpho/config.rs

// 1. Define the contract type
pub struct Bundler3Contract;
impl ContractType for Bundler3Contract { /* ... */ }

// 2. Define the config
pub struct MorphoConfig { /* ... */ }
impl MorphoConfig {
    // 3. Add address and registration logic
    pub fn bundler3_address() -> Address { ... }
    pub fn register_contracts(registry: &mut ContractRegistry) { ... }
}
```

### Step 3: Implement the Core Logic (`contracts/bundler.rs`)

This is where the main decoding happens.

1.  **Define ABIs with `sol!`**: Use the `sol!` macro from `alloy-sol-types` to create Rust types from the Solidity interfaces in the spec.
2.  **Create the Visualizer Struct**: `pub struct BundlerVisualizer;`
3.  **Implement the Main Entry Point**: Create `visualize_multicall`, which decodes the outer `multicall` and gets the array of `Call` structs.
4.  **Implement the Dispatcher**: Create a `decode_nested_call` helper. This function takes a `Call` struct, `match`es on its `selector`, and calls the appropriate decoder for the nested operation.
5.  **Implement Individual Decoders**: For each nested operation (`permit`, `transfer`, `deposit`), create a function that decodes its specific parameters, queries the `ContractRegistry` for token info, and formats the data into a `SignablePayloadField` (like `TextV2` or `PreviewLayout`).

### Step 4: Integrate the Protocol

Now, plug the new module into the application.

1.  **`src/chain_parsers/visualsign-ethereum/src/protocols/mod.rs`**:
    *   Declare the new module: `pub mod morpho;`
    *   Call its registration function: `morpho::register(registry);`
2.  **`src/chain_parsers/visualsign-ethereum/src/lib.rs`**:
    *   In the main `visualize_ethereum_transaction` function, add logic to detect if the transaction is for the Morpho Bundler. If it is, instantiate `BundlerVisualizer` and call `visualize_multicall`.

### Step 5: Test Your Implementation

Use the raw transaction from the spec to create tests.

*   **Unit Tests**: Write tests for each individual decoder function (e.g., `test_decode_permit`).
*   **Integration Test**: Write a test for the top-level `visualize_multicall` function using the full, real-world transaction data. This ensures all parts work together correctly.
*   Run tests with `cargo test -p visualsign-ethereum`.

### Step 6: Document Your Work

Finally, create the `IMPLEMENTATION_STATUS.md` file as requested. This document is vital for future maintainers. It should clearly state:
*   What is implemented (✅).
*   What is not yet implemented (⏳).
*   A guide on how to add new commands, so the next developer can follow your pattern.

## 5. Conclusion

By following this structured approach—starting with a detailed spec and moving through implementation, integration, testing, and documentation—you can efficiently and accurately extend VisualSign with new protocol decoders. This process not only ensures correctness but also promotes a clean, maintainable, and consistent codebase.

Happy coding!
