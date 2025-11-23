# Uniswap V4 Development Workflow

This guide sets up a workflow for implementing and testing the Uniswap V4 parser using `anvil` and `cast`.

## 1. Prerequisites

- [Foundry](https://book.getfoundry.sh/getting-started/installation) (includes `anvil` and `cast`)
- Rust toolchain

## 2. Start Anvil (Fork Sepolia)

Uniswap V4 is currently deployed on testnets. Forking Sepolia allows you to simulate interactions with the real V4 PoolManager contract.

**Command:**

```bash
# Replace <RPC_URL> with your Sepolia RPC endpoint (Infura, Alchemy, etc.)
anvil --fork-url <RPC_URL>
```

*Note: Public RPCs might be rate-limited. Using an API key is recommended.*

**V4 PoolManager Address (Sepolia):**
`0x000000000004444c5dc75cB358380D2e3dE08A90`

## 3. Generating Test Calldata

You can use `cast` to generate calldata for specific V4 functions without needing to send a transaction.

### Initialize Pool

```bash
cast calldata "initialize((address,address,uint24,int24,address),uint160,bytes)" \
  "(0x0000000000000000000000000000000000000001,0x0000000000000000000000000000000000000002,3000,60,0x0000000000000000000000000000000000000000)" \
  79228162514264337593543950336 0x
```

### Swap

```bash
cast calldata "swap((address,address,uint24,int24,address),(bool,int256,uint160),bytes)" \
  "(0x0000000000000000000000000000000000000001,0x0000000000000000000000000000000000000002,3000,60,0x0000000000000000000000000000000000000000)" \
  "(true,1000000000000000000,0)" 0x
```

## 4. Using the Parser CLI

You can run the parser against any raw transaction hex using the CLI.

**Build the CLI:**
```bash
cargo build -p parser_cli
```

**Run against generated calldata:**
(Note: The parser expects a full transaction hex, but for simple calldata testing you might need to wrap it or use the unit tests. However, if you have a full raw transaction from `cast send --unsigned` or Etherscan, you can pipe it here).

Example:
```bash
cargo run -p parser_cli -- \
  --chain ethereum \
  --transaction 0x...<RAW_TX_HEX>... \
  --output human
```

## 5. Running Parser Tests

We have added test cases in `v4_pool.rs` that use the generated calldata.

**Run tests:**

```bash
./src/chain_parsers/visualsign-ethereum/scripts/test_v4.sh
```

As you implement the decoding logic in `v4_pool.rs`, update the tests to assert `result.is_some()`.

## 6. Development Loop

1.  **Modify `v4_pool.rs`**: Implement decoding for a function (e.g., `initialize`).
2.  **Run Tests**: `cargo test ...`
3.  **Verify**: Ensure the visualized output matches expected parameters.
4.  **Repeat**: Move to the next function (`swap`, `modifyLiquidity`, etc.).

## 7. Sending Transactions (Optional)

If you want to send real transactions to your local fork:

1.  Get some test ETH on Sepolia (if not using default anvil accounts).
2.  Send tx:
    ```bash
    cast send --rpc-url http://localhost:8545 \
      --private-key <YOUR_KEY> \
      0x000000000004444c5dc75ce358914d1d13574277 \
      "initialize((...)...)" ...
    ```
3.  Capture the transaction hash/data and feed it to the parser manually if needed.
