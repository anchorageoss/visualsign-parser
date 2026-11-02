# UTXO Ownership Metadata — Design Spec (RFC)

Issue: #640
Status: RFC, open for review
Scope: Bitcoin and Bitcoin-like UTXO chains (Bitcoin first; Litecoin, Dogecoin,
Bitcoin Cash and Liquid as later adopters)

## Goal

Let an approver tell their own coins from everyone else's. Today the Bitcoin
parser renders every PSBT output as a bare address, so "pay 0.0006 BTC and
return 0.00039 BTC as change" looks the same as "send 0.00099 BTC away". The
parser has no wallet context, and `docs/chains/bitcoin.mdx` says so: derivation
data in a PSBT "cannot establish a change claim or ownership".

This spec adds wallet context as signed request metadata:

- **Accounts** (xpubs with key origin), so the parser can prove an input or
  output belongs to the wallet by re-deriving its script.
- **Previous transactions**, so amounts and fees can be verified where the
  signature does not commit to them.
- **Labels** (deposit IDs, invoice references), shown as attributed text.

Two rules govern all of it:

1. **Metadata is a set of hints the parser proves or attributes, never a source
   of conclusions.** An output is "yours" only when the parser re-derives its
   script. A label is shown as a label with its source, never in place of an
   address or amount.
2. **Every piece of context the metadata contributes is visible in the payload,
   with its source.** Nothing metadata-derived is applied silently, and nothing
   changes when no metadata is supplied.

## Non-goals (this spec)

- Litecoin MWEB, Bitcoin Cash CashTokens and Liquid asset metadata. Each needs
  its own rendering decision; see [Per-chain profiles](#per-chain-profiles).
- Output descriptors and miniscript policies. Single-sig is covered first, and
  multisig is a defined follow-up ([Multisig](#multisig-follow-up)).
- Turnkey policy integration. Turnkey's policy engine evaluates its own parse of
  the transaction; see [Policy engines](#policy-engines).

## Prior art

Every signer surveyed treats a PSBT's derivation claims as hints. Change is
accepted only after re-deriving the script from an account the device already
trusts:

| Signer | Where the trusted account comes from | Change rule |
|---|---|---|
| Trezor (`core/src/apps/bitcoin/sign_tx/change_detector.py`) | the inputs being spent | same script type, path prefix and multisig fingerprint as the inputs; change chain ≤ 1; index ≤ 1,000,000 |
| Ledger (`app-bitcoin-new`, `preprocess_outputs.c`) | a BIP-388 wallet policy registered on device (HMAC'd), or a default single-sig policy | key origin matches a policy key; script re-derived under the same policy; ≤ 10 change outputs |
| Coldcard (`consider_dangerous_change`) | the inputs, or a registered multisig wallet | warns on unusual path shape or an index more than 200 past the inputs |
| Bitcoin Core | the wallet database | IsMine and not in the address book |

Two lessons carry over directly:

- **Re-derive, never trust the claim.** A PSBT can carry our fingerprint and a
  path next to an output that pays someone else ("fake change").
- **The account set itself needs an anchor.** Re-deriving proves an output
  matches *an account the request supplied*, not that the account is the
  wallet's. Hardware wallets solve this with on-device registration (BIP-388).
  The enclave has no registration step, so this spec anchors the account set
  with a metadata signature from an allowlisted key
  ([Attestation](#attestation)).

Labels have no signer precedent: no hardware signer displays host-supplied free
text. BIP-329 is the closest format (JSON Lines records keyed by `txid:vout`,
labels up to 255 characters, unauthenticated).

## Proto

One shared message type, one `ChainMetadata` arm per chain, so each chain can
adopt it without changing the others:

```proto
message ChainMetadata {
  oneof metadata {
    EthereumMetadata ethereum = 1;
    SolanaMetadata solana = 2;
    NearMetadata near = 3;
    UtxoWalletMetadata bitcoin = 4;
    // later: litecoin, dogecoin, bitcoin_cash, liquid, each UtxoWalletMetadata
  }
}

message UtxoWalletMetadata {
  string network = 1;                    // "bitcoin", "testnet4", "signet", "regtest", ...
  string wallet_id = 2;                  // the wallet the signer key is enrolled for
  repeated UtxoAccount accounts = 3;
  repeated UtxoPrevout prevouts = 4;     // previous transactions, hex
  repeated UtxoLabel labels = 5;
  optional SignatureMetadata signature = 6;
}

message UtxoAccount {
  string name = 1;                       // display name, e.g. "Savings"
  string master_fingerprint = 2;         // 8 hex chars
  string origin_path = 3;                // e.g. "m/84'/0'/0'"
  string xpub = 4;
  UtxoScriptType script_type = 5;        // P2PKH, P2SH_P2WPKH, P2WPKH, P2TR
}

message UtxoPrevout {
  string txid = 1;
  string tx_hex = 2;                     // hex, as Solana's simulated_transaction_result
}

message UtxoLabel {
  oneof target {
    string outpoint = 1;                 // "txid:vout", an input
    uint32 output_index = 2;             // an output of this transaction
  }
  string text = 3;
}
```

- `metadata_digest` already covers `ChainMetadata`, so the metadata used for a
  parse is bound into the enclave-signed response with no further plumbing.
- Appending `bitcoin = 4` must not change the Borsh encoding of the existing
  arms. A test pins the discriminant bytes of `ethereum`, `solana` and `near`
  before the new arm lands.
- Binary data travels as hex strings, not `bytes`, so the host-side JSON forms
  stay readable (the Solana `simulated_transaction_result` precedent).
- `accounts` uses explicit xpub + origin rather than a descriptor string, so the
  first version needs no descriptor parser in the enclave.

## Attestation

Metadata is trusted only when it is signed by an allowlisted key, through the
mechanism the Ethereum ABI, Solana IDL and NEAR token-metadata paths already use
(`visualsign/src/signing.rs`).

**Signature.** `metadata_signing_prehash_v1(chain_tag, scope, body)`:

- `chain_tag`: one per chain, `bitcoin-wallet-metadata` first (later
  `litecoin-wallet-metadata`, ...), so a signature for one chain never verifies
  for another.
- `scope`: length-prefixed `network`, `wallet_id` and the unsigned txid. Binding
  the txid means a signature can't be replayed onto a different transaction.
- `body`: the Borsh encoding of the `UtxoWalletMetadata` with `signature`
  cleared.

**Policy.** The converter takes a deploy-time `MetadataTrustPolicy`, as NEAR's
does. Production runs `RequireAllowlistedSigner`. Nothing in a request can move
the parser between postures. The parse-request stamp is not treated as
attestation of the metadata. It authenticates transport. The metadata carries
its own signature so that its attester is known regardless of who relays it.

**Keys per wallet.** Allowlist entries are scoped as `(chain_tag, wallet_id,
pubkey)`, length-prefixed exactly as NEAR's `scoped_signer_key(domain, pubkey)`.
One key can be enrolled for many wallets, or each wallet can have its own; that
is configuration, not code. A key enrolled for wallet A can't vouch for wallet
B. It can still vouch for any account in wallet A, so high-value wallets should
have their own keys.

**Named entries.** `SignerAllowlist` gains an optional display name per key,
from deploy-time configuration only, never from the request. The change is
additive (a map instead of a set; `insert` and `contains` unchanged;
`insert_named` and `name_of` added). Env vars accept `name=hexkey` as well as a
bare `hexkey`. Names are checked at load time: printable ASCII, at most 32
characters, passing `validate_charset`. Every chain using the shared allowlist
can then render a signer name.

**Unsigned or unrecognized metadata.** Under `RequireAllowlistedSigner` the
request fails with an error naming the reason. Under `AcceptUnsigned` (dev and
CLI only) metadata is used but every row says `unsigned request metadata` in
place of a signer name, following NEAR's `TokenProvenance::Unsigned`.

**Enrolment cadence (open question).** Allowlists live in the deployment's
configuration, so adding a wallet or rotating its key needs a redeploy. If that
is too slow, the alternative is a pinned root key per chain that signs short
certificates ("key K may attest wallet W until T") carried in the metadata.

## Verification

All metadata is validated before any of it is used. Any failure rejects the
whole request with `Invalid wallet metadata: <reason>`. Nothing is dropped
silently.

**1. Validate.**

- Signature verifies under the deploy-time policy (above).
- `network` matches the converter's network.
- Each account's xpub depth and child number agree with its `origin_path`, and
  the origin's coin type matches the chain profile (below).
- Each prevout's `tx_hex` hashes to its `txid`, and the txid is spent by this
  transaction.
- Each label targets an outpoint this transaction spends or an output index it
  has, at most once per target.
- Label text is 1–64 printable ASCII characters (relaxed only by #622).

**2. Merge previous transactions.** Each prevout the PSBT lacks becomes its
`non_witness_utxo`, and the existing analysis runs unchanged. A prevout that
contradicts the PSBT's `witness_utxo` is rejected, as today.

**3. Classify each input and output.** The candidates are the `(fingerprint,
path)` pairs in `bip32_derivation` and `tap_key_origins`. For each pair whose
fingerprint matches an account:

- The path must extend the account origin by exactly two unhardened steps
  (`branch/index`). Otherwise the claim is unverified.
- Derive the key, build the account's script type (with the BIP-341 tweak for
  P2TR key-path) and compare it with the actual `scriptPubKey` (for an input,
  the prevout script).

| Result | Meaning | Counted as |
|---|---|---|
| `OwnVerified` | a derivation re-derives to the actual script | returning to the wallet |
| `ClaimedOwnUnverified` | a derivation names our fingerprint but doesn't re-derive | **sending**, with a warning |
| `External` | no derivation names our accounts | sending |

An output that is ours but carries no derivation data is `External`. That
undercounts the wallet, never overstates it.

**4. Path-shape warnings** (still `OwnVerified`): branch other than 0 or 1;
index above the account's highest issued index (later field; until then a fixed
bound of 10,000); more than one account used for change.

## Display

Facts from the transaction (address, amount, script type, outpoint, fee) keep
their existing fields and text. Metadata only adds fields, using the types the
Anchorage wallet renders (`docs/specs/anchorage-wallet-render-rules.md`):
`text_v2`, `address_v2`, `amount_v2`, `diagnostic`, inside `preview_layout`.

**Summary** (first field, `preview_layout`):

- `Sending`: `amount_v2`, the total of every output not `OwnVerified`, with
  "includes N falsely claiming to be yours" when any are `ClaimedOwnUnverified`.
- `Returning to your accounts`: `amount_v2`, the total of `OwnVerified`
  outputs.
- `Fee`: as today.
- `Ownership Source`: `text_v2`, naming the signer: `Wallet treasury-1 - account
  list signed by Anchorage BTC ops (03a1…9f), checked against the transaction`.

**Per input and output**, before the existing fields:

| Status | Text |
|---|---|
| Change, verified | `Change - Savings [73c5da0a] m/84'/0'/0'/1/7 - verified` |
| Own input, verified | `From your account - Savings [73c5da0a] m/84'/0'/0'/0/3 - verified` |
| Claimed but contradicted | `diagnostic`: `Claims to be your change but does not derive from account Savings; counted as sending` |
| Unusual path | `diagnostic`: `Unusual derivation path for account Savings: <why>` |
| External | `External - not an account in this wallet's metadata` |
| Label | `Label: Deposit #1234 (from wallet metadata)`, after the address |

- **Fingerprints on screen.** Each own row shows the account's master
  fingerprint, so a human can spot an unexpected account.
- **Labels after the address, in their own field.** A label next to an external
  address is the address-poisoning pattern ("Invoice (Acme Corp)" beside an
  attacker's address). Labels never appear in the condensed subtitle ahead of
  the address.
- **"Verified" on inputs** means the PSBT's prevout script re-derives. The
  amount is verified only when the prevout's full transaction is present; the
  existing `Amount Source` field states which.
- **Warnings use `diagnostic`**, which the wallet never omits.
- **No metadata, no change.** With no metadata the converter takes today's path
  and the payload is byte-identical (all golden fixtures unchanged).

## Intermediate output

The chain emits a Borsh `intermediate_output` (opt-in, as for Solana) with the
classification, so downstream policy can enforce on it rather than re-parse the
display:

```rust
pub struct UtxoOwnershipV1 {
    pub wallet_id: String,
    pub signer: Option<[u8; 33]>,          // the allowlisted key that signed
    pub inputs: Vec<Ownership>,            // in input order
    pub outputs: Vec<Ownership>,           // in output order
    pub sending_sat: u64,
    pub returning_sat: u64,
}

pub enum Ownership {
    OwnVerified { account: u32, branch: u32, index: u32 },
    ClaimedOwnUnverified { reason: String },
    External,
}
```

## Policy engines

Turnkey's policy engine evaluates its own parse of a Bitcoin transaction
(`bitcoin.tx.outputs`, `bitcoin.tx.inputs`, `bitcoin.tx.fee`), not visualsign
output. Ownership shown here informs the human approver but does not constrain
Turnkey policy by itself. A backend that wants policy to rely on change must
restate it in policy terms (for example an allowlist of change addresses, or a
consensus rule requiring an independent approver). The intermediate output above
is for policy engines that consume visualsign.

## Threats

| Threat | Signature | Parser check | Display |
|---|---|---|---|
| Client edits or injects metadata | stops it | re-derive; re-hash prevouts | every row names the signer |
| Signer key compromised | no | stops fake change against honest accounts, not a substituted account | fingerprints on screen; per-wallet keys limit reach; independent approver |
| Fake change (our fingerprint, foreign script) | n/a | `ClaimedOwnUnverified`, counted as sending | `diagnostic` |
| Ransom change (our key, odd path) | n/a | branch and index bounds | `diagnostic`, still verified |
| Input amount lie (CVE-2020-14199) | no | prevout must hash to txid and agree with `witness_utxo` | amount shown as claimed otherwise |
| Metadata replayed onto another transaction | txid in scope | every target must exist in the transaction | `metadata_digest` in the signed response |
| Multisig cosigner swap | no | out of scope until the multisig follow-up | single-sig only |
| Label spoofing or bidi tricks | limits who writes labels | ASCII, length cap, target must exist | label after the address, attributed |

## Per-chain profiles

The algorithm is shared. A chain profile supplies what differs:

| | Bitcoin | Litecoin | Dogecoin | Bitcoin Cash | Liquid |
|---|---|---|---|---|---|
| SLIP-44 coin type (mainnet) | 0 | 2 | 3 | 145 | 1776 |
| Script types | P2PKH, P2SH-P2WPKH, P2WPKH, P2TR | as Bitcoin (taproot to confirm) | P2PKH, P2SH | P2PKH, P2SH | P2WPKH, P2SH-P2WPKH, P2TR (confidential) |
| Address encoding | base58, bech32/bech32m `bc` | base58, bech32 `ltc` | base58 | CashAddr | blech32 / confidential |
| Signature commits to input amounts | segwit v0: own input only; taproot: all | as Bitcoin | **no**: every amount needs its prevout | **yes** (`SIGHASH_FORKID`) | blinded |
| Unsigned transaction format | PSBT | PSBT (MWEB extensions) | no PSBT standard (to decide) | no PSBT standard (to decide) | PSET |
| Extra rendering decision | — | MWEB peg-in/out: refuse at first | — | CashTokens: refuse at first | unblinding data; asset IDs |

- The coin-type check in validation stops an account enrolled for one chain from
  being presented as another chain's.
- A chain is ready to adopt this spec when its parser exists and its profile row
  is confirmed. The cells marked "to confirm" or "to decide" are not yet.

## Multisig (follow-up)

Accounts gain a `UtxoMultisigAccount` (threshold, sorted cosigner xpubs with
origins, script type). An output is change only if it re-derives under the
**identical** cosigner set and threshold as the inputs being spent, at the same
`branch/index` for every cosigner (Trezor's rule, which defeats
index-randomizing ransom attacks). Descriptor strings (BIP-380–386) can replace
the structured form once the enclave takes a descriptor parser.

## Prototype

A single-sig prototype exists in `visualsign-bitcoin` (not yet merged). It
covers P2PKH, P2SH-P2WPKH, P2WPKH and BIP-86 P2TR. It has no proto wiring or
signature check, and uses generic "wallet metadata" attribution. It has 14
tests: verified change (P2WPKH, P2TR), fake change, an unknown outpoint, a
prevout with the wrong txid, a prevout contradicting `witness_utxo`, network
mismatch, unusual branches, paths outside the account and hardened sub-paths.
All golden fixtures are unchanged.

## Rollout

1. **Shared allowlist names.** `SignerAllowlist` display names (useful to every
   chain on its own).
2. **Bitcoin wiring.** Proto, codegen, host JSON forms, Borsh discriminant pin,
   network from metadata, docs and regtest invariant updated (derivation data
   *without* metadata still never yields change).
3. **Bitcoin attestation and display.** Signature check, per-wallet scoping,
   signer name on screen, `intermediate_output`.
4. **Multisig, descriptors, highest-issued index.**
5. **Other chains**, one profile at a time.

## Open questions

1. **Contradicted claims:** warn and count as sending (prototype), or fail the
   request?
2. **Labels on external outputs:** allow them, or only on own coins?
3. **Wording before attestation lands:** "your accounts" or "accounts in this
   request"?
4. **Segwit v0 prevouts:** require them before an own input is called verified?
5. **Enrolment:** per-wallet keys in deploy-time config, or a root key issuing
   per-wallet certificates?
6. **Descriptors:** take `miniscript` into the enclave, or a narrow parser for
   `wpkh`, `tr` and `wsh(sortedmulti)`?
7. **Dogecoin and Bitcoin Cash:** what unsigned format a backend would send.
