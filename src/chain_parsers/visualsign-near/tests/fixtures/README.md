# Captured fixtures

Payloads recorded from the live NEAR Intents app at near.com, rather than
written here.

A wallet was announced over the near-connect protocol from a page-context
script, carrying a freshly generated ed25519 keypair. A NEAR implicit account
is the hex of its own public key, so that wallet could sign NEP-413 for itself
legitimately -- no impersonation, and nothing that could execute, since the
account holds nothing. The app's balance reads were answered locally so the
flow could be reached; the real solver relay then rejected the operation with
INSUFFICIENT_BALANCE.

These matter because every other NEAR fixture in this repository was written
by hand from the specification. A fixture the app itself produced is the only
kind that can disagree with what we assumed.

## near-com-transfer-intent.json

A NEP-413 envelope whose message is a `transfer` intent moving a balance
between accounts inside `intents.near`. Its `tokens` map is keyed by defuse
asset id, which is the shape the app produces and which hand-written fixtures
had not covered.
