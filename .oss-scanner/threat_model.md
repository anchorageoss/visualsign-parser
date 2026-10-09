# Threat model

## What this project does and where untrusted input enters
- visualsign-parser converts raw blockchain transactions (Ethereum, Solana, Sui, Tron) into a deterministic
  VisualSign JSON payload that a human reads before signing. It is a signing control: the rendered payload must
  faithfully describe the bytes that will be signed.
- Untrusted input: the raw transaction (hex/base64) in every gRPC/CLI request, and caller-supplied decoding
  metadata: Ethereum `abi_mappings` and Solana `idl_mappings` (these drive how calldata/instructions are rendered).
- The gRPC server (`src/parser/grpc-server`) and `parser_app` (`src/parser/app`, enclave/VM binary with
  vsock + protobuf IPC) are the network-facing entry points. `parser_cli` reads local files (capped at 10MB).

## Components that matter most / least
- Most: `src/chain_parsers/visualsign-*` (decoders, protocol visualizers), `src/visualsign` (payload types, field
  builders, signing/metadata verification in `visualsign::signing`, hex handling in `visualsign::encodings`),
  `src/parser/app`.
- Less: `src/parser/cli`, `src/examples`, `src/health_check`, `src/metrics`.
- Out of scope: `src/generated` (protobuf codegen output), test-only code (`solana_test_utils`, `test_utils`,
  fixtures), `docs/`, `tools/` deploy helpers.

## How to exercise it
- `make -C src build` then `make -C src test`.
- `cargo run --bin parser_cli -- decode --chain ethereum --network ETHEREUM_MAINNET --output human -t <hex>`.
- Fixture pairs live in `src/chain_parsers/*/tests/fixtures/{name}.input` / `{name}.expected`.

## How you rate severity
- Critical: the rendered payload differs from what the transaction does (hidden/misleading recipient, amount,
  token, or call), or an ABI/IDL signature or trust-policy check is bypassed so untrusted metadata is honoured.
- High: panics or aborts on attacker-controlled input in parser_app/grpc-server; non-determinism in output that
  feeds hashing/signing; memory or output amplification (small input -> very large output or RSS) at protocol limits.
- Medium: resource-exhaustion DoS that needs a large but valid input; incorrect but non-misleading rendering.
- Low: issues only in the local dev server (it hardcodes accept-unsigned ABIs by design) or CLI-only helpers.

## Anything to leave alone
- Unsigned Ethereum `abi_mappings` being accepted under `--accept-unsigned-abis`, and unsigned Solana
  `idl_mappings` always being accepted, are documented trust postures, not bugs. A present-but-invalid signature
  must still be rejected.
- `tools/tvc-deploy` not yet emitting the trust-posture flags is a known, tracked follow-up.
- Workspace lints forbid `unwrap`, `expect`, `panic!` and `unsafe`; report violations reachable from untrusted input.
