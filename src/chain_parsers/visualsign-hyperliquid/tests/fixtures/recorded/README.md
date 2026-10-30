# Recorded Hyperliquid testnet payloads

These `unsigned_payload` envelopes were captured from real end-to-end runs,
not generated for the tests. They come from a second, independent producer next to
the Python-SDK vectors in `../../vectors/`.

- Producer: `@nktkas/hyperliquid` 0.33.3 (JavaScript SDK) built the actions.
- Signer: an EIP-712 wallet signing the typed data as given (`eth_signTypedData_v4`).
- Posted to the Hyperliquid testnet API on 2026-10-26 between 14:10 and 14:13 UTC,
  from an unfunded test wallet. The API answered "does not exist" for that user,
  which shows it recovered the expected signer from each signature.
- The `connectionId` of `a1-order` and `a2-cancel` was recomputed independently
  with Python msgpack 1.0.8 and pycryptodome keccak; both match.
- `r1-remove-agent` and `t1-accept-terms` come from a later run on 2026-10-28, where the
  Hyperliquid testnet web app built the requests (its "Remove API Wallet" and terms
  prompts) and the same kind of wallet signed them. Hyperliquid accepted both.

| File | Kind |
| --- | --- |
| `a1-order.input.json` | L1 `order`, testnet |
| `a2-cancel.input.json` | L1 `cancel`, testnet |
| `b2-approve-agent-0x1.input.json` | user-signed `ApproveAgent`, chainId 1, no expiry |
| `e1-approve-expiring.input.json` | user-signed `ApproveAgent`, chainId 1, `valid_until` |
| `r1-remove-agent.input.json` | user-signed `ApproveAgent` to the zero address (the app's "Remove API Wallet"), chainId 1 |
| `t1-accept-terms.input.json` | user-signed `Hyperliquid:AcceptTerms`, chainId 1 |

`<name>.expected` is the rendered payload, checked by `tests/vectors.rs`
(`REGEN_FIXTURES=1` to update).
