# Hyperliquid samples

Input envelopes and what `parser_cli` renders for them, for looking at or loading into a viewer.
Each `<name>.input.json` is the `unsigned_payload` sent with `--chain hyperliquid`. Each
`<name>.rendered.json` is the resulting `SignablePayload`, or `<name>.error.txt` when the parser
refuses.

| Prefix | Source |
|---|---|
| `testnet-` | Recorded on Hyperliquid testnet on 2026-10-26 (`r1-` and `t1-` on 2026-10-28). Signed by an EIP-712 wallet, and posted by `@nktkas/hyperliquid` 0.33.3 (`r1-` and `t1-` by the Hyperliquid testnet web app). See `tests/fixtures/recorded/README.md`. |
| `sdk-` | Built by hyperliquid-python-sdk 0.24.0 (`tests/vectors/generate.py`). |
| `refused-` | `testnet-a1-order` with the order size changed after signing. The recomputed connectionId no longer matches the signed one, so the parser refuses. |

Covered: order (limit; trigger with cloid and builder fee), cancel, subaccount and vault transfers,
HIP-3 perp deploy, HIP-4 outcome deploy, and the user-signed USDC send, withdrawal, agent approval
(with and without an expiry), agent removal and terms acceptance.

Regenerate a rendering from `src/`:

```bash
cargo run -q --bin parser_cli -- decode --chain hyperliquid --output json \
  -t "$(cat chain_parsers/visualsign-hyperliquid/samples/<name>.input.json)"
```

These files are examples, not test fixtures. The golden tests in `tests/vectors.rs` are what CI checks.
