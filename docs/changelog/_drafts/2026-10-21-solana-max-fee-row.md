---
category: solana
label: "Added Solana max fee cap and row"
description: ""
tags: ["Wallet API"]
---
Every Solana payload now shows a top-level fee cap row in SOL right after the
header rows: the caller's new `SolanaMetadata.max_fee_lamports`
(`maxFeeLamports` on the HTTP gateway) when set, labelled "Max Fee", otherwise
the parser's default cap of 0.1 SOL, labelled "Max Fee (default)". The parser
rejects the request when the fee the transaction commits to exceeds the cap in
force: 5000 lamports per signature, counting the ones precompile instructions
verify, plus the compute budget's priority fee at the requested unit limit or,
without one, at the runtime's default limit for the message. Compute budget
requests decode as the runtime decodes them, and a request the runtime rejects
(undecodable, repeated, out of range, or unused) is refused too. The row is
therefore a true upper bound on what the transaction pays. A consumer that
re-prices the compute budget instructions after parsing must re-check the final
fee against the cap itself. The CLI accepts the same value as
`--max-fee-lamports`.

The Solana intermediate output moves to schema version 5 with two additions.
A `native_instruction_decoder` tag right after the version says which native
decoding the build has, so a consumer can tell a build without `json-parsed`
from a decode failure, and a build without `simulation` now marks a supplied
simulation `SimulationUnsupported` instead of refusing the request. A trailing
`max_fee_lamports` field carries the cap in force, so a consumer that checks
the transaction's fee reads the same number the signer saw. A decoder that
asserts the version must read both fields before it accepts the new output.

The `max_fee_lamports` field is part of the Borsh-encoded `ChainMetadata`, so
`metadata_digest` changes for every Solana request that sends `chain_metadata`,
even when the new field is absent. A client that recomputes the digest must add
the trailing field to its Borsh mirror. The parser and that mirror can ship in
either order, as long as both are live before the client sends Solana
`chain_metadata`.
