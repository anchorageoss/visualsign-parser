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
export X402_TVC_VERIFIER_PUBKEY_HEX=$(make print-tvc-pubkey-hex)
make dev-up-payai
X402_TVC_VERIFIER_PUBKEY_HEX=$X402_TVC_VERIFIER_PUBKEY_HEX \
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

### Step 2 — Compute the pinned TVC verifier pubkey

The gateway must be told the enclave's expected ephemeral public key at
launch. For local-dev we re-use the test fixture key:

```sh
cd src
cargo run -q --bin print_ephemeral_pubkey 2>/dev/null || \
  cargo test -p integration --test x402_payai_devnet_test load_devnet_keypair_round_trips -- --nocapture 2>&1 \
  | grep -E '^\[fixture\]' || true

# Easier: derive the hex inline (matches what parser_grpc_server will sign with)
EPH_HEX=$(cargo run --quiet -p integration --example print_tvc_pubkey 2>/dev/null \
  || python3 - <<'PY'
# Fallback: extract from fixtures/ephemeral.pub if present
import sys, pathlib
p = pathlib.Path("integration/fixtures/ephemeral.pub")
print(p.read_text().strip())
PY
)
echo "TVC pubkey hex: $EPH_HEX"
cd ..
export X402_TVC_VERIFIER_PUBKEY_HEX="$EPH_HEX"
```

If you don't have a quick way to print the hex, run the gateway once with
`X402_TVC_VERIFIER_PUBKEY_HEX=00...00` (any wrong value). It will start,
serve a request, log `attestation verification failed: public key
mismatch: ... response key <REAL HEX>`. Copy the real hex, kill the
gateway, and re-export.

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
export X402_TVC_VERIFIER_PUBKEY_HEX     # already exported in Step 2
make dev-up-payai
```

The compose file pulls in `parser_grpc_server` (gRPC backend) and
`parser_gateway` (HTTP, x402). The gateway probes
`https://facilitator.payai.network/supported` at startup; you should see
in the logs:

```
x402 facilitator probe OK
x402 attestation: pinned TVC pubkey 04abc123…<last 8>
parser_gateway v… listening on 0.0.0.0:8080
```

Confirm the 402 challenge directly:

```sh
curl -i -X POST http://127.0.0.1:8080/visualsign/api/v2/parse \
  -H 'content-type: application/json' \
  -d '{"request":{"unsigned_payload":"0xdeadbeef","chain":"CHAIN_ETHEREUM"}}' \
  | head -20
```

Expected: `HTTP/1.1 402 Payment Required` + a `payment-required` header
whose base64-JSON includes a `solana-devnet` entry.

### Step 5 — Run the TS demo client to pay & parse

```sh
cd scripts
npm install      # one-time
export GATEWAY_URL=http://127.0.0.1:8080
export RPC_URL=https://api.devnet.solana.com
npx tsx x402-solana-devnet-demo.ts
```

What you should see:

```
── Wallet ─────────────────────────────────…
buyer address : x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
buyer balance : 1.9234 SOL on devnet
── Probe 402 ──────────────────────────────…
accepts: solana-devnet
price: 1000 atoms USDC -> x2iWww6X… on solana-devnet
── Sign X-PAYMENT ─────────────────────────…
header length: 1842 chars
── Paid request ───────────────────────────…
status: 200
X-PAYMENT-RESPONSE: eyJzdWNjZX…
signature pubkey: 04abc123…
── Independent P256 verification ──────────…
response signature verifies against pinned TVC pubkey ✓
── Done ───────────────────────────────────…
payload bytes: 1284
```

The `solana-devnet ✓` line is the assertion that closes the loop: the
gateway returned a 200 *and* the response signature verifies against the
pinned enclave pubkey using `@noble/curves/p256` (cross-impl with the
Rust verifier).

### Step 6 — Watch the gateway logs

In another terminal:

```sh
make dev-logs
```

You'll see one of two flows per request:

- `(verified)` and `x402 settled in <ms>` for a happy path
- `attestation verification failed: …` + the 502 — if you ever boot the
  gateway with a wrong `X402_TVC_VERIFIER_PUBKEY_HEX`, this is what
  prevents payment for an unattested response.

### Step 7 — Tear down

```sh
make dev-down
```

### Run the gated devnet test from cargo (optional)

```sh
cd src
X402_E2E=1 cargo test -p integration --test x402_payai_devnet_test \
  -- --ignored --nocapture
```

This boots its own stack (the same binaries, run natively rather than in
containers) and runs the same end-to-end flow from Rust. Useful as a
regression gate in a CI job labeled `e2e-devnet`.

### Common failures (Part 1)

| Symptom | Cause | Fix |
| --- | --- | --- |
| `WARNING: x402 disabled; facilitator probe failed` | No egress to `facilitator.payai.network` | Check VPN / corp proxy; the v2 route stays unmounted otherwise |
| `FATAL: X402_TVC_VERIFIER_PUBKEY_HEX … required for X402_PROFILE=payai` | Forgot to export the pubkey | See Step 2 |
| `buyer ATA … has only N USDC atoms` (panic from cargo test) | Wallet underfunded | faucet.circle.com |
| `attestation verification failed: public key mismatch` on every request | Wrong pubkey pinned (env stale, or the parser_grpc_server image rebuilt with a different ephemeral fixture) | Re-derive the hex from `fixtures/ephemeral.pub` |
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
into the gateway as `X402_TVC_VERIFIER_PUBKEY_HEX`.

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
export X402_TVC_VERIFIER_PUBKEY_HEX="$TVC_ENCLAVE_PUBKEY"
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
X402_TVC_VERIFIER_PUBKEY_HEX=<from Step 2; read from enclave attested boot>
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
X402_TVC_VERIFIER_PUBKEY_HEX="$TVC_ENCLAVE_PUBKEY" \
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
X402_TVC_VERIFIER_PUBKEY_HEX="$WRONG" \
  npx tsx x402-solana-devnet-demo.ts
```

The script's independent verification fails. (The gateway itself still
succeeds — it doesn't know what the client pinned. The point is that
**any consumer** can repeat the same check the gateway does, with no
trust in the gateway's word.)

### Common failures (Part 2)

| Symptom | Cause | Fix |
| --- | --- | --- |
| `FATAL: X402_TVC_VERIFIER_PUBKEY_HEX … required` at gateway boot | Env not propagated | Check your TVC deploy manifest's env block |
| Gateway returns 502 on every request | `X402_TVC_VERIFIER_PUBKEY_HEX` doesn't match the live enclave's ephemeral key | Re-read the pubkey from the enclave's attested boot record after re-deploy; rotating the parser_app deploy generates a new ephemeral key |
| Gateway 200 but client `Independent P256 verification FAILED` | You pinned the wrong pubkey *only on the client* (gateway has the right one) | Re-export `X402_TVC_VERIFIER_PUBKEY_HEX` for the client |
| Client gets `402` even after sending X-PAYMENT | x402-axum middleware rejected the header (malformed amount, wrong network, expired blockhash) | Inspect gateway logs; the most common cause is a stale blockhash — retry within ~90 s of building the tx |

---

## Promote devnet → mainnet (when you're ready)

Two changes once the devnet rehearsal is clean:

1. Set `X402_NETWORK=solana` (not `solana-devnet`) in the gateway env.
2. Set `RPC_URL=https://api.mainnet-beta.solana.com` in the TS client
   and fund the receiver wallet with **real USDC** (`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`).

Same trust pair, same flow. The gateway code doesn't change.
