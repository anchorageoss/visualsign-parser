# Prod Release flow (initiate / approve / promote) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split prod parser_app deploys into CI-driven `initiate`/`promote` steps and a human `approve` step so CI never holds the operator key, and pin the post-promote smoke to the exact released binary.

**Architecture:** A `verify --expected-pivot-hash` check is added to the Go turnkey-client so the smoke can assert the live enclave's manifest `Pivot.Hash` equals the deployed digest. The Rust `tvc-deploy` helper is refactored behind a `TvcOps` trait and split into `initiate`/`approve`/`promote` subcommands (dev `deploy` composes them). Two manually-triggered prod workflows (Release, Promote) plus the renamed dev workflow drive it.

**Tech Stack:** Go (urfave/cli v3, near/borsh-go) in visualsign-turnkeyclient; Rust 2024 (xshell, anyhow, lexopt, serde_json, qos_p256) in visualsign-parser `tools/tvc-deploy`; Bash + jq for `scripts/smoke.sh`; GitHub Actions YAML.

## Global Constraints

- Rust: workspace lints deny `unwrap_used`/`expect_used`/`panic`, forbid `unsafe`; test modules add `#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. Use inline format strings. Place `use` at the top of the module/test module. Every Rust task ends green on `cargo clippy --all-targets -- -D warnings` and `cargo fmt`.
- ASCII only in helper/script output (`>=` not `≥`, `->` not `→`).
- Go: `gofmt`, `go vet ./...`, `go test ./...` all clean.
- CI separation of duties: Release and Promote workflows carry only the Turnkey API key (`TVC_ORG_ID`/`TVC_API_KEY_PUBLIC`/`TVC_API_KEY_PRIVATE`). The operator/quorum seed is NEVER referenced in a workflow.
- `tvc` CLI pinned `--version 0.7.0 --locked` in all workflows.
- Smoke exit codes unchanged: 0 = pass/skip, 1 = up-but-failed, 2 = harness could not run the client.
- Prod placeholder values in workflows use `vars.TVC_PROD_*` / `secrets.TVC_PROD_*`; do not hardcode prod ids.

---

## Phase 1 — turnkey-client: `verify --expected-pivot-hash`

Repo: `visualsign-turnkeyclient`, branch `feat/dev-path-and-container` (PR #23). Env for all commands: `export PATH="/usr/local/go/bin:$HOME/go/bin:$PATH" GOPATH="$HOME/go" GOFLAGS=-mod=mod`.

### Task 1: Pivot-hash check in verify

**Files:**
- Create: `verify/pivot_hash.go`
- Test: `verify/pivot_hash_test.go`
- Modify: `verify/types.go` (add `ExpectedPivotHashHex` to `VerifyRequest`)
- Modify: `verify/service.go` (set `result.PivotBinaryHash` from the manifest; call the check)
- Modify: `cmd/verify.go` (add `--expected-pivot-hash` flag, thread into the request)

**Interfaces:**
- Produces: `func CheckExpectedPivotHash(m *manifest.Manifest, expectedHex string) error` — returns nil when `expectedHex` is empty or equals `hex(m.Pivot.Hash)`; error otherwise.
- Consumes: `manifest.Manifest` (`Pivot.Hash` is `manifest.Hash256` = `[32]byte`, `manifest/types.go`).

- [ ] **Step 1: Write the failing test**

```go
// verify/pivot_hash_test.go
package verify

import (
	"testing"

	"github.com/anchorageoss/visualsign-turnkeyclient/manifest"
)

func mkManifest(b byte) *manifest.Manifest {
	var m manifest.Manifest
	for i := range m.Pivot.Hash {
		m.Pivot.Hash[i] = b
	}
	return &m
}

func TestCheckExpectedPivotHash(t *testing.T) {
	m := mkManifest(0xab) // Pivot.Hash = 32 * 0xab
	full := "abababababababababababababababababababababababababababababababab"

	if err := CheckExpectedPivotHash(m, ""); err != nil {
		t.Fatalf("empty expected should pass, got %v", err)
	}
	if err := CheckExpectedPivotHash(m, full); err != nil {
		t.Fatalf("matching hash should pass, got %v", err)
	}
	if err := CheckExpectedPivotHash(m, "ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB"+"AB"); err != nil {
		t.Fatalf("case-insensitive match should pass, got %v", err)
	}
	if err := CheckExpectedPivotHash(m, "00"+full[2:]); err == nil {
		t.Fatal("mismatched hash should fail")
	}
	if err := CheckExpectedPivotHash(nil, full); err == nil {
		t.Fatal("nil manifest with expected set should fail")
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `go test ./verify/ -run TestCheckExpectedPivotHash -v`
Expected: FAIL (undefined: `CheckExpectedPivotHash`).

- [ ] **Step 3: Write minimal implementation**

```go
// verify/pivot_hash.go
package verify

import (
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/anchorageoss/visualsign-turnkeyclient/manifest"
)

// CheckExpectedPivotHash asserts the manifest's pivot binary hash equals the
// expected hex (the value deployed as expectedPivotDigest). A blank expected
// disables the check.
func CheckExpectedPivotHash(m *manifest.Manifest, expectedHex string) error {
	if expectedHex == "" {
		return nil
	}
	if m == nil {
		return fmt.Errorf("cannot verify pivot hash: no manifest decoded")
	}
	actual := hex.EncodeToString(m.Pivot.Hash[:])
	if !strings.EqualFold(actual, expectedHex) {
		return fmt.Errorf("pivot hash mismatch: manifest %s != expected %s", actual, expectedHex)
	}
	return nil
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `go test ./verify/ -run TestCheckExpectedPivotHash -v`
Expected: PASS.

- [ ] **Step 5: Surface the real pivot hash and wire the request**

In `verify/types.go`, add to `VerifyRequest`:

```go
	ExpectedPivotHashHex string
```

In `verify/service.go`, where the manifest is decoded and `result.PivotBinaryHash` is currently assigned (it is presently set to the manifest hash — correct it), set it from the manifest and run the check:

```go
	// result.Manifest is the decoded manifest; set the real pivot binary hash.
	if result.Manifest != nil {
		result.PivotBinaryHash = hex.EncodeToString(result.Manifest.Pivot.Hash[:])
	}
	if err := CheckExpectedPivotHash(result.Manifest, req.ExpectedPivotHashHex); err != nil {
		return nil, err
	}
```

(Add `"encoding/hex"` to the `verify/service.go` imports if not present.)

In `cmd/verify.go`, add the flag to the `Flags` slice:

```go
			&cli.StringFlag{
				Name:  "expected-pivot-hash",
				Usage: "Expected pivot binary hash (hex). Fails verification unless the manifest's pivot hash matches.",
			},
```

and thread it into the `VerifyRequest` built in `runVerifyCommand`:

```go
		ExpectedPivotHashHex: cmd.String("expected-pivot-hash"),
```

- [ ] **Step 6: Run the full suite + vet**

Run: `go test ./... && go vet ./...`
Expected: all packages `ok`, no vet output.

- [ ] **Step 7: Commit**

```bash
git add verify/pivot_hash.go verify/pivot_hash_test.go verify/types.go verify/service.go cmd/verify.go
git commit -m "feat: verify --expected-pivot-hash pins the manifest pivot binary hash

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 8: Manual confirmation against dev (optional, needs keys)**

Run (with the local binary built and dev keys present):
```bash
make build && BIN=bin/visualsign-turnkeyclient
PAYLOAD="$(tr -d '[:space:]' < ../visualsign-parser/testdata/solana_v0_alt.b64)"
$BIN verify --dev-path --expected-pivot-hash ce9f2733d412b9e0e3e5d582f1ab3c399910c311985ab6e208da78726bbe7649 \
  --host https://api.turnkey.com --organization-id d7f51c3d-fb9d-47c1-9b2e-a02b1cd5ff14 \
  --key-name dev --unsigned-payload "$PAYLOAD" --chain CHAIN_SOLANA; echo "exit=$?"
```
Expected: exit 0 with the matching pivot hash; a wrong `--expected-pivot-hash` exits non-zero with "pivot hash mismatch". (`ce9f2733...` is the dev app's current pivot hash; if it changed, read the current value from `.pivotBinaryHash` in the output.)

---

## Phase 2 — helper: `TvcOps` trait + initiate/approve/promote

Repo: `visualsign-parser`, branch `feat/prod-release-flow`. File: `tools/tvc-deploy/src/main.rs`. Env: `export PATH="$HOME/.cargo/bin:$PATH"`. Build/test from `tools/tvc-deploy`.

### Task 2: Introduce `TvcOps` trait and pure config builder; refactor `deploy`

This refactor preserves dev behavior while making the orchestration unit-testable. The trait abstracts every external (tvc/docker) call; the subcommand functions become pure orchestration over the trait + local fs.

**Files:**
- Modify: `tools/tvc-deploy/src/main.rs`
- Test: `tools/tvc-deploy/src/main.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `fn build_deploy_config(app_id, qos, image, host_ip, host_port: u16, digest: &str) -> serde_json::Value`
  - `trait TvcOps { fn verify_image_digest(&self, image: &str, expected: &str) -> Result<()>; fn create(&self, cfg_path: &Path) -> Result<String>; fn approve(&self, deploy_id: &str, operator_id: &str, seed: Option<&Path>) -> Result<()>; fn poll_health(&self, app_id: &str, deploy_id: &str, timeout: Duration) -> Result<()>; fn set_live(&self, deploy_id: &str, timeout: Duration) -> Result<()>; }`
  - `struct RealTvc<'a> { sh: &'a Shell }` implementing `TvcOps` by moving the existing `cmd!` calls into the trait methods.
  - `fn do_deploy(ops: &impl TvcOps, flags, ...) -> Result<()>` composing initiate -> approve -> promote (the existing dev behavior).

- [ ] **Step 1: Write the failing test (config builder + recording fake compose order)**

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct RecordingTvc {
        calls: RefCell<Vec<String>>,
    }
    impl TvcOps for RecordingTvc {
        fn verify_image_digest(&self, _image: &str, _expected: &str) -> Result<()> {
            self.calls.borrow_mut().push("verify_image_digest".into());
            Ok(())
        }
        fn create(&self, _cfg_path: &Path) -> Result<String> {
            self.calls.borrow_mut().push("create".into());
            Ok("deploy-123".into())
        }
        fn approve(&self, deploy_id: &str, _operator_id: &str, _seed: Option<&Path>) -> Result<()> {
            self.calls.borrow_mut().push(format!("approve:{deploy_id}"));
            Ok(())
        }
        fn poll_health(&self, _app: &str, deploy_id: &str, _t: Duration) -> Result<()> {
            self.calls.borrow_mut().push(format!("poll:{deploy_id}"));
            Ok(())
        }
        fn set_live(&self, deploy_id: &str, _t: Duration) -> Result<()> {
            self.calls.borrow_mut().push(format!("set_live:{deploy_id}"));
            Ok(())
        }
    }

    fn flags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn config_has_grpc_health_and_digest() {
        let cfg = build_deploy_config("app", "v1", "img", "0.0.0.0", 3000, "deadbeef");
        assert_eq!(cfg["healthCheckType"], "TVC_HEALTH_CHECK_TYPE_GRPC");
        assert_eq!(cfg["expectedPivotDigest"], "deadbeef");
        assert_eq!(cfg["pivotPath"], "/parser_app");
        assert_eq!(cfg["healthCheckPort"], 3000);
    }

    #[test]
    fn deploy_runs_gate_create_approve_poll_setlive_in_order() {
        let ops = RecordingTvc::default();
        let f = flags(&[
            ("app-id", "app"),
            ("image-url", "img"),
            ("expected-digest", "deadbeef"),
            ("operator-id", "op"),
            ("operator-seed", "/tmp/seed"),
        ]);
        do_deploy(&ops, &f).unwrap();
        assert_eq!(
            *ops.calls.borrow(),
            vec![
                "verify_image_digest",
                "create",
                "approve:deploy-123",
                "poll:deploy-123",
                "set_live:deploy-123",
            ]
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p tvc-deploy 2>&1 | tail -20` (from `tools/tvc-deploy`: `cargo test`)
Expected: FAIL — `build_deploy_config`, `TvcOps`, `do_deploy` not found.

- [ ] **Step 3: Implement the trait, RealTvc, builder, and do_deploy**

Extract the JSON assembly from today's `deploy` into `build_deploy_config`:

```rust
fn build_deploy_config(
    app_id: &str,
    qos: &str,
    image: &str,
    host_ip: &str,
    host_port: u16,
    digest: &str,
) -> serde_json::Value {
    serde_json::json!({
        "appId": app_id,
        "qosVersion": qos,
        "pivotContainerImageUrl": image,
        "pivotPath": "/parser_app",
        "pivotArgs": ["--host-ip", host_ip, "--host-port", host_port.to_string()],
        "expectedPivotDigest": digest,
        "debugMode": false,
        "healthCheckType": "TVC_HEALTH_CHECK_TYPE_GRPC",
        "healthCheckPort": host_port,
        "publicIngressPort": host_port,
    })
}
```

Define the trait and move the existing shell-outs into `RealTvc`:

```rust
trait TvcOps {
    fn verify_image_digest(&self, image: &str, expected: &str) -> Result<()>;
    fn create(&self, cfg_path: &Path) -> Result<String>;
    fn approve(&self, deploy_id: &str, operator_id: &str, seed: Option<&Path>) -> Result<()>;
    fn poll_health(&self, app_id: &str, deploy_id: &str, timeout: Duration) -> Result<()>;
    fn set_live(&self, deploy_id: &str, timeout: Duration) -> Result<()>;
}

struct RealTvc<'a> {
    sh: &'a Shell,
}

impl TvcOps for RealTvc<'_> {
    fn verify_image_digest(&self, image: &str, expected: &str) -> Result<()> {
        verify_image_digest(self.sh, image, expected) // existing free fn
    }
    fn create(&self, cfg_path: &Path) -> Result<String> {
        let created = cmd!(self.sh, "tvc deploy create --config-file {cfg_path}")
            .read()
            .context("tvc deploy create")?;
        parse_after(&created, "Deployment ID:")
            .with_context(|| format!("no deployment id in create output:\n{created}"))
    }
    fn approve(&self, deploy_id: &str, operator_id: &str, seed: Option<&Path>) -> Result<()> {
        let mut seed_args: Vec<OsString> = Vec::new();
        if let Some(p) = seed {
            seed_args.push("--operator-seed".into());
            seed_args.push(p.into());
        }
        cmd!(self.sh, "tvc deploy approve --deploy-id {deploy_id} --operator-id {operator_id} {seed_args...} --dangerous-skip-interactive")
            .run()
            .context("tvc deploy approve")
    }
    fn poll_health(&self, app_id: &str, deploy_id: &str, timeout: Duration) -> Result<()> {
        poll_health(self.sh, app_id, deploy_id, timeout) // existing free fn
    }
    fn set_live(&self, deploy_id: &str, timeout: Duration) -> Result<()> {
        set_live(self.sh, deploy_id, timeout) // existing free fn
    }
}
```

Add the composed dev deploy (operator seed resolution unchanged via `resolve_seed_file`):

```rust
fn do_deploy(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<()> {
    let app_id = req(flags, "app-id")?;
    let operator_id = req(flags, "operator-id")?;
    let seed = resolve_seed_file(flags)?; // Option<(PathBuf, bool)>
    let deploy_id = initiate(ops, flags)?;
    let seed_path = seed.as_ref().map(|(p, _)| p.as_path());
    ops.approve(&deploy_id, operator_id, seed_path)?;
    println!("approved manifest for {deploy_id}");
    if let Some((path, true)) = &seed {
        let _ = std::fs::remove_file(path);
    }
    ops.poll_health(app_id, &deploy_id, POLL_TIMEOUT)?;
    ops.set_live(&deploy_id, SETLIVE_TIMEOUT)?;
    println!("deployment {deploy_id} is healthy and live");
    Ok(())
}
```

Update `run()` so `"deploy" => do_deploy(&RealTvc { sh: &sh }, &flags)`. Keep `verify_image_digest`, `poll_health`, `set_live`, `parse_after`, `resolve_seed_file`, `temp_path`, `build_deploy_config`, `validate_digest` as free functions. (`initiate` is added in Task 3; for this task, inline the gate+create into `do_deploy` or land Task 3 first — implement `initiate` here as part of the refactor so `do_deploy` can call it.)

- [ ] **Step 4: Run tests to verify pass**

Run (from `tools/tvc-deploy`): `cargo test`
Expected: PASS (`config_has_grpc_health_and_digest`, `deploy_runs_gate_create_approve_poll_setlive_in_order`).

- [ ] **Step 5: clippy + fmt**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add tools/tvc-deploy/src/main.rs
git commit -m "refactor(tools): TvcOps trait + pure config builder behind tvc-deploy

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### Task 3: `initiate` subcommand

**Files:** Modify/Test: `tools/tvc-deploy/src/main.rs`

**Interfaces:**
- Produces: `fn initiate(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<String>` — runs `validate_digest`, `ops.verify_image_digest`, writes the config temp file, `ops.create`, prints and returns the deploy id. No approve/poll/set_live.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn initiate_runs_only_gate_and_create() {
        let ops = RecordingTvc::default();
        let f = flags(&[
            ("app-id", "app"),
            ("image-url", "img"),
            ("expected-digest", "deadbeef"),
        ]);
        let id = initiate(&ops, &f).unwrap();
        assert_eq!(id, "deploy-123");
        assert_eq!(*ops.calls.borrow(), vec!["verify_image_digest", "create"]);
    }

    #[test]
    fn initiate_rejects_bad_digest() {
        let ops = RecordingTvc::default();
        let f = flags(&[("app-id", "a"), ("image-url", "i"), ("expected-digest", "xyz")]);
        assert!(initiate(&ops, &f).is_err());
        assert!(ops.calls.borrow().is_empty());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test initiate`
Expected: FAIL (if `initiate` not yet present) or compile error.

- [ ] **Step 3: Implement `initiate`**

```rust
fn initiate(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<String> {
    let app_id = req(flags, "app-id")?;
    let image = req(flags, "image-url")?;
    let digest = req(flags, "expected-digest")?;
    let qos = flags.get("qos-version").map(String::as_str).unwrap_or("v2026.2.6");
    let host_ip = flags.get("host-ip").map(String::as_str).unwrap_or("0.0.0.0");
    let host_port: u16 = match flags.get("host-port") {
        Some(s) => s.parse().with_context(|| format!("invalid --host-port: {s}"))?,
        None => 3000,
    };
    validate_digest(digest)?;
    ops.verify_image_digest(image, digest)?;
    let cfg = build_deploy_config(app_id, qos, image, host_ip, host_port, digest);
    let cfg_path = temp_path("tvc-deploy", "json");
    std::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg)?)
        .with_context(|| format!("write {}", cfg_path.display()))?;
    let deploy_id = ops.create(&cfg_path);
    let _ = std::fs::remove_file(&cfg_path);
    let deploy_id = deploy_id?;
    println!("created deployment {deploy_id}");
    Ok(deploy_id)
}
```

Add `"initiate" => { initiate(&RealTvc { sh: &sh }, &flags).map(|_| ()) }` to `run()`'s match. Update `USAGE` with the `initiate` line.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: clippy + fmt + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add tools/tvc-deploy/src/main.rs
git commit -m "feat(tools): tvc-deploy initiate subcommand (digest gate + create)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### Task 4: `promote` subcommand

**Files:** Modify/Test: `tools/tvc-deploy/src/main.rs`

**Interfaces:**
- Produces: `fn promote(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<()>` — `req` `app-id` + `deploy-id`, then `ops.poll_health` then `ops.set_live`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn promote_polls_then_sets_live() {
        let ops = RecordingTvc::default();
        let f = flags(&[("app-id", "app"), ("deploy-id", "deploy-9")]);
        promote(&ops, &f).unwrap();
        assert_eq!(*ops.calls.borrow(), vec!["poll:deploy-9", "set_live:deploy-9"]);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test promote`
Expected: FAIL (`promote` not found).

- [ ] **Step 3: Implement `promote`**

```rust
fn promote(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<()> {
    let app_id = req(flags, "app-id")?;
    let deploy_id = req(flags, "deploy-id")?;
    ops.poll_health(app_id, deploy_id, POLL_TIMEOUT)?;
    ops.set_live(deploy_id, SETLIVE_TIMEOUT)?;
    println!("deployment {deploy_id} is healthy and live");
    Ok(())
}
```

Add `"promote" => promote(&RealTvc { sh: &sh }, &flags)` to `run()`. Update `USAGE`.

- [ ] **Step 4: Run tests + clippy + fmt + commit**

```bash
cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings
git add tools/tvc-deploy/src/main.rs
git commit -m "feat(tools): tvc-deploy promote subcommand (poll-to-healthy + set-live)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### Task 5: `approve` subcommand (re-verify then approve)

**Files:** Modify/Test: `tools/tvc-deploy/src/main.rs`

**Interfaces:**
- Produces: `fn approve(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<()>` — `req` `deploy-id`, `operator-id`, `image-url`, `expected-digest`; runs `validate_digest`, `ops.verify_image_digest` (independent re-verify), then `ops.approve` with the resolved seed; cleans up an env-sourced temp seed.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn approve_reverifies_then_approves() {
        let ops = RecordingTvc::default();
        let f = flags(&[
            ("deploy-id", "deploy-7"),
            ("operator-id", "op"),
            ("image-url", "img"),
            ("expected-digest", "deadbeef"),
            ("operator-seed", "/tmp/seed"),
        ]);
        approve(&ops, &f).unwrap();
        assert_eq!(*ops.calls.borrow(), vec!["verify_image_digest", "approve:deploy-7"]);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test approve_reverifies`
Expected: FAIL (`approve` not found).

- [ ] **Step 3: Implement `approve`**

```rust
fn approve(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<()> {
    let deploy_id = req(flags, "deploy-id")?;
    let operator_id = req(flags, "operator-id")?;
    let image = req(flags, "image-url")?;
    let digest = req(flags, "expected-digest")?;
    validate_digest(digest)?;
    ops.verify_image_digest(image, digest)?; // independent confirmation before signing
    let seed = resolve_seed_file(flags)?;
    let result = ops.approve(deploy_id, operator_id, seed.as_ref().map(|(p, _)| p.as_path()));
    if let Some((path, true)) = &seed {
        let _ = std::fs::remove_file(path);
    }
    result?;
    println!("approved manifest for {deploy_id}");
    Ok(())
}
```

Add `"approve" => approve(&RealTvc { sh: &sh }, &flags)` to `run()`. Update `USAGE` with the `approve` line, documenting that the seed resolves flag -> `TVC_CI_OPERATOR_SEED` -> logged-in operator.

- [ ] **Step 4: Run tests + clippy + fmt + commit**

```bash
cargo test && cargo fmt && cargo clippy --all-targets -- -D warnings
git add tools/tvc-deploy/src/main.rs
git commit -m "feat(tools): tvc-deploy approve subcommand (re-verify digest, then approve)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Phase 3 — smoke.sh: canonical path + pivot-hash pin

Repo: `visualsign-parser`, file `scripts/smoke.sh`. Tests reuse the scratch stub harness pattern (a stub `TURNKEY_CLIENT` recording its args). Place the test under `tools/tvc-deploy/`-style scratch is not committed; instead commit a small bats-free bash test at `scripts/test_smoke_args.sh`.

### Task 6: `--canonical` and `--expected-pivot-hash`

**Files:**
- Modify: `scripts/smoke.sh`
- Create: `scripts/test_smoke_args.sh`

**Interfaces:**
- Produces: smoke flags `--canonical` (env `VSP_SMOKE_CANONICAL=1`) — omit `--dev-path`; `--expected-pivot-hash <hex>` (env `VSP_SMOKE_EXPECTED_PIVOT_HASH`) — forward `--expected-pivot-hash` to `verify`.

- [ ] **Step 1: Write the failing test**

```bash
#!/usr/bin/env bash
# scripts/test_smoke_args.sh — asserts smoke.sh forwards --canonical / --expected-pivot-hash.
set -uo pipefail
SMOKE="$(cd "$(dirname "$0")" && pwd)/smoke.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
ARGFILE="$TMP/args"
fails=0

cat >"$TMP/client" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$@" > "$ARGFILE"
echo "=== VERIFICATION COMPLETE ===" >&2
printf '%s' '{"signablePayload":"ok","valid":true,"attestationValid":true,"signatureValid":true,"moduleId":"m"}'
EOF
chmod +x "$TMP/client"

# Default dev run: includes --dev-path, no --expected-pivot-hash.
TURNKEY_CLIENT="$TMP/client" "$SMOKE" >/dev/null 2>&1
grep -qx -- "--dev-path" "$ARGFILE" || { echo "FAIL: default should pass --dev-path"; fails=$((fails+1)); }
grep -qx -- "--expected-pivot-hash" "$ARGFILE" && { echo "FAIL: default should not pin"; fails=$((fails+1)); }

# Canonical + pin: no --dev-path, forwards --expected-pivot-hash <hex>.
TURNKEY_CLIENT="$TMP/client" "$SMOKE" --canonical --expected-pivot-hash deadbeef >/dev/null 2>&1
grep -qx -- "--dev-path" "$ARGFILE" && { echo "FAIL: --canonical should omit --dev-path"; fails=$((fails+1)); }
if ! grep -qx -- "--expected-pivot-hash" "$ARGFILE" || ! grep -qx -- "deadbeef" "$ARGFILE"; then
  echo "FAIL: should forward --expected-pivot-hash deadbeef"; fails=$((fails+1));
fi

echo "---"; [ "$fails" -eq 0 ] && echo "ALL PASS" || { echo "$fails FAILED"; exit 1; }
```

`chmod +x scripts/test_smoke_args.sh`.

- [ ] **Step 2: Run to verify it fails**

Run: `bash scripts/test_smoke_args.sh`
Expected: FAIL — smoke.sh doesn't yet parse `--canonical` (it errors "unknown argument") so the canonical assertions fail.

- [ ] **Step 3: Implement the flags in `scripts/smoke.sh`**

Add to the arg-parse loop (next to `--turnkey-client-version`):

```bash
    --canonical) CANONICAL=1; shift ;;
    --expected-pivot-hash)
      [ "$#" -ge 2 ] || { echo "--expected-pivot-hash requires a value" >&2; exit 2; }
      EXPECTED_PIVOT_HASH="$2"; shift 2 ;;
    --expected-pivot-hash=*) EXPECTED_PIVOT_HASH="${1#*=}"; shift ;;
```

Initialize defaults near the other vars:

```bash
CANONICAL="${VSP_SMOKE_CANONICAL:-0}"
EXPECTED_PIVOT_HASH="${VSP_SMOKE_EXPECTED_PIVOT_HASH:-}"
```

Build the verify args before the run:

```bash
verify_args=(verify --host "$HOST" --organization-id "$ORG" --key-name "$KEY" \
  --unsigned-payload "$PAYLOAD" --chain CHAIN_SOLANA)
[ "$CANONICAL" -eq 1 ] || verify_args+=(--dev-path)
[ -n "$EXPECTED_PIVOT_HASH" ] && verify_args+=(--expected-pivot-hash "$EXPECTED_PIVOT_HASH")
```

Replace the existing fixed invocation with:

```bash
OUT="$($CLIENT "${verify_args[@]}" 2>"$ERRFILE")"
```

Update the header usage block to document `--canonical` and `--expected-pivot-hash`.

- [ ] **Step 4: Run the new test + the existing harnesses**

Run:
```bash
bash scripts/test_smoke_args.sh
shellcheck scripts/smoke.sh scripts/test_smoke_args.sh
```
Expected: `ALL PASS`; shellcheck clean.

- [ ] **Step 5: Commit**

```bash
git add scripts/smoke.sh scripts/test_smoke_args.sh
git commit -m "feat: smoke.sh --canonical + --expected-pivot-hash (pin served binary)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Phase 4 — workflows

Repo: `visualsign-parser`, `.github/workflows/`. Validate each with `python3 -c "import yaml;yaml.safe_load(open('<file>'))"`.

### Task 7: Rename dev workflow to "TVC Deploy (Dev)"

**Files:**
- Rename: `.github/workflows/tvc-deploy.yml` -> `.github/workflows/tvc-deploy-dev.yml`
- Modify: the `name:` field.

- [ ] **Step 1: Rename + retitle**

```bash
git mv .github/workflows/tvc-deploy.yml .github/workflows/tvc-deploy-dev.yml
```
Change line 1 `name: TVC Deploy` to `name: TVC Deploy (Dev)`.

- [ ] **Step 2: Validate YAML**

Run: `python3 -c "import yaml;yaml.safe_load(open('.github/workflows/tvc-deploy-dev.yml'));print('ok')"`
Expected: `ok`.

- [ ] **Step 3: Commit**

```bash
git add -A .github/workflows/
git commit -m "ci(tools): rename TVC Deploy -> TVC Deploy (Dev)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### Task 8: Release workflow (prod, initiate-only)

**Files:** Create `.github/workflows/release.yml`

- [ ] **Step 1: Create the workflow**

```yaml
name: Release
# Manually-triggered PROD deploy INITIATE: digest gate + tvc deploy create.
# Holds only the Turnkey API key; a human operator approves out-of-band.
on:
  workflow_dispatch:
    inputs:
      app_id: { description: "Prod TVC app id", required: true }
      image_url: { description: "Pinned parser_app image, ghcr.io/...@sha256:...", required: true }
      expected_digest: { description: "Expected pivot binary sha256 (hex)", required: true }
      qos_version: { description: "QOS version", required: false, default: "v2026.2.6" }
      host_ip: { description: "parser_app listen IP", required: false, default: "0.0.0.0" }
      host_port: { description: "parser_app listen port", required: false, default: "3000" }
concurrency:
  group: release-${{ inputs.app_id }}
  cancel-in-progress: false
permissions:
  contents: read
jobs:
  initiate:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Install Rust
        uses: actions-rust-lang/setup-rust-toolchain@fb51252c7ba57d633bc668f941da052e410add48 # v1.13.0
      - name: Install tvc CLI
        run: cargo install tvc --version 0.7.0 --locked
      - name: Build deploy helper
        run: cargo build --release --manifest-path tools/tvc-deploy/Cargo.toml
      - name: Initiate
        env:
          TVC_ORG_ID: ${{ secrets.TVC_PROD_ORG_ID }}
          TVC_API_KEY_PUBLIC: ${{ secrets.TVC_PROD_API_KEY_PUBLIC }}
          TVC_API_KEY_PRIVATE: ${{ secrets.TVC_PROD_API_KEY_PRIVATE }}
          APP_ID: ${{ inputs.app_id }}
          IMAGE_URL: ${{ inputs.image_url }}
          EXPECTED_DIGEST: ${{ inputs.expected_digest }}
          QOS_VERSION: ${{ inputs.qos_version }}
          HOST_IP: ${{ inputs.host_ip }}
          HOST_PORT: ${{ inputs.host_port }}
        run: |
          ./tools/tvc-deploy/target/release/tvc-deploy initiate \
            --app-id "$APP_ID" --image-url "$IMAGE_URL" \
            --expected-digest "$EXPECTED_DIGEST" --qos-version "$QOS_VERSION" \
            --host-ip "$HOST_IP" --host-port "$HOST_PORT" | tee "$RUNNER_TEMP/initiate.out"
          echo "### Release initiated" >> "$GITHUB_STEP_SUMMARY"
          grep "created deployment" "$RUNNER_TEMP/initiate.out" >> "$GITHUB_STEP_SUMMARY" || true
          echo "Operator: approve with \`tvc-deploy approve --deploy-id <id> --operator-id <id> --image-url \"$IMAGE_URL\" --expected-digest \"$EXPECTED_DIGEST\"\` then run Promote." >> "$GITHUB_STEP_SUMMARY"
```

- [ ] **Step 2: Validate YAML + commit**

```bash
python3 -c "import yaml;yaml.safe_load(open('.github/workflows/release.yml'));print('ok')"
git add .github/workflows/release.yml
git commit -m "ci(tools): Release workflow (prod initiate-only)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### Task 9: Promote workflow (prod, set-live + pinned smoke)

**Files:** Create `.github/workflows/promote.yml`

- [ ] **Step 1: Create the workflow**

```yaml
name: Promote
# Manually-triggered PROD set-live for an already-approved deployment, then a
# pinned smoke against the canonical /visualsign endpoint. API key only.
on:
  workflow_dispatch:
    inputs:
      app_id: { description: "Prod TVC app id", required: true }
      deploy_id: { description: "Approved deployment id from the Release run", required: true }
      expected_digest: { description: "Pivot binary sha256 deployed (pins the smoke)", required: true }
      turnkey_client_version: { description: "turnkey-client image tag", required: false, default: "latest" }
concurrency:
  group: promote-${{ inputs.app_id }}
  cancel-in-progress: false
permissions:
  contents: read
  packages: read
jobs:
  promote:
    runs-on: ubuntu-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Install Rust
        uses: actions-rust-lang/setup-rust-toolchain@fb51252c7ba57d633bc668f941da052e410add48 # v1.13.0
      - name: Install tvc CLI
        run: cargo install tvc --version 0.7.0 --locked
      - name: Build deploy helper
        run: cargo build --release --manifest-path tools/tvc-deploy/Cargo.toml
      - name: Promote (set-live)
        env:
          TVC_ORG_ID: ${{ secrets.TVC_PROD_ORG_ID }}
          TVC_API_KEY_PUBLIC: ${{ secrets.TVC_PROD_API_KEY_PUBLIC }}
          TVC_API_KEY_PRIVATE: ${{ secrets.TVC_PROD_API_KEY_PRIVATE }}
          APP_ID: ${{ inputs.app_id }}
          DEPLOY_ID: ${{ inputs.deploy_id }}
        run: |
          ./tools/tvc-deploy/target/release/tvc-deploy promote \
            --app-id "$APP_ID" --deploy-id "$DEPLOY_ID"
      - name: Smoke (canonical, pinned)
        if: success()
        env:
          GHCR_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TVC_API_KEY_PUBLIC: ${{ secrets.TVC_PROD_API_KEY_PUBLIC }}
          TVC_API_KEY_PRIVATE: ${{ secrets.TVC_PROD_API_KEY_PRIVATE }}
          VSP_SMOKE_ORG: ${{ secrets.TVC_PROD_ORG_ID }}
          VSP_SMOKE_CLIENT_VERSION: ${{ inputs.turnkey_client_version || vars.TVC_SMOKE_CLIENT_VERSION || 'latest' }}
          EXPECTED_DIGEST: ${{ inputs.expected_digest }}
        run: |
          echo "$GHCR_TOKEN" | docker login ghcr.io -u "${{ github.actor }}" --password-stdin
          umask 077
          mkdir -p "$HOME/.config/turnkey/keys"
          printf '%s' "$TVC_API_KEY_PUBLIC"       > "$HOME/.config/turnkey/keys/dev.public"
          printf '%s:p256' "$TVC_API_KEY_PRIVATE" > "$HOME/.config/turnkey/keys/dev.private"
          ./scripts/smoke.sh --canonical --expected-pivot-hash "$EXPECTED_DIGEST"
      - name: Scrub secrets
        if: always()
        run: rm -rf "$HOME/.config/turnkey/keys"
```

- [ ] **Step 2: Validate YAML + commit**

```bash
python3 -c "import yaml;yaml.safe_load(open('.github/workflows/promote.yml'));print('ok')"
git add .github/workflows/promote.yml
git commit -m "ci(tools): Promote workflow (prod set-live + pinned smoke)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

### Task 10: Runbook — prod procedure

**Files:** Notion runbook (sub-page `38e5b28f-3091-81ba-b1f6-e2d6e641ddbb`). Not in-repo.

- [ ] **Step 1:** Append a "Prod release (initiate -> approve -> promote)" section: trigger Release (record the deploy id), operator approves with `tvc-deploy approve` using the **prod** operator seed (1Password item TBD), trigger Promote with the deploy id + expected_digest, confirm the pinned smoke PASS. Note CI never holds the operator seed.
- [ ] **Step 2:** No commit (Notion). Mention in the PR description that the runbook was updated.

---

## Self-Review

**Spec coverage:** initiate/approve/promote subcommands -> Tasks 2-5; dev `deploy` composition -> Task 2; `verify --expected-pivot-hash` + surfaced pivot hash -> Task 1; smoke canonical + pin -> Task 6; rename dev workflow -> Task 7; Release -> Task 8; Promote + canonical pinned smoke -> Task 9; separation of duties -> Tasks 8/9 (API-key-only env, no seed); runbook -> Task 10. All spec sections covered.

**Placeholder scan:** No TBD/TODO in steps; the prod org/app/operator ids are intentionally `secrets.TVC_PROD_*`/inputs per the Global Constraints (real values supplied at run time), and the 1Password prod item is an explicit open prerequisite from the spec, not a code placeholder.

**Type consistency:** `TvcOps` method names (`verify_image_digest`, `create`, `approve`, `poll_health`, `set_live`) are identical across Tasks 2-5 and the `RecordingTvc` fake. `initiate` returns `Result<String>`, consumed by `do_deploy`. `build_deploy_config` signature matches its test and `initiate` caller. Go `CheckExpectedPivotHash(*manifest.Manifest, string) error` matches its test and the `service.go` call site.
