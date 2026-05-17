# parser_http_server on TVC — e2e validation, 2026-05-16

Status of the TVC-enforced x402 stack after today's debugging round.
Use this as a runbook for re-validating the next deployment.

## What's live

| Field | Value |
|---|---|
| App ID | `<APP_ID>` |
| Targeted deployment | `<DEPLOY_ID_LIVE>` |
| Image | `ghcr.io/anchorageoss/parser_http_server:pr-304@sha256:<IMAGE_MANIFEST_DIGEST>` |
| Executable digest | `<BINARY_SHA256>` |
| QOS version | `v2026.2.6` |
| Enclave ephemeral pubkey | `<ENCLAVE_EPHEMERAL_PREFIX>…` |
| Gateway signing pubkey (pinned in pivotArgs) | `<GATEWAY_SIGNING_PUBKEY_HEX>…` |
| Public URL | `https://app-<APP_ID>.turnkey.cloud` |
| Replicas | 3/3 healthy |

Gateway private key (matching pinned pubkey above) is at `~/.config/visualsign/gateway/private.key` (mode 600).

## Three fixes that made it work

Each one's a separate commit on `spec/x402-tvc-enforced`:

1. **`5147793` — CLI args replace env vars.** `tvc deploy create` has no env-injection mechanism, so the binary's startup config must arrive via `pivotArgs`. parser_http_server now takes `--port` and `--gateway-signing-pubkey-hex`; clap keeps env-var fallback so compose/integration tests still work.

2. **`703f43d` — chain feature flags.** `Cargo.toml` gets a `[features]` block forwarding to parser_app, Containerfile picks them up via `CHAIN_FEATURES`. Default still builds all five chains; `docker build --build-arg CHAIN_FEATURES=solana …` (or any subset) shrinks the image.

3. **`1171203` — build with `qos_core/vm`.** Without this feature, `qos_core::EPHEMERAL_KEY_FILE` compiled to the dev path `./local-enclave/qos.ephemeral.key`; the pivot panicked at startup trying to read it and the pod crash-looped to `0/3 healthy`. The new `vsock` feature in `parser_http_server/Cargo.toml` switches qos_core to the in-enclave path `/qos.ephemeral.key`; Containerfile passes it.

And cherry-picked from PR #306 (off main): `9e3dc1a` strips the `sha256:` prefix from `expectedPivotDigest` in `stagex.yml`'s TVC deployment block, which fixed `tvc deploy approve` failing on `OddLength at line 8 column 85`. PR #306 is the canonical fix for that.

## Smoke test: direct TVC pivot

Validates the deployed binary is up and serving — no gateway involved.

```bash
APP_URL="https://app-<APP_ID>.turnkey.cloud"

# 1. /health should be 200 with empty body
curl -sS -o /dev/null -w "/health: %{http_code}\n" "${APP_URL}/health"

# 2. v1 (open route) parses an Ethereum tx and signs the response with the
#    enclave's ephemeral key. The publicKey in the response should match the
#    Ephemeral Key in `tvc deploy provisioning-details`.
ETH_TX="0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

curl -sS -X POST "${APP_URL}/visualsign/api/v1/parse" \
  -H "Content-Type: application/json" \
  -d "{\"request\":{\"unsigned_payload\":\"${ETH_TX}\",\"chain\":\"CHAIN_ETHEREUM\"}}" \
  | jq '.response.parsedTransaction.signature.publicKey'

# 3. v2 without VPM should reject with "payment marker is required". This
#    confirms TVC enforcement is on (the --gateway-signing-pubkey-hex
#    pivotArg propagated correctly).
curl -sS -X POST "${APP_URL}/visualsign/api/v2/parse" \
  -H "Content-Type: application/json" \
  -d "{\"request\":{\"unsigned_payload\":\"${ETH_TX}\",\"chain\":\"CHAIN_ETHEREUM\"}}" \
  | jq '.error'
# Expected: "payment marker is required for this endpoint"
```

## Full x402 → gateway → TVC pivot loop

Validates the entire trust pair: gateway verifies + settles payment, signs a `VerifiedPaymentMarker` with our private key, the TVC pivot verifies the VPM signature against the pinned pubkey, parses, signs the response with the enclave ephemeral key.

Prereq: `parser_gateway` and `mock_facilitator` are built locally (`make -C src build` or `cargo build -p parser_gateway -p mock_facilitator`).

```bash
cd src

# Start mock_facilitator on :8090 (in-process settlement, never touches a chain).
nohup ./target/debug/mock_facilitator >/tmp/mock_fac.log 2>&1 < /dev/null &
disown
sleep 2
curl -sS -o /dev/null -w "mock_fac /supported: %{http_code}\n" http://127.0.0.1:8090/supported

# Start parser_gateway on :8080 pointed at the TVC backend.
# - GATEWAY_SIGNING_KEY_FILE holds the private half of the keypair whose public
#   half is pinned in the TVC pivot's --gateway-signing-pubkey-hex arg.
# - HTTP_BACKEND_URL is the *.turnkey.cloud URL (Cloudflare in front of the pivot).
# - GRPC_ADDR is still required even though /v2 routes over HTTP_BACKEND_URL;
#   the gateway's gRPC channel exists for v1, which is auto-disabled when the
#   gateway is in TVC-enforced mode (no parser_grpc_server needed for /v2).
nohup env \
  GATEWAY_PORT=8080 \
  GRPC_ADDR=http://127.0.0.1:44020 \
  X402_PROFILE=local \
  X402_FACILITATOR_URL=http://127.0.0.1:8090 \
  GATEWAY_SIGNING_KEY_FILE=$HOME/.config/visualsign/gateway/private.key \
  HTTP_BACKEND_URL=https://app-<APP_ID>.turnkey.cloud \
  ./target/debug/parser_gateway >/tmp/gateway.log 2>&1 < /dev/null &
disown
```

Wait for the gateway's startup banner in `/tmp/gateway.log`:

```
gateway signer loaded; pubkey <GATEWAY_SIGNING_PUBKEY_HEX>...
x402 facilitator probe OK
HTTP backend: https://app-<APP_ID>…turnkey.cloud (gRPC channel unused on /v2)
parser_gateway 0.700.21+spec-x402-tvc-enforced-… listening on 0.0.0.0:8080
```

> `/health` returns 503 because the gateway's health check probes the unused gRPC backend. This is cosmetic in HTTP_BACKEND_URL mode; ignore it.

Drive the request:

```bash
GW="http://127.0.0.1:8080/visualsign/api/v2/parse"

# Step 1: fetch the Payment-Required header (unpaid request → 402).
REQS=$(curl -sS -i -X POST "${GW}" \
  -H "Content-Type: application/json" \
  -d '{"request":{"unsigned_payload":"0x","chain":"CHAIN_ETHEREUM"}}' \
  | awk 'BEGIN{IGNORECASE=1} /^payment-required:/{ sub(/^[^:]+: */,""); print; exit }' \
  | tr -d '\r')

# Step 2: build a V2 Payment-Signature header. For X402_PROFILE=local the
# mock_facilitator accepts any payload; only the envelope shape matters.
ACCEPTED=$(echo "${REQS}" | base64 -d | jq '.accepts[0]')
PAY_HDR=$(jq -nc --argjson accepted "${ACCEPTED}" '{
  x402Version: 2,
  accepted: $accepted,
  payload: { payer: "0x000000000000000000000000000000000000AAAA", signature: "0xdeadbeef" }
}' | base64 -w0)

# Step 3: POST the paid request with a real EIP-155 tx.
ETH_TX="0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

curl -sS -X POST "${GW}" \
  -H "Content-Type: application/json" \
  -H "Payment-Signature: ${PAY_HDR}" \
  -d "{\"request\":{\"unsigned_payload\":\"${ETH_TX}\",\"chain\":\"CHAIN_ETHEREUM\"}}" \
  | jq '{
       error,
       payloadLen: (.response.parsedTransaction.payload.signablePayload // "" | length),
       enclavePubKey: .response.parsedTransaction.signature.publicKey
     }'
```

Expected output:

```json
{
  "error": null,
  "payloadLen": 734,
  "enclavePubKey": "<ENCLAVE_EPHEMERAL_PUBKEY>"
}
```

The `enclavePubKey` should byte-match the `Ephemeral Key` field in `tvc deploy provisioning-details --deploy-id <DEPLOY_ID_LIVE>…` — that's how you know the parse actually ran inside the attested enclave and wasn't intercepted.

## Variant: real settlement on Solana devnet (payai facilitator)

Same gateway → TVC pivot trust pair, but now the payment is a real signed USDC transfer on Solana devnet, settled by payai's hosted facilitator rather than the mock. Validates the full external-trust-boundary path.

Prereqs: the devnet buyer fixture at `src/integration/fixtures/devnet/wallet.seed` (32-byte seed, public address `x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW`) needs devnet SOL (≥0.05) and devnet USDC. Check with:

```bash
solana balance x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW --url devnet
solana spl-token accounts --owner x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW --url devnet
```

Top up via <https://faucet.solana.com> (SOL) and <https://faucet.circle.com> (USDC mint `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`).

Re-launch the gateway with the payai profile (kill the local-profile one first):

```bash
cd src

# Kill the existing gateway on :8080 (look up by port, not pkill, so the
# shell doesn't kill itself).
GW_PID=$(ss -tlnp 2>&1 | awk '/:8080 /{ for(i=1;i<=NF;i++) if(match($i, /pid=([0-9]+)/, m)) print m[1] }' | head -1)
[ -n "$GW_PID" ] && kill "$GW_PID"

nohup env \
  GATEWAY_PORT=8080 \
  GRPC_ADDR=http://127.0.0.1:44020 \
  X402_PROFILE=payai \
  X402_NETWORK=solana-devnet \
  X402_FACILITATOR_URL=https://facilitator.payai.network \
  X402_PAYTO=x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW \
  GATEWAY_SIGNING_KEY_FILE=$HOME/.config/visualsign/gateway/private.key \
  HTTP_BACKEND_URL=https://app-<APP_ID>.turnkey.cloud \
  ./target/debug/parser_gateway >/tmp/gateway.payai.log 2>&1 < /dev/null &
disown
```

The new 402 advertises devnet USDC instead of base-sepolia USDC:

```json
{
  "amount":  "1000",
  "asset":   "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
  "extra":   { "feePayer": "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4" },
  "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
  "payTo":   "x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW",
  "scheme":  "exact"
}
```

Drive the demo:

```bash
cd scripts
GATEWAY_URL=http://127.0.0.1:8080 ./x402-solana-devnet-demo.ts
```

The TS demo uses the official `x402-solana` client (per payai's reference): it signs the payment with the buyer's keypair, posts to the facilitator, gets the settled transaction back, retries the parse request with the `Payment-Signature` header, and unwraps the parsed response.

Expected tail:

```
[x402-solana] Retry response status: 200
status: 200
settlement: { network: "solana:Et…", payer: "x2iWww6…", success: true,
              transaction: "<88-char devnet sig>" }
payload bytes: 734
```

The `transaction` is a real Solana devnet signature — `getTransaction` it against `https://api.devnet.solana.com` to see two signers (buyer + payai's `feePayer`) and a 10001-lamport fee. Explorer link: `https://explorer.solana.com/tx/<sig>?cluster=devnet`.

A successful settled run validates the entire deployment topology end-to-end:

```
TS client (buyer keypair)
    │ (1) 402 + Payment-Required
    ▼
parser_gateway (local, X402_PROFILE=payai)
    │ (2) /verify, /settle to payai facilitator
    ▼
payai facilitator
    │ (3) signs + broadcasts USDC transfer on devnet
    ▼
Solana devnet                     (real tx, observable on explorer)
    │ (4) settlement receipt
    ▼
parser_gateway signs VPM with ~/.config/visualsign/gateway/private.key
    │ (5) POST /v2/parse + base64-borsh VPM
    ▼
parser_http_server (TVC, app-<APP_ID>…turnkey.cloud)
    │ (6) verify VPM sig vs pinned --gateway-signing-pubkey-hex
    │ (7) parse the ETH tx
    │ (8) sign response with enclave ephemeral key
    ▼
gateway → TS client (200 with signed parse)
```

## Variant: real settlement on Base Sepolia (payai facilitator)

Same trust pair, EVM side. A real USDC `transferWithAuthorization` (EIP-3009) on Base Sepolia, settled by payai. Validates the gateway's EVM 402 shape and the @x402/evm exact-scheme client path. Proven on-chain — tx `0x61a3afa9af9c1f6fa1c9d0d4b8bda942096894786a7fac3719073c2e5efccd93` (block 41,672,691) was the first successful run.

Prereqs: `~/.config/visualsign/base-sepolia/wallet.key` (32-byte hex private key, mode 600) is funded with Base Sepolia USDC. Fund at <https://faucet.circle.com> (pick "Base Sepolia"; USDC mint is `0x036CbD53842c5426634e7929541eC2318f3dCF7e`). ETH for gas is not strictly needed — payai's facilitator broadcasts via Multicall3 and covers gas — but a few drops as a fallback don't hurt.

Generate the wallet locally (don't print the private bytes; the file is mode 600):

```bash
mkdir -p ~/.config/visualsign/base-sepolia
node -e '
  const { generatePrivateKey, privateKeyToAccount } = require("./scripts/node_modules/viem/accounts");
  const fs = require("fs");
  const pk = generatePrivateKey();
  fs.writeFileSync(process.env.HOME + "/.config/visualsign/base-sepolia/wallet.key", pk.slice(2) + "\n", { mode: 0o600 });
  fs.writeFileSync(process.env.HOME + "/.config/visualsign/base-sepolia/wallet.address", privateKeyToAccount(pk).address + "\n");
  console.log(privateKeyToAccount(pk).address);
'
```

Re-launch the gateway with the payai profile aimed at base-sepolia (kill the previous one first):

```bash
cd src

GW_PID=$(ss -tlnp 2>&1 | awk '/:8080 /{ for(i=1;i<=NF;i++) if(match($i, /pid=([0-9]+)/, m)) print m[1] }' | head -1)
[ -n "$GW_PID" ] && kill "$GW_PID"

ADDR=$(cat ~/.config/visualsign/base-sepolia/wallet.address | tr -d '\n')
nohup env \
  GATEWAY_PORT=8080 \
  GRPC_ADDR=http://127.0.0.1:44020 \
  X402_PROFILE=payai \
  X402_NETWORK=base-sepolia \
  X402_PAYTO=$ADDR \
  GATEWAY_SIGNING_KEY_FILE=$HOME/.config/visualsign/gateway/private.key \
  HTTP_BACKEND_URL=https://app-<APP_ID>.turnkey.cloud \
  ./target/debug/parser_gateway >/tmp/gateway.basesep.log 2>&1 < /dev/null &
disown
```

The 402 from this gateway carries the v2-correct shape (notice the differences from the Solana variant):

```json
{
  "amount":  "100",
  "asset":   "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
  "extra":   { "name": "USDC", "version": "2" },
  "maxTimeoutSeconds": 300,
  "network": "eip155:84532",
  "payTo":   "0x7850b376011285f023603e8ad09b550b47f05bf5",
  "scheme":  "exact"
}
```

`network` is CAIP-2 form (`eip155:84532`), `asset` is the USDC contract, and `extra` carries the EIP-712 domain (`{name: "USDC", version: "2"}` matches Circle's FiatTokenV2_2). These three are what `@x402/evm/exact/client` needs to sign — the gateway translates them in `parse_tvc.rs::translate_to_canonical` per chain.

Drive the demo:

```bash
cd scripts
GATEWAY_URL=http://127.0.0.1:8080 ./x402-base-sepolia-demo.ts
```

The demo uses `@x402/core/client` + `@x402/evm/exact/client` (already in `scripts/package.json`). It signs the EIP-3009 authorization, retries `/v2/parse` with the `Payment-Signature` header, decodes the settled tx hash from the `Payment-Response` header, and validates the parse response.

Expected tail:

```
-- Paid POST /visualsign/api/v2/parse ----------------------------------
status: 200
response.error : null
payloadLen     : 734
enclavePubKey  : 049d817479c5e931137524e579e5af3581c5e0d02688101c30fa324c2a365995a5c8483d2725f1846c0f937501de671de5ef5a6388ddd4d8fbc0a3e6b673d186b904f1d2c385779acdb048c26324c38ff49ad696c49d8a6fc86a4e872b9d8076a7534ad190e8174ba8836aaa2b5cf51eec28529f393981702b5e410e965cc8c6fd21

-- Settlement (Payment-Response) ---------------------------------------
{
  "network": "base-sepolia",
  "payer": "0x7850b376011285f023603e8ad09b550b47f05bf5",
  "success": true,
  "transaction": "0x<66-char base-sepolia tx hash>"
}
explorer       : https://sepolia.basescan.org/tx/0x…
```

Inspect the on-chain settlement:

```bash
curl -sS -X POST https://sepolia.base.org -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionReceipt\",\"params\":[\"<0xhash>\"]}" \
  | jq '.result | {blockNumber, status, from, to, gasUsed, logs_count: (.logs | length)}'
```

Expect `status: 0x1`, two log entries on the USDC contract (`AuthorizationUsed` from the buyer, then `Transfer` from buyer → payTo), `from` = a payai-controlled executor, `to` = Multicall3 (`0xca11bde05977b3631167028862be2a173976ca11`).

The `enclavePubKey` in the parse response is identical to the value in the Solana variant — same deployed pivot, same attested ephemeral key answering both chain flows.

## Teardown

```bash
pgrep -f /target/debug/mock_facilitator | xargs -r kill
pgrep -f /target/debug/parser_gateway | xargs -r kill
```

(Don't `pkill -f` from a shell that has those strings in its own command line — it'll kill itself.)

## Open items

- **Rotate the gateway key when this leaves devnet POC.** The current pubkey came from `cargo run -p parser_gateway --bin gateway_keygen`; the private half is sitting at `~/.config/visualsign/gateway/private.key` mode 600. For prod, this key should be generated and held in a real secrets store, with the public half pinned into the pivot via `--gateway-signing-pubkey-hex` at deploy time only.
- **Delete the old broken deploy** once you're confident the new one is stable: `tvc deploy delete --deploy-id <DEPLOY_ID_BROKEN>`.
- **`tvc deploy provisioning-details` returned 500 on the broken deploy** but succeeds on the fixed one. Same OddLength root cause — it deserializes the same Manifest the approve flow does. If 500 reappears on a future deploy with this error class, run the dump-bytes patch on `rust-sdk/tvc` (`TVC_MANIFEST_DUMP=/tmp/x.json tvc deploy approve …`) and look at line 8 col 85.
- **PR #306 merges and propagates the digest fix to main** — once it's in, future stagex builds won't need the cherry-pick we did on the x402 stack.
