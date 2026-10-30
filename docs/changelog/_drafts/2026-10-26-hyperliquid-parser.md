---
category: core                    # no dedicated hyperliquid changelog stream yet
label: "Added Hyperliquid parser (opt-in)"
description: ""
tags: ["Wallet API"]
---
Hyperliquid L1 actions, which are signed as an opaque EIP-712 `Agent` hash, now
decode into VisualSign payloads: the parser recomputes the `connectionId` from the
action, nonce, vault address and expiry, refuses any mismatch, and shows the
order, cancel, transfer or deploy that the hash commits to. User-signed actions
(ApproveAgent, UsdSend, SpotSend, Withdraw, SendAsset, TokenDelegate,
ApproveBuilderFee, UsdClassTransfer) render from their typed data. Unknown action
types, unknown fields and floats are refused. The `hyperliquid` feature is
on in `parser_cli` and opt-in for `parser_app` and the http server. See
[Hyperliquid](/chains/hyperliquid).
