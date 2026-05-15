# parser_gateway

HTTP gateway in front of `parser_app`'s gRPC service. Terminates client
requests, optionally gates `/visualsign/api/v2/parse` behind an x402
(HTTP 402 Payment Required) handshake, and verifies the TVC enclave's
signature on every parse response before returning it.

## Routes

| Method | Path                          | Gated by x402? | Notes                              |
| ------ | ----------------------------- | -------------- | ---------------------------------- |
| GET    | `/health`                     | no             | proxy to backend gRPC health       |
| POST   | `/visualsign/api/v1/parse`    | no             | legacy, open                       |
| POST   | `/visualsign/api/v2/parse`    | **yes**        | configured via env (see below)     |

The v2 route is only mounted if the configured x402 facilitator responds
to a `/supported` probe at startup. If the facilitator is unreachable the
gateway logs and continues serving v1 + health only.

## TVC attestation

Every successful v2 (and v1) parse response is signed by `parser_app`'s
ephemeral P256 keypair, provisioned into the enclave at boot. The gateway
verifies the signature against a **pinned** public key. On failure it
returns `502 Bad Gateway`; the x402 middleware's settle-on-success
contract then skips `/settle`, so an unattested response is never
charged to the payer.

The pinned pubkey is provided to the gateway as a launch argument by the
TVC stack. The value is `qos_hex::encode(P256Public::to_bytes())` — the
exact format `parser_app` emits in the wire signature's `publicKey` field.

```sh
# Set by TVC at boot (or via your local-dev compose file)
X402_TVC_VERIFIER_PUBKEY_HEX=<260 hex chars>
# Or, equivalently:
X402_TVC_VERIFIER_PUBKEY_FILE=/path/to/pubkey.hex
```

If neither is set:
- `X402_PROFILE=local`: the gateway logs a warning and skips attestation.
- otherwise: the gateway **exits with code 1** at startup (fail-closed).

## x402 configuration

All env vars are read at startup. Bad values fail-closed (gateway exits 1).

| Env var                          | Required? | Default                             | Meaning                                                                                |
| -------------------------------- | --------- | ----------------------------------- | -------------------------------------------------------------------------------------- |
| `GATEWAY_PORT`                   | no        | `8080`                              | bind port                                                                              |
| `GRPC_ADDR`                      | no        | `http://127.0.0.1:44020`            | parser_app / parser_grpc_server endpoint                                               |
| `X402_PROFILE`                   | no        | `local`                             | one of `local`, `payai`, `custom`                                                      |
| `X402_FACILITATOR_URL`           | depends   | profile-default                     | overrides per-profile default                                                          |
| `X402_FACILITATOR_TIMEOUT_SECS`  | no        | `5`                                 | facilitator HTTP timeout                                                               |
| `X402_NETWORK`                   | no        | profile-default                     | `base-sepolia`, `base`, `solana`, `solana-devnet`                                      |
| `X402_PAYTO`                     | depends   | burn address for `local`            | EVM `0x…` or Solana base58                                                             |
| `X402_PRICE_TAGS_JSON`           | no        | seeded from profile + `X402_NETWORK` | full multi-tag override; see the JSON shape in `x402_config.rs`                        |
| `X402_TVC_VERIFIER_PUBKEY_HEX`   | **yes** (non-local) | —                          | pinned enclave pubkey, hex                                                             |
| `X402_TVC_VERIFIER_PUBKEY_FILE`  | no        | —                                   | alternative to `_HEX`: file holding the hex                                            |

### Profiles

- `local` — `X402_FACILITATOR_URL` defaults to `http://127.0.0.1:8090`
  (the bundled `mock_facilitator`). `X402_NETWORK` defaults to
  `base-sepolia`. Designed for offline dev.
- `payai` — facilitator defaults to `https://facilitator.payai.network`.
  `X402_NETWORK` defaults to `base`; set it to `solana-devnet` for the
  devnet flow.
- `custom` — bring your own facilitator URL and price tags via env.

### Network egress requirement

The `payai` profile requires outbound HTTPS to
`facilitator.payai.network` from wherever the gateway runs. In TVC
deployments the gateway runs on the host VM (outside the enclave); the
enclave-host networking already provides egress for Turnkey integrations.

## Local-dev stacks (containerized)

Two thin `docker-compose` files at the repo root consume the same
stagex-built OCI images that ship to GHCR in production. Build them
first with `make non-oci-docker-images`.

```sh
# Offline / fully self-contained — uses bundled mock_facilitator.
make dev-up-mock

# Real payai facilitator on Solana devnet.
export X402_PAYTO=<your devnet receiver pubkey>
export X402_TVC_VERIFIER_PUBKEY_HEX=<260-char hex from parser_app>
make dev-up-payai

# Tear down either stack.
make dev-down
```

To target a TVC-deployed gateway image instead of a local build, edit
`compose.payai.yml` and replace
`image: anchorageoss-visualsign-parser/parser_gateway:latest` with the
GHCR digest from the release notes, e.g.
`image: ghcr.io/anchorageoss/parser_gateway:v0.1.2@sha256:<digest>`.

## End-to-end demo (TypeScript)

Drives the gated endpoint with a real x402 payment against payai +
Solana devnet:

```sh
cd scripts
npm install
GATEWAY_URL=http://127.0.0.1:8080 \
X402_TVC_VERIFIER_PUBKEY_HEX=<260-char hex> \
npx tsx x402-solana-devnet-demo.ts
```

Uses the reproducible buyer wallet derived from
`src/integration/fixtures/devnet/wallet.seed`. The current address —
`x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW` — must be funded on
devnet with SOL + USDC before running. See
`src/integration/fixtures/devnet/README.md` for faucet links.

## Integration tests

```sh
# Always-on (offline, mock facilitator) — 6 paths including signature
# tamper detection.
make -C src test

# Gated devnet E2E (real payai + Solana devnet). Requires the
# reproducible buyer wallet to be funded.
X402_E2E=1 cargo test -p integration --test x402_payai_devnet_test -- --ignored
```
