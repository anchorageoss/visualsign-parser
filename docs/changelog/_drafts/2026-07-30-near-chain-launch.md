---
category: near
label: "Added NEAR chain support and the NEAR Intents preset"
description: ""
tags: ["Wallet API"]
---
NEAR joins the supported chains: borsh transaction decoding, native transfers,
and token-movement `FunctionCall`s (`ft_transfer`, `ft_transfer_call`,
`ft_withdraw`). The NEAR Intents (Defuse Protocol) preset renders both the
pre-signature intent envelope (`near::sign_intent`) and signed
`execute_intents` batches across all seven signature standards (NEP-413,
ERC-191, TIP-191, raw ed25519, WebAuthn, TonConnect, SEP-53) and eleven intent
types. See [NEAR](/chains/near).
