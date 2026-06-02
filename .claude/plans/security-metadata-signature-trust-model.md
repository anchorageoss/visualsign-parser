# Plan: Harden the ABI/IDL metadata signature trust model

> Bootstrapped by `/draft` from commit history (no `/plan` was run). Intent
> reconstruction, not authored intent. Anchor for P4 alignment and
> address-pr-comments scope checks; treat thresholds as soft.

## Goal

Caller-supplied ABI (Ethereum) and IDL (Solana) metadata can carry a
secp256k1 signature used to vouch for the human-readable rendering of
calldata. The original scheme proved only that *some* key signed the JSON
body. It bound nothing to the chain or the on-chain identity, and accepted
any signer. Close those gaps so a signature means "this exact body, for this
exact identity, from an authorized signer."

## Scope

- **Domain-separated prehash (core).** Add a shared v1 prehash in
  `visualsign/src/signing.rs`: `SHA-256(le_u64(len)||DOMAIN || le_u64(len)||chain_tag
  || le_u64(len)||scope || le_u64(len)||body)`. Ethereum scope = 8-byte big-endian
  chain_id || 20-byte contract address; Solana scope = 32-byte program id. Every
  field is length-prefixed (LE u64) so the encoding is injective for arbitrary/empty
  field contents. Document the layout for external signers.
- **Bind signatures to identity.** Thread chain id + contract address
  (Ethereum) and program id (Solana) into signature validation and the CLI
  signer. A signature is valid only for the exact identity it was produced for.
- **Fail-closed signer allowlist.** A signed ABI/IDL is accepted only if the
  recovered signer is on an allowlist. Production lists come from env
  (`VISUALSIGN_ETH_ABI_SIGNERS`, `VISUALSIGN_SOL_IDL_SIGNERS`). Empty allowlist
  rejects all signed metadata. Unsigned metadata stays accepted (other guards
  apply). Shared core allowlist type reused across chains.
- **Gate the dev signing key.** Move `CLI_DEV_SIGNING_KEY_SEED` / `sign_abi`
  behind a `dev-signing` cargo feature (plus `cfg(test)`). CLI enables it;
  enclave + gRPC server link neither. Document that it must stay off prod
  parser_app builds (Cargo feature unification hazard).

## Non-goals

- No external/production signer keys are provisioned yet; format stays v1.
- No change to unsigned-metadata trust (trusted-program, reserved-name guards).
- No backwards compatibility for existing signatures; they must be re-issued.

## Acceptance criteria

- Replaying a signature across a different chain/address/program id is rejected.
- A signed ABI/IDL from a non-allowlisted key is rejected; empty allowlist
  rejects all signed metadata.
- Dev signing key/symbols absent from enclave and gRPC server builds.
- Prehash encoding is injective (regression test for embedded-separator
  collision); workspace builds, clippy `-D warnings`, and all tests pass.
