# x402 demo playbook — local devnet, then live TVC

A copy-paste playbook to validate the x402-gated `/visualsign/api/v2/parse`
end to end, twice:

- **Part 1** — local stagex-built containers + real payai facilitator +
  real Solana devnet. The whole stack runs on your laptop; the only thing
  off-machine is the facilitator + Solana RPC.
- **Part 2** — live TVC-deployed gateway, same payment client.

If you only have 10 minutes, skip to Part 1's "tl;dr".

---

## Prerequisites

Install once:

```sh
# Rust toolchain (workspace pins 2024 edition)
rustup toolchain install nightly --component rustfmt clippy

# Docker + buildx (for stagex images AND the Solana CLI wrapper below)
docker --version
docker buildx version

# Node 20+ (for the TS x402 client)
node --version  # must be >= 20
```

**Solana CLI via Docker** — no host install required. Define this once in
your shell (or drop it into `~/.bashrc` / `~/.zshrc`):

> Image choice: we use `solanalabs/solana:v1.18.26` because Anza (the
> renamed Solana Labs org) does not publish an official CLI image, and
> the four client commands this playbook uses (`solana balance`,
> `solana airdrop`, `solana config`, `spl-token balance`) are wire-
> compatible across v1.18 → v3.x. The image was last pushed ~April 2024
> and is stable for client-side RPC calls. If you'd rather pin a fresher
> community Agave image (e.g. `andreaskasper/solana` or
> `dysnix/docker-agave`), replace the image reference in the helpers
> below — the rest of the playbook is unchanged.

```sh
# Persist the Solana config dir on the host so `solana config set`,
# generated keypairs, and the RPC URL survive between invocations.
mkdir -p "$HOME/.config/solana"

solana() {
  docker run --rm -i \
    -v "$HOME/.config/solana:/root/.config/solana" \
    -v "$PWD:/work" -w /work \
    solanalabs/solana:v1.18.26 solana "$@"
}

spl-token() {
  docker run --rm -i \
    -v "$HOME/.config/solana:/root/.config/solana" \
    -v "$PWD:/work" -w /work \
    solanalabs/solana:v1.18.26 spl-token "$@"
}

# First-time setup
docker pull solanalabs/solana:v1.18.26
solana --version
solana config set --url https://api.devnet.solana.com
```

This image bundles `solana`, `solana-keygen`, and `spl-token`. The `-v
$PWD:/work` mount lets commands read/write files in your current
directory (useful for `solana-keygen new -o ./key.json`).

> macOS / Windows note: drop the `-i` flag if you hit "the input device is
> not a TTY" errors; or replace `-i` with `-it` for interactive prompts.

Repo:

```sh
git clone git@github.com:anchorageoss/visualsign-parser.git
cd visualsign-parser
git checkout spec/x402-gated-http-api
```

---

## Part 1 — local stack, real payai, real Solana devnet

### tl;dr (assumes wallet already funded)

```sh
make non-oci-docker-images                                  # build stagex images
cd scripts && npm install && cd ..                          # one-time TS deps
export X402_PAYTO=x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
export TVC_DEMO_PINNED_PUBKEY_HEX=$(make print-tvc-pubkey-hex)
make dev-up-payai
TVC_DEMO_PINNED_PUBKEY_HEX=$TVC_DEMO_PINNED_PUBKEY_HEX \
  npx --prefix scripts tsx scripts/x402-solana-devnet-demo.ts
make dev-down
```

If anything's surprising, walk through the explicit steps below.

### Step 1 — Build the stagex container images

```sh
make non-oci-docker-images
```

This produces four images locally, all built from `stagex/pallet-rust:1.88.0`
with `--network=none` (no transitive deps at build time):

- `anchorageoss-visualsign-parser/parser_app`
- `anchorageoss-visualsign-parser/parser_gateway`
- `anchorageoss-visualsign-parser/parser_grpc_server`
- `anchorageoss-visualsign-parser/mock_facilitator`

Verify:

```sh
docker image ls | grep anchorageoss-visualsign-parser
```

You should see all four. Build takes ~5 min cold, ~30 sec warm.

### Step 2 — Set the pinned TVC verifier pubkey

The gateway must be told the enclave's expected ephemeral public key at
launch. For local-dev the fixture keypair is committed to the repo and
the *public* half is right there in `fixtures/ephemeral.pub` — just
`cat` it into the env var:

```sh
export TVC_DEMO_PINNED_PUBKEY_HEX=$(cat src/integration/fixtures/ephemeral.pub)
```

This is the same value `parser_grpc_server` will sign every parse
response with, so the gateway's verification will pass.

In production TVC, the enclave's `parser_app` is provisioned by Turnkey
with a *different* ephemeral key and exposes its public half through the
attested boot record. Part 2 walks through how to read that and pin it.

### Step 3 — Fund the reproducible test wallet

The seed in `src/integration/fixtures/devnet/wallet.seed` (non-secret,
devnet only) derives a fixed Solana address. Today that's:

```
x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
```

Check the current balance (the `solana` + `spl-token` shell functions
from Prerequisites delegate to the Docker image):

```sh
ADDR=x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
USDC_DEVNET_MINT=4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU

solana balance "$ADDR"
spl-token balance --owner "$ADDR" "$USDC_DEVNET_MINT" 2>/dev/null \
  || echo "no USDC account yet"
```

Top up if needed:

```sh
# Devnet SOL (rate-limited; retry if it fails)
solana airdrop 2 "$ADDR"

# Devnet USDC: open https://faucet.circle.com in a browser, paste $ADDR,
# pick "Solana Devnet", request. Takes ~30 s to land. (CLI airdrop is not
# available for USDC — Circle's faucet is the canonical path.)
```

You need at least **0.05 SOL** and **1.00 USDC** in atomic units
(`1_000_000`).

### Step 4 — Bring up the local stack against payai

The receiver in this demo is the wallet itself (self-transfer), so any
USDC moved goes back to the same account. Override with a different
`X402_PAYTO` if you want a real seller.

```sh
export X402_PAYTO="$ADDR"
export TVC_DEMO_PINNED_PUBKEY_HEX     # already exported in Step 2
make dev-up-payai
```

The compose file pulls in `parser_grpc_server` (gRPC backend) and
`parser_gateway` (HTTP, x402). The gateway probes
`https://facilitator.payai.network/supported` at startup; you should see
in the logs:

```
x402 facilitator probe OK
x402 attestation: pinned TVC pubkey 04716208..ed68bd57
parser_gateway dev listening on 0.0.0.0:8080
```

Confirm the 402 challenge directly:

```sh
curl -s -i -X POST http://127.0.0.1:8080/visualsign/api/v2/parse \
  -H 'content-type: application/json' \
  -d '{"request":{"unsigned_payload":"0xdeadbeef","chain":"CHAIN_ETHEREUM"}}' \
  | head -3
```

Expected: `HTTP/1.1 402 Payment Required` + a `payment-required` header
whose base64-JSON includes an entry with network
`solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1` (the CAIP-2 form of Solana
devnet that payai emits). To peek inside:

```sh
HDR=$(curl -s -i -X POST http://127.0.0.1:8080/visualsign/api/v2/parse \
  -H 'content-type: application/json' \
  -d '{"request":{"unsigned_payload":"0xdeadbeef","chain":"CHAIN_ETHEREUM"}}' \
  | awk -F': ' 'tolower($1)=="payment-required"{print $2}' | tr -d '\r')
echo "$HDR" | base64 -d | python3 -m json.tool
```

### Step 5 — Run the TS demo client to pay & parse

```sh
cd scripts
npm install            # one-time
cd ..
export GATEWAY_URL=http://127.0.0.1:8080
export RPC_URL=https://api.devnet.solana.com
node --experimental-strip-types --no-warnings scripts/x402-solana-devnet-demo.ts
```

(`tsx` works too, but Node 22.6+ strips TS types natively — no
build/transpile step needed.)

What you should see:

```
-- Wallet -----------------------------------------------------------
buyer address : x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
buyer balance : 5.0000 SOL on devnet

-- x402 client (payai/x402-solana) ----------------------------------
client constructed; making paid request...

-- Paid POST /visualsign/api/v2/parse -------------------------------
[x402-solana] Making initial request to: http://127.0.0.1:8080/visualsign/api/v2/parse
[x402-solana] Initial response status: 402
[x402-solana] Got 402, parsing payment requirements...
[x402-solana] Creating signed transaction...
[x402-solana] Transaction signed successfully
[x402-solana] Retrying request with PAYMENT-SIGNATURE header...
[x402-solana] Retry response status: 200
status: 200
settlement: {
  "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
  "payer": "x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW",
  "success": true,
  "transaction": "<base58 solana tx signature, ~88 chars>"
}

-- Independent P256 verification ------------------------------------
response signature verifies against pinned TVC pubkey ✓

-- Done -------------------------------------------------------------
payload bytes: 734
```

Three independent things just happened:

1. **payai settled a real USDC transfer on Solana devnet.** Paste the
   `settlement.transaction` value into
   `https://explorer.solana.com/tx/<signature>?cluster=devnet` to see
   it on chain.
2. **The gateway returned 200,** meaning its server-side P256
   verification of the parse response against the pinned TVC pubkey
   passed. A 502 here would mean settlement skipped (no charge).
3. **The TS demo independently re-verified the response signature**
   using `@noble/curves/p256` against the same pinned pubkey, proving
   you don't have to trust the gateway's word — any consumer can run
   the same check.

### Step 6 — Watch the gateway logs

In another terminal:

```sh
make dev-logs
```

You'll see one of two flows per request:

- `attestation: pinned TVC pubkey …` at startup, then quiet 200s
- `attestation verification failed: …` + the 502 — if you ever boot the
  gateway with a wrong `TVC_DEMO_PINNED_PUBKEY_HEX`, this is what
  prevents payment for an unattested response.

### Step 7 — Tear down

```sh
make dev-down
```

### Common failures (Part 1)

| Symptom | Cause | Fix |
| --- | --- | --- |
| `WARNING: x402 disabled; facilitator probe failed` | No egress to `facilitator.payai.network` | Check VPN / corp proxy; the v2 route stays unmounted otherwise |
| `FATAL: TVC_DEMO_PINNED_PUBKEY_HEX … required for X402_PROFILE=payai` | Forgot to export the pubkey | See Step 2 |
| Demo aborts with `paid request failed: 402` | Wallet underfunded on devnet (USDC or SOL) | Top up via faucet.circle.com / faucet.solana.com |
| `attestation verification failed: public key mismatch` on every request | Wrong pubkey pinned (env stale) | Re-export `TVC_DEMO_PINNED_PUBKEY_HEX` from `fixtures/ephemeral.pub` |
| Demo throws `independent P256 verification FAILED` | `TVC_DEMO_PINNED_PUBKEY_HEX` set differently for the gateway vs the demo | Use the same value in both shells |
| Container immediately exits with `syntax error: unterminated quoted string` | You replaced `entrypoint:` with `command:` in compose | Restore `entrypoint: ["/binary"]` — stagex/core-busybox sets ENTRYPOINT=`/bin/sh` |
| Gateway crashes at startup with `No CA certificates were loaded from the system` | Building from a Containerfile that didn't COPY the CA bundle | Pull CA certs from `stagex/core-ca-certificates` into `/etc/ssl/certs/` (see `images/parser_gateway/Containerfile`) |
| Demo hangs on "Sign X-PAYMENT" | Solana devnet RPC slow / blockhash fetch timeout | Switch `RPC_URL` to your own RPC endpoint |

---

## Part 2 — live TVC deployment

Once Part 1 is green you trust the gateway+client wire format. Part 2
just swaps the image source from your local docker daemon to a TVC-pinned
GHCR digest, and uses an enclave-provisioned ephemeral key instead of the
local fixture.

### Step 1 — Find the published digests

After a release build, `.github/workflows/stagex.yml` writes a TVC
deployment block into the GitHub release notes for both `parser_app`
and `parser_gateway`. Open the release on GitHub and copy:

- `parser_app` pinned URL: `ghcr.io/anchorageoss/parser_app:vX.Y.Z@sha256:<A>`
- `parser_app` expected executable digest: `sha256:<a>`
- `parser_gateway` pinned URL: `ghcr.io/anchorageoss/parser_gateway:vX.Y.Z@sha256:<B>`
- `parser_gateway` expected executable digest: `sha256:<b>`

The two digests are the **trust pair**: `parser_app` is what runs inside
the enclave and signs; `parser_gateway` is the host-side binary that
verifies + handles x402.

### Step 2 — Deploy `parser_app` to TVC

Same workflow you use for the non-x402 parser deploy:

```sh
tvc deploy init -o tvc-deploy.json
# Edit tvc-deploy.json to set:
#   "pivotContainerImageUrl": "ghcr.io/anchorageoss/parser_app:vX.Y.Z@sha256:<A>"
#   "pivotPath": "/parser_app"
#   "expectedPivotDigest": "sha256:<a>"
#   "qosVersion": "v2026.2.6"
#   "appId": "<your env-specific app id>"
tvc deploy create tvc-deploy.json
```

Once the deploy reaches "running", **read back the enclave's ephemeral
public key** from the TVC console or API. This is the value you pin
into the gateway as `TVC_DEMO_PINNED_PUBKEY_HEX`.

In a typical Turnkey TVC deploy this surfaces as a field on the deployed
app's attested boot record (the value `parser_app` writes when it loads
its provisioned ephemeral key). Save it:

```sh
export TVC_ENCLAVE_PUBKEY=<260-hex-char string from TVC console>
```

### Step 3 — Run `parser_gateway` against the live enclave

You have two options:

#### 3a. Run the gateway locally against the live enclave's gRPC

Use this when you want to test from your laptop without committing to a
hosted gateway yet. The gateway runs as a stagex container on your host,
but `GRPC_ADDR` points at the enclave-fronted gRPC endpoint Turnkey
exposes for your deploy.

```sh
# Edit compose.payai.yml: change the parser_gateway service's
# `image:` line to the GHCR digest from Step 1:
#
#   image: ghcr.io/anchorageoss/parser_gateway:vX.Y.Z@sha256:<B>
#
# Remove the local parser_grpc_server service (you're pointing at the
# enclave instead). Set GRPC_ADDR to the enclave URL.

export X402_PAYTO=<your real receiver, devnet or mainnet>
export TVC_DEMO_PINNED_PUBKEY_HEX="$TVC_ENCLAVE_PUBKEY"
docker compose -f compose.payai.yml up
```

#### 3b. Deploy the gateway as a sidecar on the TVC host

The production layout. The gateway runs alongside the enclave on the
same TVC-managed host VM, with the enclave's gRPC exposed only on
localhost.

The deploy mechanism is environment-specific (Turnkey ops, helm chart,
k8s manifest, etc.). Whatever it is, the gateway container needs the
following env, set by the TVC platform at launch:

```
GRPC_ADDR=http://127.0.0.1:44020
X402_PROFILE=payai
X402_NETWORK=solana-devnet           # or solana on mainnet
X402_FACILITATOR_URL=https://facilitator.payai.network
X402_FACILITATOR_TIMEOUT_SECS=10
X402_PAYTO=<receiver pubkey>
TVC_DEMO_PINNED_PUBKEY_HEX=<from Step 2; read from enclave attested boot>
```

The image to pull is `ghcr.io/anchorageoss/parser_gateway:vX.Y.Z@sha256:<B>`
from Step 1. The hash pin matters: the verifier-key logic that consumes
the enclave's signed payload lives in this exact build, and replacing it
with an unpinned `:latest` defeats the trust pair.

### Step 4 — Probe the live gateway

```sh
GATEWAY=https://<your-gateway-host>
curl -i -X POST "$GATEWAY/visualsign/api/v2/parse" \
  -H 'content-type: application/json' \
  -d '{"request":{"unsigned_payload":"0xdeadbeef","chain":"CHAIN_ETHEREUM"}}'
```

Expect `402 Payment Required` with `payment-required` listing a
`solana-devnet` entry (or `solana` on mainnet).

### Step 5 — Pay against the live gateway

Use the same TS client; just point it at the live URL and the live
pubkey:

```sh
cd scripts
GATEWAY_URL=https://<your-gateway-host> \
RPC_URL=https://api.devnet.solana.com \
TVC_DEMO_PINNED_PUBKEY_HEX="$TVC_ENCLAVE_PUBKEY" \
  npx tsx x402-solana-devnet-demo.ts
```

Same flow as Part 1 Step 5. The `Independent P256 verification ✓` line
is now verifying the **live enclave's** signature using the public key
read from the **live attested boot record** — i.e., it asserts the
end-to-end trust pair holds.

### Step 6 — Watch a tamper attempt fail (optional sanity check)

Set the pubkey env to a single-bit-wrong value and re-run the client:

```sh
WRONG=$(echo "$TVC_ENCLAVE_PUBKEY" | sed 's/.$/0/' )
GATEWAY_URL=https://<your-gateway-host> \
TVC_DEMO_PINNED_PUBKEY_HEX="$WRONG" \
  npx tsx x402-solana-devnet-demo.ts
```

The script's independent verification fails. (The gateway itself still
succeeds — it doesn't know what the client pinned. The point is that
**any consumer** can repeat the same check the gateway does, with no
trust in the gateway's word.)

### Common failures (Part 2)

| Symptom | Cause | Fix |
| --- | --- | --- |
| `FATAL: TVC_DEMO_PINNED_PUBKEY_HEX … required` at gateway boot | Env not propagated | Check your TVC deploy manifest's env block |
| Gateway returns 502 on every request | `TVC_DEMO_PINNED_PUBKEY_HEX` doesn't match the live enclave's ephemeral key | Re-read the pubkey from the enclave's attested boot record after re-deploy; rotating the parser_app deploy generates a new ephemeral key |
| Gateway 200 but client `Independent P256 verification FAILED` | You pinned the wrong pubkey *only on the client* (gateway has the right one) | Re-export `TVC_DEMO_PINNED_PUBKEY_HEX` for the client |
| Client gets `402` even after sending X-PAYMENT | x402-axum middleware rejected the header (malformed amount, wrong network, expired blockhash) | Inspect gateway logs; the most common cause is a stale blockhash — retry within ~90 s of building the tx |

---

## Promote devnet → mainnet (when you're ready)

Two changes once the devnet rehearsal is clean:

1. Set `X402_NETWORK=solana` (not `solana-devnet`) in the gateway env.
2. Set `RPC_URL=https://api.mainnet-beta.solana.com` in the TS client
   and fund the receiver wallet with **real USDC** (`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`).

Same trust pair, same flow. The gateway code doesn't change.

---

## Part 4 — TVC-enforced payment (stacked PR #304)

Plan v3 lands on a separate branch (`spec/x402-tvc-enforced`, PR #304)
that replaces the demo response-attestation verifier with parser-app-side
payment enforcement. The enclave refuses to process any parse request
that doesn't carry a valid gateway-signed `VerifiedPaymentMarker`.

What changes vs Parts 1–2:

- The gateway has its own P256 signing identity (`GATEWAY_SIGNING_KEY_FILE`).
  It hand-rolls the call order `verify → settle → sign VPM → forward` so
  the post-settle txid is committed to in the marker before parser_app
  sees the request.
- `parser_app` pins the gateway's public key via `GATEWAY_SIGNING_PUBKEY_HEX`.
  Mismatch → `FailedPrecondition` → gateway translates to HTTP 402.
- The v1 open route is unmounted when TVC enforcement is on (the policy
  is global at parser_app, so an open route would 402 everything).
- The demo `TVC_DEMO_PINNED_PUBKEY_HEX` becomes optional; the new
  GATEWAY pubkey is the actual trust anchor.

### Step 1 — Mint a gateway signing key

```sh
cargo run -p parser_gateway --bin gateway_keygen -- /tmp/gateway_signer.json
# Prints: GATEWAY_SIGNING_PUBKEY_HEX=<260-char-hex>
```

Save the printed hex — it's what parser_app will pin.

In production, the same JSON file (or a JSON blob with `{private, public}`
fields) is mounted via Cloud Run Secret Manager / k8s Secret volumes at
the configured `GATEWAY_SIGNING_KEY_FILE` path.

### Step 2 — Bring the stack up

```sh
git checkout spec/x402-tvc-enforced
make non-oci-docker-images          # rebuilds stagex images with the
                                     # TVC-enforced flow

export GATEWAY_SIGNING_KEY_FILE_HOST=/tmp/gateway_signer.json
export GATEWAY_SIGNING_PUBKEY_HEX=$(jq -r .public /tmp/gateway_signer.json)
export X402_PAYTO=x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW

docker compose -f compose.payai.yml up -d
docker logs visualsign-parser-parser_gateway-1 | tail
# Expect:
#   gateway signer loaded; pubkey 04xxxxxx...
#   x402 facilitator probe OK
#   parser_gateway dev listening on 0.0.0.0:8080
```

### Step 3 — Pay + parse

Same TS demo:

```sh
GATEWAY_URL=http://127.0.0.1:8080 RPC_URL=https://api.devnet.solana.com \
  node --experimental-strip-types --no-warnings scripts/x402-solana-devnet-demo.ts
```

Expected output (the parser-app verification line is the new bit):

```
[x402-solana] Retry response status: 200
status: 200
settlement: {
  "network":     "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
  "payer":       "x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW",
  "success":     true,
  "transaction": "<base58 solana sig>"
}
```

In `docker logs visualsign-parser-parser_grpc_server-1` you should see
zero rejection log lines (the parser silently accepts a valid VPM).

### Step 4 — Tamper sanity-check

```sh
make dev-down
# Pin parser_app to a DIFFERENT pubkey than the gateway is signing with.
docker run --rm -v /tmp:/tmp parser_gateway_keygen /tmp/wrong_signer.json \
  > /dev/null  # or run cargo gateway_keygen
WRONG=$(jq -r .public /tmp/wrong_signer.json)
GATEWAY_SIGNING_PUBKEY_HEX="$WRONG" \
GATEWAY_SIGNING_KEY_FILE_HOST=/tmp/gateway_signer.json \
X402_PAYTO=x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW \
  docker compose -f compose.payai.yml up -d

GATEWAY_URL=http://127.0.0.1:8080 RPC_URL=https://api.devnet.solana.com \
  node --experimental-strip-types --no-warnings scripts/x402-solana-devnet-demo.ts || true
# Expect: paid request failed: 402 - "payment required"
docker logs visualsign-parser-parser_grpc_server-1 | grep 'gateway_pubkey_hex'
# Expect: payment marker gateway_pubkey_hex does not match pinned key
```

Known quirk: payai still settles before parser_app rejects, so an
operator misconfiguration costs the buyer one demo payment per attempt.
v3.1 will tighten this by gating settle on a pre-validation step.

### What's still open in v3.0 (deferred to v3.1)

- Gateway pubkey is statically pinned; v3.1 will derive it dynamically
  from a chained gateway attestation.
- Payer Ed25519 sig on the Solana tx isn't re-verified inside parser_app;
  payai's `/verify` checks it before settle, and the VPM's
  `x_payment_hash` binds the marker to the exact X-PAYMENT body.
- Settle-then-reject quirk (above).

---

## Part 5 — Deploying to real Turnkey TVC

Verified against a deployed Turnkey TVC app whose pivot is `parser_app`
(gRPC, port 3000, `healthCheckType: TVC_HEALTH_CHECK_TYPE_GRPC`):

- **`*.turnkey.cloud` public ingress accepts HTTP only.** Cloudflare in
  front of the public domain rejects gRPC traffic with a 403 + `text/html`
  body, regardless of `content-type: application/grpc` or any auth
  header. We verified this from three transports (tonic + TLS, grpcurl,
  raw `curl --http2-prior-knowledge`); all hit the same Cloudflare WAF
  block. So a deployed TVC pivot whose only listener is gRPC cannot be
  reached by external callers — the bytes never reach the app.
- The `healthCheckType: TVC_HEALTH_CHECK_TYPE_GRPC` in the deploy config
  applies to Turnkey's internal-network health probes; it does NOT mean
  the public ingress speaks gRPC. To accept external calls, deploy a
  pivot that speaks HTTP and set `healthCheckType: TVC_HEALTH_CHECK_TYPE_HTTP`.

### `parser_http_server` — the HTTP-speaking pivot

New stagex image at `images/parser_http_server/Containerfile`. Built
identically to `parser_grpc_server` but exposes:

| Route | Behavior |
| --- | --- |
| `GET /health` | 200 OK. Maps to `TVC_HEALTH_CHECK_TYPE_HTTP`. |
| `POST /visualsign/api/v1/parse` | Open. Turnkey envelope JSON in, signed `parsedTransaction` JSON out. Same wire shape the Go `visualsign-turnkey-client` reference uses. |
| `POST /visualsign/api/v2/parse` | TVC-enforced if `GATEWAY_SIGNING_PUBKEY_HEX` is set (parser_app verifies a `VerifiedPaymentMarker` first), otherwise behaves like v1. |

Re-uses the existing `parser_app::routes::parse::parse` function — no
new parsing logic. The Turnkey wire-envelope types (`TurnkeyRequestWrapper`
etc.) moved to `host_primitives::turnkey` so both `parser_gateway` and
`parser_http_server` produce the exact same bytes.

### Deploy steps

1. Build + publish the image:

   ```sh
   make non-oci-docker-images          # builds locally
   # CI: push to ghcr.io/anchorageoss/parser_http_server on tagged release
   ```

   The CI release notes from `stagex.yml` will print a paste-ready
   `tvc deploy create` recipe with the pinned digest.

2. Create or update the TVC deployment config (`tvc deploy init`,
   then edit):

   ```json
   {
     "appId": "<your-tvc-app-id>",
     "qosVersion": "v2026.2.6",
     "pivotContainerImageUrl": "ghcr.io/anchorageoss/parser_http_server:vX.Y.Z@sha256:<digest>",
     "pivotPath": "/parser_http_server",
     "pivotArgs": [],
     "expectedPivotDigest": "sha256:<binary-digest-from-release-notes>",
     "debugMode": false,
     "healthCheckType": "TVC_HEALTH_CHECK_TYPE_HTTP",
     "healthCheckPort": 3000,
     "publicIngressPort": 3000
   }
   ```

   Critical fields vs the existing gRPC deployment:
   - `pivotContainerImageUrl` → `parser_http_server` (was `parser_app`)
   - `pivotPath` → `/parser_http_server` (was `/parser_app`)
   - `healthCheckType` → `TVC_HEALTH_CHECK_TYPE_HTTP` (was `_GRPC`)

3. `tvc deploy create tvc-deploy.json`

4. Approve the manifest (`tvc deploy approve`) — operator-side ceremony,
   same as today.

5. Once `tvc app status --app-id <id>` shows N/N replicas healthy:

   ```sh
   curl -X POST https://app-<your-app-id>.turnkey.cloud/visualsign/api/v1/parse \
     -H 'content-type: application/json' \
     -d '{"request":{"unsigned_payload":"0xf86c808504a817c800...","chain":"CHAIN_ETHEREUM"}}'
   ```

   Should return a Turnkey envelope with the parsed payload + ephemeral-key
   signature. No X-Stamp required (v1 is open).

### Turning on TVC-enforced payment (canonical deploy)

Once `parser_http_server` is live in TVC, deploying the v3 trust pair is
two side-by-side steps. The gateway sits outside TVC (Cloud Run /
k8s / wherever has egress), and addresses the enclave over HTTP because
that's the only transport `*.turnkey.cloud` ingress accepts.

```
                       Solana devnet                facilitator.payai.network
                            ▲                                ▲
                            │ on-chain settle                │ verify + settle
                            │                                │
   browser / SDK            │                                │
        │                   │                                │
        ▼                   │                                │
  ┌───────────────┐  HTTP  ┌─┴────────────────┐ HTTPS ┌──────┴────────────────────┐
  │ x402 client   │ ─────▶ │ parser_gateway   │ ────▶ │ parser_http_server (TVC)  │
  │ (TS demo /    │  402   │ (Cloud Run, etc.)│       │ ghcr…/parser_http_server  │
  │  custom)      │ ◀───── │  verify→settle→  │       │  GATEWAY_SIGNING_PUBKEY_  │
  └───────────────┘   200  │  sign VPM →      │       │  HEX pinned at boot       │
                           │  forward HTTP    │       └───────────────────────────┘
                           └──────────────────┘
```

Steps:

1. **Mint the gateway signing key:**

   ```sh
   cargo run -p parser_gateway --bin gateway_keygen -- /tmp/gateway_signer.json
   # Prints GATEWAY_SIGNING_PUBKEY_HEX=<260-char-hex>
   ```

2. **Deploy `parser_http_server` to TVC** with the public half pinned in
   its env. Add to the TVC deploy config's env block:

   ```json
   {
     "envVars": [
       { "name": "GATEWAY_SIGNING_PUBKEY_HEX",
         "value": "<260-char-hex from step 1>" }
     ]
   }
   ```

   After re-deploy, `parser_app` (linked into `parser_http_server`)
   rejects every parse request whose `payment_marker` doesn't carry a
   VPM signed by exactly this key.

3. **Deploy `parser_gateway` outside TVC** with `HTTP_BACKEND_URL`
   pointing at the TVC app URL:

   ```sh
   # Example: Cloud Run-style env block
   GATEWAY_PORT=8080
   X402_PROFILE=payai
   X402_NETWORK=solana-devnet
   X402_FACILITATOR_URL=https://facilitator.payai.network
   X402_PAYTO=<receiver pubkey>
   GATEWAY_SIGNING_KEY_FILE=/etc/secrets/gateway/signer.json
   HTTP_BACKEND_URL=https://app-<uuid>.turnkey.cloud
   ```

   When `HTTP_BACKEND_URL` is set, the v2 handler POSTs the
   verified+settled+VPM-signed parse to
   `${HTTP_BACKEND_URL}/visualsign/api/v2/parse` (Turnkey JSON envelope
   with `payment_marker_b64` carrying the borsh-encoded VPM) instead of
   calling parser_grpc_server over gRPC. The `GRPC_ADDR` env is ignored
   in this mode.

   The mock-facilitator path (`X402_PROFILE=local`) works identically;
   you can rehearse this whole topology locally by pointing
   `HTTP_BACKEND_URL` at a containerized `parser_http_server` before
   deploying to TVC.

4. **Smoke-test from the client:**

   ```sh
   GATEWAY_URL=https://<your-gateway-host> \
   RPC_URL=https://api.devnet.solana.com \
     node --experimental-strip-types --no-warnings scripts/x402-solana-devnet-demo.ts
   ```

   The 200 response now means: payai settled on-chain → gateway signed
   a VPM → TVC enclave verified the VPM signature + request_hash
   binding + pubkey pinning → enclave signed the parse response. Any
   failure short-circuits to 402 / 4xx before any payment is settled.

Validated end-to-end locally with `parser_gateway` (HTTP_BACKEND_URL
mode) → `parser_http_server` against real payai + Solana devnet — the
on-chain settle landed at
`2Q4vB1fQcJfyuW94YvjPKRYuoJgqWZtgSmkbcttJRGdT6FHQsNAypGVXEU6jTfxnTQUg9wpMq6shZzXxYBcgmuoR`.

### Diagnostic tools

- `scripts/turnkey-probe.ts` — minimal Node script that sends an
  X-Stamp-signed HTTP request to any Turnkey URL. Reads your API key
  from `~/.config/turnkey/keys/<name>.{private,public}`. Use it to poke
  at `api.turnkey.com` queries (list_tvc_apps, get_tvc_deployment, etc.)
  or at your deployed app's HTTP routes.

- `cargo run -p parser_gateway --bin tvc_probe -- <APP_URL> <ORG_ID>` —
  Rust + tonic + TLS + X-Stamp gRPC probe. Useful to confirm what we
  found about the gRPC public-ingress block. Returns Cloudflare 403 from
  any deployed TVC app today.
