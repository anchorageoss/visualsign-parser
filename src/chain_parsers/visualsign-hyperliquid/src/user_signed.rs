//! User-signed actions: plain EIP-712 with readable fields.
//!
//! Each allowlisted `primaryType` pins its EIP-712 field list exactly (names, types and
//! order, from hyperliquid-python-sdk 0.24.0 `utils/signing.py`). The message may also
//! carry the request's `type` and `signatureChainId`, which the SDK leaves in the dict
//! it signs; they are not part of the EIP-712 struct, so they must agree with the
//! signed data and are not displayed. Any other key is refused.

use std::collections::BTreeMap;

use serde_json::Value;

/// One allowlisted user-signed action.
pub struct UserSignedSpec {
    /// Short name used in labels and errors.
    pub name: &'static str,
    /// The exact EIP-712 `primaryType`.
    pub primary_type: &'static str,
    /// Value of the request's `type` key for this action.
    pub action_type: &'static str,
    /// The pinned EIP-712 fields, in order.
    pub fields: &'static [(&'static str, &'static str)],
}

pub const SPECS: &[UserSignedSpec] = &[
    UserSignedSpec {
        name: "ApproveAgent",
        primary_type: "HyperliquidTransaction:ApproveAgent",
        action_type: "approveAgent",
        fields: &[
            ("hyperliquidChain", "string"),
            ("agentAddress", "address"),
            ("agentName", "string"),
            ("nonce", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "TokenDelegate",
        primary_type: "HyperliquidTransaction:TokenDelegate",
        action_type: "tokenDelegate",
        fields: &[
            ("hyperliquidChain", "string"),
            ("validator", "address"),
            ("wei", "uint64"),
            ("isUndelegate", "bool"),
            ("nonce", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "Withdraw",
        primary_type: "HyperliquidTransaction:Withdraw",
        action_type: "withdraw3",
        fields: &[
            ("hyperliquidChain", "string"),
            ("destination", "string"),
            ("amount", "string"),
            ("time", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "UsdSend",
        primary_type: "HyperliquidTransaction:UsdSend",
        action_type: "usdSend",
        fields: &[
            ("hyperliquidChain", "string"),
            ("destination", "string"),
            ("amount", "string"),
            ("time", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "SpotSend",
        primary_type: "HyperliquidTransaction:SpotSend",
        action_type: "spotSend",
        fields: &[
            ("hyperliquidChain", "string"),
            ("destination", "string"),
            ("token", "string"),
            ("amount", "string"),
            ("time", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "SendAsset",
        primary_type: "HyperliquidTransaction:SendAsset",
        action_type: "sendAsset",
        fields: &[
            ("hyperliquidChain", "string"),
            ("destination", "string"),
            ("sourceDex", "string"),
            ("destinationDex", "string"),
            ("token", "string"),
            ("amount", "string"),
            ("fromSubAccount", "string"),
            ("nonce", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "ApproveBuilderFee",
        primary_type: "HyperliquidTransaction:ApproveBuilderFee",
        action_type: "approveBuilderFee",
        fields: &[
            ("hyperliquidChain", "string"),
            ("maxFeeRate", "string"),
            ("builder", "address"),
            ("nonce", "uint64"),
        ],
    },
    UserSignedSpec {
        name: "UsdClassTransfer",
        primary_type: "HyperliquidTransaction:UsdClassTransfer",
        action_type: "usdClassTransfer",
        fields: &[
            ("hyperliquidChain", "string"),
            ("amount", "string"),
            ("toPerp", "bool"),
            ("nonce", "uint64"),
        ],
    },
    // Signed once when the Hyperliquid app first connects a wallet. It moves no funds.
    // Unlike the others, its primaryType has no `HyperliquidTransaction:` prefix.
    UserSignedSpec {
        name: "AcceptTerms",
        primary_type: "Hyperliquid:AcceptTerms",
        action_type: "acceptTerms",
        fields: &[("hyperliquidChain", "string"), ("time", "uint64")],
    },
];

/// A typed message value, checked against its EIP-712 type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Str(String),
    Address(String),
    U64(u64),
    Bool(bool),
}

pub fn spec_for(primary_type: &str) -> Option<&'static UserSignedSpec> {
    SPECS.iter().find(|s| s.primary_type == primary_type)
}

fn is_address(s: &str) -> bool {
    s.len() == 42
        && (s.starts_with("0x") || s.starts_with("0X"))
        && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Check the message against the pinned field list and return the field values in order.
pub fn check_message(
    spec: &UserSignedSpec,
    chain_id: u64,
    message: &BTreeMap<String, Value>,
) -> Result<Vec<(&'static str, FieldValue)>, String> {
    for key in message.keys() {
        let known = spec.fields.iter().any(|(name, _)| name == key)
            || key == "type"
            || key == "signatureChainId";
        if !known {
            return Err(format!("unexpected field '{key}' in {} message", spec.name));
        }
    }
    if let Some(t) = message.get("type")
        && t.as_str() != Some(spec.action_type)
    {
        return Err(format!(
            "message type {t} does not match primaryType {} (expected \"{}\")",
            spec.name, spec.action_type
        ));
    }
    if let Some(sig_chain) = message.get("signatureChainId") {
        let parsed = sig_chain
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .and_then(|h| u64::from_str_radix(h, 16).ok());
        if parsed != Some(chain_id) {
            return Err(format!(
                "signatureChainId {sig_chain} does not match domain chainId {chain_id}"
            ));
        }
    }

    let mut out = Vec::with_capacity(spec.fields.len());
    for (name, ty) in spec.fields {
        let value = message
            .get(*name)
            .ok_or_else(|| format!("missing field '{name}' in {} message", spec.name))?;
        let typed = match *ty {
            "string" => value.as_str().map(|s| FieldValue::Str(s.to_string())),
            "address" => value
                .as_str()
                .filter(|s| is_address(s))
                .map(|s| FieldValue::Address(s.to_string())),
            "uint64" => value.as_u64().map(FieldValue::U64),
            "bool" => value.as_bool().map(FieldValue::Bool),
            _ => None,
        }
        .ok_or_else(|| format!("field '{name}' is not a valid {ty}: {value}"))?;
        out.push((*name, typed));
    }
    Ok(out)
}

/// Parse `valid_until <ms>` out of an agent name, as the Hyperliquid UI writes it.
///
/// Returns `Ok(None)` when the marker is absent, and an error when it is present but
/// not followed by a millisecond timestamp, so a garbled expiry is never shown as
/// "no expiry".
pub fn agent_valid_until(agent_name: &str) -> Result<Option<u64>, String> {
    const MARKER: &str = "valid_until ";
    let Some(idx) = agent_name.find(MARKER) else {
        return Ok(None);
    };
    let rest = &agent_name[idx + MARKER.len()..];
    let digits: &str = rest
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("");
    // The marker must end the name: anything after the digits (including a second
    // marker) makes the expiry ambiguous.
    if digits.is_empty() || digits.len() != rest.len() {
        return Err(format!(
            "agentName has a malformed valid_until: '{agent_name}'"
        ));
    }
    digits
        .parse::<u64>()
        .map(Some)
        .map_err(|e| format!("agentName valid_until is not a u64: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn valid_until_is_parsed() {
        assert_eq!(
            agent_valid_until("bot valid_until 1767225600000").unwrap(),
            Some(1_767_225_600_000)
        );
        assert_eq!(agent_valid_until("bot").unwrap(), None);
        assert_eq!(agent_valid_until("").unwrap(), None);
    }

    #[test]
    fn malformed_valid_until_is_refused() {
        assert!(agent_valid_until("bot valid_until ").is_err());
        assert!(agent_valid_until("bot valid_until 12x").is_err());
        assert!(agent_valid_until("valid_until 1 valid_until 2").is_err());
    }
}
