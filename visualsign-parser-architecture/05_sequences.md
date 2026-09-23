# Sequence Diagrams

## 1. Standard Parse Flow

The most common operation: a client asks the system to explain a transaction.

```mermaid
sequenceDiagram
    participant Client
    participant Host
    participant Enclave as App (Enclave)
    participant Registry as Chain Registry
    participant Parser as Specific Parser

    Client->>Host: gRPC Parse(payload, chain)
    Host->>Enclave: Forward via Vsock
    Enclave->>Registry: get_parser(chain)
    Registry-->>Enclave: Parser Instance (e.g., Solana)
    Enclave->>Parser: parse(payload)
    
    alt Known Program (Preset)
        Parser->>Parser: Decode specific instruction
    else Unknown Program
        Parser->>Parser: Fallback to generic/raw view
        note right of Parser: Or use provided Metadata (ABI)
    end

    Parser-->>Enclave: ParsedTransaction
    Enclave-->>Host: ParseResponse
    Host-->>Client: ParseResponse
```

## 2. Startup & Health Check

Ensuring the enclave is healthy and reachable.

```mermaid
sequenceDiagram
    participant Orchestrator as K8s/User
    participant Host
    participant Enclave

    Orchestrator->>Host: Start Container
    Host->>Host: Initialize Networking
    Host->>Enclave: Spawn Enclave (nitro-cli run)
    
    loop Health Check
        Host->>Enclave: grpc.health.v1.Check
        Enclave-->>Host: SERVING
    end

    Orchestrator->>Host: grpc.health.v1.Check (External)
    Host->>Enclave: Forward Check
    Enclave-->>Host: SERVING
    Host-->>Orchestrator: SERVING
```

