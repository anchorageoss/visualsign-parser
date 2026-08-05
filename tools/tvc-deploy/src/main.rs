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
    /// Run only the digest gate: extract /parser_app from the image and compare its sha256
    VerifyDigest(VerifyDigestArgs),
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
// The deployed parser's ABI trust posture. Exactly one of the two flags below must
// be given: they become `pivotArgs` in the manifest the operators sign, so the
// posture the enclave runs is fixed at deploy time and auditable out of band
// instead of being implied by what each request happens to contain (PRS-556).
#[command(group(
    clap::ArgGroup::new("abi_trust").required(true).multiple(false)
))]
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
    /// Deploy a parser that accepts caller-supplied ABI mappings with no signature
    /// (integrity and provenance unverified)
    #[arg(long, group = "abi_trust")]
    accept_unsigned_abis: bool,
    /// Deploy a parser that only accepts caller-supplied ABI mappings signed by this
    /// hex secp256k1 public key. Repeatable
    #[arg(long, group = "abi_trust", value_name = "HEX_PUBKEY")]
    accept_signatures_from_pubkey: Vec<String>,
    /// Skip the check for an existing pending deploy activity for this app-id
    #[arg(long)]
    force: bool,
    #[command(flatten)]
    org: invite::OrgArgs,
}

#[derive(clap::Args)]
struct VerifyDigestArgs {
    #[arg(long)]
    image_url: String,
    /// Expected sha256 of the image's /parser_app binary (64 hex chars)
    #[arg(long)]
    expected_digest: String,
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
        Command::VerifyDigest(args) => verify_digest(&sh, &args),
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
    for key in &args.accept_signatures_from_pubkey {
        validate_signer_pubkey(key)?;
    }

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
    let (app_id, image, digest, operator_id, qos, host_port) = (
        &args.app_id,
        &args.image_url,
        &args.expected_digest,
        &args.operator_id,
        &args.qos_version,
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
            "pivotArgs": pivot_args(args),
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

/// Build the pivot cmdline the manifest commits to.
///
/// The ABI trust flags are part of it on purpose: a signer verifying a deployment
/// reads the posture straight off `pivotArgs` instead of trusting a per-request
/// signal or parser logs (which Turnkey does not surface today). Clap's arg group
/// guarantees exactly one of the two is set, so this always appends one posture.
fn pivot_args(args: &DeployArgs) -> Vec<String> {
    let mut pivot = vec![
        "--host-ip".to_string(),
        args.host_ip.to_string(),
        "--host-port".to_string(),
        args.host_port.to_string(),
    ];
    if args.accept_unsigned_abis {
        pivot.push("--accept-unsigned-abis".to_string());
    }
    for key in &args.accept_signatures_from_pubkey {
        pivot.push("--accept-signatures-from-pubkey".to_string());
        pivot.push(key.clone());
    }
    pivot
}

/// Standalone digest gate, for callers that must record the expected digest
/// somewhere else before `deploy` runs. `deploy`'s own gate only fires once it
/// is running, too late to stop a wrong digest being committed elsewhere first.
/// Same check, same message, one implementation.
fn verify_digest(sh: &Shell, args: &VerifyDigestArgs) -> Result<()> {
    validate_digest(&args.expected_digest)?;
    verify_image_digest(sh, &args.image_url, &args.expected_digest)
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

/// Two-phase validation of a hex signer pubkey.
///
/// Phase 1 (format): rejects inputs that aren't even well-formed hex SEC1.
/// Accepts 33-byte compressed (02/03/05 prefix) and 65-byte uncompressed (04
/// prefix).  05 is SEC1's "compact" tag: same 33-byte length as compressed, but
/// the y-coordinate is derived rather than carried, and `canonical_pubkey_from_hex`
/// (what `parser_app` actually runs on `--accept-signatures-from-pubkey`) accepts
/// it same as 02/03/04, so it must not be rejected here.
///
/// Phase 2 (on-curve): decodes the bytes through `k256::PublicKey::from_sec1_bytes`
/// to confirm the point is actually on the secp256k1 curve.  Catching an off-curve
/// key here, at deploy time, avoids burning a consensus round on an enclave that
/// would immediately fail the same decode at startup and never report healthy.
fn validate_signer_pubkey(hex_str: &str) -> Result<()> {
    let stripped = hex_str
        .strip_prefix("0x")
        .or_else(|| hex_str.strip_prefix("0X"))
        .unwrap_or(hex_str);
    let valid_len = matches!(stripped.len(), 66 | 130);
    let valid_prefix = match stripped.len() {
        66 => {
            stripped.starts_with("02") || stripped.starts_with("03") || stripped.starts_with("05")
        }
        130 => stripped.starts_with("04"),
        _ => false,
    };
    if !(valid_len && valid_prefix && stripped.bytes().all(|b| b.is_ascii_hexdigit())) {
        bail!(
            "--accept-signatures-from-pubkey must be a 33-byte (02/03/05-prefixed) or \
             65-byte (04-prefixed) hex secp256k1 public key, got {}",
            truncate_for_error(hex_str)
        );
    }

    // Format alone is not enough: a well-formed hex string can still fail to decode
    // to a point on the curve. parser_app decodes it for real at startup, so catching
    // it here is the difference between a local error and a burned quorum round.
    //
    // k256::PublicKey::from_sec1_bytes accepts all four SEC1 tags: 02/03 (compressed),
    // 04 (uncompressed), and 05 (compact — same 33-byte length as compressed, with the
    // y-coordinate derived rather than carried). Tested explicitly in
    // `validate_signer_pubkey_compact_through_k256` below; if a future k256 release ever
    // drops 05 support, that test will catch it before a deployment reaches the enclave.
    let bytes = decode_hex_bytes(stripped)?;
    let key_len = bytes.len();
    if k256::PublicKey::from_sec1_bytes(&bytes).is_err() {
        let tag = if key_len == 33 {
            match bytes.first() {
                Some(0x02) => "02 (compressed)",
                Some(0x03) => "03 (compressed)",
                Some(0x05) => "05 (compact)",
                _ => "unknown",
            }
        } else {
            "04 (uncompressed)"
        };
        bail!(
            "--accept-signatures-from-pubkey is well-formed hex (SEC1 {tag}, {key_len} bytes) \
             but does not decode to a point on the secp256k1 curve, got {}",
            truncate_for_error(hex_str)
        );
    }
    Ok(())
}

/// Decode an even-length ASCII hex string that has already been charset-checked.
fn decode_hex_bytes(stripped: &str) -> Result<Vec<u8>> {
    (0..stripped.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&stripped[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("invalid hex byte at offset {i}: {e}"))
        })
        .collect()
}

/// Bound an operator-supplied value echoed back in an error, so a mistaken paste of
/// a whole file does not land verbatim in CI logs.
fn truncate_for_error(value: &str) -> String {
    const MAX: usize = 64;
    if value.chars().count() <= MAX {
        format!("{value:?}")
    } else {
        // Truncate on char boundaries: the value is operator-supplied and need not be ASCII.
        let head: String = value.chars().take(MAX).collect();
        format!(
            "{head:?} (truncated, {} chars total)",
            value.chars().count()
        )
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

    /// Parse a `deploy` invocation, returning just its args.
    fn deploy_args(extra: &[&str]) -> DeployArgs {
        let digest = "a".repeat(64);
        let base = [
            "tvc-deploy",
            "deploy",
            "--app-id",
            "app",
            "--image-url",
            "img",
            "--expected-digest",
            &digest,
            "--operator-id",
            "op",
        ];
        let argv: Vec<String> = base
            .iter()
            .map(|s| (*s).to_string())
            .chain(extra.iter().map(|s| (*s).to_string()))
            .collect();
        match Cli::parse_from(argv).command {
            Command::Deploy(args) => args,
            _ => panic!("expected the deploy subcommand"),
        }
    }

    /// Parse a `deploy` invocation expected to fail, returning clap's error kind.
    fn deploy_error_kind(extra: &[&str]) -> clap::error::ErrorKind {
        let digest = "a".repeat(64);
        let base = [
            "tvc-deploy",
            "deploy",
            "--app-id",
            "app",
            "--image-url",
            "img",
            "--expected-digest",
            &digest,
            "--operator-id",
            "op",
        ];
        let argv: Vec<String> = base
            .iter()
            .map(|s| (*s).to_string())
            .chain(extra.iter().map(|s| (*s).to_string()))
            .collect();
        Cli::try_parse_from(argv)
            .map(|_| ())
            .expect_err("these args must not parse")
            .kind()
    }

    /// The permissive posture is appended to the pivot cmdline the manifest commits
    /// to, so a signer can read the posture straight off the deployment.
    #[test]
    fn pivot_args_carry_accept_unsigned() {
        let args = deploy_args(&["--accept-unsigned-abis"]);
        assert_eq!(
            pivot_args(&args),
            vec![
                "--host-ip",
                "0.0.0.0",
                "--host-port",
                "3000",
                "--accept-unsigned-abis"
            ]
        );
    }

    /// Every signer key is carried through, in order, each with its own flag.
    #[test]
    fn pivot_args_carry_every_signer_pubkey() {
        let args = deploy_args(&[
            "--accept-signatures-from-pubkey",
            "04aa",
            "--accept-signatures-from-pubkey",
            "04bb",
        ]);
        assert_eq!(
            pivot_args(&args),
            vec![
                "--host-ip",
                "0.0.0.0",
                "--host-port",
                "3000",
                "--accept-signatures-from-pubkey",
                "04aa",
                "--accept-signatures-from-pubkey",
                "04bb"
            ]
        );
    }

    /// A deploy with no posture is rejected, so `pivot_args` can never emit a
    /// cmdline `parser_app` would refuse to start on.
    #[test]
    fn deploy_requires_a_posture() {
        assert_eq!(
            deploy_error_kind(&[]),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    /// The two postures are mutually exclusive.
    #[test]
    fn deploy_rejects_both_postures() {
        assert_eq!(
            deploy_error_kind(&[
                "--accept-unsigned-abis",
                "--accept-signatures-from-pubkey",
                "04aa",
            ]),
            clap::error::ErrorKind::ArgumentConflict
        );
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

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A real secp256k1 public key, in the requested SEC1 encoding. Derived rather
    /// than hardcoded so the fixtures stay on the curve now that `validate_signer_pubkey`
    /// decodes them for real.
    fn real_pubkey_hex(compressed: bool) -> String {
        use k256::elliptic_curve::sec1::ToEncodedPoint;
        let sk = k256::SecretKey::from_slice(&[0x42u8; 32]).expect("valid scalar");
        hex_of(sk.public_key().to_encoded_point(compressed).as_bytes())
    }

    /// SEC1 "compact" form: `0x05 || x`. Built from a real key's x-coordinate, which
    /// is by construction an x that has a square root.
    fn real_compact_pubkey_hex() -> String {
        let uncompressed = real_pubkey_hex(false);
        // Skip the "04" tag, keep the 32-byte x coordinate.
        format!("05{}", &uncompressed[2..66])
    }

    #[test]
    fn validate_signer_pubkey_accepts_compressed_and_uncompressed() {
        let compressed = real_pubkey_hex(true);
        assert!(validate_signer_pubkey(&compressed).is_ok());
        assert!(validate_signer_pubkey(&real_pubkey_hex(false)).is_ok());
        // 0x-prefixed, case-insensitive.
        assert!(
            validate_signer_pubkey(&format!("0x{}", real_pubkey_hex(false).to_uppercase())).is_ok()
        );
        // SEC1 "compact" tag: same 33-byte length as compressed, and accepted by
        // `canonical_pubkey_from_hex` (what parser_app actually runs), so a false
        // rejection here would block a legitimate deployment.
        assert!(validate_signer_pubkey(&real_compact_pubkey_hex()).is_ok());
    }

    /// Prove `k256::PublicKey::from_sec1_bytes` accepts SEC1 "compact" form (05
    /// prefix). The format-only check passes 05 through; this test ensures the
    /// downstream k256 decode the error message names does the same. If a future
    /// k256 release ever drops compact support, this test will catch it before the
    /// error message becomes misleading.
    #[test]
    fn validate_signer_pubkey_compact_through_k256() {
        let compact = real_compact_pubkey_hex();
        let bytes = (0..compact.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&compact[i..i + 2], 16).expect("valid hex"))
            .collect::<Vec<u8>>();
        assert_eq!(bytes[0], 0x05, "compact tag");
        assert_eq!(bytes.len(), 33, "compact is 33 bytes");
        k256::PublicKey::from_sec1_bytes(&bytes)
            .expect("k256 must accept SEC1 compact (05) form");
    }

    #[test]
    fn validate_signer_pubkey_rejects_truncated_or_malformed() {
        // The exact truncated shape used elsewhere in this file's own tests.
        assert!(validate_signer_pubkey("04aa").is_err());
        assert!(validate_signer_pubkey("").is_err());
        assert!(validate_signer_pubkey(&format!("06{}", "a".repeat(64))).is_err());
        assert!(validate_signer_pubkey(&format!("02{}", "g".repeat(64))).is_err());
        // Wrong length for its prefix (compressed prefix, uncompressed length).
        assert!(validate_signer_pubkey(&format!("02{}", "a".repeat(128))).is_err());
    }

    #[test]
    fn validate_signer_pubkey_rejects_well_formed_hex_that_is_off_curve() {
        // Correct length and tag, valid hex, but not a point on secp256k1. This is the
        // case the format-only check used to wave through into the signed manifest.
        // `ff..ff` exceeds the field prime, so it is not even a valid x coordinate.
        let err = validate_signer_pubkey(&format!("02{}", "f".repeat(64)))
            .expect_err("an off-curve key must be rejected locally");
        assert!(
            err.to_string().contains("does not decode to a point"),
            "unexpected error: {err}"
        );
        assert!(validate_signer_pubkey(&format!("04{}", "f".repeat(128))).is_err());
    }

    #[test]
    fn validate_signer_pubkey_error_truncates_a_huge_paste() {
        let huge = format!("02{}", "a".repeat(4096));
        let err = validate_signer_pubkey(&huge).expect_err("wrong length must be rejected");
        let rendered = err.to_string();
        assert!(
            rendered.contains("truncated"),
            "unexpected error: {rendered}"
        );
        assert!(
            !rendered.contains(&huge),
            "error must not echo the whole paste verbatim"
        );
        assert!(
            rendered.len() < 300,
            "error should stay bounded, got {} chars",
            rendered.len()
        );
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
