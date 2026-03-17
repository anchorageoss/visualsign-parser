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

## Running the Gateway

The `parser_gateway` is an HTTP REST proxy (Turnkey-compatible) that forwards requests to the gRPC parser. It exposes the same `/visualsign/api/v1/parse` endpoint used in production, making it useful for local development and integration testing.

**Start the gRPC server and gateway:**

```sh
# Terminal 1: start the gRPC parser server
cd src && make grpc-server

# Terminal 2: start the HTTP gateway
cd src && make parser_gateway
```

**Environment variables:**

| Variable | Default | Description |
|----------|---------|-------------|
| `GATEWAY_PORT` | `8080` | HTTP port the gateway listens on |
| `GRPC_ADDR` | `http://127.0.0.1:44020` | Address of the gRPC parser server |

**Example request:**

```sh
curl -X POST http://localhost:8080/visualsign/api/v1/parse \
  -H "Content-Type: application/json" \
  -d '{"request": {"unsigned_payload": "0x02f8...", "chain": "CHAIN_ETHEREUM"}}'
```

**Docker usage:**

```sh
# Build the gateway image (run from the repo root, not src/)
make non-oci-docker-images

# Run the gateway container (assumes gRPC server is accessible at host.docker.internal:44020)
docker run -p 8080:8080 -e GRPC_ADDR=http://host.docker.internal:44020 anchorageoss-visualsign-parser/parser_gateway
```

## Documentation

Full documentation at **https://visualsign.dev**:

- [Quickstart](https://visualsign.dev/quickstart) — Test your DApp's transactions
- [Wallet Integration](https://visualsign.dev/wallet-integration/overview) — Integrate the parser into your wallet
- [API Reference](https://visualsign.dev/api-reference) — gRPC service definition
- [Field Types](https://visualsign.dev/field-types) — VisualSign JSON schema
- [Contributing a Visualization](https://visualsign.dev/contributor-guides/contributing-visualization) — Add support for your protocol

## Contributing

See the [Contributing guide](https://visualsign.dev/contributing) and [About](https://visualsign.dev/about) page for governance details.
