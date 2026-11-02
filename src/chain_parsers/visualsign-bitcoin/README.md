# visualsign-bitcoin

Bitcoin PSBT (BIP-174) parser for VisualSign. See `src/lib.rs` and
`src/analysis.rs` for what is shown and which amounts/fees are treated as
verified.

## Validation

Two layers check the parser against sources other than itself, in the same
spirit as the Solana surfpool lane.

**Offline, in `make test`:**

- `tests/vectors_test.rs` replays every PSBT test vector from BIP-174, BIP-370
  and BIP-371 (`tests/fixtures/vectors/bip_vectors.json`, extracted by
  `gen_bip_vectors.py` from a pinned commit of bitcoin/bips, sha256 recorded).
  Invalid vectors must be rejected, valid ones must render the same outputs,
  input amounts and fee as rust-bitcoin's own PSBT accessors, and PSBTv2 must be
  refused. Known gaps are listed in the file and pinned by `#[ignore]`d tests.
- `tests/mainnet_test.rs` rebuilds unsigned PSBTs from six confirmed mainnet
  transactions (P2PKH, P2SH-P2WPKH, P2WPKH, P2WSH, P2TR, OP_RETURN; raw
  transactions and the explorer's view in `tests/fixtures/mainnet/`) and checks
  the rendered fee, addresses and amounts against the explorer.

**Live node, label `test:bitcoin-regtest`** (`.github/workflows/bitcoin-regtest.yml`):
`tests/regtest.rs` starts `bitcoind -regtest`, has Bitcoin Core build PSBTs
spending every address type, checks the rendering against `decodepsbt`, signs,
finalizes and `testmempoolaccept`s them, then replays tampered PSBTs (lying
`witness_utxo`, mismatched `non_witness_utxo`, fake change derivations,
non-default sighashes).

Run it locally with the pinned release (`.github/bitcoind.env`):

```sh
bin="$(scripts/install-bitcoind.sh /tmp/bitcoind | tail -1)"
cd src && BITCOIND="$bin/bitcoind" BITCOIN_CLI="$bin/bitcoin-cli" \
  cargo test -p visualsign-bitcoin --test regtest -- --ignored --nocapture
```
