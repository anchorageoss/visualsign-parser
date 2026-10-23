---
category: solana
label: "Added Solana max fee row"
description: ""
tags: ["Wallet API"]
---
`SolanaMetadata` gains an optional `max_fee_lamports` field (`maxFeeLamports`
on the HTTP gateway). When set, the payload shows it as a top-level "Max Fee"
row in SOL right after the header rows. The parser rejects the request when
the fee the transaction commits to exceeds the cap: 5000 lamports per
signature, counting the ones precompile instructions verify, plus the compute
budget's priority fee. The row is therefore a true upper bound on what the
transaction pays. The CLI accepts the same value as `--max-fee-lamports`.

The field is part of the Borsh-encoded `ChainMetadata`, so `metadata_digest`
changes for every Solana request that sends `chain_metadata`, even when the
new field is absent. Deploy the parser first, then release the client that
recomputes the digest with the trailing field in its Borsh mirror, and only
then start sending the field; between the two deploys the client's digest
check fails closed for Solana requests that carry `chain_metadata`. A payload
signed before the deploy keeps the digest and the bytes it was created with,
so it still verifies. To roll back, revert the client mirror and the parser
together and stop sending the field first.
