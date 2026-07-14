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

mod invite;

const POLL_TIMEOUT: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_secs(15);
const SETLIVE_TIMEOUT: Duration = Duration::from_secs(300);

const USAGE: &str = "usage:\n  \
    tvc-deploy gen-operator-key --out <path>\n  \
    tvc-deploy deploy --app-id <id> --image-url <url> --expected-digest <hex> --operator-id <id> \
    [--operator-seed <path>] [--qos-version 0.12.0] [--host-ip 0.0.0.0] [--host-port 3000]\n  \
    (operator seed may instead come from env TVC_CI_OPERATOR_SEED, or be omitted \
    to approve with the logged-in org operator key)\n  \
    ";

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
        "deploy" => deploy(&sh, &flags),
        "invite" => invite::invite(&flags),
        "dismiss-invite" => invite::dismiss_invite(&flags),
        "approve-activity" => invite::approve_activity(&flags),
        "reject-activity" => invite::reject_activity(&flags),
        "list-tags" => invite::list_tags(&flags),
        "list-policies" => invite::list_policies(&flags),
        "create-policy" => invite::create_policy(&flags),
        "create-policies" => invite::create_policies(&flags),
        other => bail!("unknown subcommand {other:?}\n{USAGE}\n{}", invite::USAGE),
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
    let subcmd = subcmd.ok_or_else(|| anyhow!("missing subcommand\n{USAGE}\n{}", invite::USAGE))?;
    Ok((subcmd, flags))
}

fn req<'a>(flags: &'a HashMap<String, String>, key: &str) -> Result<&'a String> {
    flags.get(key).with_context(|| format!("missing --{key}"))
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

fn deploy(sh: &Shell, flags: &HashMap<String, String>) -> Result<()> {
    let app_id = req(flags, "app-id")?;
    let image = req(flags, "image-url")?;
    let digest = req(flags, "expected-digest")?;
    let operator_id = req(flags, "operator-id")?;
    let qos = flags
        .get("qos-version")
        .map(String::as_str)
        .unwrap_or("0.12.0");
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
    // Safety gate: re-derive the pivot binary digest from the image and confirm
    // it matches --expected-digest, tying the submitted digest to the real binary.
    verify_image_digest(sh, image, digest)?;

    let seed = resolve_seed_file(flags)?;
    // Pass --operator-seed only when we have one; otherwise tvc approves with the
    // logged-in org operator key (the local `tvc login` path).
    let seed_args: Vec<OsString> = match &seed {
        Some((path, _)) => vec!["--operator-seed".into(), path.clone().into_os_string()],
        None => {
            println!("no operator seed provided; approving with the logged-in org operator key");
            Vec::new()
        }
    };
    let cfg_path = temp_path("tvc-deploy", "json");

    // Everything that can fail after the seed file exists runs inside this
    // closure, so the seed + config temp files are always cleaned up below
    // (otherwise an early `?` would leave the operator seed on disk).
    let outcome = (|| -> Result<String> {
        // Assemble the deployment config (gRPC health is mandatory for parser_app).
        let cfg = serde_json::json!({
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
        });
        std::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg)?)
            .with_context(|| format!("write {}", cfg_path.display()))?;

        let created = cmd!(sh, "tvc deploy create --config-file {cfg_path}")
            .read()
            .context("tvc deploy create")?;
        let deploy_id = parse_after(&created, "Deployment ID:")
            .with_context(|| format!("no deployment id in create output:\n{created}"))?;
        println!("created deployment {deploy_id}");

        cmd!(sh, "tvc deploy approve --deploy-id {deploy_id} --operator-id {operator_id} {seed_args...} --dangerous-skip-interactive")
            .run()
            .context("tvc deploy approve")?;
        println!("approved manifest for {deploy_id}");

        // TVC refuses to target a deployment with zero healthy replicas, so poll
        // to healthy BEFORE set-live. A fresh app auto-targets its first deploy.
        poll_health(sh, app_id, &deploy_id, POLL_TIMEOUT)?;
        set_live(sh, &deploy_id, SETLIVE_TIMEOUT)?;
        Ok(deploy_id)
    })();

    if let Some((path, true)) = &seed {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_file(&cfg_path);

    let deploy_id = outcome?;
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
/// an "already live" error -- treat that as success. Requires both "already"
/// and "live" in the message (not a bare "already" substring) so an unrelated
/// failure that happens to contain "already" (e.g. a retry-exhaustion message)
/// isn't misreported as success.
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
        if msg.contains("already") && msg.contains("live") {
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

    #[test]
    fn deployment_health_reads_ratio_for_matching_deployment() {
        let status = "\
Deployment: deploy-other
  Healthy / Desired Replicas: 0/3
Deployment: deploy-123
  Healthy / Desired Replicas: 2/3
Deployment: deploy-another
  Healthy / Desired Replicas: 5/5
";
        assert_eq!(
            deployment_health(status, "deploy-123"),
            Some("2/3".to_owned())
        );
    }

    #[test]
    fn deployment_health_returns_none_for_unknown_deployment() {
        let status = "Deployment: deploy-123\n  Healthy / Desired Replicas: 2/3\n";
        assert_eq!(deployment_health(status, "deploy-999"), None);
    }

    #[test]
    fn deployment_health_returns_none_when_ratio_line_missing() {
        let status = "Deployment: deploy-123\n  Some other field: x\n";
        assert_eq!(deployment_health(status, "deploy-123"), None);
    }

    #[test]
    fn deployment_health_ignores_ratio_lines_outside_the_matching_block() {
        // A "Healthy / Desired Replicas" line for a different deployment must not
        // leak into the block for the one we're looking for.
        let status = "\
Healthy / Desired Replicas: 9/9
Deployment: deploy-123
  Healthy / Desired Replicas: 1/2
";
        assert_eq!(
            deployment_health(status, "deploy-123"),
            Some("1/2".to_owned())
        );
    }

    #[test]
    fn validate_digest_accepts_64_hex_chars() {
        assert!(validate_digest(&"a".repeat(64)).is_ok());
        assert!(validate_digest(&"F".repeat(64)).is_ok());
    }

    #[test]
    fn validate_digest_rejects_wrong_length_or_non_hex() {
        assert!(validate_digest(&"a".repeat(63)).is_err());
        assert!(validate_digest(&"a".repeat(65)).is_err());
        assert!(validate_digest(&("g".repeat(64))).is_err());
        assert!(validate_digest("").is_err());
    }

    #[test]
    fn parse_after_finds_trimmed_remainder_of_first_matching_line() {
        let out = "some preamble\nDeployment ID: deploy-123\nmore text";
        assert_eq!(
            parse_after(out, "Deployment ID:"),
            Some("deploy-123".to_owned())
        );
    }

    #[test]
    fn parse_after_returns_none_when_marker_missing_or_value_empty() {
        assert_eq!(parse_after("no marker here", "Deployment ID:"), None);
        assert_eq!(parse_after("Deployment ID:   \n", "Deployment ID:"), None);
    }
}
