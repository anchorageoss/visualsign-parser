# VisualSign Parser Architecture

This folder contains the architectural documentation for the VisualSign Parser project.

## Navigation

-   [**00_overview.md**](./00_overview.md): High-level goals, glossary, and system boundaries.
-   [**01_context.md**](./01_context.md): Who uses it? (C4 Level 1)
-   [**02_containers.md**](./02_containers.md): Host vs. Enclave topology. (C4 Level 2)
-   [**03_components.md**](./03_components.md): Internal structure of the Parser App. (C4 Level 3)
-   [**04_data_model.md**](./04_data_model.md): Data structures and ERD.
-   [**05_sequences.md**](./05_sequences.md): Request flow and lifecycle diagrams.
-   [**06_deployment.md**](./06_deployment.md): Build and deployment view.
-   [**07_rust_design.md**](./07_rust_design.md): Code organization, traits, and patterns.

## Mapping to Code

| Architecture Concept | Code Location |
| :--- | :--- |
| **Enclave App** | `parser/app/` |
| **Host Wrapper** | `parser/host/` |
| **Core Traits** | `visualsign/src/vsptrait.rs` |
| **Chain Parsers** | `chain_parsers/` |
| **API Definition** | `proto/parser/parser.proto` |

