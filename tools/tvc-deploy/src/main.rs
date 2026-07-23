//! Standalone TVC deploy + Turnkey org-management helper for `parser_app`.
//!
//! `deploy` re-derives the pivot binary digest from the image and asserts it
//! matches `--expected-digest`, then assembles tvc-deploy.json (gRPC health),
//! creates the deployment, approves, polls until healthy, and sets it live.
//! The operator seed resolves flag -> env `TVC_CI_OPERATOR_SEED` -> none; when
//! none is given, approval uses the logged-in org operator key (`tvc login`).
//!
//! See `tvc-deploy --help` for the full subcommand list (invite/dismiss-invite,
//! activity approve/reject, tag and policy CRUD -- all in `invite.rs`).
//!
//! Deploy's Turnkey API actions shell out to the `tvc` CLI (it owns
//! auth/consensus); this binary owns config assembly, the image-digest safety
//! gate, and polling. The `invite`/tag/policy subcommands call the Turnkey API
//! directly instead (see `invite.rs`'s module doc).

use std::ffi::OsString;
use std::fs::{OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use qos_p256::P256Pair;
use xshell::{cmd, Shell};

mod invite;

const POLL_TIMEOUT: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_secs(15);
const SETLIVE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Parser)]
#[command(
    name = "tvc-deploy",
    about = "TVC deploy + Turnkey org-management helper for parser_app"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mint a qos_p256 operator key: writes the seed to --out (mode 0600), prints only the public key
    GenOperatorKey(GenOperatorKeyArgs),
    /// Deploy parser_app: digest-gate, create, approve, poll healthy, set live
    Deploy(DeployArgs),
    /// Delete a single deployment by id (consensus via approve-activity)
    DeleteDeployment(invite::DeleteDeploymentArgs),
    /// Prune old deployments for an app, keeping the live one + newest --keep
    Prune(invite::PruneArgs),
    /// Invite one person, or a batch from --file (see README)
    Invite(invite::InviteArgs),
    /// Delete an existing invitation
    DismissInvite(invite::DismissInviteArgs),
    /// List an org's invitations (pending/accepted/revoked)
    ListInvitations(invite::OrgArgs),
    /// Approve a Turnkey activity that needs consensus
    ApproveActivity(invite::ActivityIdArgs),
    /// Reject a Turnkey activity that needs consensus
    RejectActivity(invite::ActivityIdArgs),
    /// List an org's activities, newest first, with optional status/type filters
    ListActivities(invite::ListActivitiesArgs),
    /// Decode a single activity's intent + votes into a human-readable summary
    ViewActivity(invite::ActivityIdArgs),
    /// Create a user tag, optionally seeding it with existing user ids
    CreateTag(invite::CreateTagArgs),
    /// Add/remove existing users from a tag, or rename it
    UpdateTag(invite::UpdateTagArgs),
    /// List user tags (id + name)
    ListTags(invite::OrgArgs),
    /// List org users (id + name + email)
    ListUsers(invite::OrgArgs),
    /// List policies (id, name, effect, notes, condition, consensus)
    ListPolicies(invite::OrgArgs),
    /// Create a single policy
    CreatePolicy(invite::CreatePolicyArgs),
    /// Create a batch of policies from a template, with {{PLACEHOLDER}} substitution
    CreatePolicies(invite::CreatePoliciesArgs),
}

#[derive(clap::Args)]
struct GenOperatorKeyArgs {
    /// Path to write the operator's 32-byte master seed (hex), mode 0600
    #[arg(long)]
    out: PathBuf,
}

#[derive(clap::Args)]
struct DeployArgs {
    #[arg(long)]
    app_id: String,
    #[arg(long)]
    image_url: String,
    /// Expected sha256 of the image's /parser_app binary (64 hex chars)
    #[arg(long)]
    expected_digest: String,
    #[arg(long)]
    operator_id: String,
    /// Path to the operator seed file; falls back to env TVC_CI_OPERATOR_SEED,
    /// then to the logged-in org operator key, if omitted
    #[arg(long)]
    operator_seed: Option<PathBuf>,
    #[arg(long, default_value = "0.12.0")]
    qos_version: String,
    #[arg(long, default_value = "0.0.0.0")]
    host_ip: String,
    #[arg(long, default_value_t = 3000)]
    host_port: u16,
    /// Skip the check for an existing pending deploy activity for this app-id
    #[arg(long)]
    force: bool,
    #[command(flatten)]
    org: invite::OrgArgs,
}

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
    let cli = Cli::parse();
    let sh = Shell::new()?;
    match cli.command {
        Command::GenOperatorKey(args) => gen_operator_key(&args),
        Command::Deploy(args) => deploy(&sh, &args),
        Command::DeleteDeployment(args) => invite::delete_deployment(&args),
        Command::Prune(args) => invite::prune(&sh, &args),
        Command::Invite(args) => invite::invite(&args),
        Command::DismissInvite(args) => invite::dismiss_invite(&args),
        Command::ListInvitations(args) => invite::list_invitations(&args),
        Command::ApproveActivity(args) => invite::approve_activity(&args),
        Command::RejectActivity(args) => invite::reject_activity(&args),
        Command::ListActivities(args) => invite::list_activities(&args),
        Command::ViewActivity(args) => invite::view_activity(&args),
        Command::CreateTag(args) => invite::create_tag(&args),
        Command::UpdateTag(args) => invite::update_tag(&args),
        Command::ListTags(args) => invite::list_tags(&args),
        Command::ListUsers(args) => invite::list_users(&args),
        Command::ListPolicies(args) => invite::list_policies(&args),
        Command::CreatePolicy(args) => invite::create_policy(&args),
        Command::CreatePolicies(args) => invite::create_policies(&args),
    }
}

fn gen_operator_key(args: &GenOperatorKeyArgs) -> Result<()> {
    let pair = P256Pair::generate().map_err(|e| anyhow::anyhow!("key generation failed: {e:?}"))?;
    // qos_p256 owns the master-seed / pubkey hex formats.
    let seed_hex = String::from_utf8(pair.to_master_seed_hex()).context("seed hex not utf8")?;
    let pub_hex =
        String::from_utf8(pair.public_key().to_hex_bytes()).context("pubkey hex not utf8")?;
    write_secret_file(&args.out, &seed_hex)?;
    // SECURITY: only the public key is ever printed; the seed stays in the file.
    println!("{pub_hex}");
    eprintln!(
        "operator seed written to {} (mode 0600); public key printed above",
        args.out.display()
    );
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

fn deploy(sh: &Shell, args: &DeployArgs) -> Result<()> {
    validate_digest(&args.expected_digest)?;

    if !args.force {
        // Turnkey has no dedup for create_tvc_deployment: submitting the same
        // deploy twice while the first is still ConsensusNeeded creates a
        // second, independent activity instead of reusing it (see README).
        let pending = invite::find_pending_deployments(args.org.as_deref(), &args.app_id)?;
        if !pending.is_empty() {
            let ids: Vec<&str> = pending.iter().map(|a| a.id.as_str()).collect();
            bail!(
                "app {} already has {} deployment activity(ies) awaiting consensus: {}\n\
                 approve or reject the existing one first (tvc-deploy approve-activity / \
                 reject-activity --activity-id <id>), or pass --force to submit anyway",
                args.app_id,
                ids.len(),
                ids.join(", ")
            );
        }
    }

    // Safety gate: re-derive the pivot binary digest from the image and confirm
    // it matches --expected-digest, tying the submitted digest to the real binary.
    verify_image_digest(sh, &args.image_url, &args.expected_digest)?;

    let seed = resolve_seed_file(args.operator_seed.as_deref())?;
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
    let (app_id, image, digest, operator_id, qos, host_ip, host_port) = (
        &args.app_id,
        &args.image_url,
        &args.expected_digest,
        &args.operator_id,
        &args.qos_version,
        &args.host_ip,
        args.host_port,
    );

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
fn resolve_seed_file(operator_seed: Option<&Path>) -> Result<Option<(PathBuf, bool)>> {
    if let Some(p) = operator_seed {
        return Ok(Some((p.to_path_buf(), false)));
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
    use clap::CommandFactory;

    #[test]
    fn cli_parses_all_subcommands() {
        Cli::command().debug_assert();
    }

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
