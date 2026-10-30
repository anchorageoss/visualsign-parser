//! Input parsing and verification.
//!
//! The `unsigned_payload` is one JSON document:
//!
//! ```text
//! {"kind":"l1","typedData":{...},"action":{...},"nonce":<u64>,"vaultAddress":null|"0x..","expiresAfter":null|<u64>}
//! {"kind":"userSigned","typedData":{...}}
//! ```
//!
//! For `l1`, `typedData` is what gets signed and `action` is only a claim about what it
//! commits to. The claim is accepted only when the `connectionId` recomputed from the
//! action's exact bytes (plus nonce, vault and expiry) equals the one in `typedData`,
//! and when the typed struct the display is built from re-encodes to those same bytes.

use std::collections::BTreeMap;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::hash;
use crate::l1::{ALLOWED_TYPES, L1Action};
use crate::user_signed::{self, FieldValue, UserSignedSpec};

/// Upper bound on the whole input, checked before any parsing.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;
/// Upper bound on JSON nesting anywhere in the input. The deepest supported action
/// (`perpDeploy.setOracle.markPxs`) nests 6 levels below the envelope.
pub const MAX_DEPTH: usize = 16;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
const EIP712_DOMAIN_FIELDS: &[(&str, &str)] = &[
    ("name", "string"),
    ("version", "string"),
    ("chainId", "uint256"),
    ("verifyingContract", "address"),
];
const AGENT_FIELDS: &[(&str, &str)] = &[("source", "string"), ("connectionId", "bytes32")];

/// A verified L1 action.
#[derive(Debug, Clone)]
pub struct L1Request {
    pub mainnet: bool,
    pub action: L1Action,
    /// `type` of the action, as received.
    pub action_type: String,
    pub nonce: u64,
    pub vault_address: Option<[u8; 20]>,
    pub expires_after: Option<u64>,
    pub connection_id: [u8; 32],
}

/// A checked user-signed action.
#[derive(Debug, Clone)]
pub struct UserSignedRequest {
    pub spec_name: &'static str,
    pub chain_id: u64,
    pub fields: Vec<(&'static str, FieldValue)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Envelope<'a> {
    kind: String,
    #[serde(borrow)]
    typed_data: &'a RawValue,
    #[serde(default, borrow)]
    action: Option<&'a RawValue>,
    #[serde(default)]
    nonce: Option<u64>,
    #[serde(default)]
    vault_address: Option<String>,
    #[serde(default)]
    expires_after: Option<u64>,
}

/// A JSON object that refuses duplicate keys.
///
/// serde_json keeps the last of two equal keys, but another EIP-712 encoder could keep
/// the first, so a duplicate in `types` or in a user-signed `message` would let the
/// display and the signature disagree. Structs already refuse duplicate fields; these
/// two are free-form maps, so they are checked here.
struct StrictMap<V>(BTreeMap<String, V>);

impl<'de, V: Deserialize<'de>> Deserialize<'de> for StrictMap<V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct StrictVisitor<V>(std::marker::PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for StrictVisitor<V> {
            type Value = StrictMap<V>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object without duplicate keys")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = BTreeMap::new();
                while let Some((key, value)) = access.next_entry::<String, V>()? {
                    if map.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!("duplicate key '{key}'")));
                    }
                    map.insert(key, value);
                }
                Ok(StrictMap(map))
            }
        }
        d.deserialize_map(StrictVisitor(std::marker::PhantomData))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedField {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Domain {
    name: String,
    version: String,
    chain_id: u64,
    verifying_contract: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TypedData<M> {
    types: StrictMap<Vec<TypedField>>,
    primary_type: String,
    domain: Domain,
    message: M,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AgentMessage {
    source: String,
    connection_id: String,
}

/// A parsed and verified input.
#[derive(Debug, Clone)]
pub enum Request {
    L1(Box<L1Request>),
    UserSigned(UserSignedRequest),
}

pub fn parse(input: &str) -> Result<Request, String> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "input is {} bytes; the limit is {MAX_INPUT_BYTES}",
            input.len()
        ));
    }
    check_depth(input)?;
    let envelope: Envelope<'_> =
        serde_json::from_str(input.trim()).map_err(|e| format!("invalid envelope: {e}"))?;
    match envelope.kind.as_str() {
        "l1" => parse_l1(&envelope).map(|r| Request::L1(Box::new(r))),
        "userSigned" => {
            if envelope.action.is_some()
                || envelope.nonce.is_some()
                || envelope.vault_address.is_some()
                || envelope.expires_after.is_some()
            {
                return Err(
                    "userSigned input takes only typedData; action, nonce, vaultAddress and expiresAfter are L1 fields"
                        .to_string(),
                );
            }
            parse_user_signed(envelope.typed_data.get()).map(Request::UserSigned)
        }
        other => Err(format!(
            "unknown kind '{other}' (expected l1 or userSigned)"
        )),
    }
}

/// Refuse JSON nested deeper than [`MAX_DEPTH`], scanning bytes outside strings.
fn check_depth(input: &str) -> Result<(), String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for b in input.bytes() {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(format!("input nests deeper than {MAX_DEPTH} levels"));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn check_types(
    types: &BTreeMap<String, Vec<TypedField>>,
    primary_type: &str,
    expected: &[(&str, &str)],
) -> Result<(), String> {
    let matches = |fields: &Vec<TypedField>, want: &[(&str, &str)]| {
        fields.len() == want.len()
            && fields
                .iter()
                .zip(want)
                .all(|(f, (n, t))| f.name == *n && f.ty == *t)
    };
    if types.len() != 2 {
        return Err(format!(
            "typedData.types must hold exactly EIP712Domain and {primary_type}"
        ));
    }
    let domain = types
        .get("EIP712Domain")
        .ok_or("typedData.types is missing EIP712Domain")?;
    if !matches(domain, EIP712_DOMAIN_FIELDS) {
        return Err("typedData.types.EIP712Domain does not match the pinned definition".into());
    }
    let primary = types
        .get(primary_type)
        .ok_or_else(|| format!("typedData.types is missing {primary_type}"))?;
    if !matches(primary, expected) {
        return Err(format!(
            "typedData.types.{primary_type} does not match the pinned definition"
        ));
    }
    Ok(())
}

fn parse_hex_array<const N: usize>(what: &str, s: &str) -> Result<[u8; N], String> {
    if !(s.starts_with("0x") || s.starts_with("0X")) {
        return Err(format!("{what} must be 0x-prefixed hex"));
    }
    visualsign::encodings::decode_hex_array::<N>(s).map_err(|e| format!("Invalid {what} {e}"))
}

fn parse_l1(envelope: &Envelope<'_>) -> Result<L1Request, String> {
    let typed: TypedData<AgentMessage> = serde_json::from_str(envelope.typed_data.get())
        .map_err(|e| format!("typedData is not a Hyperliquid L1 (Agent) payload: {e}"))?;
    if typed.primary_type != "Agent" {
        return Err(format!(
            "L1 primaryType must be Agent, got '{}'",
            typed.primary_type
        ));
    }
    check_types(&typed.types.0, "Agent", AGENT_FIELDS)?;
    let d = &typed.domain;
    if d.name != "Exchange"
        || d.version != "1"
        || d.chain_id != 1337
        || !d.verifying_contract.eq_ignore_ascii_case(ZERO_ADDRESS)
    {
        return Err(
            "L1 domain must be {name: Exchange, version: 1, chainId: 1337, verifyingContract: 0x0}"
                .to_string(),
        );
    }
    let mainnet = match typed.message.source.as_str() {
        "a" => true,
        "b" => false,
        other => return Err(format!("unknown L1 source '{other}' (expected a or b)")),
    };
    let signed_connection_id: [u8; 32] =
        parse_hex_array("connectionId", &typed.message.connection_id)?;

    let raw_action = envelope.action.ok_or("l1 input requires the action")?.get();
    let nonce = envelope.nonce.ok_or("l1 input requires the nonce")?;
    let vault_address = envelope
        .vault_address
        .as_deref()
        .map(|v| parse_hex_array::<20>("vaultAddress", v))
        .transpose()?;

    // Integers only: a float would hash as msgpack float64 and could not be shown as signed.
    let action_value: Value =
        serde_json::from_str(raw_action).map_err(|e| format!("invalid action JSON: {e}"))?;
    refuse_non_integers(&action_value)?;
    let action_type = action_value
        .get("type")
        .and_then(Value::as_str)
        .ok_or("action has no string type")?
        .to_string();
    if !ALLOWED_TYPES.contains(&action_type.as_str()) {
        return Err(format!("unsupported L1 action type '{action_type}'"));
    }

    let packed = hash::action_msgpack(raw_action)?;
    let computed = hash::connection_id(
        &packed,
        nonce,
        vault_address.as_ref(),
        envelope.expires_after,
    );
    if computed != signed_connection_id {
        return Err(format!(
            "connectionId mismatch: the action, nonce, vaultAddress and expiresAfter hash to 0x{}, \
             but the typed data signs 0x{}; refusing to display an action that is not the one signed",
            hex_lower(&computed),
            hex_lower(&signed_connection_id)
        ));
    }

    let action: L1Action = serde_json::from_str(raw_action)
        .map_err(|e| format!("unsupported {action_type} action shape: {e}"))?;
    action.validate()?;
    let reencoded =
        rmp_serde::to_vec_named(&action).map_err(|e| format!("cannot re-encode action: {e}"))?;
    if reencoded != packed {
        return Err(format!(
            "{action_type} action does not round-trip: the displayed fields would not be the \
             exact signed bytes (field order, omitted vs null, or integer type differs)"
        ));
    }

    Ok(L1Request {
        mainnet,
        action,
        action_type,
        nonce,
        vault_address,
        expires_after: envelope.expires_after,
        connection_id: computed,
    })
}

fn refuse_non_integers(value: &Value) -> Result<(), String> {
    match value {
        Value::Number(n) if !(n.is_u64() || n.is_i64()) => Err(format!(
            "action contains a non-integer number {n}; Hyperliquid actions carry decimals as strings"
        )),
        Value::Array(items) => items.iter().try_for_each(refuse_non_integers),
        Value::Object(map) => map.values().try_for_each(refuse_non_integers),
        _ => Ok(()),
    }
}

fn parse_user_signed(typed_data: &str) -> Result<UserSignedRequest, String> {
    let typed: TypedData<StrictMap<Value>> = serde_json::from_str(typed_data)
        .map_err(|e| format!("typedData is not a Hyperliquid user-signed payload: {e}"))?;
    let spec: &UserSignedSpec = user_signed::spec_for(&typed.primary_type).ok_or_else(|| {
        format!(
            "unsupported user-signed primaryType '{}'",
            typed.primary_type
        )
    })?;
    check_types(&typed.types.0, &typed.primary_type, spec.fields)?;
    let d = &typed.domain;
    if d.name != "HyperliquidSignTransaction"
        || d.version != "1"
        || !d.verifying_contract.eq_ignore_ascii_case(ZERO_ADDRESS)
    {
        return Err(
            "user-signed domain must be {name: HyperliquidSignTransaction, version: 1, verifyingContract: 0x0}"
                .to_string(),
        );
    }
    let fields = user_signed::check_message(spec, d.chain_id, &typed.message.0)?;
    match fields.iter().find(|(n, _)| *n == "hyperliquidChain") {
        Some((_, FieldValue::Str(chain))) if chain == "Mainnet" || chain == "Testnet" => {}
        Some((_, other)) => {
            return Err(format!(
                "hyperliquidChain must be Mainnet or Testnet, got {other:?}"
            ));
        }
        None => return Err("missing hyperliquidChain".to_string()),
    }
    if spec.name == "ApproveAgent"
        && let Some((_, FieldValue::Str(name))) = fields.iter().find(|(n, _)| *n == "agentName")
    {
        user_signed::agent_valid_until(name)?;
    }
    Ok(UserSignedRequest {
        spec_name: spec.name,
        chain_id: d.chain_id,
        fields,
    })
}

pub fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
