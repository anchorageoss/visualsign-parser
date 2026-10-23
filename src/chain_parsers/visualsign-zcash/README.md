# visualsign-zcash

Keyless Zcash PCZT parser for VisualSign. See `src/lib.rs` for what is shown
and checked, and `docs/chains/zcash.mdx`.

## Validation

Four layers check the parser against sources other than itself. Each gives
ground truth for different fields:

| Layer | Ground truth for |
| --- | --- |
| Construction + viewing keys | shielded recipients, values, memos, change, padding, fee |
| Spec vectors | ZIP-244 digests, ZIP-225 encoding, ZIP-316/320 addresses, Orchard note commitments and memo recovery |
| Mainnet replay | txid, ZIP-244 digests (via on-chain signatures), transparent inputs/outputs, value balances, fee, counts |
| Regtest | all of the above on transactions a node accepts and mines |

**Offline, in `make test`:**

- `tests/ground_truth_test.rs` builds PCZTs from deterministic keys with the
  librustzcash roles a coordinator runs. The parser's recipients, values,
  memos and fee must equal the construction and the outputs trial-decrypted
  with the recipients' IVKs and the sender's OVKs. Tampered outputs
  (recipient, value, `rseed`, `cmx`/`cmu` or ciphertext swapped) must be
  refused.
- `tests/spec_vectors_test.rs` runs zcash-test-vectors files (pinned commit,
  sha256 checked) through the parser: ZIP-244 digests via a rebuilt PCZT,
  ZIP-320 TEX, ZIP-316 unified addresses (revisions 0 and 2), and Orchard
  note-encryption vectors.
- `tests/mainnet_test.rs` rebuilds the PCZTs of ten mainnet transactions
  (`tests/fixtures/mainnet/`, transparent, Sapling, Orchard and Ironwood) and
  checks the public fields only: explorers cannot see shielded amounts or
  recipients. The parser's ZIP-244 digests must verify each transaction's own
  ECDSA, RedPallas and RedJubjub signatures.

**Live node, label `test:zcash-regtest`** (`.github/workflows/zcash-regtest.yml`):
`tests/regtest.rs` starts `zebrad` on regtest (NU6.2, then NU6.3), shields
coinbase into Orchard, Sapling and Ironwood, spends between pools, and checks
the parser against the construction and against the mined transactions
trial-decrypted with the test keys. Adversarial PCZTs (understated input
amount, recipient or ciphertext swapped after proving, oversized action count,
wrong declared network, edited value balance) must be refused or flagged, and
the node shows what a signer would really have signed.

Run it locally with the pinned release (`.github/zebrad.env`):

```sh
bin="$(scripts/install-zebrad.sh /tmp/zebrad | tail -1)"
cd src && ZEBRAD="$bin/zebrad" ZCASH_PARAMS="$bin" \
  cargo test --release -p visualsign-zcash --test regtest -- --ignored --nocapture
```
