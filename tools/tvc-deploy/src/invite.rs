//! Turnkey org-management subcommands: invite/dismiss-invite, activity
//! approve/reject, tag CRUD (create/update/list), user listing, and policy
//! CRUD (create single/batch, list). See each `*Args` struct's doc comments
//! (surfaced via `--help`) and `tools/tvc-deploy/README.md` for the batch
//! invite / batch policy workflows and file schemas.
//!
//! Auth resolves the same way as the official `tvc` CLI: env vars
//! (TVC_ORG_ID / TVC_API_KEY_PUBLIC / TVC_API_KEY_PRIVATE / TVC_API_BASE_URL)
//! take priority; otherwise falls back to ~/.config/turnkey/tvc.config.toml
//! (written by `tvc login`), selecting --org or the active org.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::Args;
use serde::Deserialize;
use turnkey_api_key_stamper::{Stamp, StampHeader, StamperError};
use turnkey_client::generated::external::data::v1::InvitationStatus;
use turnkey_client::generated::immutable::common::v1::{AccessType, Effect};
use turnkey_client::generated::{
    ApproveActivityIntent, CreateInvitationsIntent, CreatePoliciesIntent, CreatePolicyIntentV3,
    CreateUserTagIntent, DeleteInvitationIntent, GetActivityRequest, GetOrganizationRequest,
    GetOrganizationResponse, GetPoliciesRequest, GetUsersRequest, GetWhoamiRequest,
    InvitationParams, ListUserTagsRequest, RejectActivityIntent, UpdateUserTagIntent,
};
use turnkey_client::TurnkeyP256ApiKey;
use turnkey_client::{TurnkeyClient, TurnkeyClientError, TurnkeySecp256k1ApiKey};

const DEFAULT_API_BASE_URL: &str = "https://api.turnkey.com";
const ENV_ORG_ID: &str = "TVC_ORG_ID";
const ENV_API_BASE_URL: &str = "TVC_API_BASE_URL";
const ENV_API_KEY_PUBLIC: &str = "TVC_API_KEY_PUBLIC";
const ENV_API_KEY_PRIVATE: &str = "TVC_API_KEY_PRIVATE";

/// `--org <alias-or-id>`: shared by every subcommand that talks to Turnkey.
/// Accepts either the config alias (e.g. what `tvc login` calls the org) or
/// the org's own UUID; falls back to the active org if omitted.
#[derive(Args, Debug)]
pub struct OrgArgs {
    /// Org alias from tvc.config.toml, or the org's own UUID; defaults to the active org
    #[arg(long)]
    pub org: Option<String>,
}

impl OrgArgs {
    pub fn as_deref(&self) -> Option<&str> {
        self.org.as_deref()
    }
}

#[derive(Args, Debug)]
pub struct InviteArgs {
    /// Batch-invite from a JSON file instead of a single --user-name/--email
    /// (see README for the invitees.json schema: accessType/tags/allowExisting
    /// defaults, each overridable per invitee)
    #[arg(long)]
    pub file: Option<String>,
    /// Display name for a single invitee (requires --email; use --file for a batch)
    #[arg(long)]
    pub user_name: Option<String>,
    /// Email for a single invitee
    #[arg(long)]
    pub email: Option<String>,
    /// web|api|all -- default access type; a batch file's own accessType overrides this
    #[arg(long, default_value = "all")]
    pub access_type: String,
    /// Comma-separated tag ids (single-invitee mode only; batch mode uses the file's tags)
    #[arg(long)]
    pub tags: Option<String>,
    /// Existing user id to use as senderUserId; defaults to whoami
    #[arg(long)]
    pub sender_user_id: Option<String>,
    /// Bypass the existing-member alias check for this single invitee
    #[arg(long)]
    pub allow_existing: bool,
    /// Disable the existing-member alias check entirely (skips the get_users lookup)
    #[arg(long)]
    pub include_existing: bool,
    #[command(flatten)]
    pub org: OrgArgs,
}

#[derive(Args, Debug)]
pub struct DismissInviteArgs {
    #[arg(long)]
    pub invitation_id: String,
    #[command(flatten)]
    pub org: OrgArgs,
}

#[derive(Args, Debug)]
pub struct ActivityIdArgs {
    #[arg(long)]
    pub activity_id: String,
    #[command(flatten)]
    pub org: OrgArgs,
}

#[derive(Args, Debug)]
pub struct CreateTagArgs {
    #[arg(long)]
    pub name: String,
    /// Comma-separated existing user ids to tag immediately
    #[arg(long)]
    pub user_ids: Option<String>,
    #[command(flatten)]
    pub org: OrgArgs,
}

#[derive(Args, Debug)]
pub struct UpdateTagArgs {
    #[arg(long)]
    pub tag_id: String,
    /// Comma-separated existing user ids to add to the tag
    #[arg(long)]
    pub add_user_ids: Option<String>,
    /// Comma-separated existing user ids to remove from the tag
    #[arg(long)]
    pub remove_user_ids: Option<String>,
    /// Rename the tag
    #[arg(long)]
    pub name: Option<String>,
    #[command(flatten)]
    pub org: OrgArgs,
}

#[derive(Args, Debug)]
pub struct CreatePolicyArgs {
    #[arg(long)]
    pub name: String,
    /// allow|deny
    #[arg(long)]
    pub effect: String,
    #[arg(long, default_value = "")]
    pub notes: String,
    /// TQL expression; see an existing org's `list-policies` output for examples
    #[arg(long)]
    pub condition: Option<String>,
    /// TQL expression scoping who the policy applies to (e.g. by user tag)
    #[arg(long)]
    pub consensus: Option<String>,
    #[command(flatten)]
    pub org: OrgArgs,
}

#[derive(Args, Debug)]
pub struct CreatePoliciesArgs {
    /// policies.json: {"policies": [{"policyName", "effect": "allow"|"deny",
    /// "condition", "consensus", "notes"}, ...]}; condition/consensus may
    /// contain {{PLACEHOLDER}} tokens filled in from --vars
    #[arg(long)]
    pub file: String,
    /// Flat JSON object of {"PLACEHOLDER": "value"} to render --file's {{PLACEHOLDER}} tokens
    #[arg(long)]
    pub vars: Option<String>,
    /// Render and print the batch without submitting, to check a template before it hits an org
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub org: OrgArgs,
}

/// API key as stored on disk in `api_key.json` by `tvc login`.
#[derive(Deserialize)]
struct StoredApiKey {
    public_key: String,
    private_key: String,
    curve: KeyCurve,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum KeyCurve {
    P256,
    Secp256k1,
}

#[derive(Deserialize)]
struct TvcConfig {
    active_org: Option<String>,
    #[serde(default)]
    orgs: HashMap<String, OrgEntry>,
}

#[derive(Deserialize)]
struct OrgEntry {
    id: String,
    api_key_path: PathBuf,
    #[serde(default = "default_api_base_url")]
    api_base_url: String,
}

fn default_api_base_url() -> String {
    DEFAULT_API_BASE_URL.to_string()
}

/// Either curve of Turnkey API key, so callers don't need to know which one
/// a given org uses ahead of time.
enum AnyApiKey {
    P256(TurnkeyP256ApiKey),
    Secp256k1(TurnkeySecp256k1ApiKey),
}

impl Stamp for AnyApiKey {
    fn stamp(&self, body: &[u8]) -> Result<StampHeader, StamperError> {
        match self {
            AnyApiKey::P256(k) => k.stamp(body),
            AnyApiKey::Secp256k1(k) => k.stamp(body),
        }
    }
}

struct Auth {
    org_id: String,
    api_base_url: String,
    client: TurnkeyClient<AnyApiKey>,
}

fn read_env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// Raw credential material, before the key bytes are parsed into a signer.
struct ResolvedCreds {
    org_id: String,
    api_base_url: String,
    public_key: String,
    private_key: String,
    curve: KeyCurve,
}

/// Resolve org_id/api_base_url/api_key from env vars, or from
/// ~/.config/turnkey/tvc.config.toml (selecting `--org` or the active org).
fn resolve_auth(org_override: Option<&str>) -> Result<Auth> {
    let creds = match resolve_from_env()? {
        Some(v) => v,
        None => resolve_from_config(org_override)?,
    };

    let api_key = match creds.curve {
        KeyCurve::P256 => AnyApiKey::P256(
            TurnkeyP256ApiKey::from_strings(&creds.private_key, Some(&creds.public_key))
                .context("failed to load P256 API key")?,
        ),
        KeyCurve::Secp256k1 => AnyApiKey::Secp256k1(
            TurnkeySecp256k1ApiKey::from_strings(&creds.private_key, Some(&creds.public_key))
                .context("failed to load secp256k1 API key")?,
        ),
    };

    let client = TurnkeyClient::builder()
        .api_key(api_key)
        .base_url(creds.api_base_url.clone())
        .build()
        .map_err(|e| anyhow!("failed to build Turnkey client: {e}"))?;

    Ok(Auth {
        org_id: creds.org_id,
        api_base_url: creds.api_base_url,
        client,
    })
}

/// Env vars always assume a P256 key (the only curve TVC_API_KEY_* supports today).
///
/// NOTE: `TVC_API_BASE_URL` only takes effect alongside the other three env
/// vars -- if org id/public/private key are all unset, this returns `None`
/// and falls back to the config file entirely (which carries its own
/// per-org `api_base_url`), rather than overriding just the base URL for a
/// config-file org. This intentionally mirrors the official `tvc` CLI's own
/// `load_credentials_from_env_vars`, since `TVC_API_BASE_URL` exists for the
/// fully-env-var-driven CI path, not to override one field of a file-backed org.
fn resolve_from_env() -> Result<Option<ResolvedCreds>> {
    let org_id = read_env_var(ENV_ORG_ID);
    let public_key = read_env_var(ENV_API_KEY_PUBLIC);
    let private_key = read_env_var(ENV_API_KEY_PRIVATE);
    let api_base_url = read_env_var(ENV_API_BASE_URL).unwrap_or_else(default_api_base_url);

    let set = [
        org_id.is_some(),
        public_key.is_some(),
        private_key.is_some(),
    ];
    if set.iter().all(|s| !s) {
        return Ok(None);
    }
    if !set.iter().all(|s| *s) {
        bail!(
            "partial env var auth: set all three of {ENV_ORG_ID}, {ENV_API_KEY_PUBLIC}, \
             {ENV_API_KEY_PRIVATE}, or none"
        );
    }
    Ok(Some(ResolvedCreds {
        org_id: org_id.ok_or_else(|| anyhow!("missing {ENV_ORG_ID}"))?,
        api_base_url,
        public_key: public_key.ok_or_else(|| anyhow!("missing {ENV_API_KEY_PUBLIC}"))?,
        private_key: private_key.ok_or_else(|| anyhow!("missing {ENV_API_KEY_PRIVATE}"))?,
        curve: KeyCurve::P256,
    }))
}

/// Resolve `--org` (or the active org, if `None`) to a config entry. `--org`
/// may be the config alias (e.g. what `tvc login` calls the org) or the
/// org's own UUID; alias lookup is tried first, then a fall back to matching
/// on [`OrgEntry::id`], since the org's UUID alone is what most people reach
/// for and doesn't require knowing the alias it happens to be stored under.
fn resolve_org<'a>(config: &'a TvcConfig, org_override: Option<&str>) -> Option<&'a OrgEntry> {
    match org_override {
        Some(given) => config.orgs.get(given).or_else(|| {
            config
                .orgs
                .values()
                .find(|o| o.id.eq_ignore_ascii_case(given))
        }),
        None => config
            .active_org
            .as_ref()
            .and_then(|alias| config.orgs.get(alias)),
    }
}

fn resolve_from_config(org_override: Option<&str>) -> Result<ResolvedCreds> {
    let config_path = dirs_config_path()?;
    let content = std::fs::read_to_string(&config_path).with_context(|| {
        format!(
            "no TVC_ORG_ID/TVC_API_KEY_PUBLIC/TVC_API_KEY_PRIVATE env vars, and no config at {}; run `tvc login` first",
            config_path.display()
        )
    })?;
    let config: TvcConfig =
        toml::from_str(&content).with_context(|| format!("parse {}", config_path.display()))?;

    let org = resolve_org(&config, org_override).ok_or_else(|| match org_override {
        Some(given) => anyhow!(
            "org {given:?} not found (by alias or id) in {}",
            config_path.display()
        ),
        None => anyhow!(
            "no --org given and no active org in {}",
            config_path.display()
        ),
    })?;

    let key_content = std::fs::read_to_string(&org.api_key_path)
        .with_context(|| format!("read {}", org.api_key_path.display()))?;
    let key: StoredApiKey = serde_json::from_str(&key_content)
        .with_context(|| format!("parse {}", org.api_key_path.display()))?;

    Ok(ResolvedCreds {
        org_id: org.id.clone(),
        api_base_url: org.api_base_url.clone(),
        public_key: key.public_key,
        private_key: key.private_key,
        curve: key.curve,
    })
}

fn dirs_config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("turnkey")
        .join("tvc.config.toml"))
}

fn current_timestamp_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Build a fresh single-threaded tokio runtime and drive `fut` to completion.
/// Each subcommand runs exactly one async block per process invocation, so a
/// shared runtime isn't needed -- this just avoids repeating the
/// builder/`.build()` boilerplate at every call site.
fn block_on<F: std::future::Future>(fut: F) -> Result<F::Output> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    Ok(rt.block_on(fut))
}

fn parse_access_type(s: &str) -> Result<AccessType> {
    match s.to_ascii_lowercase().as_str() {
        "web" => Ok(AccessType::Web),
        "api" => Ok(AccessType::Api),
        "all" => Ok(AccessType::All),
        other => bail!("--access-type must be one of web|api|all, got {other:?}"),
    }
}

fn parse_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// One team member to invite, as listed in a `--file` batch (see [`InviteArgs::file`]).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InviteeEntry {
    user_name: String,
    email: String,
    /// Overrides the file-level `tags` default entirely (not merged) when present.
    tags: Option<Vec<String>>,
    /// Overrides the file-level `access_type`, which overrides `--access-type`.
    access_type: Option<String>,
    /// Overrides the file-level `allow_existing`. See [`InviteesFile::allow_existing`].
    allow_existing: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InviteesFile {
    access_type: Option<String>,
    /// Default tag id(s) applied to every invitee that doesn't set its own `tags`.
    #[serde(default)]
    tags: Vec<String>,
    /// Default for whether invitees may bypass the existing-member alias check
    /// (see [`partition_existing`]); a per-invitee `allowExisting` overrides
    /// this. Put the exact addresses you know alias an existing member here
    /// (or per-invitee) rather than passing them again on the command line.
    #[serde(default)]
    allow_existing: bool,
    invitees: Vec<InviteeEntry>,
}

/// Build the invitation list from either `--file <path>` (batch) or the
/// singular `--user-name`/`--email`/`--tags` flags (one invite), paired with
/// whether each may bypass the existing-member alias check. `sender_user_id`
/// is left blank here; the caller fills it in once resolved.
fn build_invitations(args: &InviteArgs) -> Result<Vec<(InvitationParams, bool)>> {
    if let Some(path) = &args.file {
        let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
        let file: InviteesFile =
            serde_json::from_str(&content).with_context(|| format!("parse {path}"))?;
        if file.invitees.is_empty() {
            bail!("{path} lists no invitees");
        }
        let default_access = match &file.access_type {
            Some(s) => parse_access_type(s)?,
            None => parse_access_type(&args.access_type)?,
        };
        let default_tags = file.tags;
        let default_allow_existing = file.allow_existing;
        file.invitees
            .into_iter()
            .map(|e| {
                let access_type = match &e.access_type {
                    Some(s) => parse_access_type(s)?,
                    None => default_access,
                };
                let receiver_user_tags = e.tags.unwrap_or_else(|| default_tags.clone());
                let allow_existing = e.allow_existing.unwrap_or(default_allow_existing);
                Ok((
                    InvitationParams {
                        receiver_user_name: e.user_name,
                        receiver_user_email: e.email,
                        receiver_user_tags,
                        access_type,
                        sender_user_id: String::new(),
                    },
                    allow_existing,
                ))
            })
            .collect()
    } else {
        let user_name = args
            .user_name
            .clone()
            .ok_or_else(|| anyhow!("--user-name is required without --file"))?;
        let email = args
            .email
            .clone()
            .ok_or_else(|| anyhow!("--email is required without --file"))?;
        let access_type = parse_access_type(&args.access_type)?;
        let tags = args.tags.as_deref().map(parse_tags).unwrap_or_default();
        Ok(vec![(
            InvitationParams {
                receiver_user_name: user_name,
                receiver_user_email: email,
                receiver_user_tags: tags,
                access_type,
                sender_user_id: String::new(),
            },
            args.allow_existing,
        )])
    }
}

/// Lowercase, and drop any `+suffix` from the local part, so
/// `Alice+dev1@Co.com` and `alice@co.com` compare equal -- plus-addressed
/// aliases of the same mailbox map to the same canonical identity.
fn canonical_email(email: &str) -> String {
    let email = email.to_lowercase();
    match email.split_once('@') {
        Some((local, domain)) => {
            let base_local = local.split('+').next().unwrap_or(local);
            format!("{base_local}@{domain}")
        }
        None => email,
    }
}

/// Split `(invitation, allow_existing)` pairs into (kept, skipped): an
/// invitee is skipped when its canonical email (see [`canonical_email`])
/// matches an existing org member's, UNLESS its own `allow_existing` is true
/// -- e.g. to deliberately invite a `+dev` alias of someone who's already a
/// real member. Set per-invitee in the file (or file-wide via its top-level
/// `allowExisting`) rather than repeated on the command line -- see
/// [`InviteesFile::allow_existing`].
fn partition_existing(
    invitations: Vec<(InvitationParams, bool)>,
    existing_emails: &HashSet<String>,
) -> (Vec<InvitationParams>, Vec<InvitationParams>) {
    let (kept, skipped): (Vec<_>, Vec<_>) =
        invitations.into_iter().partition(|(i, allow_existing)| {
            *allow_existing || !existing_emails.contains(&canonical_email(&i.receiver_user_email))
        });
    (
        kept.into_iter().map(|(i, _)| i).collect(),
        skipped.into_iter().map(|(i, _)| i).collect(),
    )
}

pub fn invite(args: &InviteArgs) -> Result<()> {
    let invitations_with_allow = build_invitations(args)?;

    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let mut invitations = if args.include_existing {
            invitations_with_allow.into_iter().map(|(i, _)| i).collect()
        } else {
            let org_data = fetch_org_data(&auth).await?;
            let existing_emails: HashSet<String> = org_data
                .users
                .into_iter()
                .filter_map(|u| u.user_email)
                .map(|e| canonical_email(&e))
                .chain(
                    // Also skip emails with a still-pending invitation, not just
                    // accepted members -- a revoked/accepted invitation frees up
                    // its email again, but "created" (unaccepted) does not.
                    org_data
                        .invitations
                        .into_iter()
                        .filter(|i| i.status == InvitationStatus::Created)
                        .map(|i| canonical_email(&i.receiver_email)),
                )
                .collect();

            let (kept, skipped) = partition_existing(invitations_with_allow, &existing_emails);
            for s in &skipped {
                println!(
                    "skipping {} <{}>: matches an existing member or pending invitation in org {} \
                     (set \"allowExisting\": true for this invitee, or at the top level of \
                     the file, to invite it anyway)",
                    s.receiver_user_name, s.receiver_user_email, auth.org_id
                );
            }
            if kept.is_empty() {
                println!(
                    "nothing to invite: everyone in the list already matches an existing member \
                     or pending invitation"
                );
                return Ok(());
            }
            kept
        };

        let sender_user_id = match &args.sender_user_id {
            Some(id) => id.clone(),
            None => {
                let who = auth
                    .client
                    .get_whoami(GetWhoamiRequest {
                        organization_id: auth.org_id.clone(),
                    })
                    .await
                    .map_err(|e| anyhow!("get_whoami failed: {e}"))?;
                who.user_id
            }
        };
        for invitation in &mut invitations {
            invitation.sender_user_id = sender_user_id.clone();
        }

        let outcome = auth
            .client
            .create_invitations(
                auth.org_id.clone(),
                current_timestamp_ms(),
                CreateInvitationsIntent { invitations },
            )
            .await;

        match outcome {
            Ok(result) => {
                println!("activity {} status={:?}", result.activity_id, result.status);
                for id in result.result.invitation_ids {
                    println!("invitation id: {id}");
                }
                println!("org: {} ({})", auth.org_id, auth.api_base_url);
                Ok(())
            }
            Err(TurnkeyClientError::ActivityRequiresApproval(activity_id)) => {
                println!(
                    "activity {activity_id} needs consensus; approve it with:\n  \
                     tvc-deploy approve-activity --activity-id {activity_id} --org <alias-or-id>"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!("create_invitations failed: {e}")),
        }
    })?
}

fn parse_effect(s: &str) -> Result<Effect> {
    match s.to_ascii_lowercase().as_str() {
        "allow" => Ok(Effect::Allow),
        "deny" => Ok(Effect::Deny),
        other => bail!("--effect must be allow|deny, got {other:?}"),
    }
}

pub fn list_policies(args: &OrgArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let response = auth
            .client
            .get_policies(GetPoliciesRequest {
                organization_id: auth.org_id.clone(),
            })
            .await
            .map_err(|e| anyhow!("get_policies failed: {e}"))?;

        if response.policies.is_empty() {
            println!("no policies in org {}", auth.org_id);
            return Ok(());
        }
        for policy in response.policies {
            println!(
                "{}  {}  {:?}  notes={:?} condition={:?} consensus={:?}",
                policy.policy_id,
                policy.policy_name,
                policy.effect,
                policy.notes,
                policy.condition,
                policy.consensus
            );
        }
        Ok(())
    })?
}

pub fn create_policy(args: &CreatePolicyArgs) -> Result<()> {
    let effect = parse_effect(&args.effect)?;

    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let outcome = auth
            .client
            .create_policy(
                auth.org_id.clone(),
                current_timestamp_ms(),
                CreatePolicyIntentV3 {
                    policy_name: args.name.clone(),
                    effect,
                    condition: args.condition.clone(),
                    consensus: args.consensus.clone(),
                    notes: args.notes.clone(),
                },
            )
            .await;

        match outcome {
            Ok(result) => {
                println!("activity {} status={:?}", result.activity_id, result.status);
                println!("policy id: {}", result.result.policy_id);
                Ok(())
            }
            Err(TurnkeyClientError::ActivityRequiresApproval(activity_id)) => {
                println!(
                    "activity {activity_id} needs consensus; approve it with:\n  \
                     tvc-deploy approve-activity --activity-id {activity_id} --org <alias-or-id>"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!("create_policy failed: {e}")),
        }
    })?
}

/// One policy as listed in a `--file` batch for `create-policies` (see
/// [`CreatePoliciesArgs::file`]). Uses the CLI's friendly "allow"/"deny"
/// rather than Turnkey's own "EFFECT_ALLOW"/"EFFECT_DENY" wire format, for
/// consistency with `create-policy`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PolicyEntry {
    policy_name: String,
    effect: String,
    #[serde(default)]
    condition: Option<String>,
    #[serde(default)]
    consensus: Option<String>,
    #[serde(default)]
    notes: String,
}

#[derive(Deserialize)]
struct PoliciesFile {
    policies: Vec<PolicyEntry>,
}

/// Names of every `{{PLACEHOLDER}}` token in `content`, in order of appearance
/// (duplicates included).
fn find_placeholders(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        names.push(after[..end].to_string());
        rest = &after[end + 2..];
    }
    names
}

fn render_template(content: &str, vars: &HashMap<String, String>) -> String {
    let mut rendered = content.to_string();
    for (key, value) in vars {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
    }
    rendered
}

/// Read `path`, substitute any `{{PLACEHOLDER}}` tokens from the JSON object at
/// `vars_path` (or none, if not given), and parse the result into the batch of
/// policies to create. Pure I/O + parsing, no network -- used by both the real
/// `--dry-run` path and tests.
fn load_policies_file(path: &str, vars_path: Option<&str>) -> Result<Vec<CreatePolicyIntentV3>> {
    let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;

    let vars: HashMap<String, String> = match vars_path {
        Some(vars_path) => {
            let vars_content =
                std::fs::read_to_string(vars_path).with_context(|| format!("read {vars_path}"))?;
            serde_json::from_str(&vars_content).with_context(|| format!("parse {vars_path}"))?
        }
        None => HashMap::new(),
    };

    let missing: Vec<String> = find_placeholders(&content)
        .into_iter()
        .filter(|name| !vars.contains_key(name))
        .collect();
    if !missing.is_empty() {
        bail!(
            "{path} references placeholders with no --vars entry: {}",
            missing.join(", ")
        );
    }
    let rendered = render_template(&content, &vars);

    let file: PoliciesFile =
        serde_json::from_str(&rendered).with_context(|| format!("parse rendered {path}"))?;
    if file.policies.is_empty() {
        bail!("{path} lists no policies");
    }
    file.policies
        .into_iter()
        .map(|p| {
            Ok(CreatePolicyIntentV3 {
                policy_name: p.policy_name,
                effect: parse_effect(&p.effect)?,
                condition: p.condition,
                consensus: p.consensus,
                notes: p.notes,
            })
        })
        .collect()
}

pub fn create_policies(args: &CreatePoliciesArgs) -> Result<()> {
    let policies = load_policies_file(&args.file, args.vars.as_deref())?;
    let intent = CreatePoliciesIntent { policies };

    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&intent)?);
        return Ok(());
    }

    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let outcome = auth
            .client
            .create_policies(auth.org_id.clone(), current_timestamp_ms(), intent)
            .await;

        match outcome {
            Ok(result) => {
                println!("activity {} status={:?}", result.activity_id, result.status);
                for id in result.result.policy_ids {
                    println!("policy id: {id}");
                }
                Ok(())
            }
            Err(TurnkeyClientError::ActivityRequiresApproval(activity_id)) => {
                println!(
                    "activity {activity_id} needs consensus; approve it with:\n  \
                     tvc-deploy approve-activity --activity-id {activity_id} --org <alias-or-id>"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!("create_policies failed: {e}")),
        }
    })?
}

/// Fetch the org's full data blob -- users, invitations, tags, policies, etc.
/// Not wrapped in a convenience method upstream (unlike `get_users`), but the
/// request/response types are in the generated client, and `process_request`
/// is public, so this is a normal query, not a private/undocumented API.
async fn fetch_org_data(
    auth: &Auth,
) -> Result<turnkey_client::generated::external::data::v1::OrganizationData> {
    let response: GetOrganizationResponse = auth
        .client
        .process_request(
            &GetOrganizationRequest {
                organization_id: auth.org_id.clone(),
            },
            "/public/v1/query/get_organization".to_string(),
        )
        .await
        .map_err(|e| anyhow!("get_organization failed: {e}"))?;
    response
        .organization_data
        .ok_or_else(|| anyhow!("get_organization returned no organization_data"))
}

pub fn list_invitations(args: &OrgArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;
        let org_data = fetch_org_data(&auth).await?;

        if org_data.invitations.is_empty() {
            println!("no invitations in org {}", auth.org_id);
            return Ok(());
        }
        for invitation in org_data.invitations {
            println!(
                "{}  {}  <{}>  {:?}",
                invitation.invitation_id,
                invitation.receiver_user_name,
                invitation.receiver_email,
                invitation.status
            );
        }
        Ok(())
    })?
}

pub fn list_users(args: &OrgArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let response = auth
            .client
            .get_users(GetUsersRequest {
                organization_id: auth.org_id.clone(),
            })
            .await
            .map_err(|e| anyhow!("get_users failed: {e}"))?;

        if response.users.is_empty() {
            println!("no users in org {}", auth.org_id);
            return Ok(());
        }
        for user in response.users {
            println!(
                "{}  {}  <{}>",
                user.user_id,
                user.user_name,
                user.user_email.as_deref().unwrap_or("no email")
            );
        }
        Ok(())
    })?
}

pub fn update_tag(args: &UpdateTagArgs) -> Result<()> {
    let add_user_ids = args
        .add_user_ids
        .as_deref()
        .map(parse_tags)
        .unwrap_or_default();
    let remove_user_ids = args
        .remove_user_ids
        .as_deref()
        .map(parse_tags)
        .unwrap_or_default();

    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let outcome = auth
            .client
            .update_user_tag(
                auth.org_id.clone(),
                current_timestamp_ms(),
                UpdateUserTagIntent {
                    user_tag_id: args.tag_id.clone(),
                    new_user_tag_name: args.name.clone(),
                    add_user_ids,
                    remove_user_ids,
                },
            )
            .await;

        match outcome {
            Ok(result) => {
                println!("activity {} status={:?}", result.activity_id, result.status);
                println!("tag id: {}", result.result.user_tag_id);
                Ok(())
            }
            Err(TurnkeyClientError::ActivityRequiresApproval(activity_id)) => {
                println!(
                    "activity {activity_id} needs consensus; approve it with:\n  \
                     tvc-deploy approve-activity --activity-id {activity_id} --org <alias-or-id>"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!("update_user_tag failed: {e}")),
        }
    })?
}

pub fn create_tag(args: &CreateTagArgs) -> Result<()> {
    let user_ids = args.user_ids.as_deref().map(parse_tags).unwrap_or_default();

    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let outcome = auth
            .client
            .create_user_tag(
                auth.org_id.clone(),
                current_timestamp_ms(),
                CreateUserTagIntent {
                    user_tag_name: args.name.clone(),
                    user_ids,
                },
            )
            .await;

        match outcome {
            Ok(result) => {
                println!("activity {} status={:?}", result.activity_id, result.status);
                println!("tag id: {}", result.result.user_tag_id);
                Ok(())
            }
            Err(TurnkeyClientError::ActivityRequiresApproval(activity_id)) => {
                println!(
                    "activity {activity_id} needs consensus; approve it with:\n  \
                     tvc-deploy approve-activity --activity-id {activity_id} --org <alias-or-id>"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!("create_user_tag failed: {e}")),
        }
    })?
}

pub fn list_tags(args: &OrgArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let response = auth
            .client
            .list_user_tags(ListUserTagsRequest {
                organization_id: auth.org_id.clone(),
            })
            .await
            .map_err(|e| anyhow!("list_user_tags failed: {e}"))?;

        if response.user_tags.is_empty() {
            println!("no user tags in org {}", auth.org_id);
            return Ok(());
        }
        for tag in response.user_tags {
            println!("{}  {}", tag.tag_id, tag.tag_name);
        }
        Ok(())
    })?
}

pub fn dismiss_invite(args: &DismissInviteArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;

        let outcome = auth
            .client
            .delete_invitation(
                auth.org_id.clone(),
                current_timestamp_ms(),
                DeleteInvitationIntent {
                    invitation_id: args.invitation_id.clone(),
                },
            )
            .await;

        match outcome {
            Ok(result) => {
                println!("activity {} status={:?}", result.activity_id, result.status);
                println!("deleted invitation id: {}", result.result.invitation_id);
                Ok(())
            }
            Err(TurnkeyClientError::ActivityRequiresApproval(activity_id)) => {
                println!(
                    "activity {activity_id} needs consensus; approve it with:\n  \
                     tvc-deploy approve-activity --activity-id {activity_id} --org <alias-or-id>"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!("delete_invitation failed: {e}")),
        }
    })?
}

/// Fetch an activity's fingerprint, the artifact `approve_activity`/`reject_activity`
/// sign over (not the activity id).
async fn fetch_fingerprint(auth: &Auth, activity_id: &str) -> Result<String> {
    let response = auth
        .client
        .get_activity(GetActivityRequest {
            organization_id: auth.org_id.clone(),
            activity_id: activity_id.to_string(),
        })
        .await
        .map_err(|e| anyhow!("get_activity failed: {e}"))?;
    let activity = response
        .activity
        .ok_or_else(|| anyhow!("get_activity returned no activity for {activity_id}"))?;
    Ok(activity.fingerprint)
}

pub fn approve_activity(args: &ActivityIdArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;
        let fingerprint = fetch_fingerprint(&auth, &args.activity_id).await?;

        let activity = auth
            .client
            .approve_activity(
                auth.org_id.clone(),
                current_timestamp_ms(),
                ApproveActivityIntent { fingerprint },
            )
            .await
            .map_err(|e| anyhow!("approve_activity failed: {e}"))?;

        println!("activity {} status={:?}", activity.id, activity.status);
        Ok(())
    })?
}

pub fn reject_activity(args: &ActivityIdArgs) -> Result<()> {
    block_on(async {
        let auth = resolve_auth(args.org.as_deref())?;
        let fingerprint = fetch_fingerprint(&auth, &args.activity_id).await?;

        let activity = auth
            .client
            .reject_activity(
                auth.org_id.clone(),
                current_timestamp_ms(),
                RejectActivityIntent { fingerprint },
            )
            .await
            .map_err(|e| anyhow!("reject_activity failed: {e}"))?;

        println!("activity {} status={:?}", activity.id, activity.status);
        Ok(())
    })?
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn invite_args(file: Option<&str>, user_name: Option<&str>, email: Option<&str>) -> InviteArgs {
        InviteArgs {
            file: file.map(String::from),
            user_name: user_name.map(String::from),
            email: email.map(String::from),
            access_type: "all".to_string(),
            tags: None,
            sender_user_id: None,
            allow_existing: false,
            include_existing: false,
            org: OrgArgs { org: None },
        }
    }

    #[test]
    fn partition_existing_splits_by_case_insensitive_email() {
        let invitations = vec![
            (
                InvitationParams {
                    receiver_user_name: "Alice".to_string(),
                    receiver_user_email: "Alice@Example.com".to_string(),
                    receiver_user_tags: vec![],
                    access_type: AccessType::All,
                    sender_user_id: String::new(),
                },
                false,
            ),
            (
                InvitationParams {
                    receiver_user_name: "Bob".to_string(),
                    receiver_user_email: "bob@example.com".to_string(),
                    receiver_user_tags: vec![],
                    access_type: AccessType::All,
                    sender_user_id: String::new(),
                },
                false,
            ),
        ];
        let mut existing = HashSet::new();
        existing.insert("alice@example.com".to_string());

        let (kept, skipped) = partition_existing(invitations, &existing);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].receiver_user_name, "Bob");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].receiver_user_name, "Alice");
    }

    #[test]
    fn canonical_email_lowercases_and_strips_plus_suffix() {
        assert_eq!(
            canonical_email("Alice+dev1@Example.com"),
            "alice@example.com"
        );
        assert_eq!(canonical_email("alice@example.com"), "alice@example.com");
        assert_eq!(
            canonical_email("ALICE+DEV2@EXAMPLE.COM"),
            "alice@example.com"
        );
    }

    #[test]
    fn partition_existing_treats_plus_alias_as_matching_existing_member() {
        let invitations = vec![(
            InvitationParams {
                receiver_user_name: "Alice Dev".to_string(),
                receiver_user_email: "alice+dev1@example.com".to_string(),
                receiver_user_tags: vec![],
                access_type: AccessType::All,
                sender_user_id: String::new(),
            },
            false,
        )];
        let mut existing = HashSet::new();
        existing.insert("alice@example.com".to_string()); // canonical form, as stored by the caller

        let (kept, skipped) = partition_existing(invitations, &existing);
        assert!(kept.is_empty());
        assert_eq!(skipped.len(), 1);
    }

    #[test]
    fn partition_existing_per_invitee_allow_existing_bypasses_alias_match() {
        let invitations = vec![(
            InvitationParams {
                receiver_user_name: "Alice Dev".to_string(),
                receiver_user_email: "Alice+Dev1@Example.com".to_string(),
                receiver_user_tags: vec![],
                access_type: AccessType::All,
                sender_user_id: String::new(),
            },
            true,
        )];
        let mut existing = HashSet::new();
        existing.insert("alice@example.com".to_string());

        let (kept, skipped) = partition_existing(invitations, &existing);
        assert_eq!(kept.len(), 1);
        assert!(skipped.is_empty());
    }

    fn test_config() -> TvcConfig {
        let mut orgs = HashMap::new();
        orgs.insert(
            "dev".to_string(),
            OrgEntry {
                id: "11111111-1111-1111-1111-111111111111".to_string(),
                api_key_path: PathBuf::from("/dev/key.json"),
                api_base_url: DEFAULT_API_BASE_URL.to_string(),
            },
        );
        orgs.insert(
            "prod".to_string(),
            OrgEntry {
                id: "22222222-2222-2222-2222-222222222222".to_string(),
                api_key_path: PathBuf::from("/prod/key.json"),
                api_base_url: DEFAULT_API_BASE_URL.to_string(),
            },
        );
        TvcConfig {
            active_org: Some("dev".to_string()),
            orgs,
        }
    }

    #[test]
    fn resolve_org_by_alias() {
        let config = test_config();
        let org = resolve_org(&config, Some("prod")).unwrap();
        assert_eq!(org.id, "22222222-2222-2222-2222-222222222222");
    }

    #[test]
    fn resolve_org_by_id_case_insensitive() {
        let config = test_config();
        let org = resolve_org(&config, Some("22222222-2222-2222-2222-222222222222")).unwrap();
        assert_eq!(org.api_key_path, PathBuf::from("/prod/key.json"));

        let org = resolve_org(&config, Some("AAAAAAAA-1111-1111-1111-111111111111"));
        assert!(org.is_none());

        let org = resolve_org(&config, Some("11111111-1111-1111-1111-111111111111")).unwrap();
        assert_eq!(org.api_key_path, PathBuf::from("/dev/key.json"));
    }

    #[test]
    fn resolve_org_falls_back_to_active_org_when_none_given() {
        let config = test_config();
        let org = resolve_org(&config, None).unwrap();
        assert_eq!(org.api_key_path, PathBuf::from("/dev/key.json"));
    }

    #[test]
    fn resolve_org_returns_none_for_unknown_alias_or_id() {
        let config = test_config();
        assert!(resolve_org(&config, Some("nonexistent")).is_none());
    }

    #[test]
    fn build_invitations_single_from_flags() {
        let mut args = invite_args(None, Some("Alice"), Some("alice@example.com"));
        args.tags = Some("tag-a, tag-b".to_string());
        args.access_type = "web".to_string();

        let invitations = build_invitations(&args).unwrap();
        assert_eq!(invitations.len(), 1);
        assert_eq!(invitations[0].0.receiver_user_name, "Alice");
        assert_eq!(invitations[0].0.receiver_user_email, "alice@example.com");
        assert_eq!(invitations[0].0.receiver_user_tags, vec!["tag-a", "tag-b"]);
        assert_eq!(invitations[0].0.access_type, AccessType::Web);
        assert!(!invitations[0].1, "allow_existing should default to false");
    }

    #[test]
    fn build_invitations_single_allow_existing_flag() {
        let mut args = invite_args(None, Some("Alice"), Some("alice@example.com"));
        args.allow_existing = true;

        let invitations = build_invitations(&args).unwrap();
        assert!(invitations[0].1);
    }

    #[test]
    fn build_invitations_defaults_access_type_to_all() {
        let args = invite_args(None, Some("Alice"), Some("alice@example.com"));

        let invitations = build_invitations(&args).unwrap();
        assert_eq!(invitations[0].0.access_type, AccessType::All);
        assert!(invitations[0].0.receiver_user_tags.is_empty());
    }

    #[test]
    fn build_invitations_batch_from_file_applies_default_and_override() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"{{
                "accessType": "all",
                "invitees": [
                    {{"userName": "Alice", "email": "alice@example.com", "tags": ["tag-a"]}},
                    {{"userName": "Bob", "email": "bob@example.com", "accessType": "web"}}
                ]
            }}"#
        )
        .unwrap();

        let args = invite_args(Some(file.path().to_str().unwrap()), None, None);

        let invitations = build_invitations(&args).unwrap();
        assert_eq!(invitations.len(), 2);
        assert_eq!(invitations[0].0.receiver_user_name, "Alice");
        assert_eq!(invitations[0].0.access_type, AccessType::All);
        assert_eq!(invitations[0].0.receiver_user_tags, vec!["tag-a"]);
        assert_eq!(invitations[1].0.receiver_user_name, "Bob");
        assert_eq!(invitations[1].0.access_type, AccessType::Web);
        assert!(invitations[1].0.receiver_user_tags.is_empty());
    }

    #[test]
    fn build_invitations_file_level_tags_default_and_per_invitee_override() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"{{
                "tags": ["group-tag"],
                "invitees": [
                    {{"userName": "Alice", "email": "alice@example.com"}},
                    {{"userName": "Bob", "email": "bob@example.com", "tags": ["bob-only-tag"]}},
                    {{"userName": "Carl", "email": "carl@example.com", "tags": []}}
                ]
            }}"#
        )
        .unwrap();

        let args = invite_args(Some(file.path().to_str().unwrap()), None, None);

        let invitations = build_invitations(&args).unwrap();
        assert_eq!(invitations[0].0.receiver_user_tags, vec!["group-tag"]);
        assert_eq!(invitations[1].0.receiver_user_tags, vec!["bob-only-tag"]);
        // An explicit empty array overrides the default entirely (not merged).
        assert!(invitations[2].0.receiver_user_tags.is_empty());
    }

    #[test]
    fn build_invitations_file_level_allow_existing_default_and_per_invitee_override() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"{{
                "allowExisting": true,
                "invitees": [
                    {{"userName": "Alice", "email": "alice@example.com"}},
                    {{"userName": "Bob", "email": "bob@example.com", "allowExisting": false}}
                ]
            }}"#
        )
        .unwrap();

        let args = invite_args(Some(file.path().to_str().unwrap()), None, None);

        let invitations = build_invitations(&args).unwrap();
        assert!(invitations[0].1, "Alice inherits the file-level default");
        assert!(!invitations[1].1, "Bob overrides the file-level default");
    }

    #[test]
    fn build_invitations_rejects_empty_file_invitee_list() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, r#"{{"invitees": []}}"#).unwrap();

        let args = invite_args(Some(file.path().to_str().unwrap()), None, None);

        let err = build_invitations(&args).unwrap_err();
        assert!(err.to_string().contains("no invitees"));
    }

    #[test]
    fn parse_access_type_accepts_known_values_case_insensitively() {
        assert_eq!(parse_access_type("WEB").unwrap(), AccessType::Web);
        assert_eq!(parse_access_type("api").unwrap(), AccessType::Api);
        assert_eq!(parse_access_type("All").unwrap(), AccessType::All);
    }

    #[test]
    fn parse_access_type_rejects_unknown_value() {
        assert!(parse_access_type("bogus").is_err());
    }

    #[test]
    fn parse_tags_trims_and_drops_empty_entries() {
        assert_eq!(
            parse_tags(" tag-a ,tag-b,, tag-c"),
            vec!["tag-a", "tag-b", "tag-c"]
        );
    }

    #[test]
    fn parse_effect_accepts_known_values_case_insensitively() {
        assert_eq!(parse_effect("ALLOW").unwrap(), Effect::Allow);
        assert_eq!(parse_effect("deny").unwrap(), Effect::Deny);
    }

    #[test]
    fn parse_effect_rejects_unknown_value() {
        assert!(parse_effect("bogus").is_err());
    }

    #[test]
    fn find_placeholders_extracts_all_tokens_in_order() {
        let content = "a {{FOO}} b {{BAR}} c {{FOO}}";
        assert_eq!(
            find_placeholders(content),
            vec!["FOO".to_string(), "BAR".to_string(), "FOO".to_string()]
        );
    }

    #[test]
    fn find_placeholders_returns_empty_for_no_tokens() {
        assert!(find_placeholders("no placeholders here").is_empty());
    }

    #[test]
    fn render_template_substitutes_all_occurrences() {
        let mut vars = HashMap::new();
        vars.insert("TAG".to_string(), "abc-123".to_string());
        assert_eq!(
            render_template("x={{TAG}} y={{TAG}}", &vars),
            "x=abc-123 y=abc-123"
        );
    }

    #[test]
    fn load_policies_file_errors_on_missing_var() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"{{"policies": [{{"policyName": "p", "effect": "allow", "consensus": "{{{{MISSING}}}}", "notes": ""}}]}}"#
        )
        .unwrap();

        let err = load_policies_file(file.path().to_str().unwrap(), None).unwrap_err();
        assert!(err.to_string().contains("MISSING"));
    }

    #[test]
    fn load_policies_file_renders_checked_in_releaser_initiator_template() {
        let mut vars = NamedTempFile::new().unwrap();
        write!(
            vars,
            r#"{{"RELEASER_TAG_ID": "releaser-uuid", "INITIATORS_TAG_ID": "initiators-uuid"}}"#
        )
        .unwrap();

        let policies = load_policies_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/templates/releaser-initiator-policies.json"
            ),
            Some(vars.path().to_str().unwrap()),
        )
        .unwrap();

        assert_eq!(policies.len(), 3);
        assert!(policies.iter().all(|p| p.effect == Effect::Allow));
        assert!(policies
            .iter()
            .filter(|p| p.policy_name.contains("releaser"))
            .all(|p| p.consensus.as_deref()
                == Some("approvers.any(user, user.tags.contains('releaser-uuid'))")));
        assert_eq!(
            policies
                .iter()
                .find(|p| p.policy_name.contains("initiators"))
                .unwrap()
                .consensus
                .as_deref(),
            Some("approvers.any(user, user.tags.contains('initiators-uuid'))")
        );
        // No stray {{...}} tokens should survive rendering.
        assert!(policies.iter().all(|p| !p
            .consensus
            .as_deref()
            .unwrap_or_default()
            .contains("{{")));
    }

    #[test]
    fn load_policies_file_renders_checked_in_readonly_template() {
        let mut vars = NamedTempFile::new().unwrap();
        write!(vars, r#"{{"READONLY_TAG_ID": "readonly-uuid"}}"#).unwrap();

        let policies = load_policies_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/templates/readonly-policy.json"
            ),
            Some(vars.path().to_str().unwrap()),
        )
        .unwrap();

        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].effect, Effect::Deny);
        assert_eq!(policies[0].condition.as_deref(), Some("true"));
        assert_eq!(
            policies[0].consensus.as_deref(),
            Some("approvers.any(user, user.tags.contains('readonly-uuid'))")
        );
    }
}
