# VisualSign Parser

Rust-based transaction parser that converts raw blockchain transactions into human-readable VisualSign payloads.

## What is VisualSign?

VisualSign transforms opaque transaction data (hex strings, base64 blobs) into structured, human-readable JSON that clearly shows what a transaction will do. See the [full documentation](https://visualsign.dev).

## Supported Chains

- Ethereum (+ L2s: Arbitrum, Optimism, Base, Polygon)
- Solana
- Sui
- Tron

See the [Adding a New Chain](https://visualsign.dev/adding-new-chain) guide to add support for another blockchain. [Join the community on Telegram](https://t.me/+B03D2m1WlBBiYTdh) if you're interested in contributing.

## Quick Start

```sh
# Parse a transaction from hex
cargo run --bin parser_cli -- --chain ethereum --network ETHEREUM_MAINNET --output human -t <transaction_hex>

# Try a real Uniswap swap from the test fixtures
cargo run --bin parser_cli -- --chain ethereum --network ETHEREUM_MAINNET --output human \
  -t "$(cat chain_parsers/visualsign-ethereum/tests/fixtures/1559.input)"
```

See the [Quickstart](https://visualsign.dev/quickstart) for more examples and the [Parser CLI](https://visualsign.dev/parser-cli) reference for all options.

## Documentation

Full documentation at **https://visualsign.dev**:

- [Quickstart](https://visualsign.dev/quickstart) — Test your DApp's transactions
- [Wallet Integration](https://visualsign.dev/getting-started) — Integrate the parser into your wallet
- [API Reference](https://visualsign.dev/api-reference) — gRPC service definition
- [Field Types](https://visualsign.dev/field-types) — VisualSign JSON schema
- [Contributing a Visualization](https://visualsign.dev/contributing/contributing-visualization) — Add support for your protocol

## Contributing

See the [Contributing guide](https://visualsign.dev/contributing/contributing-visualization) and [About](https://visualsign.dev/about) page for governance details.
