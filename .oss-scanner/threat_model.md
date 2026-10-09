# Threat model

## What this project does and where untrusted input enters
- visualsign-parser converts raw blockchain transactions (Ethereum, Solana, Sui, Tron, NEAR) into a deterministic
  VisualSign JSON payload that a human reads before signing. It is a signing control: the rendered payload must
  faithfully describe the bytes that will be signed.
- Untrusted input: the raw transaction (hex/base64) in every gRPC/CLI request, and caller-supplied decoding
  metadata: Ethereum `abi_mappings` and Solana `idl_mappings` (these drive how calldata/instructions are rendered).
- Network-facing entry points: `parser_app` (`src/parser/app`, enclave/VM binary with vsock + protobuf IPC),
  `parser_gateway` (`src/parser/gateway`, public HTTP in front of `parser_app`: optional shared bearer-token gate,
  x402 payment gating on `/visualsign/api/v2/parse`, and verification of the enclave's signature on every parse
  response against a pinned key), and `parser_grpc_server` (`src/parser/grpc-server`, the non-attested dev server).
  `parser_cli` reads local files (capped at 10MB).
- The gateway signs a payment marker (`src/host_primitives/src/payment_marker.rs`) that `parser_app` can verify
  (`src/parser/app/src/payment_verify.rs`). That check exists but every call site currently passes
  `PaymentPolicy::Disabled`, so it is not enforced yet; treat its logic as in scope, and note that settlement-side
  fields are deliberately not cross-checked until a later version.

## Components that matter most / least
- Most: `src/chain_parsers/visualsign-*` (decoders, protocol visualizers), `src/visualsign` (payload types, field
  builders, signing/metadata verification in `visualsign::signing`, hex handling in `visualsign::encodings`),
  `src/parser/app`, `src/parser/gateway`, `src/host_primitives`.
- Less: `src/parser/cli`, `src/examples`, `src/health_check`, `src/metrics`.
- Out of scope: `src/generated` (protobuf codegen output), test-only code (`solana_test_utils`, `test_utils`,
  fixtures), `docs/`, `tools/` deploy helpers.

## How to exercise it
- `make -C src build` then `make -C src test`.
- `cargo run --bin parser_cli -- decode --chain ethereum --network ETHEREUM_MAINNET --output human -t <hex>`.
- Fixture pairs live in `src/chain_parsers/*/tests/fixtures/{name}.input` / `{name}.expected`.

## How you rate severity
- Critical: the rendered payload differs from what the transaction does (hidden/misleading recipient, amount,
  token, or call), or an ABI/IDL signature or trust-policy check is bypassed so untrusted metadata is honoured,
  or a caller IDL takes effect for a program that `is_trusted_program` covers.
- High: gateway authentication or x402 payment bypass, acceptance of an enclave response that fails signature
  verification, or forgery/replay of a payment marker against a different request; panics or aborts on
  attacker-controlled input in `parser_app` or `parser_gateway`; non-determinism in output that
  feeds hashing/signing; memory or output amplification (small input -> very large output or RSS) at protocol limits.
- Medium: resource-exhaustion DoS that needs a large but valid input; incorrect but non-misleading rendering.
- Low: issues only in `parser_grpc_server` (the non-attested dev server, which defaults to accept-unsigned ABIs
  when no trust posture flag is given) or in CLI-only helpers.

## Anything to leave alone
- Unsigned Ethereum `abi_mappings` being accepted under `--accept-unsigned-abis`, and unsigned Solana
  `idl_mappings` being accepted for programs that are not trusted built-ins or presets, are documented trust
  postures, not bugs. A present-but-invalid signature must still be rejected, and a caller IDL must never take
  effect for a program that `is_trusted_program` covers.
- `tools/tvc-deploy` not yet emitting the trust-posture flags is a known, tracked follow-up.
- Workspace lints forbid `unwrap`, `expect`, `panic!` and `unsafe`; report violations reachable from untrusted input.
