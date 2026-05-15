# Devnet x402 test fixtures

**NON-SECRET.** These files seed a Solana devnet wallet used by
`x402_payai_devnet_test.rs` and `scripts/x402-solana-devnet-demo.ts`. Devnet
funds only; no production wallet ever derives from this seed.

## Files

- `wallet.seed`: 32-byte ASCII seed for the buyer wallet. Both Rust and TS
  paths derive a Solana keypair via `Keypair::from_seed(read("wallet.seed"))`.
  Trailing whitespace/newlines are trimmed before use.
- `wallet.address`: cached base58 buyer pubkey (the address you fund). Kept
  for documentation / quick-copy. The current value:
  `x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW`.

The devnet test self-transfers USDC (buyer == receiver), so no separate
`receiver.pub` is required — the buyer wallet is on both sides. If you need
a distinct receiver, set `X402_PAYTO` in the gateway env before launch.

## Why this seed is committed

The wallet is a long-lived devnet fixture funded with SOL + USDC. Rotating
the seed each run would invalidate the funded balance and force every
contributor (and CI) to re-airdrop before each test. The seed never controls
real assets.

## Refilling

- Devnet SOL: `solana airdrop 2 <address> --url devnet` or
  <https://faucet.solana.com>
- Devnet USDC: <https://faucet.circle.com> — mint
  `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`, 6 decimals
