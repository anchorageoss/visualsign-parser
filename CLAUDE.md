# CLAUDE.md

This file provides guidance to coding agents (Claude Code and pi) when working with code in this repository. It is symlinked as `AGENTS.md` so both agents load the same instructions.

## Build & Development Commands

All commands run from `src/`:

```bash
make -C src build          # Build all workspace crates
make -C src test           # Build all, then run all tests (integration tests need binaries)
make -C src lint           # cargo clippy --all-targets -- -D warnings
make -C src fmt            # cargo fmt
make -C src generated      # Regenerate protobuf types (tonic_build), then fmt
make -C src grpc-server    # Run the gRPC server locally
```

Run a single test:

```bash
cargo test -p visualsign-ethereum test_name
```

Parse a transaction locally:

```bash
cargo run --bin parser_cli -- decode --chain ethereum --network ETHEREUM_MAINNET --output human -t <hex>

# Browse a directory of raw-tx files in a local web UI (feature-gated):
cargo run --bin parser_cli --features serve -- serve --chain ethereum --network ETHEREUM_MAINNET --dir ./txs
```

CI requires: codegen produces no diff, clippy passes with `-D warnings`, all tests pass. Protoc v21.4.

## Architecture

**Multi-chain transaction parser** — converts raw blockchain transactions (hex/base64) into structured VisualSign JSON payloads for human-readable display in wallets.

### Core Flow

```
Raw tx bytes → ChainPlugin (CLI) or gRPC request
  → TransactionConverterRegistry (dispatches by chain)
    → VisualSignConverter<T> (chain-specific conversion)
      → SignablePayload (deterministic JSON output)
```

### Workspace Layout (src/)

- **`visualsign`** — Core library: `SignablePayload` types, field builders, `Transaction`/`VisualSignConverter` traits, `DeterministicOrdering` trait, error types
- **`chain_parsers/visualsign-{ethereum,solana,sui,tron,unspecified}`** — Per-chain converter crates. Ethereum and Solana are feature-gated (both on by default)
- **`parser/cli`** — CLI binary with `ChainPlugin` trait for per-chain args/metadata/registration
- **`parser/app`** — Enclave/VM binary using vsock + protobuf IPC (links qos_* modules)
- **`parser/grpc-server`** — tonic gRPC server wrapping parser_app
- **`generated`** — Protobuf codegen output (do not edit; run `make generated`)
- **`codegen`** — tonic_build script that generates protobuf types with serde+borsh derives
- **`integration`** — gRPC integration tests against parser_app

### Key Traits

- **`Transaction`** — Parse from string, identify transaction type
- **`VisualSignConverter<T>`** — Convert a `Transaction` into `SignablePayload`
- **`VisualSignConverterAny`** — Type-erased version for polymorphic registry storage
- **`ChainPlugin`** — CLI-only: register converter + build chain metadata from args
- **`DeterministicOrdering`** — Alphabetical field ordering for stable metadata hashing

### Ethereum-Specific Patterns

- **`VisualizerContext`** — Carries chain_id, sender, contract, calldata, registries; cloned with incremented depth for nested calls
- **`ContractRegistry`** — Maps `(chain_id, Address) → TokenMetadata` for token resolution
- **`LayeredRegistry<T>`** — Composes wallet-provided + compiled-in data
- **Protocol decoders** — Use `sol!` macro for type-safe ABI decoding; follow 4-step pattern: decode params → resolve tokens → format amounts → return field
- **Field builders** (`visualsign::field_builders`) — Always use `create_text_field`, `create_amount_field`, `create_number_field`, `create_address_field`, `create_raw_data_field` instead of constructing field structs directly
- **ASCII only** — Use `>=` not `≥`, `->` not `→` (terminal compatibility)

### Testing Patterns

- Fixture-based snapshot tests: `tests/fixtures/{name}.input` + `{name}.expected` pairs per chain crate
- Integration tests in `integration/tests/` use gRPC client against built binaries
- `test_utils` module in `visualsign` provides shared test helpers
- Place all `use` imports at the top of the test module, not inside individual test functions

### Local Dev Container

A unified Docker container (see `images/parser_app/Containerfile`) bundles parser_app + simulator_enclave + parser_host + Go gateway into a single image for non-TEE local development. Same API as production TDX deployment, only difference is no attestation. REST at `:8080`, gRPC at `:44020`. Build with `make non-oci-docker-images` from repo root.

### Workspace Lint Policy

Workspace-level clippy lints are enforced in `src/Cargo.toml`:

- **`unwrap_used = "deny"`** — Use `?` operator or explicit error handling instead of `.unwrap()`
- **`expect_used = "deny"`** — Same; use `?` or `.ok_or_else(|| ...)?`
- **`panic = "deny"`** — Return `Err(...)` instead of `panic!()`
- **`unsafe_code = "forbid"`** — No `unsafe` blocks

Exceptions: test modules use `#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. Build scripts allow `unwrap_used`. Some crates have temporary crate-level exemptions with `TODO(#231)` pending cleanup.

### Deploy-Time ABI Trust Posture

Whether the parser honours caller-supplied Ethereum ABI mappings that carry no signature
is a deploy-time choice, not a per-request one. `parser_app` requires exactly one of:

- `--accept-unsigned-abis` — unsigned `abi_mappings` are registered. A signature that
  *is* present must still verify (integrity), but its signer is not checked against an
  allowlist.
- `--accept-signatures-from-pubkey <hex>` (repeatable) — every mapping must be signed
  by one of the given secp256k1 keys; unsigned or otherwise-signed mappings are dropped.

This posture only governs Ethereum `abi_mappings`. Solana `idl_mappings` go through a
separate, unsigned-accepting path gated by the `VISUALSIGN_SOL_IDL_SIGNERS` env var,
unaffected by either flag.

The intended end state is for the flags to land in the TVC deployment manifest's
`pivotArgs` (see `tools/tvc-deploy`), so a signer can verify which posture a deployment
runs out of band; wiring `tools/tvc-deploy` to emit them has not landed yet and is
tracked as a follow-up. Represented in code by `visualsign::signing::MetadataTrustPolicy`,
threaded through `parser_app::config::ParserConfig` into
`EthereumVisualSignConverter::with_policy`. `parser_grpc_server` currently hardcodes
accept-unsigned (non-attested dev server); exposing the same flags there is a follow-up.
`parser_cli` runs require-signed against the local dev key it signs its own ABI files with.

### Design Decisions

- **Deterministic serialization everywhere** — BTreeMap for proto maps, `DeterministicOrdering` trait, alphabetical field ordering for stable metadata hashing (borsh encoding)
- **Bounded readers** — File loading capped at 10MB to prevent DoS
- **Type-erased converters** — `VisualSignConverterAny` trait objects for polymorphic registry without generics overhead
- **Feature gates for chains** — Ethereum/Solana gated, extensible to new chains
- **Rust edition 2024** on nightly channel 1.88
- **Unified hex/`0x` handling** — All hex inputs (raw transactions, signatures, public keys, addresses) decode through `visualsign::encodings`: `decode_hex` (strip + decode), `strip_hex_prefix`, and `split_hex_prefix`. These accept an optional `0x`/`0X` prefix (case-insensitive). Do not hand-roll prefix stripping per chain. Where a prefix is mandatory (e.g. JSON-RPC quantities/data), use `split_hex_prefix` and turn `None` into an error. New chains and address parsers reuse these rather than introducing their own prefix rules.
