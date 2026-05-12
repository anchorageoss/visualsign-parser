# x402-gated HTTP API on parser_gateway

**Status:** Approved (design)
**Date:** 2026-05-12
**Owner:** André Stielau

## Problem

We want a public HTTP endpoint on the parser that monetizes per-call usage via the x402 v2 payment protocol, while preserving the existing Turnkey deployment path (which must not require payment). The endpoint must work in three deployment shapes:

- **Local dev** — points at a bundled mock facilitator so the full request/response flow is exercisable without a real wallet or settlement.
- **PayAI** — points at the public PayAI facilitator (`https://facilitator.payai.network`), Solana + EVM, no API key.
- **Custom / other** — facilitator URL, network, asset, recipient, price all overridable via env vars.

Price-per-call must be configurable, defaulting to *very low* values in `local` and `payai` profiles so dev and pilot deployments don't accidentally over-charge.

## Goals

- New `POST /visualsign/api/v2/parse` route with `x402-axum` middleware enforcing payment.
- `GET /health` and `POST /visualsign/api/v1/parse` (existing Turnkey route) remain open and behaviorally unchanged.
- Single source of truth for x402 config via env vars + named profiles.
- Multi-tag config support (`Vec<PriceTagConfig>`) even though typical deploys use one tag.
- Bundled `mock_facilitator` binary for local dev and integration tests; **never** built into TDX/Turnkey prod images.
- Fail-fast at startup on any config error; no silently-broken endpoints at request time.

## Non-goals

- No changes to `parser_app` (the in-enclave gRPC service). All payment gating sits in the gateway, in front of gRPC.
- No end-to-end test with a real wallet signing real x402 v2 payments. Out of scope; upstream `x402-rs` covers that.
- No support for facilitator-side smart-wallet sig schemes beyond what `x402-axum` provides by default. We layer the middleware as documented; protocol-level additions come for free with upstream versions.
- No Turnkey-signed bypass header for the v2 route. Turnkey continues to use v1.

## Architecture

```
                    ┌──────────────────────────────────────────┐
                    │  parser_gateway  (axum 0.8)              │
                    │                                          │
   client ─POST──▶  │  GET  /health                       open │
                    │  POST /visualsign/api/v1/parse      open │  (Turnkey-compatible, unchanged)
                    │  POST /visualsign/api/v2/parse  ┌──x402──┤
                    │                                 │ layer  │
                    └─────────────────────────────────┼────────┘
                          │ gRPC ParserService.Parse  │ verify/settle (HTTP)
                          ▼                           ▼
                    ┌──────────────┐         ┌──────────────────────────────┐
                    │  parser_app  │         │  facilitator                 │
                    │  (TDX/vsock) │         │  local  : mock_facilitator   │
                    │  or grpc-srv │         │  payai  : facilitator.payai…  │
                    └──────────────┘         │  custom : X402_FACILITATOR_URL│
                                             └──────────────────────────────┘
```

Two binaries change; one is new. Everything stays in the existing Cargo workspace.

- **`parser_gateway`** — axum 0.6 → 0.8 upgrade. New `/visualsign/api/v2/parse` route layered with `x402-axum`'s `X402Middleware`. Existing routes untouched.
- **`mock_facilitator`** — new tiny crate at `src/parser/mock-facilitator/`. Implements the x402 facilitator HTTP surface (`/verify`, `/settle`, `/supported`) and approves everything with a synthetic tx hash. Bundled into the local Containerfile only.
- **`parser_app`** — no changes.

## Components

### parser_gateway — extended

File layout (small refactor; current `main.rs` is 300 LoC and mixes concerns):

```
src/parser/gateway/src/
  main.rs              # bin entry, env reads, router assembly, shutdown
  state.rs             # AppState { grpc_client, health_client }
  handlers/
    health.rs          # GET /health (extracted as-is)
    parse.rs           # shared parse logic (Turnkey envelope → gRPC → Turnkey envelope)
  turnkey.rs           # TurnkeyRequestWrapper / TurnkeyResponseWrapper structs
  x402_config.rs       # X402Profile enum + X402Config loader (env → struct)
```

The v1 and v2 routes share the **same** handler body (`handlers::parse::handle`). Both take the Turnkey envelope and emit the Turnkey response shape. The only difference is that v2 is layered with `x402-axum`'s `X402Middleware`. The middleware exposes a chainable `.with_price_tag(...)` builder; for an N-element `price_tags` config, we call it N times during router construction so all tags appear in the 402 `accepts` array. v1 has no x402 layer.

### x402_config.rs — environment → middleware

```rust
pub enum X402Profile { Local, PayAi, Custom }

pub struct X402Config {
    pub facilitator_url: Url,
    pub facilitator_timeout: Duration,
    pub protocol_version: ProtocolVersion,   // V2 default
    pub price_tags: Vec<PriceTagConfig>,     // 1..N, non-empty
}

pub struct PriceTagConfig {
    pub network: Network,                    // base-sepolia, base, solana, ...
    pub asset: AssetSpec,                    // USDC (resolves to per-network address)
    pub price_usd: Decimal,
    pub pay_to: PayToAddress,                // EVM Address or Solana Pubkey
    pub scheme: Scheme,                      // Exact (default) | Upto
}

impl X402Config {
    pub fn from_env() -> Result<Self, ConfigError> { /* per matrix below */ }
}
```

Profile chooses a seeded `price_tags` of length 1; individual env vars or the JSON override mutate from there.

### mock_facilitator — new crate

`src/parser/mock-facilitator/` — axum 0.8 binary, ~100 LoC:

- `POST /verify` → `{ "isValid": true, "payer": <echoed> }`
- `POST /settle` → `{ "success": true, "transaction": "0xmock<random hex>", "network": <echoed>, "payer": <echoed> }`
- `GET /supported` → enumerates the local profile's network + asset
- Listens on `MOCK_FACILITATOR_PORT` (default `8090`)
- No real crypto; the gateway's x402-axum middleware does protocol-level header parsing and trusts the facilitator verdict.

### Container changes

- **`images/parser_app/Containerfile`** (local dev image): adds `mock_facilitator` to the build stage; entrypoint script starts it on `:8090` alongside existing processes; image defaults `X402_PROFILE=local` and `X402_FACILITATOR_URL=http://127.0.0.1:8090`.
- **TDX / Turnkey prod**: unchanged. `mock_facilitator` is neither built nor shipped.

## Config matrix

| Env var | Required? | `local` default | `payai` default | `custom` default |
|---|---|---|---|---|
| `X402_PROFILE` | no — defaults to `local` | `local` | set explicitly to `payai` | set explicitly to `custom` |
| `X402_FACILITATOR_URL` | no in `local`/`payai` | `http://127.0.0.1:8090` | `https://facilitator.payai.network` | **required** |
| `X402_FACILITATOR_TIMEOUT_SECS` | no | `5` | `5` | `5` |
| `X402_PROTOCOL_VERSION` | no | `v2` | `v2` | `v2` |
| `X402_PAYTO` | yes in `payai`/`custom`; optional in `local` | `0x000000000000000000000000000000000000dEaD` | **required** | **required** |
| `X402_PRICE_TAGS_JSON` | no | unset | unset | unset |

When `X402_PRICE_TAGS_JSON` is unset, the seeded `price_tags` from the profile is used (with `X402_PAYTO` filled in for `payai`/`custom`). When set, it discards the seeded tag and replaces with the parsed list; example:

```
X402_PRICE_TAGS_JSON='[
  { "network":"base",   "asset":"USDC", "payTo":"0xabc...", "priceUsd":"0.001", "scheme":"exact" },
  { "network":"solana", "asset":"USDC", "payTo":"So1...",   "priceUsd":"0.001", "scheme":"exact" }
]'
```

Seeded defaults per profile:

| Profile | seeded `price_tags[0]` |
|---|---|
| `local` | `{ network: base-sepolia, asset: USDC, priceUsd: 0.0001, payTo: $X402_PAYTO or 0x000…dEaD, scheme: exact }` |
| `payai` | `{ network: base,         asset: USDC, priceUsd: 0.001,  payTo: $X402_PAYTO (required),   scheme: exact }` |
| `custom`| no seed — `X402_PRICE_TAGS_JSON` required |

## Data flow

```
1.  client ──POST /visualsign/api/v2/parse (no X-PAYMENT)──▶ gateway
2.  gateway x402 layer ──▶ 402 Payment Required
       body: { x402Version: 2, accepts: [<price_tag_0>, <price_tag_1>, ...] }
3.  client signs payment authorization
4.  client ──POST /visualsign/api/v2/parse + X-PAYMENT: <base64>──▶ gateway
5.  gateway x402 layer ──POST /verify──▶ facilitator
6.  facilitator ──{ isValid: true, payer }──▶ gateway
7.  gateway parse handler ──gRPC ParserService.Parse──▶ parser_app (vsock/local)
8.  parser_app ──ParseResponse──▶ gateway
9.  gateway x402 layer ──POST /settle──▶ facilitator   (post-handler, only on 2xx)
10. facilitator ──{ success: true, transaction }──▶ gateway
11. gateway ──200 + Turnkey envelope + X-PAYMENT-RESPONSE header──▶ client
```

Notes:

- **Settle-after-success:** x402-axum settles only when the wrapped handler returned 2xx. A failed parse (bad chain, malformed payload) is 4xx and we do **not** settle — the payer is not charged for a broken request. Default x402-axum behavior; we do not override it.
- **Body limit:** the existing `DefaultBodyLimit::max(GRPC_MAX_RECV_MSG_SIZE)` layer stays applied router-wide so v2 inherits it.
- **Timeouts:** parse stays at 30s; facilitator verify/settle gets its own 5s timeout (configurable via `X402_FACILITATOR_TIMEOUT_SECS`).

## Error handling

| HTTP status | When | Body |
|---|---|---|
| `200` | parse + payment settled | Turnkey envelope |
| `400` | malformed JSON, unknown chain, bad payload | Turnkey envelope with `error` field (same shape as v1) |
| `402` | missing/invalid `X-PAYMENT` header | x402-axum default: `{ x402Version: 2, accepts: [...], error: "..." }` |
| `403` | facilitator says `isValid: false` | x402-axum default: `{ ..., error: "verification failed" }` |
| `500` | gRPC unavailable, missing fields in response | `{ error: "internal error" }` (no gRPC detail leak; matches v1) |
| `502` | facilitator unreachable or transport error | `{ error: "facilitator unavailable" }` |
| `504` | parse > 30s OR facilitator > 5s | `{ error: "request timed out" }` or `{ error: "facilitator timed out" }` |

Three concrete rules for the gateway code:

- **Don't settle on handler error.** Rely on x402-axum's default `settle_on_success` behavior. Verify in the integration test.
- **Don't leak gRPC `Status` messages.** The existing v1 handler maps `InvalidArgument → 400`, `NotFound → 404`, everything else to a generic 500 with `eprintln!` of the real error. The shared `handlers::parse::handle` keeps that mapping; v2 inherits it.
- **Fail-fast on startup, not per-request.** Any config error (missing `X402_PAYTO` for `payai`, malformed `X402_PRICE_TAGS_JSON`, invalid network/asset/address, unreachable facilitator on the startup probe) panics the binary at startup with a clear message. The startup probe is a single `GET /supported` request to the configured facilitator URL with the same 5s timeout; if it fails the gateway does not bind its listening port. No silently-broken v2 endpoint.

## Testing

**Unit tests** (`gateway/src/x402_config.rs`):
- `from_env` parses each profile correctly with no overrides.
- `X402_PAYTO` override fills in the seeded tag for `payai`/`custom`.
- `X402_PRICE_TAGS_JSON` override replaces the seeded list and rejects malformed input.
- Missing required fields per profile return a clear `ConfigError` (don't panic at this layer; the binary panics, the config does not).

**Mock facilitator unit tests** (`mock-facilitator/`):
- `/verify`, `/settle`, `/supported` return the expected shapes for a synthetic request.

**Gateway-level integration test** (extend `src/integration/tests/`):
- Spin up `mock_facilitator` + `parser_grpc_server` + `parser_gateway`.
- **Path 1:** `POST /visualsign/api/v2/parse` with no `X-PAYMENT` → asserts `402`, asserts body contains `accepts: [...]` with the local-profile defaults.
- **Path 2:** same request with a stub `X-PAYMENT` header → asserts `200`, correct Turnkey envelope, `X-PAYMENT-RESPONSE` header present.
- **Path 3:** same request with a malformed payload → asserts `400` AND asserts the mock facilitator never received `/settle`.
- **Path 4:** `POST /visualsign/api/v1/parse` with no `X-PAYMENT` → asserts `200`. Proves v1 stays unprotected.
- **Path 5:** `GET /health` with no `X-PAYMENT` → asserts `200`. Proves health stays unprotected.

**Manual smoke** (documented, not automated):
- Build the local Containerfile, `docker run`, curl `/health` and `/visualsign/api/v2/parse` from outside the container.

**Explicitly out of scope for this PR:**
- End-to-end test with a real wallet signing real x402 v2 payments.
- Load test or pricing math test.
- Test that exercises payai's real facilitator (deploy-time smoke, not CI).

## Risks / open questions

- **axum 0.6 → 0.8 upgrade**: the existing Turnkey handler uses standard extractors (`Json`, `State`, `DefaultBodyLimit`) and standard handler signatures. Changes are expected to be mechanical, but the upgrade is bundled into this PR and may surface a transitive dep conflict (e.g., `tonic` ↔ `hyper`/`tower` versions). Mitigation: validate `make -C src build` + `make -C src test` early; if blocked, fall back to hand-rolled middleware (one of the rejected options).
- **PayAI facilitator schema drift**: PayAI implements the x402 spec but the schema may drift from upstream `x402-types`. Mitigation: pin `x402-axum` to a known-good version; bump deliberately.
- **Solana payTo address shape**: `PayToAddress` needs to accept both EVM (20 bytes hex) and Solana (32 bytes base58). The `x402-types` crate should already model this; if not, our wrapper enum handles the discrimination at parse time.
