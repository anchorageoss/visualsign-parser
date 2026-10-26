# Native multisig SDK vectors

`generate.py` invokes the official `hyperliquid-python-sdk` signing functions at
revision `2fdb18f9517675ea03695a0962bd19eece9c83f0`. It uses publicly known private
integers 1, 2 and 3 solely as disposable test vectors. Never fund those accounts.
No network operations occur. The Rust implementation is not used to generate
expected hashes or signatures.

Reproduce with `eth-account` and `msgpack` installed and that SDK checkout on
`PYTHONPATH`:

```sh
PYTHONPATH=/path/to/pinned-sdk python generate.py
```

The 43 fixtures cover 16 supported action types, membership removal, initial
conversion, member and leader envelopes, a minimal-width signature scalar,
mainnet versus testnet, and vault/expiry hashing. `unsignedPayload` is a raw JSON
string to retain SDK field order. Digests are captured directly from the SDK's
EIP-712 signing operation. These are offline vectors, not evidence of live
network submission, current account membership or device acceptance.

Seven review snapshots cover member/leader order review, initial conversion,
threshold change, multisig removal, USD transfer and API-agent delegation. They
retain explicit caller-supplied membership trust, quorum and delegation disclosures.
