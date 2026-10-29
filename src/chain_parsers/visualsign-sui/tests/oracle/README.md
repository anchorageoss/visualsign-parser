# Sui migration oracle

Run from `src/` with `cargo test -p visualsign-sui differential_oracle --lib`.
The old Sui dependencies are **dev-dependencies only**. They use
`mainnet-v1.68.1` (`3c0f387ebb40b8be292d3b7bd3f5bee8ad226d33`), the same
revision as the parser before PR #576. Cargo.lock retains its existing yanked
`core2` dependency so the historical oracle remains reproducible.

The corpus contains 58 named operations from the existing Cetus, Suilend,
Momentum and Hashi fixtures, plus the two existing decoder cases copied into
`decode_fixtures.json`: **60 cases, 40 distinct transaction encodings**. The
count assertion prevents unnoticed deletion of existing coverage. An additional
independent test covers the 14 Fluid cases (10 current-deployment and 4
previous-deployment fixtures) added on upstream main while this
migration was under review, for 74 named protocol/decoder cases in total.

For each case the test compares the SDK-backed view against the upstream
`SuiTransactionBlockData::try_from_with_module_cache` with the same empty module
resolver used by the old parser. It checks sender, gas owner/price/budget,
input JSON and pure types, object reference metadata, command rendering, and
the re-encoded TransactionData bytes used by Transaction Details.

`legacy_intermediate.rs` is the unchanged intermediate builder from parser
commit `91eef67d9f6f6e6f22273880aa32ad9738a9f694`, retained independently of
the replacement implementation. It runs against the old Sui types and package
digest implementation. Every supported corpus case must produce intermediate
output, and the new converter must emit the exact same Borsh bytes. Synthetic
cases also compare native command typing order and Publish/Upgrade digests.
A programmable system transaction must match the old view normalization and
intentionally omit intermediate output. Three newer SDK fixtures must decode
with the SDK, fail the historical decoder, and fail public conversion.

This oracle does not compare the complete old and new VisualSign payload JSON.
Existing protocol tests separately assert their selected rendered fields. No
network or chain access occurs when these tests run.
