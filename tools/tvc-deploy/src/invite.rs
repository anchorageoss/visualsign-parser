//! Turnkey org invitation management: `invite` and `dismiss-invite` subcommands.
//!
//! Auth resolves the same way as the official `tvc` CLI: env vars
//! (TVC_ORG_ID / TVC_API_KEY_PUBLIC / TVC_API_KEY_PRIVATE / TVC_API_BASE_URL)
//! take priority; otherwise falls back to ~/.config/turnkey/tvc.config.toml
//! (written by `tvc login`), selecting --org or the active org.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use turnkey_api_key_stamper::{Stamp, StampHeader, StamperError};
use turnkey_client::generated::immutable::common::v1::{AccessType, Effect};
use turnkey_client::generated::{
    ApproveActivityIntent, CreateInvitationsIntent, CreatePoliciesIntent, CreatePolicyIntentV3,
    CreateUserTagIntent, DeleteInvitationIntent, GetActivityRequest, GetPoliciesRequest,
    GetUsersRequest, GetWhoamiRequest, InvitationParams, ListUserTagsRequest, RejectActivityIntent,
};
use turnkey_client::TurnkeyP256ApiKey;
use turnkey_client::{TurnkeyClient, TurnkeyClientError, TurnkeySecp256k1ApiKey};

const DEFAULT_API_BASE_URL: &str = "https://api.turnkey.com";
const ENV_ORG_ID: &str = "TVC_ORG_ID";
const ENV_API_BASE_URL: &str = "TVC_API_BASE_URL";
const ENV_API_KEY_PUBLIC: &str = "TVC_API_KEY_PUBLIC";
const ENV_API_KEY_PRIVATE: &str = "TVC_API_KEY_PRIVATE";

pub const USAGE: &str = "\
    tvc-deploy invite --user-name <name> --email <email> \
    [--access-type web|api|all] [--tags t1,t2,...] [--sender-user-id <id>] \
    [--allow-existing true] [--org <alias-or-id>]\n  \
    tvc-deploy invite --file <invitees.json> [--access-type web|api|all] [--sender-user-id <id>] \
    [--include-existing true] [--org <alias-or-id>]\n  \
        (invitees.json: {\"accessType\": \"all\", \"allowExisting\": false, \"tags\": [\"tag-id\", \
    ...], \"invitees\": [{\"userName\": \"...\", \"email\": \"...\", \"tags\": [\"tag-id\", ...], \
    \"accessType\": \"...\", \"allowExisting\": true}, ...]}; per-invitee accessType/tags/ \
    allowExisting each override that same file-level default (accessType also falls back to \
    --access-type; tags default is [] if unset). By default, invitees whose canonical email \
    (lowercased, +suffix stripped, so alice+dev1@co.com and alice@co.com match) equals an \
    existing org member's are skipped with a printed reason instead of re-invited. Set \
    \"allowExisting\": true for the specific invitee (or at the file's top level, for the whole \
    batch) to invite it anyway, e.g. when alice@co.com is already a member but you deliberately \
    want alice+dev1@co.com as a separate dev-test account -- do this in the file you already \
    curated, not by repeating emails on the command line. --include-existing true instead \
    disables the whole existing-member check (skips the get_users lookup entirely).)\n  \
    tvc-deploy dismiss-invite --invitation-id <id> [--org <alias-or-id>]\n  \
    tvc-deploy approve-activity --activity-id <id> [--org <alias-or-id>]\n  \
    tvc-deploy reject-activity --activity-id <id> [--org <alias-or-id>]\n  \
    tvc-deploy create-tag --name <name> [--user-ids id1,id2,...] [--org <alias-or-id>]\n  \
        (--user-ids retroactively tags existing users; to tag people being invited, put the \
    new tag's id in invitees.json's top-level \"tags\" -- applied to every invitee that \
    doesn't set its own \"tags\" -- since invitees aren't users yet and can't be passed here)\n  \
    tvc-deploy list-tags [--org <alias-or-id>]\n  \
        (prints user-tag id + name pairs; use the id in invitees.json \"tags\", not the name)\n  \
    tvc-deploy list-policies [--org <alias-or-id>]\n  \
    tvc-deploy create-policy --name <name> --effect allow|deny --notes <text> \
    [--condition <tql>] [--consensus <tql>] [--org <alias-or-id>]\n  \
    tvc-deploy create-policies --file <policies.json> [--vars <vars.json>] [--dry-run true] \
    [--org <alias-or-id>]\n  \
        (policies.json: {\"policies\": [{\"policyName\": \"...\", \"effect\": \"allow\"|\"deny\", \
    \"condition\": \"<tql>\", \"consensus\": \"<tql>\", \"notes\": \"...\"}, ...]}; \
    condition/consensus may contain {{PLACEHOLDER}} tokens filled in from --vars, a flat JSON \
    object of {\"PLACEHOLDER\": \"value\"}; --dry-run (any value; every flag needs one) renders \
    and prints without submitting, for checking a template before it hits an org)\n  \
    (auth resolves via TVC_ORG_ID/TVC_API_KEY_PUBLIC/TVC_API_KEY_PRIVATE env vars, \
    else ~/.config/turnkey/tvc.config.toml from `tvc login`; if an invite/dismiss activity \
    needs consensus, it prints the activity id -- approve or reject it with the subcommands above, \
    authenticated as another quorum member if needed)";

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
/// may be the config alias (e.g. "Anchorage Digital (Development)") or the
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

    let org = resolve_org(&config, org_override);
    let org = org.ok_or_else(|| match org_override {
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

fn parse_access_type(s: &str) -> Result<AccessType> {
    match s.to_ascii_lowercase().as_str() {
        "web" => Ok(AccessType::Web),
        "api" => Ok(AccessType::Api),
        "all" => Ok(AccessType::All),
        other => bail!("--access-type must be one of web|api|all, got {other:?}"),
    }
}

fn req<'a>(flags: &'a HashMap<String, String>, key: &str) -> Result<&'a String> {
    flags.get(key).with_context(|| format!("missing --{key}"))
}

fn parse_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// One team member to invite, as listed in a `--file` batch (see [`USAGE`]).
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
fn build_invitations(flags: &HashMap<String, String>) -> Result<Vec<(InvitationParams, bool)>> {
    let flag_access_type = flags
        .get("access-type")
        .map(String::as_str)
        .unwrap_or("all");

    if let Some(path) = flags.get("file") {
        let content = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
        let file: InviteesFile =
            serde_json::from_str(&content).with_context(|| format!("parse {path}"))?;
        if file.invitees.is_empty() {
            bail!("{path} lists no invitees");
        }
        let default_access = match &file.access_type {
            Some(s) => parse_access_type(s)?,
            None => parse_access_type(flag_access_type)?,
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
        let user_name = req(flags, "user-name")?.clone();
        let email = req(flags, "email")?.clone();
        let access_type = parse_access_type(flag_access_type)?;
        let tags = flags.get("tags").map(|s| parse_tags(s)).unwrap_or_default();
        let allow_existing = flags.contains_key("allow-existing");
        Ok(vec![(
            InvitationParams {
                receiver_user_name: user_name,
                receiver_user_email: email,
                receiver_user_tags: tags,
                access_type,
                sender_user_id: String::new(),
            },
            allow_existing,
        )])
    }
}

/// Split `invitations` into (kept, skipped) by case-insensitive email
/// membership in `existing_emails`.
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
    existing_emails: &std::collections::HashSet<String>,
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

pub fn invite(flags: &HashMap<String, String>) -> Result<()> {
    let invitations_with_allow = build_invitations(flags)?;
    let include_existing = flags.contains_key("include-existing");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

        let mut invitations = if include_existing {
            invitations_with_allow.into_iter().map(|(i, _)| i).collect()
        } else {
            let users = auth
                .client
                .get_users(GetUsersRequest {
                    organization_id: auth.org_id.clone(),
                })
                .await
                .map_err(|e| anyhow!("get_users failed: {e}"))?;
            let existing_emails: std::collections::HashSet<String> = users
                .users
                .into_iter()
                .filter_map(|u| u.user_email)
                .map(|e| canonical_email(&e))
                .collect();

            let (kept, skipped) = partition_existing(invitations_with_allow, &existing_emails);
            for s in &skipped {
                println!(
                    "skipping {} <{}>: matches an existing member of org {} \
                     (set \"allowExisting\": true for this invitee, or at the top level of \
                     the file, to invite it anyway)",
                    s.receiver_user_name, s.receiver_user_email, auth.org_id
                );
            }
            if kept.is_empty() {
                println!(
                    "nothing to invite: everyone in the list already matches an existing member"
                );
                return Ok(());
            }
            kept
        };

        let sender_user_id = match flags.get("sender-user-id") {
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
    })
}

fn parse_effect(s: &str) -> Result<Effect> {
    match s.to_ascii_lowercase().as_str() {
        "allow" => Ok(Effect::Allow),
        "deny" => Ok(Effect::Deny),
        other => bail!("--effect must be allow|deny, got {other:?}"),
    }
}

pub fn list_policies(flags: &HashMap<String, String>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

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
    })
}

pub fn create_policy(flags: &HashMap<String, String>) -> Result<()> {
    let policy_name = req(flags, "name")?.clone();
    let effect = parse_effect(req(flags, "effect")?)?;
    let notes = flags.get("notes").cloned().unwrap_or_default();
    let condition = flags.get("condition").cloned();
    let consensus = flags.get("consensus").cloned();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

        let outcome = auth
            .client
            .create_policy(
                auth.org_id.clone(),
                current_timestamp_ms(),
                CreatePolicyIntentV3 {
                    policy_name,
                    effect,
                    condition,
                    consensus,
                    notes,
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
    })
}

/// One policy as listed in a `--file` batch for `create-policies` (see [`USAGE`]).
/// Uses the CLI's friendly "allow"/"deny" rather than Turnkey's own
/// "EFFECT_ALLOW"/"EFFECT_DENY" wire format, for consistency with `create-policy`.
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

pub fn create_policies(flags: &HashMap<String, String>) -> Result<()> {
    let path = req(flags, "file")?;
    let policies = load_policies_file(path, flags.get("vars").map(String::as_str))?;
    let intent = CreatePoliciesIntent { policies };

    if flags.contains_key("dry-run") {
        println!("{}", serde_json::to_string_pretty(&intent)?);
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

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
    })
}

pub fn create_tag(flags: &HashMap<String, String>) -> Result<()> {
    let user_tag_name = req(flags, "name")?.clone();
    let user_ids = flags
        .get("user-ids")
        .map(|s| parse_tags(s))
        .unwrap_or_default();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

        let outcome = auth
            .client
            .create_user_tag(
                auth.org_id.clone(),
                current_timestamp_ms(),
                CreateUserTagIntent {
                    user_tag_name,
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
    })
}

pub fn list_tags(flags: &HashMap<String, String>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

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
    })
}

pub fn dismiss_invite(flags: &HashMap<String, String>) -> Result<()> {
    let invitation_id = req(flags, "invitation-id")?.clone();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;

        let outcome = auth
            .client
            .delete_invitation(
                auth.org_id.clone(),
                current_timestamp_ms(),
                DeleteInvitationIntent { invitation_id },
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
    })
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

pub fn approve_activity(flags: &HashMap<String, String>) -> Result<()> {
    let activity_id = req(flags, "activity-id")?.clone();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;
        let fingerprint = fetch_fingerprint(&auth, &activity_id).await?;

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
    })
}

pub fn reject_activity(flags: &HashMap<String, String>) -> Result<()> {
    let activity_id = req(flags, "activity-id")?.clone();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    rt.block_on(async {
        let auth = resolve_auth(flags.get("org").map(String::as_str))?;
        let fingerprint = fetch_fingerprint(&auth, &activity_id).await?;

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
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

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
        let mut existing = std::collections::HashSet::new();
        existing.insert("alice@example.com".to_string());

        let (kept, skipped) = partition_existing(invitations, &existing);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].receiver_user_name, "Bob");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].receiver_user_name, "Alice");
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
        let mut existing = std::collections::HashSet::new();
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
        let mut existing = std::collections::HashSet::new();
        existing.insert("alice@example.com".to_string());

        let (kept, skipped) = partition_existing(invitations, &existing);
        assert_eq!(kept.len(), 1);
        assert!(skipped.is_empty());
    }

    #[test]
    fn build_invitations_single_from_flags() {
        let mut flags = HashMap::new();
        flags.insert("user-name".to_string(), "Alice".to_string());
        flags.insert("email".to_string(), "alice@example.com".to_string());
        flags.insert("tags".to_string(), "tag-a, tag-b".to_string());
        flags.insert("access-type".to_string(), "web".to_string());

        let invitations = build_invitations(&flags).unwrap();
        assert_eq!(invitations.len(), 1);
        assert_eq!(invitations[0].0.receiver_user_name, "Alice");
        assert_eq!(invitations[0].0.receiver_user_email, "alice@example.com");
        assert_eq!(invitations[0].0.receiver_user_tags, vec!["tag-a", "tag-b"]);
        assert_eq!(invitations[0].0.access_type, AccessType::Web);
        assert!(!invitations[0].1, "allow_existing should default to false");
    }

    #[test]
    fn build_invitations_single_allow_existing_flag() {
        let mut flags = HashMap::new();
        flags.insert("user-name".to_string(), "Alice".to_string());
        flags.insert("email".to_string(), "alice@example.com".to_string());
        flags.insert("allow-existing".to_string(), "true".to_string());

        let invitations = build_invitations(&flags).unwrap();
        assert!(invitations[0].1);
    }

    #[test]
    fn build_invitations_defaults_access_type_to_all() {
        let mut flags = HashMap::new();
        flags.insert("user-name".to_string(), "Alice".to_string());
        flags.insert("email".to_string(), "alice@example.com".to_string());

        let invitations = build_invitations(&flags).unwrap();
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

        let mut flags = HashMap::new();
        flags.insert(
            "file".to_string(),
            file.path().to_str().unwrap().to_string(),
        );

        let invitations = build_invitations(&flags).unwrap();
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

        let mut flags = HashMap::new();
        flags.insert(
            "file".to_string(),
            file.path().to_str().unwrap().to_string(),
        );

        let invitations = build_invitations(&flags).unwrap();
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

        let mut flags = HashMap::new();
        flags.insert(
            "file".to_string(),
            file.path().to_str().unwrap().to_string(),
        );

        let invitations = build_invitations(&flags).unwrap();
        assert!(invitations[0].1, "Alice inherits the file-level default");
        assert!(!invitations[1].1, "Bob overrides the file-level default");
    }

    #[test]
    fn build_invitations_rejects_empty_file_invitee_list() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, r#"{{"invitees": []}}"#).unwrap();

        let mut flags = HashMap::new();
        flags.insert(
            "file".to_string(),
            file.path().to_str().unwrap().to_string(),
        );

        let err = build_invitations(&flags).unwrap_err();
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
