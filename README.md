# VisualSign Parser

Rust-based transaction parser that converts raw blockchain transactions into human-readable VisualSign payloads.

[Documentation](https://visualsign.dev)

## What is VisualSign?

VisualSign transforms opaque transaction data (hex strings, base64 blobs) into structured, human-readable JSONs that clearly decode transaction details. See the [full documentation](https://anchorageoss.github.io/visualsign-display).
You can follow the [Wallet Integration Guide](https://github.com/anchorageoss/visualsign-turnkeyclient/blob/main/WALLET_INTEGRATION_GUIDE.md) that uses [AWS Nitro Verifier Library](https://github.com/anchorageoss/awsnitroverifier) and our minimal [VisualSign TurnkeyClient](https://github.com/anchorageoss/visualsign-turnkeyclient) to help you understand and bootstrap the security process.

## Supported Chains

- Ethereum (+ L2s: Arbitrum, Optimism, Base, Polygon)
- Solana
- Sui
- Tron

You can follow the [Chain Addition Guide](https://github.com/anchorageoss/visualsign-parser/wiki/Adding-a-new-chain-to-Visualsign-Parser) to learn how to add a new chain. Often the basic chain addition can be done within a working day if you have a high quality Rust SDK, but we are currently deploying a single binary and are not focusing on expanding chains broadly until we have further solidified design patterns and dApp Frameworks. If you are a blockchain that wants to be included in VisualSign, [join the community on Telegram](https://t.me/+B03D2m1WlBBiYTdh).

## Architecture

```mermaid
flowchart TD
    subgraph API ["API Layer"]
        grpc["gRPC Server"]
    end

    subgraph Engine ["Parsing Engine"]
        router["Chain Router"]
        core["VisualSign Core"]
    end

    subgraph Chains ["Chain Parsers"]
        eth["Ethereum"]
        sol["Solana"]
        sui["Sui"]
        trn["Tron"]
    end

    grpc --> router
    router --> eth & sol & sui & trn
    eth & sol & sui & trn --> core
    core --> grpc
```

## Quick Start

### CLI

```sh
cargo run --bin parser_cli -- --chain ethereum -t '0xf86c...'
```

Output:
```json
{
  "Version": "0",
  "Title": "Ethereum Transaction",
  "PayloadType": "EthereumTx",
  "Fields": [
    {"Label": "To", "FallbackText": "0x3535...", "Type": "address_v2", "AddressV2": {"Address": "0x3535..."}},
    {"Label": "Value", "FallbackText": "1 ETH", "Type": "amount_v2", "AmountV2": {"Amount": "1", "Abbreviation": "ETH"}}
  ]
}
```

### gRPC Server

```sh
make -C src parser  # Starts server on port 44020
```

```sh
grpcurl -plaintext -d '{"unsigned_payload": "0x...", "chain": "CHAIN_ETHEREUM"}' \
  localhost:44020 parser.ParserService/Parse
```

## Documentation

Full documentation at **https://anchorageoss.github.io/visualsign-display**:
- [Field Types Reference](https://anchorageoss.github.io/visualsign-display/docs/field-types)
- [Integration Guide](https://anchorageoss.github.io/visualsign-display/docs/integration)
- [Parser CLI](https://anchorageoss.github.io/visualsign-display/docs/parser-cli)

## Development

```sh
make -C src test    # Run tests
make -C src fmt     # Format code
make -C src lint    # Run clippy
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development workflow and [GOVERNANCE.md](GOVERNANCE.md) for project governance.

## Security

Report vulnerabilities per [SECURITY.md](SECURITY.md).

## License

Apache 2.0
