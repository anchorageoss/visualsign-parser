//! Standalone TVC deploy helper for `parser_app`.
//!
//! Subcommands:
//!   gen-operator-key --out <path>
//!       Mint a qos_p256 operator key. Writes the 32-byte master seed (hex) to
//!       <path> with mode 0600 and prints ONLY the public key hex to stdout.
//!   deploy --app-id <id> --image-url <url> --expected-digest <hex>
//!          --operator-id <id> [--operator-seed <path>] [--qos-version v]
//!          [--host-ip 0.0.0.0] [--host-port 3000]
//!       Re-derive the pivot binary digest from the image and assert it matches
//!       --expected-digest, then assemble tvc-deploy.json (gRPC health), create
//!       the deployment, approve, poll until healthy, and set it live.
//!       The operator seed resolves flag -> env TVC_CI_OPERATOR_SEED -> none; when
//!       none is given, approval uses the logged-in org operator key (`tvc login`).
//!
//! All Turnkey API actions shell out to the `tvc` CLI (it owns auth/consensus);
//! this binary owns config assembly, the image-digest safety gate, and polling.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qos_p256::P256Pair;
use xshell::{cmd, Shell};

const POLL_TIMEOUT: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_secs(15);
const SETLIVE_TIMEOUT: Duration = Duration::from_secs(300);

const USAGE: &str = "usage:\n  \
    tvc-deploy gen-operator-key --out <path>\n  \
    tvc-deploy initiate --app-id <id> --image-url <url> --expected-digest <hex> \
    [--qos-version v2026.2.6] [--host-ip 0.0.0.0] [--host-port 3000]\n  \
    tvc-deploy deploy --app-id <id> --image-url <url> --expected-digest <hex> --operator-id <id> \
    [--operator-seed <path>] [--qos-version v2026.2.6] [--host-ip 0.0.0.0] [--host-port 3000]\n  \
    (operator seed may instead come from env TVC_CI_OPERATOR_SEED, or be omitted \
    to approve with the logged-in org operator key)";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let (subcmd, flags) = parse_args()?;
    let sh = Shell::new()?;
    match subcmd.as_str() {
        "gen-operator-key" => gen_operator_key(&flags),
        "initiate" => initiate(&RealTvc { sh: &sh }, &flags).map(|_| ()),
        "deploy" => do_deploy(&RealTvc { sh: &sh }, &flags),
        other => bail!("unknown subcommand {other:?}\n{USAGE}"),
    }
}

/// Parse `<subcommand> --key value ...` into the subcommand and a flag map.
fn parse_args() -> Result<(String, HashMap<String, String>)> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_env();
    let mut subcmd: Option<String> = None;
    let mut flags = HashMap::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Value(v) if subcmd.is_none() => subcmd = Some(v.string()?),
            Long(name) => {
                let name = name.to_owned();
                let val = parser
                    .value()
                    .with_context(|| format!("--{name} requires a value"))?
                    .string()?;
                flags.insert(name, val);
            }
            other => return Err(other.unexpected().into()),
        }
    }
    let subcmd = subcmd.ok_or_else(|| anyhow!("missing subcommand\n{USAGE}"))?;
    Ok((subcmd, flags))
}

fn req<'a>(flags: &'a HashMap<String, String>, key: &str) -> Result<&'a String> {
    flags.get(key).with_context(|| format!("missing --{key}"))
}

/// Trait abstracting all external TVC/Docker operations for testability.
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
        verify_image_digest(self.sh, image, expected)
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
        poll_health(self.sh, app_id, deploy_id, timeout)
    }
    fn set_live(&self, deploy_id: &str, timeout: Duration) -> Result<()> {
        set_live(self.sh, deploy_id, timeout)
    }
}

fn gen_operator_key(flags: &HashMap<String, String>) -> Result<()> {
    let out = req(flags, "out")?;
    let pair = P256Pair::generate().map_err(|e| anyhow!("key generation failed: {e:?}"))?;
    // qos_p256 owns the master-seed / pubkey hex formats.
    let seed_hex = String::from_utf8(pair.to_master_seed_hex()).context("seed hex not utf8")?;
    let pub_hex =
        String::from_utf8(pair.public_key().to_hex_bytes()).context("pubkey hex not utf8")?;
    write_secret_file(Path::new(out), &seed_hex)?;
    // SECURITY: only the public key is ever printed; the seed stays in the file.
    println!("{pub_hex}");
    eprintln!("operator seed written to {out} (mode 0600); public key printed above");
    Ok(())
}

fn write_secret_file(path: &Path, contents: &str) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    // mode() only applies when the file is newly created; force 0600 in case it
    // pre-existed with broader perms, so the secret is never world-readable.
    f.set_permissions(Permissions::from_mode(0o600))
        .with_context(|| format!("chmod {}", path.display()))?;
    f.write_all(contents.as_bytes())
        .with_context(|| format!("write {}", path.display()))
}

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

fn initiate(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<String> {
    let app_id = req(flags, "app-id")?;
    let image = req(flags, "image-url")?;
    let digest = req(flags, "expected-digest")?;
    let qos = flags
        .get("qos-version")
        .map(String::as_str)
        .unwrap_or("v2026.2.6");
    let host_ip = flags
        .get("host-ip")
        .map(String::as_str)
        .unwrap_or("0.0.0.0");
    let host_port: u16 = match flags.get("host-port") {
        Some(s) => s
            .parse()
            .with_context(|| format!("invalid --host-port: {s}"))?,
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

fn do_deploy(ops: &impl TvcOps, flags: &HashMap<String, String>) -> Result<()> {
    let app_id = req(flags, "app-id")?;
    let operator_id = req(flags, "operator-id")?;
    let seed = resolve_seed_file(flags)?;
    let deploy_id = initiate(ops, flags)?;
    let seed_path = seed.as_ref().map(|(p, _)| p.as_path());
    // Approve, then ALWAYS clean up an env-sourced seed temp file -- even if
    // approve failed -- before propagating, so the operator seed never leaks.
    let approved = ops.approve(&deploy_id, operator_id, seed_path);
    if let Some((path, true)) = &seed {
        let _ = std::fs::remove_file(path);
    }
    approved?;
    println!("approved manifest for {deploy_id}");
    ops.poll_health(app_id, &deploy_id, POLL_TIMEOUT)?;
    ops.set_live(&deploy_id, SETLIVE_TIMEOUT)?;
    println!("deployment {deploy_id} is healthy and live");
    Ok(())
}

/// Extract `/parser_app` from the image and sha256 it; it MUST equal the
/// submitted `--expected-digest`. Ties the deployed digest to the real binary.
fn verify_image_digest(sh: &Shell, image: &str, expected: &str) -> Result<()> {
    let cid = cmd!(sh, "docker create {image} /bin/true")
        .read()
        .context("docker create (digest gate)")?;
    let cid = cid.trim().to_owned();
    let bin = temp_path("parser_app", "bin");
    let target = format!("{cid}:/parser_app");
    // Extract + hash the pivot binary, then ALWAYS clean up the container and the
    // temp file regardless of where this fails (no leftover binary on error).
    let hashed = (|| -> Result<String> {
        cmd!(sh, "docker cp {target} {bin}")
            .run()
            .context("docker cp /parser_app")?;
        let sha = cmd!(sh, "sha256sum {bin}").read().context("sha256sum")?;
        Ok(sha.split_whitespace().next().unwrap_or_default().to_owned())
    })();
    let _ = cmd!(sh, "docker rm {cid}").ignore_status().quiet().run();
    let _ = std::fs::remove_file(&bin);
    let actual = hashed?;
    if !actual.eq_ignore_ascii_case(expected) {
        bail!(
            "DIGEST GATE FAILED: image /parser_app sha256 {actual} != expected {expected}; refusing to deploy"
        );
    }
    println!("digest gate passed: image /parser_app sha256 == {expected}");
    Ok(())
}

/// Set the deployment live, retrying while TVC reports the status is still
/// settling. A fresh app auto-targets its first deploy on approval, surfacing as
/// an "already" error -- treat that as success.
fn set_live(sh: &Shell, deploy_id: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        let out = cmd!(sh, "tvc app set-live-deploy --deploy-id {deploy_id}")
            .ignore_status()
            .output()
            .context("tvc app set-live-deploy")?;
        if out.status.success() {
            println!("set {deploy_id} live");
            return Ok(());
        }
        let msg = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .to_lowercase();
        if msg.contains("already") {
            println!("{deploy_id} already live (auto-targeted)");
            return Ok(());
        }
        let transient = msg.contains("not yet available")
            || msg.contains("try again")
            || msg.contains("not found")
            || msg.contains("zero healthy replicas");
        if transient && start.elapsed() < timeout {
            sleep(POLL_INTERVAL);
            continue;
        }
        bail!("set-live failed: {}", msg.trim());
    }
}

fn poll_health(sh: &Shell, app_id: &str, deploy_id: &str, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    let mut last = String::new();
    loop {
        // Status can fail transiently right after set-live while the app
        // registers; keep polling through errors until timeout.
        let status = cmd!(sh, "tvc app status --app-id {app_id}")
            .ignore_status()
            .quiet()
            .read();
        if let Ok(out) = status {
            if let Some(ratio) = deployment_health(&out, deploy_id) {
                if ratio != last {
                    println!("  {deploy_id}: {ratio}");
                    last = ratio.clone();
                }
                if let Some((h, d)) = ratio.split_once('/') {
                    if h == d && h != "0" {
                        return Ok(());
                    }
                }
            }
        }
        if start.elapsed() >= timeout {
            bail!(
                "timed out after {}s waiting for {deploy_id} to be healthy (last: {})",
                timeout.as_secs(),
                if last.is_empty() { "unknown" } else { &last }
            );
        }
        sleep(POLL_INTERVAL);
    }
}

/// From `tvc app status` output, the `Healthy / Desired Replicas: X/Y` ratio for
/// `deploy_id`'s block.
fn deployment_health(status: &str, deploy_id: &str) -> Option<String> {
    let mut in_block = false;
    for line in status.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Deployment:") {
            in_block = rest.trim() == deploy_id;
        } else if in_block {
            if let Some(rest) = t.strip_prefix("Healthy / Desired Replicas:") {
                return rest.split_whitespace().next().map(str::to_owned);
            }
        }
    }
    None
}

fn validate_digest(d: &str) -> Result<()> {
    if d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        bail!("--expected-digest must be 64 hex chars (sha256), got {d:?}");
    }
}

/// Resolve the operator seed to a file path, returning `(path, cleanup)` or
/// `None`. Prefers `--operator-seed <path>`; else reads the hex seed from env
/// `TVC_CI_OPERATOR_SEED` into a temp 0600 file (cleanup=true so the caller
/// deletes it); if neither is set, returns `None` and approval falls back to the
/// logged-in org operator key.
fn resolve_seed_file(flags: &HashMap<String, String>) -> Result<Option<(PathBuf, bool)>> {
    if let Some(p) = flags.get("operator-seed") {
        return Ok(Some((PathBuf::from(p), false)));
    }
    match std::env::var("TVC_CI_OPERATOR_SEED") {
        Ok(seed) => {
            let p = temp_path("tvc-operator", "seed");
            write_secret_file(&p, seed.trim())?;
            Ok(Some((p, true)))
        }
        Err(_) => Ok(None),
    }
}

/// Trimmed remainder of the first line containing `marker`.
fn parse_after(haystack: &str, marker: &str) -> Option<String> {
    haystack.lines().find_map(|line| {
        line.find(marker)
            .map(|i| line[i + marker.len()..].trim().to_owned())
            .filter(|s| !s.is_empty())
    })
}

fn temp_path(prefix: &str, ext: &str) -> PathBuf {
    // PID + timestamp + a per-process counter so repeated calls within one clock
    // tick can't collide (the timestamp alone is coarse on some VMs).
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{nanos}-{seq}.{ext}",
        std::process::id()
    ))
}

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
            self.calls
                .borrow_mut()
                .push(format!("set_live:{deploy_id}"));
            Ok(())
        }
    }

    fn flags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn config_has_grpc_health_and_digest() {
        let cfg = build_deploy_config(
            "app",
            "v1",
            "img",
            "0.0.0.0",
            3000,
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        );
        assert_eq!(cfg["healthCheckType"], "TVC_HEALTH_CHECK_TYPE_GRPC");
        assert_eq!(
            cfg["expectedPivotDigest"],
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        );
        assert_eq!(cfg["pivotPath"], "/parser_app");
        assert_eq!(cfg["healthCheckPort"], 3000);
    }

    #[test]
    fn deploy_runs_gate_create_approve_poll_setlive_in_order() {
        let ops = RecordingTvc::default();
        let f = flags(&[
            ("app-id", "app"),
            ("image-url", "img"),
            (
                "expected-digest",
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            ),
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

    #[test]
    fn initiate_runs_only_gate_and_create() {
        let ops = RecordingTvc::default();
        let f = flags(&[
            ("app-id", "app"),
            ("image-url", "img"),
            (
                "expected-digest",
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            ),
        ]);
        let id = initiate(&ops, &f).unwrap();
        assert_eq!(id, "deploy-123");
        assert_eq!(*ops.calls.borrow(), vec!["verify_image_digest", "create"]);
    }

    #[test]
    fn initiate_rejects_bad_digest() {
        let ops = RecordingTvc::default();
        let f = flags(&[
            ("app-id", "a"),
            ("image-url", "i"),
            ("expected-digest", "xyz"),
        ]);
        assert!(initiate(&ops, &f).is_err());
        assert!(ops.calls.borrow().is_empty());
    }

    fn leftover_operator_seeds() -> usize {
        std::fs::read_dir(std::env::temp_dir())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with("tvc-operator-"))
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn deploy_cleans_env_seed_when_approve_fails() {
        struct FailingApprove;
        impl TvcOps for FailingApprove {
            fn verify_image_digest(&self, _i: &str, _e: &str) -> Result<()> {
                Ok(())
            }
            fn create(&self, _c: &Path) -> Result<String> {
                Ok("deploy-1".into())
            }
            fn approve(&self, _d: &str, _o: &str, seed: Option<&Path>) -> Result<()> {
                assert!(
                    seed.map(Path::exists).unwrap_or(false),
                    "seed must exist at approve"
                );
                bail!("approve boom")
            }
            fn poll_health(&self, _a: &str, _d: &str, _t: Duration) -> Result<()> {
                panic!("poll_health must not run after approve failure")
            }
            fn set_live(&self, _d: &str, _t: Duration) -> Result<()> {
                panic!("set_live must not run after approve failure")
            }
        }
        let digest = "a".repeat(64);
        let f = flags(&[
            ("app-id", "app"),
            ("image-url", "img"),
            ("expected-digest", &digest),
            ("operator-id", "op"),
        ]);
        let before = leftover_operator_seeds();
        // SAFETY: this is the only test that touches this env var.
        unsafe {
            std::env::set_var("TVC_CI_OPERATOR_SEED", "00".repeat(32));
        }
        let result = do_deploy(&FailingApprove, &f);
        unsafe {
            std::env::remove_var("TVC_CI_OPERATOR_SEED");
        }
        assert!(result.is_err(), "approve failure should propagate");
        assert_eq!(
            before,
            leftover_operator_seeds(),
            "env-sourced seed leaked on approve failure"
        );
    }
}
