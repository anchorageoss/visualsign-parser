//! Standalone TVC deploy + Turnkey org-management helper for `parser_app`.
//!
//! Deploy subcommands, in increasing order of separation-of-duties:
//!   deploy    -- digest-gate, create, approve, poll healthy, set live, all in
//!                one call. Unattended; needs the operator seed. Dev use.
//!   initiate  -- digest-gate + create only, printing the deployment ID. No
//!                operator key needed (prod CI's Turnkey API key suffices).
//!   approve   -- re-run the digest gate, then approve the manifest. Meant to
//!                be run by a human operator with their own key (prod).
//!   promote   -- poll to healthy, then set live. No operator key needed.
//!
//! `deploy` is `initiate` -> `approve` -> `promote` composed internally, so
//! prod CI can run the first and last without ever touching the operator
//! seed, and a human runs `approve` out-of-band with it.
//!
//! The operator seed resolves flag -> env `TVC_CI_OPERATOR_SEED` -> none; when
//! none is given, approval uses the logged-in org operator key (`tvc login`).
//!
//! See `tvc-deploy --help` for the full subcommand list (invite/dismiss-invite,
//! activity approve/reject, tag and policy CRUD -- all in `invite.rs`).
//!
//! Turnkey API actions shell out to the `tvc` CLI (it owns auth/consensus);
//! this binary owns config assembly, the image-digest safety gate, and
//! polling -- abstracted behind the `TvcOps` trait so that orchestration (the
//! dedup check, the digest gate, cleanup-on-failure ordering) is unit-testable
//! without a real Turnkey org or Docker daemon. The `invite`/tag/policy
//! subcommands call the Turnkey API directly instead (see `invite.rs`'s module
//! doc).

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
    /// Digest-gate + create only, printing the deployment ID (prod initiate step; needs no operator key)
    Initiate(InitiateArgs),
    /// Deploy parser_app: digest-gate, create, approve, poll healthy, set live
    Deploy(DeployArgs),
    /// Re-run the digest gate, then approve the manifest (prod operator step, run with the operator key)
    Approve(ApproveArgs),
    /// Poll the deployment to healthy, then set it live (prod promote step; needs no operator key)
    Promote(PromoteArgs),
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
#[command(group(
    clap::ArgGroup::new("abi_trust").required(true).multiple(false)
))]
struct InitiateArgs {
    #[arg(long)]
    app_id: String,
    #[arg(long)]
    image_url: String,
    /// Expected sha256 of the image's /parser_app binary (64 hex chars)
    #[arg(long)]
    expected_digest: String,
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
struct ApproveArgs {
    #[arg(long)]
    deploy_id: String,
    #[arg(long)]
    operator_id: String,
    #[arg(long)]
    image_url: String,
    /// Expected sha256 of the image's /parser_app binary (64 hex chars); the
    /// operator independently re-verifies this before signing
    #[arg(long)]
    expected_digest: String,
    /// Path to the operator seed file; falls back to env TVC_CI_OPERATOR_SEED,
    /// then to the logged-in org operator key, if omitted
    #[arg(long)]
    operator_seed: Option<PathBuf>,
}

#[derive(clap::Args)]
struct PromoteArgs {
    #[arg(long)]
    app_id: String,
    #[arg(long)]
    deploy_id: String,
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
    let ops = RealTvc { sh: &sh };
    match cli.command {
        Command::GenOperatorKey(args) => gen_operator_key(&args),
        Command::Initiate(args) => initiate(&ops, &args).map(|_| ()),
        Command::Deploy(args) => deploy(&ops, &args),
        Command::Approve(args) => approve(&ops, &args),
        Command::Promote(args) => promote(&ops, &args),
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

/// Abstracts the external TVC/Docker operations behind `initiate`/`approve`/
/// `promote`/`deploy`, so their orchestration -- the dedup check, the digest
/// gate, cleanup-on-failure ordering -- is unit-testable without a real
/// Turnkey org or Docker daemon.
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

/// Everything `run_initiate` needs, gathered from either `InitiateArgs` or
/// `DeployArgs` -- the two callers that otherwise differ only in whether they
/// go on to approve/promote.
struct InitiateParams<'a> {
    app_id: &'a str,
    org: Option<&'a str>,
    force: bool,
    image_url: &'a str,
    expected_digest: &'a str,
    qos_version: &'a str,
    host_ip: &'a str,
    host_port: u16,
    accept_unsigned_abis: bool,
    accept_signatures_from_pubkey: &'a [String],
}

impl<'a> From<&'a InitiateArgs> for InitiateParams<'a> {
    fn from(args: &'a InitiateArgs) -> Self {
        Self {
            app_id: &args.app_id,
            org: args.org.as_deref(),
            force: args.force,
            image_url: &args.image_url,
            expected_digest: &args.expected_digest,
            qos_version: &args.qos_version,
            host_ip: &args.host_ip,
            host_port: args.host_port,
            accept_unsigned_abis: args.accept_unsigned_abis,
            accept_signatures_from_pubkey: &args.accept_signatures_from_pubkey,
        }
    }
}

impl<'a> From<&'a DeployArgs> for InitiateParams<'a> {
    fn from(args: &'a DeployArgs) -> Self {
        Self {
            app_id: &args.app_id,
            org: args.org.as_deref(),
            force: args.force,
            image_url: &args.image_url,
            expected_digest: &args.expected_digest,
            qos_version: &args.qos_version,
            host_ip: &args.host_ip,
            host_port: args.host_port,
            accept_unsigned_abis: args.accept_unsigned_abis,
            accept_signatures_from_pubkey: &args.accept_signatures_from_pubkey,
        }
    }
}

/// Run the pending-deployment dedup check (unless `force`), the image-digest
/// gate, then create the deployment. Returns the new deployment id. Shared by
/// `initiate` (the prod initiate step, no operator key involved) and `deploy`
/// (the composed dev command).
fn run_initiate(ops: &impl TvcOps, p: &InitiateParams<'_>) -> Result<String> {
    validate_digest(p.expected_digest)?;
    for key in p.accept_signatures_from_pubkey {
        validate_signer_pubkey(key)?;
    }

    if !p.force {
        // Turnkey has no dedup for create_tvc_deployment: submitting the same
        // deploy twice while the first is still ConsensusNeeded creates a
        // second, independent activity instead of reusing it (see README).
        let pending = invite::find_pending_deployments(p.org, p.app_id)?;
        if !pending.is_empty() {
            let ids: Vec<&str> = pending.iter().map(|a| a.id.as_str()).collect();
            bail!(
                "app {} already has {} deployment activity(ies) awaiting consensus: {}\n\
                 approve or reject the existing one first (tvc-deploy approve-activity / \
                 reject-activity --activity-id <id>), or pass --force to submit anyway",
                p.app_id,
                ids.len(),
                ids.join(", ")
            );
        }
    }

    // Safety gate: re-derive the pivot binary digest from the image and confirm
    // it matches --expected-digest, tying the submitted digest to the real binary.
    ops.verify_image_digest(p.image_url, p.expected_digest)?;

    let pivot = build_pivot_args(
        p.host_ip,
        p.host_port,
        p.accept_unsigned_abis,
        p.accept_signatures_from_pubkey,
    );
    let cfg = serde_json::json!({
        "appId": p.app_id,
        "qosVersion": p.qos_version,
        "pivotContainerImageUrl": p.image_url,
        "pivotPath": "/parser_app",
        "pivotArgs": pivot,
        "expectedPivotDigest": p.expected_digest,
        "debugMode": false,
        "healthCheckType": "TVC_HEALTH_CHECK_TYPE_GRPC",
        "healthCheckPort": p.host_port,
        "publicIngressPort": p.host_port,
    });
    let cfg_path = temp_path("tvc-deploy", "json");
    std::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg)?)
        .with_context(|| format!("write {}", cfg_path.display()))?;
    let deploy_id = ops.create(&cfg_path);
    let _ = std::fs::remove_file(&cfg_path);
    let deploy_id = deploy_id?;
    println!("created deployment {deploy_id}");
    Ok(deploy_id)
}

fn initiate(ops: &impl TvcOps, args: &InitiateArgs) -> Result<String> {
    run_initiate(ops, &args.into())
}

fn build_pivot_args(
    host_ip: &str,
    host_port: u16,
    accept_unsigned_abis: bool,
    accept_signatures_from_pubkey: &[String],
) -> Vec<String> {
    let mut pivot = vec![
        "--host-ip".to_string(),
        host_ip.to_string(),
        "--host-port".to_string(),
        host_port.to_string(),
    ];
    if accept_unsigned_abis {
        pivot.push("--accept-unsigned-abis".to_string());
    }
    for key in accept_signatures_from_pubkey {
        pivot.push("--accept-signatures-from-pubkey".to_string());
        pivot.push(key.clone());
    }
    pivot
}

/// Test-only: `run_initiate` builds pivot args from `InitiateParams` directly,
/// so this exists to let `DeployArgs`-level tests assert on the composition
/// without duplicating `InitiateParams::from`'s field mapping.
#[cfg(test)]
fn pivot_args(args: &DeployArgs) -> Vec<String> {
    let p: InitiateParams<'_> = args.into();
    build_pivot_args(
        p.host_ip,
        p.host_port,
        p.accept_unsigned_abis,
        p.accept_signatures_from_pubkey,
    )
}

/// Approve `deploy_id` as `operator_id` with the resolved seed, then ALWAYS
/// remove an env-sourced seed temp file (cleanup=true) before propagating, so
/// the operator seed never leaks on an approve failure.
fn approve_and_cleanup(
    ops: &impl TvcOps,
    deploy_id: &str,
    operator_id: &str,
    seed: &Option<(PathBuf, bool)>,
) -> Result<()> {
    let result = ops.approve(
        deploy_id,
        operator_id,
        seed.as_ref().map(|(p, _)| p.as_path()),
    );
    if let Some((path, true)) = seed {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn deploy(ops: &impl TvcOps, args: &DeployArgs) -> Result<()> {
    let deploy_id = run_initiate(ops, &args.into())?;
    // Resolve the seed only after initiate succeeds, so a digest-gate or
    // create failure never leaves an env-sourced seed temp file on disk.
    let seed = resolve_seed_file(args.operator_seed.as_deref())?;
    approve_and_cleanup(ops, &deploy_id, &args.operator_id, &seed)?;
    println!("approved manifest for {deploy_id}");
    // TVC refuses to target a deployment with zero healthy replicas, so poll
    // to healthy BEFORE set-live. A fresh app auto-targets its first deploy.
    ops.poll_health(&args.app_id, &deploy_id, POLL_TIMEOUT)?;
    ops.set_live(&deploy_id, SETLIVE_TIMEOUT)?;
    println!("deployment {deploy_id} is healthy and live");
    Ok(())
}

fn approve(ops: &impl TvcOps, args: &ApproveArgs) -> Result<()> {
    validate_digest(&args.expected_digest)?;
    // The operator independently re-verifies the digest gate before signing,
    // rather than trusting whatever `initiate` already claimed.
    ops.verify_image_digest(&args.image_url, &args.expected_digest)?;
    let seed = resolve_seed_file(args.operator_seed.as_deref())?;
    approve_and_cleanup(ops, &args.deploy_id, &args.operator_id, &seed)?;
    println!("approved manifest for {}", args.deploy_id);
    Ok(())
}

fn promote(ops: &impl TvcOps, args: &PromoteArgs) -> Result<()> {
    ops.poll_health(&args.app_id, &args.deploy_id, POLL_TIMEOUT)?;
    ops.set_live(&args.deploy_id, SETLIVE_TIMEOUT)?;
    println!("deployment {} is healthy and live", args.deploy_id);
    Ok(())
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

fn validate_signer_pubkey(hex_str: &str) -> Result<()> {
    let stripped = hex_str
        .strip_prefix("0x")
        .or_else(|| hex_str.strip_prefix("0X"))
        .unwrap_or(hex_str);
    // 05 is SEC1's "compact" tag (derived y-coordinate); `canonical_pubkey_from_hex`,
    // what parser_app actually runs on this key, accepts it same as 02/03/04.
    let tag = match stripped.len() {
        66 if stripped.starts_with("02") => Some("02 (compressed)"),
        66 if stripped.starts_with("03") => Some("03 (compressed)"),
        66 if stripped.starts_with("05") => Some("05 (compact)"),
        130 if stripped.starts_with("04") => Some("04 (uncompressed)"),
        _ => None,
    };
    if tag.is_none() || !stripped.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(
            "--accept-signatures-from-pubkey must be a 33-byte (02/03/05-prefixed) or \
             65-byte (04-prefixed) hex secp256k1 public key, got {}",
            truncate_for_error(hex_str)
        );
    }

    let bytes = decode_hex_bytes(stripped)?;
    if k256::PublicKey::from_sec1_bytes(&bytes).is_err() {
        bail!(
            "--accept-signatures-from-pubkey is well-formed hex (SEC1 {}, {} bytes) \
             but does not decode to a point on the secp256k1 curve, got {}",
            tag.unwrap_or("unknown"),
            bytes.len(),
            truncate_for_error(hex_str)
        );
    }
    Ok(())
}

fn decode_hex_bytes(stripped: &str) -> Result<Vec<u8>> {
    (0..stripped.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&stripped[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("invalid hex byte at offset {i}: {e}"))
        })
        .collect()
}

fn truncate_for_error(value: &str) -> String {
    const MAX: usize = 64;
    if value.chars().count() <= MAX {
        format!("{value:?}")
    } else {
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
    use std::cell::RefCell;
    use std::sync::Mutex;

    static SEED_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cli_parses_all_subcommands() {
        Cli::command().debug_assert();
    }

    fn deploy_argv(extra: &[&str]) -> Vec<String> {
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
        base.iter()
            .map(|s| (*s).to_string())
            .chain(extra.iter().map(|s| (*s).to_string()))
            .collect()
    }

    fn deploy_args(extra: &[&str]) -> DeployArgs {
        match Cli::parse_from(deploy_argv(extra)).command {
            Command::Deploy(args) => args,
            _ => panic!("expected the deploy subcommand"),
        }
    }

    fn deploy_error_kind(extra: &[&str]) -> clap::error::ErrorKind {
        Cli::try_parse_from(deploy_argv(extra))
            .map(|_| ())
            .expect_err("these args must not parse")
            .kind()
    }

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

    #[test]
    fn deploy_requires_a_posture() {
        assert_eq!(
            deploy_error_kind(&[]),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

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

    fn initiate_argv(extra: &[&str]) -> Vec<String> {
        let digest = "a".repeat(64);
        let base = [
            "tvc-deploy",
            "initiate",
            "--app-id",
            "app",
            "--image-url",
            "img",
            "--expected-digest",
            &digest,
        ];
        base.iter()
            .map(|s| (*s).to_string())
            .chain(extra.iter().map(|s| (*s).to_string()))
            .collect()
    }

    fn initiate_args(extra: &[&str]) -> InitiateArgs {
        match Cli::parse_from(initiate_argv(extra)).command {
            Command::Initiate(args) => args,
            _ => panic!("expected the initiate subcommand"),
        }
    }

    #[test]
    fn initiate_requires_a_posture_too() {
        assert!(Cli::try_parse_from(initiate_argv(&[])).is_err());
    }

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

    /// `--force` is required on every orchestration test's args: without it,
    /// `run_initiate` would call the real `invite::find_pending_deployments`
    /// (an actual Turnkey API call) instead of going through `RecordingTvc`.
    fn deploy_args_forced(extra: &[&str]) -> DeployArgs {
        let mut a = vec!["--force"];
        a.extend_from_slice(extra);
        deploy_args(&a)
    }

    fn initiate_args_forced(extra: &[&str]) -> InitiateArgs {
        let mut a = vec!["--force"];
        a.extend_from_slice(extra);
        initiate_args(&a)
    }

    #[test]
    fn deploy_runs_gate_create_approve_poll_setlive_in_order() {
        let ops = RecordingTvc::default();
        let args = deploy_args_forced(&["--accept-unsigned-abis", "--operator-seed", "/tmp/seed"]);
        deploy(&ops, &args).unwrap();
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
        let args = initiate_args_forced(&["--accept-unsigned-abis"]);
        let id = initiate(&ops, &args).unwrap();
        assert_eq!(id, "deploy-123");
        assert_eq!(*ops.calls.borrow(), vec!["verify_image_digest", "create"]);
    }

    #[test]
    fn approve_reverifies_then_approves() {
        let ops = RecordingTvc::default();
        let digest = "a".repeat(64);
        let args = match Cli::parse_from([
            "tvc-deploy",
            "approve",
            "--deploy-id",
            "deploy-7",
            "--operator-id",
            "op",
            "--image-url",
            "img",
            "--expected-digest",
            &digest,
            "--operator-seed",
            "/tmp/seed",
        ])
        .command
        {
            Command::Approve(args) => args,
            _ => panic!("expected the approve subcommand"),
        };
        approve(&ops, &args).unwrap();
        assert_eq!(
            *ops.calls.borrow(),
            vec!["verify_image_digest", "approve:deploy-7"]
        );
    }

    #[test]
    fn promote_polls_then_sets_live() {
        let ops = RecordingTvc::default();
        let args = match Cli::parse_from([
            "tvc-deploy",
            "promote",
            "--app-id",
            "app",
            "--deploy-id",
            "deploy-9",
        ])
        .command
        {
            Command::Promote(args) => args,
            _ => panic!("expected the promote subcommand"),
        };
        promote(&ops, &args).unwrap();
        assert_eq!(
            *ops.calls.borrow(),
            vec!["poll:deploy-9", "set_live:deploy-9"]
        );
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
        let _env = SEED_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let args = deploy_args_forced(&["--accept-unsigned-abis"]);
        let before = leftover_operator_seeds();
        // SAFETY: this is the only test that touches this env var.
        unsafe {
            std::env::set_var("TVC_CI_OPERATOR_SEED", "00".repeat(32));
        }
        let result = deploy(&FailingApprove, &args);
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

    #[test]
    fn deploy_does_not_write_seed_when_initiate_fails() {
        let _env = SEED_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        struct FailingGate;
        impl TvcOps for FailingGate {
            fn verify_image_digest(&self, _i: &str, _e: &str) -> Result<()> {
                bail!("gate boom")
            }
            fn create(&self, _c: &Path) -> Result<String> {
                panic!("create must not run when the gate fails")
            }
            fn approve(&self, _d: &str, _o: &str, _s: Option<&Path>) -> Result<()> {
                panic!("approve must not run")
            }
            fn poll_health(&self, _a: &str, _d: &str, _t: Duration) -> Result<()> {
                panic!("poll_health must not run")
            }
            fn set_live(&self, _d: &str, _t: Duration) -> Result<()> {
                panic!("set_live must not run")
            }
        }
        let args = deploy_args_forced(&["--accept-unsigned-abis"]);
        let before = leftover_operator_seeds();
        unsafe {
            std::env::set_var("TVC_CI_OPERATOR_SEED", "00".repeat(32));
        }
        let result = deploy(&FailingGate, &args);
        unsafe {
            std::env::remove_var("TVC_CI_OPERATOR_SEED");
        }
        assert!(result.is_err(), "initiate failure should propagate");
        assert_eq!(
            before,
            leftover_operator_seeds(),
            "seed must not be written when initiate fails"
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

    fn real_pubkey_hex(compressed: bool) -> String {
        use k256::elliptic_curve::sec1::ToEncodedPoint;
        let sk = k256::SecretKey::from_slice(&[0x42u8; 32]).expect("valid scalar");
        hex_of(sk.public_key().to_encoded_point(compressed).as_bytes())
    }

    fn real_compact_pubkey_hex() -> String {
        let uncompressed = real_pubkey_hex(false);
        format!("05{}", &uncompressed[2..66])
    }

    #[test]
    fn validate_signer_pubkey_accepts_compressed_and_uncompressed() {
        let compressed = real_pubkey_hex(true);
        assert!(validate_signer_pubkey(&compressed).is_ok());
        assert!(validate_signer_pubkey(&real_pubkey_hex(false)).is_ok());
        assert!(
            validate_signer_pubkey(&format!("0x{}", real_pubkey_hex(false).to_uppercase())).is_ok()
        );
        assert!(validate_signer_pubkey(&real_compact_pubkey_hex()).is_ok());
    }

    #[test]
    fn validate_signer_pubkey_compact_through_k256() {
        let compact = real_compact_pubkey_hex();
        let bytes = decode_hex_bytes(&compact).expect("valid hex");
        assert_eq!(bytes[0], 0x05, "compact tag");
        assert_eq!(bytes.len(), 33, "compact is 33 bytes");
        k256::PublicKey::from_sec1_bytes(&bytes).expect("k256 must accept SEC1 compact (05) form");
    }

    #[test]
    fn validate_signer_pubkey_rejects_truncated_or_malformed() {
        assert!(validate_signer_pubkey("04aa").is_err());
        assert!(validate_signer_pubkey("").is_err());
        assert!(validate_signer_pubkey(&format!("06{}", "a".repeat(64))).is_err());
        assert!(validate_signer_pubkey(&format!("02{}", "g".repeat(64))).is_err());
        assert!(validate_signer_pubkey(&format!("02{}", "a".repeat(128))).is_err());
    }

    #[test]
    fn validate_signer_pubkey_rejects_well_formed_hex_that_is_off_curve() {
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
