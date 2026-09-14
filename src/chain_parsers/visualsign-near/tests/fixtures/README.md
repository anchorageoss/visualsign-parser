# NEAR Intents fixtures

Two kinds of file live here, and the prefix says which.

`near-com-*.json` were **recorded** from the live NEAR Intents app at near.com.
`intent-*.json` were **constructed** from the intent definitions in
`defuse_core::intents`, for the kinds near.com does not produce.

Every file is a NEP-413 envelope -- `{message, nonce, recipient}` -- which is
what a self-custody wallet is handed. `message` is a `DefusePayload` carrying
`signer_id`, `deadline` and `intents`; the app omits `verifying_contract` and
the inner `nonce`, so these do too.

## Recorded

A wallet was announced over the near-connect protocol from a page-context
script, carrying a freshly generated ed25519 keypair. A NEAR implicit account id
is the hex of the key that controls it, so that wallet could sign NEP-413 for an
account it genuinely controlled -- no impersonation, and nothing that could
execute, since the account holds nothing. Balance reads were answered locally so
the flows could be reached; the real solver relay then rejected each operation
with `INSUFFICIENT_BALANCE`. Each session generates a new keypair, so the signer
differs between files.

These matter because every other NEAR fixture in this repository was written by
hand from a specification. A fixture the app itself produced is the only kind
that can disagree with what we assumed.

| File | Flow | Intent |
| --- | --- | --- |
| `near-com-signin-verification.json` | Verify wallet | none -- an empty batch that burns the nonce |
| `near-com-transfer-intent.json` | Move to Confidential | `transfer`, `tokens` keyed by defuse asset id |
| `near-com-ft-withdraw-migration.json` | Update tokens | `ft_withdraw` back to `intents.near`, with a prose `memo` |

near.com's swap is not here. It spends the Confidential side of a balance, which
is computed server-side rather than read through a NEAR view call, so a
recording wallet cannot hold one and the swap stays disabled. `token_diff` is
constructed below instead.

## Constructed

One file per intent kind `visualsign_intents::render` handles, so that a kind
that stops rendering fails a test rather than going unnoticed. The signer is
`alice.near` throughout -- a named account, so these cannot be mistaken for
recordings.

| File | Intent | What it covers beyond the kind itself |
| --- | --- | --- |
| `intent-token-diff.json` | `token_diff` | a swap, with a `referral` |
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

## The transactions

Not here. An intents journey also has a NEAR transaction half -- wrapping,
registering storage, depositing into `intents.near`, unwrapping -- which
near.com never asks a wallet for. Those are built and pinned in the Anchorage
repository, at
`source/js/anchorage/apps/browser-extension/src/utils/near/journeyFixture.ts`,
and the sequence spanning both halves runs in
`source/go/service/blockchain/turnkeyclient/parsersmoke`.
