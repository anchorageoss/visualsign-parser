# parser_gateway

HTTP gateway in front of `parser_app`'s gRPC service. Terminates client
requests, optionally gates `/visualsign/api/v2/parse` behind an x402
(HTTP 402 Payment Required) handshake, and verifies the TVC enclave's
signature on every parse response before returning it.

## Routes

| Method | Path                          | Gated by x402? | Notes                              |
| ------ | ----------------------------- | -------------- | ----------------------------------- |
| GET    | `/health`                     | no             | proxy to backend gRPC health       |
| POST   | `/visualsign/api/v1/parse`    | no             | open                                |
| POST   | `/visualsign/api/v2/parse`    | **yes**        | configured via env (see below)     |

The v2 route is only mounted if the configured x402 facilitator responds
to a `/supported` probe at startup. If the facilitator is unreachable the
gateway logs and continues serving v1 + health only.

## TVC attestation

Every successful v1/v2 parse response is signed by `parser_app`'s
ephemeral P256 keypair, provisioned into the enclave at boot. The gateway
verifies the signature against a **pinned** public key. On failure it
returns `502 Bad Gateway`; the x402 middleware's settle-on-success
contract then skips `/settle`, so an unattested response is never
charged to the payer.

The pinned pubkey is provided to the gateway as a launch argument by the
TVC stack. The value is `qos_hex::encode(P256Public::to_bytes())` -- the
exact format `parser_app` emits in the wire signature's `publicKey` field.

```sh
# Set by TVC at boot (or via your local-dev compose file)
TVC_DEMO_PINNED_PUBKEY_HEX=<260 hex chars>
# Or, equivalently:
TVC_DEMO_PINNED_PUBKEY_FILE=/path/to/pubkey.hex
```

If neither is set:
- `X402_PROFILE=local`: the gateway logs a warning and skips attestation.
- otherwise: the gateway **exits with code 1** at startup (fail-closed).

This is a demo-only verifier (see `attestation.rs` for the production
replacement sketch): it checks a pinned pubkey, not a real Nitro/TDX
attestation document.

## x402 configuration

All env vars are read at startup. Bad values fail-closed (gateway exits 1).

| Env var                          | Required? | Default                             | Meaning                                                                                |
| --------------------------------- | --------- | ------------------------------------ | ---------------------------------------------------------------------------------------- |
| `GATEWAY_PORT`                   | no        | `8080`                              | bind port                                                                              |
| `GRPC_ADDR`                      | no        | `http://127.0.0.1:44020`            | parser_app / parser_grpc_server endpoint                                               |
| `X402_PROFILE`                   | no        | `local`                             | one of `local`, `payai`, `custom`                                                      |
| `X402_FACILITATOR_URL`           | depends   | profile-default                     | overrides per-profile default                                                          |
| `X402_FACILITATOR_TIMEOUT_SECS`  | no        | `5`                                  | facilitator HTTP timeout                                                               |
| `X402_NETWORK`                   | no        | profile-default                     | `base-sepolia`, `base`, `solana`, `solana-devnet`                                      |
| `X402_PAYTO`                     | depends   | burn address for `local`            | EVM `0x...` or Solana base58                                                            |
| `X402_PRICE_TAGS_JSON`           | no        | seeded from profile + `X402_NETWORK` | full multi-tag override; see the JSON shape in `x402_config.rs`                        |
| `TVC_DEMO_PINNED_PUBKEY_HEX`     | **yes** (non-local) | --                         | pinned enclave pubkey, hex                                                             |
| `TVC_DEMO_PINNED_PUBKEY_FILE`    | no        | --                                   | alternative to `_HEX`: file holding the hex                                            |
| `GATEWAY_AUTH_BEARER_TOKEN`      | no        | --                                   | optional shared-bearer-token gate. When set, every route except `/health` requires `Authorization: Bearer <this-value>` or returns 401. Mutually exclusive with `_FILE`. |
| `GATEWAY_AUTH_BEARER_FILE`       | no        | --                                   | path to a file containing the bearer token (whitespace-trimmed). Preferred for Cloud Run / k8s secret-volume mounts. Mutually exclusive with `_TOKEN`. |

The bearer-token gate is a weak shared-secret intended to keep random
crawlers off the endpoint while AI-agent callers (which can set arbitrary
HTTP headers but can't easily mint per-caller identity tokens) can still
reach the x402 settlement layer. `/health` is intentionally excluded so
operators / orchestrators can probe liveness without sharing the token.

### Profiles

- `local` -- `X402_FACILITATOR_URL` defaults to `http://127.0.0.1:8090`
  (a locally-run mock facilitator). When `X402_NETWORK` is unset (the
  default), the gateway seeds both a `base-sepolia` EVM tag (burn payTo)
  and a `solana-devnet` tag (System-program burn payTo). Designed for
  offline dev.
- `payai` -- facilitator defaults to `https://facilitator.payai.network`.
  `X402_NETWORK` defaults to `base`; set it to `solana-devnet` for the
  devnet flow.
- `custom` -- bring your own facilitator URL and price tags via env.

The price tags configured at startup are static for the life of the
process: the v2 route always advertises the same network(s) regardless
of the parse request's `chain` field. Deriving the advertised network
from the request's chain is a follow-up (it needs the request body
before the 402 decision, which is a hand-rolled handler rather than the
generic x402-axum middleware layer used here).

### Network egress requirement

The `payai` profile requires outbound HTTPS to
`facilitator.payai.network` from wherever the gateway runs. In TVC
deployments the gateway runs on the host VM (outside the enclave); the
enclave-host networking already provides egress for Turnkey integrations.

## Tests

```sh
cd src && cargo test -p parser_gateway
```

Unit tests only in this crate: `attestation.rs` (pinned-pubkey
verification), `auth.rs` (bearer-token gate), `x402_config.rs` (env/profile
parsing), and `handlers/parse.rs` (Turnkey envelope wire-shape parity,
including the `bootProof` contract from issue #337). None of them need a
running facilitator or network access.
