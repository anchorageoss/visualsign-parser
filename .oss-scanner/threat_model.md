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
  response against a pinned key), `parser_http_server` (`src/parser/http-server`, the public HTTP+JSON pivot for
  TVC: binds 0.0.0.0, serves `POST /visualsign/api/v1/parse` and `/v2/parse` with a request body limit, gates both
  on an `X-Stamp` signature check against `--allowed-stamp-pubkeys-hex`, and serves an NSM boot proof), and
  `parser_grpc_server` (`src/parser/grpc-server`, the non-attested dev server). `parser_cli` reads local files
  (capped at 10MB).
- The X-Stamp signature covers the raw request body only (no timestamp, nonce, method or path), and the check must
  run against the exact bytes the client sent (`src/parser/http-server/src/stamp.rs`). When
  `--allowed-stamp-pubkeys-hex` is not set the routes are open; that is the current documented behavior.
- The gateway signs a payment marker (`src/host_primitives/src/payment_marker.rs`) that `parser_app` can verify
  (`src/parser/app/src/payment_verify.rs`). That check exists but every call site currently passes
  `PaymentPolicy::Disabled`, so it is not enforced yet; treat its logic as in scope, and note that settlement-side
  fields are deliberately not cross-checked until a later version.

## Components that matter most / least
- Most: `src/chain_parsers/visualsign-*` (decoders, protocol visualizers), `src/visualsign` (payload types, field
  builders, signing/metadata verification in `visualsign::signing`, hex handling in `visualsign::encodings`),
  `src/parser/app`, `src/parser/gateway`, `src/parser/http-server`, `src/host_primitives`.
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
- High: gateway authentication or x402 payment bypass, `parser_http_server` X-Stamp bypass (accepting a request
  not signed by an allowed key, or verifying against re-serialized rather than raw bytes), acceptance of an
  enclave response that fails signature verification, or forgery/replay of a payment marker against a different
  request; panics or aborts on attacker-controlled input in `parser_app`, `parser_gateway` or
  `parser_http_server`; non-determinism in output that
  feeds hashing/signing; memory or output amplification (small input -> very large output or RSS) at protocol limits.
- Medium: resource-exhaustion DoS that needs a large but valid input; incorrect but non-misleading rendering.
- Low: issues only in `parser_grpc_server` (the non-attested dev server, which defaults to accept-unsigned ABIs
  when no trust posture flag is given) or in CLI-only helpers.

## Anything to leave alone
- Unsigned Ethereum `abi_mappings` being accepted under `--accept-unsigned-abis`, and unsigned Solana
  `idl_mappings` being accepted for programs that are not trusted built-ins or presets, are documented trust
  postures, not bugs. A present-but-invalid signature must still be rejected, and a caller IDL must never take
  effect for a program that `is_trusted_program` covers.
- The ABI trust posture is a deploy-time choice that `tools/tvc-deploy` writes into the deployment manifest's
  `pivotArgs`, where a signer can see it. Which posture a deployment runs is not a finding.
- Workspace lints forbid `unwrap`, `expect`, `panic!` and `unsafe`; report violations reachable from untrusted input.

## How trust in a deployment is established
- A signer approves one specific sha256 of the release `parser_app` binary after reviewing it. A finding matters
  when it affects the code compiled into that binary. The scan image keeps a copy built with the same recipe as
  `images/parser_app/Containerfile` (pinned stagex image, static musl, `--release`) at `/opt/release/parser_app`.
  It is built without a `VERSION` build argument, so the version string embedded in it is `dev`; its sha256
  therefore differs from the released binary a signer approves, which embeds the real version. It is a
  representative build, not the approved artifact. The debug build under `/src/src/target/debug` is for
  exercising code and reproducing findings.

## How you would like reports to look
- State the full git commit SHA of the checkout that was built and audited.
- Say whether the finding is in code compiled into the release `parser_app` (and which chain features), and give
  the sha256 of `/opt/release/parser_app` as built in your image. That hash will not equal a released binary's
  (see above); we re-derive hashes ourselves, so do not rely on it as proof.
