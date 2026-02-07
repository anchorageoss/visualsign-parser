# Overview

**VisualSign Parser** is a secure, enclave-ready transaction parsing engine. It decodes raw, unsigned blockchain transaction payloads into human-readable "VisualSign" intent structures.

This system bridges the gap between opaque hex strings (what the blockchain needs) and semantic intent (what the user needs to verify). It is designed to run within a Trusted Execution Environment (TEE), such as AWS Nitro Enclaves, ensuring that the decoding logic is tamper-proof and verifiable.

## Goals

1.  **Security**: Operate within an enclave (TEE) to guarantee code integrity.
2.  **Transparency**: Provide a clear decoding of the functions the user will interact with.
2.  **Determinism**: Pure function behavior `(payload, metadata) -> visual_intent`.
3.  **Multi-Chain**: Support extensible parsing for EVM (Ethereum), SVM (Solana), Move (Sui), and TVM (Tron).
4.  **Statelessness**: No network access required inside the parsing boundary; all schema/metadata is provided via input or embedded.

## Non-Goals

-   **Execution Simulation**: This is not an EVM/SVM execution engine. It does not simulate state changes (balance updates), but rather decodes *intent* (e.g., "Call `swap` on Uniswap V3 with parameters X, Y").
-   **Key Management**: The parser does not hold private keys. It parses *unsigned* payloads.
-   **Network Indexing**: The parser does not query the blockchain.

## Glossary

| Term | Definition |
| :--- | :--- |
| **Unsigned Payload** | The raw binary/hex transaction data before signature (e.g., RLP encoded tx, Solana Message). |
| **VisualSign** | The standardized, human-readable output schema representing the transaction's intent. |
| **Chain Metadata** | Auxiliary data (ABIs, IDLs) provided at runtime to assist decoding dynamic contract interactions. |
| **Enclave (App)** | The trusted component running the parsing logic, isolated from the host. |
| **Host** | The untrusted wrapper managing networking and lifecycle for the Enclave. |
| **Preset** | Hardcoded parsing logic for well-known protocols (e.g., Uniswap, Jupiter) to ensure high-fidelity decoding without external ABIs. |

