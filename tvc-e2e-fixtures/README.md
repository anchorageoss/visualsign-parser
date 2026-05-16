# TVC e2e fixtures

Captured artifacts that back every step in `../TVC_E2E_DEPLOYMENT_TEST.md`,
from the live `<APP_ID>` app on 2026-05-16.
Untracked on purpose; reference for the next debugging round, not part of
the repo's source.

| File | What it is | Doc step |
|---|---|---|
| `eth_tx.hex` | The EIP-155 legacy ETH tx hex used in every example | shared |
| `01_parse_request.json` | The Turnkey-envelope request body wrapping that tx | shared |
| `02_tvc_health_response.txt` | `curl -i https://app-…turnkey.cloud/health` — 200, empty body | smoke test § 1 |
| `03_tvc_v1_parse_response.json` | `/v1/parse` direct against TVC, signed by the enclave ephemeral key | smoke test § 2 |
| `04_tvc_v2_no_vpm_rejection.txt` | `/v2/parse` direct against TVC with no VPM → `"payment marker is required"` (proves `--gateway-signing-pubkey-hex` is active) | smoke test § 3 |
| `05_gateway_local_startup.log` | `parser_gateway` startup banner under `X402_PROFILE=local` against TVC backend | full x402 loop § startup |
| `06_gateway_local_paid_response.json` | `/v2/parse` via gateway with a mock-fac-fabricated `Payment-Signature` — same envelope shape as the real-money path | full x402 loop § paid request |
| `07_gateway_payai_startup.log` | Same gateway, re-launched with `X402_PROFILE=payai` + `X402_NETWORK=solana-devnet` | real-devnet variant § startup |
| `08_gateway_payai_402_raw.txt` | Raw 402 response from the payai-mode gateway (note the Payment-Required header is base64-JSON) | real-devnet variant § 402 |
| `09_gateway_payai_402_decoded.json` | The same Payment-Required header, base64-decoded — shows the devnet USDC mint, payai's `feePayer`, the buyer pubkey as `payTo` | real-devnet variant § 402 |
| `10_devnet_settlement_tx.json` | `getTransaction` for the real Solana devnet settlement signature `WWTGNvgRBXtg…GQXX71vy` — two signers (buyer + payai feePayer), fee 10001 lamports, status null (success) | real-devnet variant § confirmation |
| `11_tvc_provisioning_details.txt` | `tvc deploy provisioning-details` for the live deploy — PCRs, ephemeral key, manifest-set approval, share-set approvals | what's-live table |
| `12_tvc_app_status.txt` | `tvc app status` — targeted deployment + replica count | what's-live table |
| `13_deploy_config_used.json` | The `deploy-*.json` that produced the live deploy (pivotArgs include the pinned pubkey, no private material) | what's-live table |

The `.json` files are all JSON; the `.txt` files are raw curl/cli output with
HTTP headers included where relevant.

## Notes

- **No secret material here.** The deploy config carries only the gateway's *public* signing key (also pinned in `pivotArgs` and visible in `12_tvc_app_status.txt`). The matching private key stays at `~/.config/visualsign/gateway/private.key` (mode 600), never copied into this directory.
- **The settlement tx is real.** `10_devnet_settlement_tx.json` is observable on the public Solana devnet explorer; the on-chain effect was a 0.001 USDC self-transfer + 10001 lamports fee paid by payai's facilitator.
- **The signing pubkey on `03` and `06` should match** the `Ephemeral Key` line in `11_tvc_provisioning_details.txt`. That match is the bit that says the parse really ran inside the attested enclave.
