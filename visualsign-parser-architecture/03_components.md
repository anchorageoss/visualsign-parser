# Component Diagram (C4 Level 3)

Focusing on the **Parser App** (the trusted component).

```mermaid
flowchart TD
    %% Styling
    classDef apiLayer fill:#e1f5fe,stroke:#01579b,stroke-width:2px;
    classDef engineLayer fill:#fff3e0,stroke:#e65100,stroke-width:2px;
    classDef libLayer fill:#f3e5f5,stroke:#4a148c,stroke-width:2px,stroke-dasharray: 5 5;
    classDef component fill:#ffffff,stroke:#333,stroke-width:1px;

    %% Subgraphs representing the layers
    subgraph API ["API Layer"]
        direction TB
        grpc["gRPC Server"]:::component
        health["Health Check"]:::component
    end

    subgraph Engine ["Parsing Engine"]
        direction TB
        router["Chain Router"]:::component
        core["VisualSign Core"]:::component
    end

    subgraph Chains ["Chain Parsers (Libs)"]
        direction TB
        sol["Solana Parser"]:::component
        eth["Ethereum Parser"]:::component
        sui["Sui Parser"]:::component
        trn["Tron Parser"]:::component
    end

    %% Connections
    grpc -- "ParseRequest" --> router
    router -- "Dispatch(Solana)" --> sol
    router -- "Dispatch(Ethereum)" --> eth
    router -- "Dispatch(Sui)" --> sui
    router -- "Dispatch(Tron)" --> trn

    sol -- "Build Intent" --> core
    eth --> core
    sui --> core
    trn --> core

    core -- "ParseResponse" --> grpc

    %% Apply Layer Styles
    class API apiLayer
    class Engine engineLayer
    class Chains libLayer
```

## Component Responsibilities

### API Layer
-   **gRPC Server**: `tonic`-based server listening on the internal socket (vsock). Deserializes `ParseRequest`.
-   **Health Check**: Implements `grpc.health.v1` to report enclave status.

### Parsing Engine
-   **Chain Router**: Determines which sub-parser to use based on the `Chain` enum (e.g., `CHAIN_SOLANA`).
-   **VisualSign Core** (`visualsign` crate):
    -   Defines standard `ParsedTransaction` and `Signature` types.
    -   Provides builder patterns and traits (`Parse` trait) for consistency.

### Chain Parsers (`chain_parsers/`)
-   **Solana Parser**: Handles `Instruction` decoding, looks up "Presets" (hardcoded parsers for System Program, SPL Token, Jupiter), or falls back to generic IDL parsing if provided.
-   **Ethereum Parser**: RLP decoding, ABI decoding (if metadata provided), ERC-20/721 standard recognition.
-   **Sui Parser**: BCS decoding, Move call inspection.
-   **Tron Parser**: Protobuf decoding (Tron uses protobuf for txs).

## Key Invariants
-   **Isolation**: Chain parsers do not share mutable state.
-   **Stateless**: Each `Parse()` call is independent.

