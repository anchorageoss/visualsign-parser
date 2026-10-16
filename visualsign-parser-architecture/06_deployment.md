# Deployment

The system is designed for OCI-compatible container orchestration, specifically targeting environments with Enclave support (AWS Nitro).

```mermaid
flowchart TD
    %% --- Styling Definitions ---
    classDef buildLayer fill:#f5f5f5,stroke:#9e9e9e,stroke-width:2px,stroke-dasharray: 5 5;
    classDef hostLayer fill:#e3f2fd,stroke:#1565c0,stroke-width:2px;
    classDef enclaveLayer fill:#fff3e0,stroke:#ef6c00,stroke-width:3px;
    classDef component fill:#ffffff,stroke:#333,stroke-width:1px,rx:5,ry:5;
    classDef external fill:#fafafa,stroke:#333,stroke-width:1px,stroke-dasharray: 3 3;

    %% --- Build Environment ---
    subgraph BuildEnv ["Build Environment"]
        direction TB
        SRC[Source Code]:::component -->|StageX / Docker| BUILD[OCI Image Build]:::component
        BUILD -->|Extract| EIF[Enclave Image File .eif]:::component
    end

    %% --- Runtime Environment ---
    subgraph RuntimeEnv ["Runtime Environment(EC2)"]
        direction TB
        
        subgraph HostOS ["Host OS (Untrusted)"]
            PROXY[Parser Host Process]:::component
            VSOCK[Vsock Driver]:::component
        end

        subgraph Nitro ["Nitro Enclave (Trusted)"]
            APP[Parser App Process]:::component
        end

        %% Vsock Communication Bridge
        PROXY <-->|CID: Port| VSOCK
        VSOCK <-->|CID: Port| APP
    end

    %% --- External / Deployment Flows ---
    LB[Load Balancer]:::external --> PROXY
    EIF -.->|Deploy via PCR| APP

    %% --- Apply Layer Styles ---
    class BuildEnv buildLayer
    class HostOS hostLayer
    class Nitro enclaveLayer

```

## Configuration

Configuration is minimal and mostly build-time to reduce runtime attack surface.

-   **Chain Support**: Compiled in via Cargo Features (e.g., `feature = "solana"`).
-   **Listen Address**: Hardcoded or CLI arg (internal vsock port).

## Feature Flags

The `Cargo.toml` should control which chain parsers are included in the binary to minimize binary size and attack surface.

| Feature | Description |
| :--- | :--- |
| `chain-ethereum` | Includes `visualsign-ethereum` crate. |
| `chain-solana` | Includes `visualsign-solana` crate. |
| `chain-sui` | Includes `visualsign-sui` crate. |
| `simulator` | Enables non-enclave simulation mode for local dev/testing. |

