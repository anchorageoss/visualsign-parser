# Rust Design

This document details the code-level architecture of the `visualsign-parser` workspace.

## Workspace Layout

```text
visualsign-parser/
├── proto/                  # gRPC definitions (shared)
├── visualsign/             # Core library (Traits, Data Models)
├── chain_parsers/          # Monorepo of chain-specific logic
│   ├── visualsign-ethereum/
│   ├── visualsign-solana/
│   └── ...
├── parser/
│   ├── app/                # Enclave Entrypoint (gRPC Server)
│   └── host/               # Host Entrypoint (Vsock Proxy)
└── integration/            # E2E tests and simulators
```

## Core Traits (`visualsign` crate)

The system relies on a unified trait to abstract chain differences.

```rust
/// Implemented by every chain parser
#[async_trait]
pub trait ChainParser: Send + Sync {
    /// Parse a raw payload into the semantic model
    async fn parse(
        &self, 
        payload: &[u8], 
        metadata: Option<&ChainMetadata>
    ) -> Result<ParsedTransaction, ParseError>;
}
```

## Async & Concurrency

-   **Runtime**: `tokio` (multi-threaded).
-   **gRPC**: `tonic`.
-   **Model**: Request-Response. No background tasks or persistent connections to chains.
-   **Statelessness**: The parsers are `Send + Sync` and generally immutable. Shared global state (like the Registry) is wrapped in `Arc`.

## Error Handling

We use a layered error strategy:

1.  **Library Errors**: `thiserror` in `visualsign-*` crates.
    -   Specific variants: `InvalidRlp`, `InstructionDecodingFailed`, `MetadataMismatch`.
2.  **Application Errors**: `anyhow` in `parser/app` for glue code.
3.  **gRPC Errors**: `tonic::Status`.
    -   We map internal errors to gRPC codes:
        -   `InvalidInput` -> `Code::InvalidArgument`
        -   `NotImplemented` -> `Code::Unimplemented`
        -   Internal panics/crashes -> `Code::Internal`

## Safety & Unsafe

-   **Policy**: `#![forbid(unsafe_code)]` in core business logic.
-   **Exceptions**:
    -   FFI bindings (if any C libraries are used for crypto).
    -   Performance critical serialization (rarely needed here; safety > speed).
-   **Cryptography**: Use pure Rust crates (`k256`, `ed25519-dalek`) where possible to avoid FFI risks.

## Testing Strategy

1.  **Unit Tests**: Inside each `chain_parser`, testing decoder logic with hardcoded hex vectors.
    -   *Fixtures*: `tests/fixtures/` containing `*.input` (hex) and `*.expected` (json).
2.  **Integration Tests**: `integration/` crate.
    -   Spins up a mock Enclave (or real one) and asserts gRPC responses.
3.  **Fuzzing**: Use `cargo-fuzz` on the raw byte inputs of parsers to find panics/crashes.

