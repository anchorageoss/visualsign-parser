# cloudflare-x402-poc

Experiment for PRS-799: an x402 v2 paywall on `POST /visualsign/api/v2/parse`
as a Cloudflare Worker. Testnet only (Base Sepolia, `eip155:84532`).

This is the fallback route: our own Worker plus the public x402 facilitator.
It does not use Cloudflare Monetization Gateway, which needs an aged account
and a proxied zone.

- `worker.js` -- 402 challenge, then facilitator `/verify`, forward, and
  `/settle` only on a 2xx. A payment that differs from the advertised
  requirements is rejected before the facilitator is called.
- `buyer.mjs` -- paying client (the #304 `@x402/evm` exact flow). It writes a
  request/response transcript to `TRANSCRIPT` (default `transcript.json`).
- `worker.test.js` -- tests with a mocked facilitator.
- `wrangler.toml` -- Worker name, entry point and `PRICE_ATOMIC`.

## Run it

Prerequisites: Node 20+, a Cloudflare account on any plan with a workers.dev
subdomain, and a Base Sepolia wallet funded with test USDC
(<https://faucet.circle.com>, pick Base Sepolia).

```sh
cd tools/cloudflare-x402-poc
npm install
npm test
```

Deploy the Worker. `PAY_TO` is the address that receives the payment:

```sh
npx wrangler login
npx wrangler deploy --var PAY_TO:0xYourReceivingAddress
```

Wrangler prints the Worker URL. Check it:

```sh
curl https://vsp-x402-poc.<your-subdomain>.workers.dev/health
```

Put the buyer's 32-byte private key (hex) in
`~/.config/visualsign/base-sepolia/wallet.key` with mode 600, or point
`WALLET_KEY` at another file. The key is never printed. Then run the paid
flow:

```sh
WORKER_URL=https://vsp-x402-poc.<your-subdomain>.workers.dev node buyer.mjs
```

Expected output: the buyer address and USDC balance, `unpaid status: 402`,
`paid status: 200`, the settlement receipt, and a
`https://sepolia.basescan.org/tx/...` link.

## Worker bindings

| Name | Required | Meaning |
| --- | --- | --- |
| `PAY_TO` | yes | receiving address |
| `PRICE_ATOMIC` | no, default `1000` | price in USDC atomic units (1000 = 0.001 USDC) |
| `PARSER_BACKEND_URL` | no | parser backend to forward to; unset serves a labelled stub |
| `BACKEND_BEARER` | no | bearer token sent to the backend; set with `npx wrangler secret put BACKEND_BEARER` |
| `FACILITATOR_URL` | no, default `https://x402.org/facilitator` | x402 facilitator |

Not done: pointing the Worker at a real parser backend, and mapping the
payment onto the payment marker.
