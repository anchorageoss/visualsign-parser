# cloudflare-x402-poc

Experiment for PRS-799: an x402 v2 paywall on `POST /visualsign/api/v2/parse`
as a Cloudflare Worker. Testnet only (Base Sepolia, `eip155:84532`).

This is the fallback route: our own Worker plus the public x402 facilitator.
It does not use Cloudflare Monetization Gateway, which needs an aged account
and a proxied zone.

- `worker.js` -- 402 challenge, then facilitator `/verify`, forward, and
  `/settle` only on a 2xx. A payment that differs from the advertised
  requirements is rejected before the facilitator is called.
- `buyer.mjs` -- paying client (the #304 `@x402/evm` exact flow). Set
  `SOCKS_PROXY=host:port` if Cloudflare is not directly reachable. The wallet
  key is read from `~/.config/visualsign/base-sepolia/wallet.key` and never
  printed. It writes a request/response transcript to `TRANSCRIPT`.
- `worker.test.js` -- `npm test`, with a mocked facilitator.

Worker bindings: `PAY_TO` (required), `PRICE_ATOMIC` (default `1000`),
`PARSER_BACKEND_URL` (unset serves a labelled stub instead of the parser),
`BACKEND_BEARER` (secret, optional), `FACILITATOR_URL` (optional).

Not done: pointing the Worker at a real parser backend, and mapping the
payment onto the payment marker.
