# NEAR Intents fixtures

Two kinds of file live here, and the prefix says which.

- `near-com-*.json` were **recorded** from the live NEAR Intents app at
  near.com.
- `intent-*.json` were **constructed** from the intent definitions in
  `defuse_core::intents`, for the kinds near.com does not produce.

`tests/intents_fixtures.rs` drives every one of them, and its last case walks
this directory, so a fixture no test claims fails rather than going unnoticed.

## What a fixture is

Every file is a NEP-413 envelope -- the shape a self-custody wallet is handed:

```json
{"message": "<DefusePayload, as a JSON string>",
 "nonce":   "<32 bytes, base64>",
 "recipient": "intents.near"}
```

The envelope is chain-specific and the content is not. This crate decodes
NEAR's three envelopes; `visualsign-intents` decodes the payload inside and
knows nothing about how it arrived, which is how the same intent renders the
same way whichever chain carried it.

```
   envelope (chain-specific)               content (chain-free)

   NEAR ─┬─ transaction: an execute_intents
         │     call to intents.near       ─┐
         │                                 │
         ├─ NEP-413 message ── every       │    ┌───────────────────────┐
         │     fixture here is one        ─┤    │ DefusePayload         │
         │                                 │    │   signer_id           │
         └─ raw message: a bare payload,   │    │   verifying_contract* │
               no envelope around it      ─┼──> │   deadline            │
                                           │    │   nonce             * │
   Solana ─── raw ed25519 message         ─┤    │   intents: [ ... ]    │
   Ethereum ─ ERC-191 message             ─┘    └───────────────────────┘
     (decoded by their own chain crate,
      not by this one)

   * a NEP-413 envelope supplies both from itself -- its recipient and its
     nonce -- so the message omits them. near.com's payloads do, and so do
     these fixtures.
```

## Recorded

A wallet was announced over the near-connect protocol from a page-context
script, carrying a freshly generated ed25519 keypair. A NEAR implicit account
id is the hex of the key that controls it, so that wallet could sign NEP-413
for an account it genuinely controlled -- no impersonation, and nothing that
could execute, since the account holds nothing. Balance reads were answered
locally so the flows could be reached; the real solver relay then rejected each
operation with `INSUFFICIENT_BALANCE`. Each session generates a new keypair, so
the signer differs between files.

These matter because every other NEAR fixture in this repository was written by
hand from a specification. A fixture the app itself produced is the only kind
that can disagree with what we assumed.

| File | Flow | Intent |
| --- | --- | --- |
| `near-com-signin-verification.json` | Verify wallet | none -- an empty batch that spends the nonce |
| `near-com-transfer-intent.json` | Move to Confidential | `transfer`, `tokens` keyed by defuse asset id |
| `near-com-ft-withdraw-migration.json` | Update tokens | `ft_withdraw` back to `intents.near`, with prose in `memo` |

The Anchorage repository carries the same recordings with their submitted
wire form and the findings that came out of them, at
`source/js/anchorage/apps/browser-extension/e2e/near-intents/captured/`.

### Why the swap is not here

near.com splits a balance into Main and Confidential, and the swap page spends
the Confidential side. Main is read with `mt_batch_balance_of` against
`intents.near`, which a page-context `fetch` can answer; the Confidential
balance reaches no NEAR view call and no JSON-RPC method -- it is computed
server-side and arrives in the page's own payload. A recording wallet can hold
a Main balance, and did, but the swap page reads zero against it and leaves the
action disabled. `token_diff` is constructed below instead.

## Constructed

One file per intent kind `visualsign_intents::render` handles, so a kind that
stops rendering fails a test. The signer is `alice.near` throughout -- a named
account, so these cannot be mistaken for recordings.

| File | Intent | What it covers beyond the kind |
| --- | --- | --- |
| `intent-token-diff.json` | `token_diff` | a swap, with a `referral`; two tokens at different decimals |
| `intent-transfer-multi-token.json` | `transfer` | more than one token, and a `memo` |
| `intent-ft-withdraw-with-storage-deposit.json` | `ft_withdraw` | paying the receiver's storage |
| `intent-nft-withdraw.json` | `nft_withdraw` | |
| `intent-mt-withdraw.json` | `mt_withdraw` | parallel `token_ids` and `amounts` |
| `intent-native-withdraw.json` | `native_withdraw` | |
| `intent-storage-deposit.json` | `storage_deposit` | |
| `intent-add-public-key.json` | `add_public_key` | an account-control warning |
| `intent-remove-public-key.json` | `remove_public_key` | an account-control warning |
| `intent-set-auth-by-predecessor-id.json` | `set_auth_by_predecessor_id` | an account-control warning |
| `intent-auth-call.json` | `auth_call` | an account-control warning |
| `intent-batch-swap-then-withdraw.json` | `token_diff` + `ft_withdraw` | two intents under one signature |
| `intent-empty-batch.json` | none | the same empty batch near.com signs, under a named signer |

A lone intent takes its type into the title (`NEAR Intent: Token Diff`); a
batch, empty or otherwise, keeps the generic `NEAR Intent`, because no single
name describes it.

## The transactions

Not here. An intents journey also has a NEAR transaction half -- wrapping,
registering storage, depositing into `intents.near`, unwrapping -- which
near.com never asks a wallet for.

```
  NEAR ──near_deposit──> wNEAR ──ft_transfer_call──> [ intents.near ]
                                  msg = account to credit      │
                                                               │
                    these fixtures ──> token_diff, ft_withdraw ┘
```

Those transactions are built and pinned in the Anchorage repository, at
`source/js/anchorage/apps/browser-extension/src/utils/near/journeyFixture.ts`,
and the sequence spanning both halves runs in
`source/go/service/blockchain/turnkeyclient/parsersmoke`.

The deposit and the swap name the same token, so they resolve it the same way:
`actions.rs` sends a NEP-141 amount through the same `token_amount_field` and
the same registry the intents renderer uses, rather than rendering base units.
