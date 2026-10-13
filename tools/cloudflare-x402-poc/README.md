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

## Backend (Cloud Run)

`backend/` builds `parser_grpc_server` + `parser_gateway` into one image for a
non-attested dev backend (checked-in dev ephemeral key, mock boot proof).
Assemble a context from the repo and deploy from source:

```sh
ctx=$(mktemp -d)
git archive HEAD src | tar -x -C "$ctx"
cp tools/cloudflare-x402-poc/backend/* "$ctx"/
gcloud run deploy vsp-parser-poc --source "$ctx" --project <project> --region <region> \
  --no-allow-unauthenticated --memory 1Gi --max-instances 2 \
  --set-secrets=/secrets/bearer=vsp-poc-bearer:latest \
  --set-env-vars=GATEWAY_AUTH_BEARER_FILE=/secrets/bearer
```

Create the `vsp-poc-bearer` secret first (a random token), and give the Cloud
Run runtime account `roles/secretmanager.secretAccessor` on it. The same token
goes into the Worker as `BACKEND_BEARER`. `terraform/` then makes the service
publicly invokable with the org's `allUsersIngress/allowed` tag, the same
pattern as protocols-research PR 40; `/v1` stays closed by the bearer token.
It also records the service and secret (imported). State is local for now.

## Worker bindings

| Name | Required | Meaning |
| --- | --- | --- |
| `PAY_TO` | yes | receiving address |
| `PRICE_ATOMIC` | no, default `1000` | price in USDC atomic units (1000 = 0.001 USDC) |
| `PARSER_BACKEND_URL` | no | parser backend to forward to (its open `/visualsign/api/v1/parse` route); unset serves a labelled stub |
| `BACKEND_BEARER` | no | bearer token sent to the backend; set with `npx wrangler secret put BACKEND_BEARER` |
| `FACILITATOR_URL` | no, default `https://x402.org/facilitator` | x402 facilitator |

The backend should be a `parser_gateway` + `parser_grpc_server` deployment with
`GATEWAY_AUTH_BEARER_TOKEN` (or `_FILE`) set, so only the Worker can call its
open `/v1` route. The Worker calls v1 because the backend's own v2 route is
x402-gated and would ask the Worker to pay a second time.

Not done: mapping the payment onto the payment marker, so the backend does not
yet see proof of payment.
