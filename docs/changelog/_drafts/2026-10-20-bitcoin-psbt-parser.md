---
category: core                    # no dedicated bitcoin changelog stream yet
label: "Added Bitcoin PSBT parser"
description: ""
tags: ["Wallet API"]
---
Bitcoin PSBTs (BIP-174, base64 or hex) now decode into VisualSign payloads:
inputs and amounts, outputs and addresses, and the fee. Fees whose input
amounts or commitments cannot be trusted are shown as unknown with a claimed
amount. Derivation metadata does not label outputs as change. NONE and SINGLE
sighashes, duplicate inputs, out-of-range amounts, v2-only fields, trailing
bytes, and unsigned transactions above 400,000 WU are refused. See [Bitcoin](/chains/bitcoin).
