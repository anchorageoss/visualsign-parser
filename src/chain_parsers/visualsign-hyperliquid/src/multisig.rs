//! Complete native HyperCore multisig requests. Configuration is caller-supplied
//! context, never a proof of current membership. Consumers must authenticate/pin it.
use std::collections::BTreeSet;

use alloy_dyn_abi::TypedData;
use alloy_primitives::keccak256;
use k256::ecdsa::{RecoveryId, Signature as EcdsaSignature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};

use crate::input::{self, L1Request, UserSignedRequest, hex_lower};
use crate::{hash, user_signed};

type Result<T> = std::result::Result<T, String>;
const ZERO: &str = "0x0000000000000000000000000000000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Conversion,
    Member,
    Leader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
}
impl Network {
    pub fn chain(self) -> &'static str {
        if self == Self::Mainnet {
            "Mainnet"
        } else {
            "Testnet"
        }
    }
    fn source(self) -> &'static str {
        if self == Self::Mainnet { "a" } else { "b" }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AccountConfig {
    pub address: String,
    pub authorized_users: Vec<String>,
    pub threshold: usize,
}

/// SDK signature quantities. Padded values are accepted and normalized before
/// constructing the native leader envelope, matching Python's hex(integer).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub r: String,
    pub s: String,
    pub v: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Envelope<'a> {
    kind: String,
    role: Role,
    network: Network,
    signer: String,
    multisig: Option<AccountConfig>,
    leader: Option<String>,
    #[serde(borrow)]
    action: &'a RawValue,
    nonce: u64,
    vault_address: Option<String>,
    expires_after: Option<u64>,
    #[serde(default)]
    signatures: Vec<Signature>,
    signing_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub enum InnerAction {
    L1(Box<L1Request>),
    UserSigned(UserSignedRequest),
}

/// Parsed request. The digest is derived from the complete validated action.
/// No signer ownership, current membership, freshness or execution is established.
#[derive(Debug, Clone)]
pub struct MultisigRequest {
    pub role: Role,
    pub network: Network,
    pub signer: String,
    pub multisig: Option<AccountConfig>,
    pub leader: Option<String>,
    pub action: InnerAction,
    pub nonce: u64,
    pub expires_after: Option<u64>,
    pub signing_digest: [u8; 32],
    pub member_digest: [u8; 32],
    pub verified_signers: Vec<String>,
    /// Exact SDK-ordered native action for a leader request. Signature scalars are
    /// minimally encoded. Consumers must use these bytes when submitting the
    /// action whose digest was reviewed, not reserialize the original request.
    /// This is public transaction data, not a broadcast operation.
    pub leader_action_json: Option<String>,
}

fn hex(bytes: &[u8]) -> String {
    format!("0x{}", hex_lower(bytes))
}
fn address(s: &str) -> Result<String> {
    if visualsign::encodings::split_hex_prefix(s).is_none() {
        return Err("address requires 0x prefix".into());
    }
    let a = visualsign::encodings::decode_hex_array::<20>(s)
        .map_err(|e| format!("invalid address {e}"))?;
    let out = hex(&a);
    if out != s {
        return Err("multisig identities must use lowercase canonical addresses".into());
    }
    Ok(out)
}
fn members(users: &[String], threshold: usize) -> Result<()> {
    if users.is_empty() || users.len() > 10 || threshold == 0 || threshold > users.len() {
        return Err("invalid multisig member count or threshold".into());
    }
    let mut seen = BTreeSet::new();
    for user in users {
        address(user)?;
        if user == ZERO || !seen.insert(user) {
            return Err("zero or duplicate multisig member".into());
        }
    }
    Ok(())
}

/// Validate the JSON string signed by ConvertToMultiSigUser. `null` removes
/// protection; it is not an empty or malformed configuration.
pub fn conversion_signers(s: &str) -> Result<Option<(Vec<String>, usize)>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Signers {
        authorized_users: Vec<String>,
        threshold: usize,
    }
    if s == "null" {
        return Ok(None);
    }
    input::check_depth(s)?;
    let v = crate::multisig_strict::parse(s)?;
    let config: Signers = serde_json::from_value(v).map_err(|e| format!("invalid signers: {e}"))?;
    members(&config.authorized_users, config.threshold)?;
    Ok(Some((config.authorized_users, config.threshold)))
}

fn typed(primary: &str, fields: &[(&str, &str, Value)], l1: bool) -> Value {
    let mut types = serde_json::Map::new();
    types.insert(
        "EIP712Domain".into(),
        json!([
            {"name":"name","type":"string"}, {"name":"version","type":"string"},
            {"name":"chainId","type":"uint256"}, {"name":"verifyingContract","type":"address"}
        ]),
    );
    types.insert(
        primary.into(),
        Value::Array(
            fields
                .iter()
                .map(|(n, t, _)| json!({"name":n,"type":t}))
                .collect(),
        ),
    );
    let message: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(n, _, v)| ((*n).into(), v.clone()))
        .collect();
    json!({"types":types,"primaryType":primary,"domain":{"name":if l1 {"Exchange"} else {"HyperliquidSignTransaction"},"version":"1","chainId":if l1 {1337} else {421614},"verifyingContract":ZERO},"message":message})
}
fn digest(data: Value) -> Result<[u8; 32]> {
    let typed: TypedData = serde_json::from_value(data).map_err(|e| e.to_string())?;
    typed
        .eip712_signing_hash()
        .map(|h| h.0)
        .map_err(|e| e.to_string())
}
fn phantom(network: Network, connection: [u8; 32]) -> Value {
    typed(
        "Agent",
        &[
            ("source", "string", json!(network.source())),
            ("connectionId", "bytes32", json!(hex(&connection))),
        ],
        true,
    )
}
fn scalar(s: &str) -> Result<[u8; 32]> {
    let h = visualsign::encodings::split_hex_prefix(s).ok_or("signature scalar needs 0x prefix")?;
    if h.is_empty() || h.len() > 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid signature scalar".into());
    }
    visualsign::encodings::decode_hex_array::<32>(&format!("{h:0>64}"))
        .map_err(|e| format!("invalid scalar {e}"))
}
fn normalized(sig: &Signature) -> Result<Signature> {
    let quantity = |s: &str| -> Result<String> {
        let bytes = scalar(s)?;
        let full = hex_lower(&bytes);
        let h = full.trim_start_matches('0');
        Ok(format!("0x{}", if h.is_empty() { "0" } else { h }))
    };
    Ok(Signature {
        r: quantity(&sig.r)?,
        s: quantity(&sig.s)?,
        v: sig.v,
    })
}
/// Recover a low-s SDK signature; the caller still supplies the trusted membership.
pub fn recover_address(digest: &[u8; 32], sig: &Signature) -> Result<String> {
    let signature = EcdsaSignature::from_scalars(scalar(&sig.r)?, scalar(&sig.s)?)
        .map_err(|e| e.to_string())?;
    if signature.normalize_s().is_some() {
        return Err("high-s signature".into());
    }
    let v = sig
        .v
        .checked_sub(27)
        .filter(|v| *v <= 1)
        .ok_or("signature v must be 27 or 28")?;
    let recovery = RecoveryId::from_byte(v).ok_or("invalid recovery id")?;
    let key = VerifyingKey::recover_from_prehash(digest, &signature, recovery)
        .map_err(|e| e.to_string())?;
    let point = key.to_encoded_point(false);
    Ok(hex(&keccak256(&point.as_bytes()[1..])[12..]))
}

pub(crate) fn parse(input: &str) -> Result<MultisigRequest> {
    let req: Envelope<'_> = serde_json::from_str(input).map_err(|e| e.to_string())?;
    if req.kind != "multisig" {
        return Err("expected multisig envelope".into());
    }
    address(&req.signer)?;
    if req.signer == ZERO {
        return Err("zero signing identity".into());
    }
    let action: Value = serde_json::from_str(req.action.get()).map_err(|e| e.to_string())?;
    let action_type = action
        .get("type")
        .and_then(Value::as_str)
        .ok_or("missing action type")?;
    if req.signatures.len() > 10 {
        return Err("too many signatures".into());
    }
    if req.role == Role::Conversion {
        if req.multisig.is_some()
            || req.leader.is_some()
            || !req.signatures.is_empty()
            || action_type != "convertToMultiSigUser"
        {
            return Err(
                "conversion requires only the original signer and conversion action".into(),
            );
        }
    } else {
        let config = req
            .multisig
            .as_ref()
            .ok_or("missing caller-supplied config")?;
        address(&config.address)?;
        if config.address == ZERO {
            return Err("zero multisig address".into());
        }
        members(&config.authorized_users, config.threshold)?;
        if config.authorized_users.contains(&config.address) {
            return Err("multisig account cannot be its own member".into());
        }
        let leader = req.leader.as_ref().ok_or("missing leader")?;
        address(leader)?;
        if !config.authorized_users.contains(leader)
            || !config.authorized_users.contains(&req.signer)
        {
            return Err("signer and direct leader must belong to supplied membership".into());
        }
        if req.role == Role::Leader && &req.signer != leader {
            return Err("leader signer mismatch".into());
        }
        if req.role == Role::Member && !req.signatures.is_empty() {
            return Err("member requests cannot carry signatures".into());
        }
    }
    let vault = req
        .vault_address
        .as_deref()
        .map(|a| {
            address(a)?;
            visualsign::encodings::decode_hex_array::<20>(a).map_err(|e| e.to_string())
        })
        .transpose()?;
    let spec = user_signed::SPECS
        .iter()
        .find(|s| s.action_type == action_type);
    let (inner, member_digest) = if let Some(spec) = spec {
        if req.vault_address.is_some() || req.expires_after.is_some() {
            return Err("user-signed actions do not bind vault/expiry".into());
        }
        if action.get("hyperliquidChain").and_then(Value::as_str) != Some(req.network.chain())
            || action.get("signatureChainId").and_then(Value::as_str) != Some("0x66eee")
        {
            return Err("user-signed network or signatureChainId mismatch".into());
        }
        if action
            .get("nonce")
            .or_else(|| action.get("time"))
            .and_then(Value::as_u64)
            != Some(req.nonce)
        {
            return Err("action nonce mismatch".into());
        }
        if action_type == "convertToMultiSigUser" {
            let next = conversion_signers(
                action
                    .get("signers")
                    .and_then(Value::as_str)
                    .ok_or("missing signers")?,
            )?;
            if let Some((users, _)) = &next {
                let account = req
                    .multisig
                    .as_ref()
                    .map(|c| &c.address)
                    .unwrap_or(&req.signer);
                if users.contains(account) {
                    return Err(
                        "multisig account cannot be a member of the new configuration".into(),
                    );
                }
            }
            if req.role == Role::Conversion && next.is_none() {
                return Err("initial conversion cannot remove multisig".into());
            }
        }
        let fields: Vec<_> = spec
            .fields
            .iter()
            .map(|(n, t)| (*n, *t, action.get(*n).cloned().unwrap_or(Value::Null)))
            .collect();
        let primary = format!("HyperliquidTransaction:{}", spec.name);
        let mut single = typed(&primary, &fields, false);
        single["message"] = action.clone();
        let inner = input::parse_user_signed(&single.to_string())?;
        let mut bound = vec![fields[0].clone()];
        if let Some(config) = &req.multisig {
            bound.push(("payloadMultiSigUser", "address", json!(config.address)));
            bound.push(("outerSigner", "address", json!(req.leader)));
        }
        bound.extend(fields.into_iter().skip(1));
        (
            InnerAction::UserSigned(inner),
            digest(typed(&primary, &bound, false))?,
        )
    } else {
        let config = req
            .multisig
            .as_ref()
            .ok_or("L1 actions require multisig context")?;
        let packed = hash::action_msgpack(req.action.get())?;
        let single_hash =
            hash::connection_id(&packed, req.nonce, vault.as_ref(), req.expires_after);
        let envelope = format!(
            "{{\"kind\":\"l1\",\"typedData\":{},\"action\":{},\"nonce\":{},\"vaultAddress\":{},\"expiresAfter\":{}}}",
            phantom(req.network, single_hash),
            req.action.get(),
            req.nonce,
            json!(req.vault_address),
            json!(req.expires_after)
        );
        let input::Request::L1(inner) = input::parse(&envelope)? else {
            return Err("invalid L1 inner action".into());
        };
        let wrapped = format!(
            "[{},{},{}]",
            json!(config.address),
            json!(req.leader),
            req.action.get()
        );
        let packed = hash::action_msgpack(&wrapped)?;
        let bound_hash = hash::connection_id(&packed, req.nonce, vault.as_ref(), req.expires_after);
        (
            InnerAction::L1(inner),
            digest(phantom(req.network, bound_hash))?,
        )
    };
    let mut verified_signers = Vec::new();
    let mut leader_action_json = None;
    let signing_digest = if req.role == Role::Leader {
        let config = req.multisig.as_ref().ok_or("missing config")?;
        if req.signatures.len() < config.threshold {
            return Err("insufficient member signatures".into());
        }
        for signature in &req.signatures {
            let signer = recover_address(&member_digest, signature)?;
            if !config.authorized_users.contains(&signer) || verified_signers.contains(&signer) {
                return Err("unauthorized or duplicate member signature".into());
            }
            verified_signers.push(signer);
        }
        let signatures: Vec<_> = req
            .signatures
            .iter()
            .map(normalized)
            .collect::<Result<_>>()?;
        // Native leader hashing excludes `type` but includes the ordered signatures.
        let outer = format!(
            "{{\"signatureChainId\":\"0x66eee\",\"signatures\":{},\"payload\":{{\"multiSigUser\":{},\"outerSigner\":{},\"action\":{}}}}}",
            serde_json::to_string(&signatures).map_err(|e| e.to_string())?,
            json!(config.address),
            json!(req.leader),
            req.action.get()
        );
        let tail = outer
            .strip_prefix('{')
            .ok_or("invalid generated leader action")?;
        leader_action_json = Some(format!("{{\"type\":\"multiSig\",{tail}"));
        let packed = hash::action_msgpack(&outer)?;
        let outer_hash = hash::connection_id(&packed, req.nonce, vault.as_ref(), req.expires_after);
        digest(typed(
            "HyperliquidTransaction:SendMultiSig",
            &[
                ("hyperliquidChain", "string", json!(req.network.chain())),
                ("multiSigActionHash", "bytes32", json!(hex(&outer_hash))),
                ("nonce", "uint64", json!(req.nonce)),
            ],
            false,
        ))?
    } else {
        member_digest
    };
    if let Some(expected) = req.signing_digest {
        let expected = visualsign::encodings::decode_hex_array::<32>(&expected)
            .map_err(|e| format!("invalid signing digest {e}"))?;
        if expected != signing_digest {
            return Err("signingDigest mismatch".into());
        }
    }
    Ok(MultisigRequest {
        role: req.role,
        network: req.network,
        signer: req.signer,
        multisig: req.multisig,
        leader: req.leader,
        action: inner,
        nonce: req.nonce,
        expires_after: req.expires_after,
        signing_digest,
        member_digest,
        verified_signers,
        leader_action_json,
    })
}
