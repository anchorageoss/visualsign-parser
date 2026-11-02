---
category: core                    # no dedicated zcash changelog stream yet
label: "Added Zcash PCZT parser (opt-in)"
description: ""
tags: ["Wallet API"]
---
Zcash PCZTs (hex or base64) now decode into VisualSign payloads across
transparent, Sapling, Orchard and Ironwood pools: outputs, memos, the fee and
its ZIP-317 check, and the shielded and per-input transparent sighashes. The
network is shown as declared and unsigned, and spend ownership is shown as
unverified because the parser has no viewing key. Redacted outputs and nonzero
outputs whose ciphertext cannot be recovered from their note fields are refused,
as are non-ALL sighashes, duplicate prevouts and oversized PCZTs; zero-value
outputs that do not decrypt are shown as padding (ZIP 326).
The chain is opt-in (`--features zcash`) and is not in the default images. See
[Zcash](/chains/zcash).
