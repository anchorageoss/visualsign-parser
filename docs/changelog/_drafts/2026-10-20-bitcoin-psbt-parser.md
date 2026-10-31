---
category: core                    # no dedicated bitcoin changelog stream yet
label: "Added Bitcoin PSBT parser"
description: ""
tags: ["Wallet API"]
---
Bitcoin PSBTs (BIP-174, base64 or hex) now decode into VisualSign payloads:
inputs and amounts, outputs and addresses, and the fee. Fees whose input
amounts or commitments cannot be trusted are shown as unknown with a claimed
amount; an all-taproot fee built from claimed amounts is marked as enforced
only by the BIP-341 signature. Derivation metadata does not label outputs as
change. NONE and SINGLE sighashes, duplicate inputs, out-of-range amounts,
v2-only fields, trailing bytes, unsigned transactions above 400,000 WU, and
PSBTs that fail BIP-174's signer script checks are refused. See [Bitcoin](/chains/bitcoin).
