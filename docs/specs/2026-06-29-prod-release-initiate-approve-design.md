# Prod Release flow — initiate / approve / promote

**Date:** 2026-06-29
**Status:** Approved (design)
**Branches:** `feat/prod-release-flow` (stacked on `feat/tvc-deploy-helper`, PR #395, visualsign-parser); `feat/dev-path-and-container` (PR #23, visualsign-turnkeyclient)

## Overview

Today the `tvc-deploy` helper drives the entire dev deploy end to end: digest gate -> create -> approve -> poll-to-healthy -> set-live. That is fine for dev, where CI can hold an operator seed. For **production** we want **separation of duties**: whoever triggers the release from CI must never hold the quorum/operator key. CI only holds the Turnkey **API key** (enough to create / read status / set-live); a human **operator** approves the manifest out-of-band with their key.

This splits the prod path into three steps across two manually-triggered workflows, plus a binary-identity check on the post-promote smoke so we can assert the live enclave is serving exactly the release we deployed.

## Goals

- Prod CI can **initiate** a deploy (create + digest gate) without any operator key.
- A human **operator** approves the created deployment with a key sourced from 1Password.
- Prod **set-live** is a separate, deliberate, manually-triggered step.
- The post-promote smoke **cryptographically pins** the served enclave to the exact binary we released.
- The existing dev flow (`TVC Deploy (Dev)`) is unchanged in behavior.

## Non-goals / out of scope

- **Staging** — deferred; only Dev (existing) and Prod (new) for now.
- Provisioning the prod **parse API key** and a prod-renderable **fixture** for the smoke (noted as a prerequisite, tracked separately).
- Automatic rollback — reverting a prod deploy means re-deploying the prior image (manual).

## Architecture

### Two repos

- **visualsign-parser** — the `tvc-deploy` helper, `smoke.sh`, and the workflows.
- **visualsign-turnkeyclient** — the `verify` command, which gains the pivot-hash pin (`--expected-pivot-hash`).

### Flow

```
Release (CI, prod API key)         Operator (human, quorum key)        Promote (CI, prod API key)
  tvc-deploy initiate                tvc-deploy approve                   tvc-deploy promote
   - digest gate                      - re-run digest gate                 - poll-to-healthy
   - tvc deploy create                - tvc deploy approve                 - set-live
   - print deploy ID            -->   (seed from 1Password)        -->     - smoke: verify --canonical
                                                                             --expected-pivot-hash <digest>
```

## Component 1 — helper subcommands (`tools/tvc-deploy`)

Refactor the current monolithic `deploy()` into three composable subcommands. The existing digest-gate, config-assembly, and polling logic is **extracted, not duplicated**.

- **`initiate --app-id --image-url --expected-digest [--qos-version --host-ip --host-port]`**
  1. `validate_digest` + `verify_image_digest` (the existing image-derived digest gate).
  2. Assemble the deploy config (gRPC health) and `tvc deploy create`.
  3. Parse and print the **deployment ID** (existing `parse_after "Deployment ID:"`).
  Requires only the Turnkey API key. No operator seed, no approve, no set-live.

- **`approve --deploy-id --operator-id --image-url --expected-digest [--operator-seed]`**
  1. **Re-run `verify_image_digest`** so the approver independently confirms the image's `/parser_app` sha256 equals `--expected-digest` before signing.
  2. `tvc deploy approve --deploy-id … --operator-id … [--operator-seed … | logged-in operator]`.
  Run locally by the human operator with their key (seed resolution unchanged: flag -> `TVC_CI_OPERATOR_SEED` -> logged-in operator).

- **`promote --app-id --deploy-id`**
  1. `poll_health` to healthy (existing).
  2. `set_live` (existing, including the "already live" + transient-retry handling).

- **`deploy …` (dev, unchanged CLI/behavior)** = `initiate` -> `approve` -> `promote` composed internally.

## Component 2 — pivot-hash pin (`turnkey-client verify`)

The post-promote smoke must assert the live enclave serves the binary we released. The deployed identity is the QoS manifest's `Pivot.Hash`, which equals the sha256 of the pivot `/parser_app` binary — i.e. the same value we pass as `--expected-digest` (TVC's `expectedPivotDigest`). It is attestation-bound: the served manifest's hash matches the attestation UserData, and that manifest contains `Pivot.Hash`.

The existing `verify` flags do **not** pin this: `--qos-manifest-hex` / `--pivot-binary-hash-hex` compare against UserData (the *manifest* hash), and the JSON output's `pivotBinaryHash` is currently mis-aliased to the manifest hash. So `verify` gains:

- **`--expected-pivot-hash <hex>`** — decode the (attestation-bound) manifest and assert `manifest.Pivot.Hash == <hex>`; fail verification on mismatch.
- Surface the **real** `Pivot.Hash` in the JSON output (fix the alias) so it is observable.

`verify` already decodes the manifest (`manifest.Pivot.Hash`, `manifest/types.go`), so this is a small, well-scoped addition. Stacks on PR #23.

## Component 3 — smoke canonical-path + pin (`scripts/smoke.sh`)

- **Canonical path:** add a `--canonical` flag (env `VSP_SMOKE_CANONICAL=1`) that runs `verify` **without** `--dev-path`, targeting the prod `/visualsign` endpoint. Dev usage is unchanged (defaults to the dev path).
- **Pin passthrough:** add `--expected-pivot-hash <hex>` / `VSP_SMOKE_EXPECTED_PIVOT_HASH` that forwards to `verify --expected-pivot-hash`. The existing `.valid / .attestationValid / .signatureValid` assertions plus the new pin make a prod PASS mean "genuine enclave, valid signature, renders, **and** running exactly the released binary."
- Prod org/app supplied via existing `VSP_SMOKE_ORG` / related env.

## Component 4 — workflows (`.github/workflows`)

- **TVC Deploy (Dev)** — existing `tvc-deploy.yml`, renamed (name, and filename to `tvc-deploy-dev.yml`). Behavior unchanged; auto-drives dev via the composed `deploy`.
- **Release** (`release.yml`, prod, `workflow_dispatch`) — inputs: `app_id`, `image_url`, `expected_digest`, `qos_version`, `host_ip`, `host_port`. Steps: install `tvc` (pinned), build helper, run `tvc-deploy initiate …`, surface the deploy ID in the job summary. Auth: prod `TVC_ORG_ID` / `TVC_API_KEY_*` only — **no operator seed**.
- **Promote** (`promote.yml`, prod, `workflow_dispatch`) — inputs: `app_id`, `deploy_id`, `expected_digest`, `turnkey_client_version`. Steps: `tvc-deploy promote …`, then `smoke.sh --canonical --expected-pivot-hash <expected_digest> …` against the prod endpoint. Auth: prod API key only. Concurrency-guarded so two promotes can't race.
- **Operator approval** (out-of-band, runbook-documented) — operator pulls the **prod** operator seed from 1Password (its own item, distinct from `Turnkey VSP Dev Deployment Operator Key`), confirms the deploy ID from the Release run, and runs `tvc-deploy approve …` (re-verifies the digest, then signs).

## Separation of duties / secrets

- CI (Release + Promote) holds the prod Turnkey **API key** — can create / status / set-live, **cannot** approve a manifest.
- The **quorum/operator key** never enters CI; it lives in 1Password and is used only by the human approver on their machine.

## Error handling & idempotency

- `initiate` / `approve`: the digest gate aborts before any irreversible action on mismatch.
- `approve`: idempotent against an already-approved manifest (delegated to `tvc`).
- `promote`: `set_live` already treats "already live" as success and retries transient settling errors; `poll_health` is bounded by timeout. Re-runnable.
- Release/Promote: concurrency guards keyed by app so overlapping runs don't race.

## Testing

- **Rust:** unit tests for the new subcommand arg parsing; a test that `deploy` still composes `initiate -> approve -> promote`. clippy `-D warnings`, fmt.
- **Go (turnkey-client):** TDD `--expected-pivot-hash` — a manifest whose `Pivot.Hash` matches passes; a mismatch fails verification. Reuse the existing manifest fixtures.
- **smoke.sh:** extend the existing `TURNKEY_CLIENT`-stub bash harness to assert the new flags — `--canonical` omits `--dev-path`, and `--expected-pivot-hash` forwards to `verify`. shellcheck clean.
- **Manual:** prove `initiate -> approve -> promote` end-to-end against the **dev test app** before any prod run.

## Rollout / stacking

- `verify --expected-pivot-hash` lands on `feat/dev-path-and-container` (PR #23).
- Helper subcommands, `smoke.sh`, and the workflows land on `feat/prod-release-flow` (stacked on PR #395).
- The runbook (Notion sub-page) gains the prod initiate -> approve -> promote procedure.

## Open items (resolve early in implementation)

- Confirm the `tvc` CLI version in use prints the deployment ID in the format `initiate` parses (existing `parse_after "Deployment ID:"` already handles dev).
- Confirm the prod Turnkey **org id / API key** and the prod **operator id** + 1Password item; confirm the prod **app id** and `/visualsign` endpoint app for the smoke.
- Provision a prod parse API key and a prod-renderable fixture for the Promote smoke (prerequisite).

## References

- visualsign-parser PR #395 (tvc-deploy helper + dev smoke).
- visualsign-turnkeyclient PR #23 (`--dev-path`/`--chain` + container; `verify --expected-pivot-hash` to be added here).
- Notion: "Runbook — parser_app dev deploy via tvc-deploy helper + verify smoke" (parent: "Runbook — Turnkey TVC deployment for visualsign-parser").
- Linear PRS-515 (TVC deploy runbook), PRS-516 (automate dev/staging TVC deploys).
