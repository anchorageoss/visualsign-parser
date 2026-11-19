//! Hyperliquid intermediate output: a Borsh-serialized, structured copy of the
//! verified action, attached to the conversion result alongside the
//! human-readable `SignablePayload` so a policy engine can check the action
//! itself and not its display text.
//!
//! [`HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION`] is the first field so a shape
//! change is a single, reviewable signal that forces mirrored decoders to update.
//!
//! Encoding rules:
//! - One envelope is one action, so there is one output and no command list.
//! - `fields` holds the action's scalar fields in wire order. A nested field is
//!   named by its dotted path (`assetRequest.coin`).
//! - `entries` holds one group of fields per element of the action's list, in
//!   wire order, and is empty for an action without one. Only
//!   `perpDeploy.setSubDeployers` has one today (its `subDeployers`).
//! - Every value is the signed value, never a display string: strings and
//!   addresses as signed (case included), integers in base 10, booleans as
//!   `true` or `false`. A JSON `null` has `data_type` `null` and an empty value.
//! - `data_type` is `string`, `address`, `uint64`, `bool` or `null`. L1 actions
//!   are msgpack and carry no address type, so their addresses are `string`.
//! - An action this module does not describe yields an error, never an empty
//!   output: a consumer that requires intermediate output must not be able to
//!   mistake "not described" for "nothing to check".

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use borsh::{BorshDeserialize, BorshSerialize};

use crate::input::{L1Request, UserSignedRequest};
use crate::l1::{HaltTrading, L1Action, PerpSetSubDeployers, RegisterAsset2};
use crate::tx::HyperliquidTransaction;
use crate::user_signed::FieldValue;

/// Version of the `HyperliquidIntermediateOutput` Borsh schema. Bump on ANY
/// change to the shape below or to the encoding rules in the module docs.
pub const HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION: u16 = 1;

/// `kind` of an L1 action.
pub const KIND_L1: &str = "l1";
/// `kind` of a user-signed action.
pub const KIND_USER_SIGNED: &str = "userSigned";

/// Top-level Hyperliquid intermediate output.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidIntermediateOutput {
    /// Always [`HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION`]. First field so
    /// decoders can gate on it before reading the rest.
    pub schema_version: u16,
    /// [`KIND_L1`] or [`KIND_USER_SIGNED`].
    pub kind: String,
    /// L1: `Mainnet` or `Testnet`, from the signed `Agent.source`.
    /// User-signed: the signed `hyperliquidChain`.
    pub network: String,
    /// L1: the action's `type`. User-signed: the EIP-712 primary type without
    /// its `HyperliquidTransaction:` prefix.
    pub action_type: String,
    /// The operation inside a sub-operation family (`haltTrading` inside
    /// `perpDeploy`); empty for an action that has none.
    pub variant: String,
    /// L1 only: the envelope nonce bound into `connectionId`. Zero for a
    /// user-signed action, whose nonce is one of its `fields`.
    pub nonce: u64,
    /// L1 only: `0x`-prefixed lowercase hex, or empty when the action is not
    /// sent for a vault.
    pub vault_address: String,
    /// L1 only: base-10 milliseconds, or empty when the action does not expire.
    pub expires_after: String,
    pub fields: Vec<HyperliquidIntermediateField>,
    pub entries: Vec<Vec<HyperliquidIntermediateField>>,
}

/// One scalar field of the action.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidIntermediateField {
    pub name: String,
    pub data_type: String,
    pub value: String,
}

fn field(name: &str, data_type: &str, value: impl Into<String>) -> HyperliquidIntermediateField {
    HyperliquidIntermediateField {
        name: name.to_string(),
        data_type: data_type.to_string(),
        value: value.into(),
    }
}

fn string(name: &str, value: &str) -> HyperliquidIntermediateField {
    field(name, "string", value)
}

fn uint(name: &str, value: u64) -> HyperliquidIntermediateField {
    field(name, "uint64", value.to_string())
}

fn boolean(name: &str, value: bool) -> HyperliquidIntermediateField {
    field(name, "bool", if value { "true" } else { "false" })
}

fn null(name: &str) -> HyperliquidIntermediateField {
    field(name, "null", "")
}

/// Build the intermediate output for a verified request.
///
/// # Errors
/// Returns a message naming the action when this module does not describe it.
pub fn build_intermediate_output(
    transaction: &HyperliquidTransaction,
) -> Result<HyperliquidIntermediateOutput, String> {
    match transaction {
        HyperliquidTransaction::L1(req) => l1_output(req),
        HyperliquidTransaction::UserSigned(req) => Ok(user_signed_output(req)),
    }
}

fn l1_output(req: &L1Request) -> Result<HyperliquidIntermediateOutput, String> {
    let L1Action::PerpDeploy(deploy) = &req.action else {
        return Err(unsupported(&req.action_type));
    };
    let (variant, fields, entries) = if let Some(a) = &deploy.register_asset2 {
        ("registerAsset2", register_asset2_fields(a), Vec::new())
    } else if let Some(a) = &deploy.halt_trading {
        ("haltTrading", halt_trading_fields(a), Vec::new())
    } else if let Some(a) = &deploy.set_sub_deployers {
        (
            "setSubDeployers",
            vec![string("dex", &a.dex)],
            sub_deployer_entries(a),
        )
    } else {
        return Err(unsupported("this perpDeploy operation"));
    };
    Ok(HyperliquidIntermediateOutput {
        schema_version: HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION,
        kind: KIND_L1.to_string(),
        network: if req.mainnet { "Mainnet" } else { "Testnet" }.to_string(),
        action_type: req.action_type.clone(),
        variant: variant.to_string(),
        nonce: req.nonce,
        vault_address: req.vault_address.map(|v| hex(&v)).unwrap_or_default(),
        expires_after: req.expires_after.map(|v| v.to_string()).unwrap_or_default(),
        fields,
        entries,
    })
}

fn unsupported(what: &str) -> String {
    format!("intermediate output is not available for {what}")
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn register_asset2_fields(a: &RegisterAsset2) -> Vec<HyperliquidIntermediateField> {
    let mut fields = vec![
        a.max_gas
            .map_or_else(|| null("maxGas"), |v| uint("maxGas", v)),
        string("assetRequest.coin", &a.asset_request.coin),
        uint("assetRequest.szDecimals", a.asset_request.sz_decimals),
        string("assetRequest.oraclePx", &a.asset_request.oracle_px),
        uint(
            "assetRequest.marginTableId",
            a.asset_request.margin_table_id,
        ),
        string("assetRequest.marginMode", &a.asset_request.margin_mode),
        string("dex", &a.dex),
    ];
    match &a.schema {
        None => fields.push(null("schema")),
        Some(schema) => {
            fields.push(string("schema.fullName", &schema.full_name));
            fields.push(uint("schema.collateralToken", schema.collateral_token));
            fields.push(schema.oracle_updater.as_deref().map_or_else(
                || null("schema.oracleUpdater"),
                |v| string("schema.oracleUpdater", v),
            ));
        }
    }
    fields
}

fn halt_trading_fields(a: &HaltTrading) -> Vec<HyperliquidIntermediateField> {
    vec![string("coin", &a.coin), boolean("isHalted", a.is_halted)]
}

fn sub_deployer_entries(a: &PerpSetSubDeployers) -> Vec<Vec<HyperliquidIntermediateField>> {
    a.sub_deployers
        .iter()
        .map(|entry| {
            vec![
                string("variant", &entry.variant),
                string("user", &entry.user),
                boolean("allowed", entry.allowed),
            ]
        })
        .collect()
}

fn user_signed_output(req: &UserSignedRequest) -> HyperliquidIntermediateOutput {
    let mut network = String::new();
    let fields = req
        .fields
        .iter()
        .map(|(name, value)| {
            if *name == "hyperliquidChain"
                && let FieldValue::Str(chain) = value
            {
                network.clone_from(chain);
            }
            match value {
                FieldValue::Str(v) => string(name, v),
                FieldValue::Address(v) => field(name, "address", v.as_str()),
                FieldValue::U64(v) => uint(name, *v),
                FieldValue::Bool(v) => boolean(name, *v),
            }
        })
        .collect();
    HyperliquidIntermediateOutput {
        schema_version: HYPERLIQUID_INTERMEDIATE_SCHEMA_VERSION,
        kind: KIND_USER_SIGNED.to_string(),
        network,
        action_type: req.spec_name.to_string(),
        variant: String::new(),
        nonce: 0,
        vault_address: String::new(),
        expires_after: String::new(),
        fields,
        entries: Vec::new(),
    }
}
