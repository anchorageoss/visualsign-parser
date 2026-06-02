# Plan: Ethereum decode follow-ups (bootstrapped)

> plan_bootstrapped: true — reconstructed from PR #354 body and branch commits, not authored before implementation. Treat alignment thresholds as soft.

## Goal

Close three follow-ups surfaced by the security review of the caller-supplied-metadata and token-decoding paths in the Ethereum visualizer, so registered-token amounts and ERC1155 calls render correctly and safely in the signing UI.

## Scope

1. **Permit2 amount truncation.** The Permit2 `approve` / `permit` / `transferFrom` decoders narrowed `uint160` amounts via `to_string().parse::<u128>().unwrap_or(0)`, collapsing any amount above `u128::MAX` to `0` for registered tokens. A malicious frontend could hide a large approval/transfer as a no-op. Switch to full-width `format_token_amount_u256`, matching the fix already applied to the Universal Router decoders.

2. **Canonical ERC1155 rendering.** The known-token short-circuit routed `ErcStandard::Erc1155` to a `None` visualizer, so a registered ERC1155 contract rendered as raw hex. Add an `ERC1155Visualizer` (mirroring the ERC20/ERC721 visualizers) decoding `safeTransferFrom` and `safeBatchTransferFrom`, wired into the canonical-token dispatch. Unrecognized selectors still fall back to raw hex.

3. **Regression coverage.** Add EIP-1559 mirrors of the caller-ABI-spoofing tests and a direct test that drives the dispatch with a poisoned request-scoped registry to assert the compiled-in (global) layer always wins. Pin exact decimal-scaled Permit2 amounts so a wrong-but-nonzero rendering is also rejected.

## Non-goals

- No change to the security lock-out: a caller-supplied ABI must never relabel a known token. This branch preserves it, not weakens it.
- No public API or output-schema changes for existing registered tokens.
- No new token registry entries; ERC1155 support is groundwork for the first canonical entry.

## Acceptance criteria

- Permit2 amounts above `u128::MAX` render accurately for registered tokens instead of as `0`.
- Registered ERC1155 `safeTransferFrom` / `safeBatchTransferFrom` decode to structured fields, not raw hex; unknown selectors still fall through to raw hex.
- `cargo test -p visualsign-ethereum`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass.
- Truncation and layered-registry-poisoning regressions fail if the respective bug is reintroduced.
