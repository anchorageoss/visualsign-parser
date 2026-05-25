//! EIP-712 payload model and `eth_signTypedData_v4` JSON parsing.

use crate::EthereumParserError;
use alloy_primitives::{Address, I256, U256};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Maximum accepted typed-data JSON size. Matches the existing transaction JSON cap.
pub const MAX_PAYLOAD_LEN: usize = 1024 * 1024;

/// EIP-712 domain separator inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    pub name: Option<String>,
    pub version: Option<String>,
    pub chain_id: Option<u64>,
    pub verifying_contract: Option<Address>,
    pub salt: Option<[u8; 32]>,
}

/// Parsed EIP-712 typed-data signing request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eip712Payload {
    pub domain: Domain,
    pub types: BTreeMap<String, Vec<TypeMember>>,
    pub primary_type: String,
    pub message: MessageValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeMember {
    pub name: String,
    pub r#type: String,
}

/// Resolved message value tree (after type-aware coercion from JSON).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageValue {
    Address(Address),
    Bool(bool),
    Bytes(Vec<u8>),
    BytesFixed(Vec<u8>), // bytesN
    Int { bits: u16, value: I256 },
    Uint { bits: u16, value: U256 },
    String(String),
    Array(Vec<MessageValue>),
    Struct(BTreeMap<String, MessageValue>),
}

#[derive(Debug, thiserror::Error)]
pub enum Eip712ParseError {
    #[error("failed to parse JSON: {0}")]
    Json(String),
    #[error("missing required field '{0}'")]
    Missing(String),
    #[error("invalid value for field '{field}': {detail}")]
    Invalid { field: String, detail: String },
    #[error("unknown type '{0}' referenced in message")]
    UnknownType(String),
    #[error("payload exceeds maximum size of {max} bytes ({actual} given)")]
    TooLarge { actual: usize, max: usize },
}

impl From<Eip712ParseError> for EthereumParserError {
    fn from(e: Eip712ParseError) -> Self {
        EthereumParserError::FailedToParseJsonTransaction(e.to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawPayload {
    /// Optional envelope discriminator. Accepted (but not required) so payloads can be
    /// routed through `EthJsonInput`-style envelopes (`{"type": "typedData", ...}`).
    #[serde(default, rename = "type")]
    _envelope_type: Option<String>,
    domain: RawDomain,
    types: BTreeMap<String, Vec<RawTypeMember>>,
    primary_type: String,
    message: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawDomain {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    chain_id: Option<serde_json::Value>,
    #[serde(default)]
    verifying_contract: Option<String>,
    #[serde(default)]
    salt: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTypeMember {
    name: String,
    r#type: String,
}

impl Eip712Payload {
    pub fn from_json(s: &str) -> Result<Self, Eip712ParseError> {
        if s.len() > MAX_PAYLOAD_LEN {
            return Err(Eip712ParseError::TooLarge {
                actual: s.len(),
                max: MAX_PAYLOAD_LEN,
            });
        }
        let raw: RawPayload =
            serde_json::from_str(s).map_err(|e| Eip712ParseError::Json(e.to_string()))?;

        let chain_id_value = raw
            .domain
            .chain_id
            .as_ref()
            .ok_or_else(|| Eip712ParseError::Missing("domain.chainId".into()))?;
        let chain_id_mv =
            parse_uint_value(chain_id_value, 256).map_err(|e| Eip712ParseError::Invalid {
                field: "domain.chainId".into(),
                detail: e,
            })?;
        let chain_id = match chain_id_mv {
            MessageValue::Uint { value, .. } => {
                u64::try_from(value).map_err(|_| Eip712ParseError::Invalid {
                    field: "domain.chainId".into(),
                    detail: "exceeds u64".into(),
                })?
            }
            _ => {
                return Err(Eip712ParseError::Invalid {
                    field: "domain.chainId".into(),
                    detail: "internal: parse_uint_value returned non-Uint".into(),
                });
            }
        };

        let verifying_contract = raw
            .domain
            .verifying_contract
            .as_deref()
            .map(|s| {
                s.parse::<Address>().map_err(|e| Eip712ParseError::Invalid {
                    field: "domain.verifyingContract".into(),
                    detail: e.to_string(),
                })
            })
            .transpose()?;

        let salt = raw
            .domain
            .salt
            .as_deref()
            .map(parse_bytes32_hex)
            .transpose()
            .map_err(|e| Eip712ParseError::Invalid {
                field: "domain.salt".into(),
                detail: e,
            })?;

        let domain = Domain {
            name: raw.domain.name,
            version: raw.domain.version,
            chain_id: Some(chain_id),
            verifying_contract,
            salt,
        };

        let types: BTreeMap<String, Vec<TypeMember>> = raw
            .types
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v.into_iter()
                        .map(|m| TypeMember {
                            name: m.name,
                            r#type: m.r#type,
                        })
                        .collect(),
                )
            })
            .collect();

        if !types.contains_key(&raw.primary_type) {
            return Err(Eip712ParseError::UnknownType(raw.primary_type));
        }

        let message = coerce_value(&raw.message, &raw.primary_type, &types)?;

        Ok(Eip712Payload {
            domain,
            types,
            primary_type: raw.primary_type,
            message,
        })
    }
}

fn coerce_value(
    json: &serde_json::Value,
    type_name: &str,
    types: &BTreeMap<String, Vec<TypeMember>>,
) -> Result<MessageValue, Eip712ParseError> {
    if let Some(inner) = type_name.strip_suffix("[]") {
        let arr = json.as_array().ok_or_else(|| Eip712ParseError::Invalid {
            field: type_name.into(),
            detail: "expected JSON array".into(),
        })?;
        let items = arr
            .iter()
            .map(|v| coerce_value(v, inner, types))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(MessageValue::Array(items));
    }

    if let Some(members) = types.get(type_name) {
        let obj = json.as_object().ok_or_else(|| Eip712ParseError::Invalid {
            field: type_name.into(),
            detail: "expected JSON object".into(),
        })?;
        let mut fields = BTreeMap::new();
        for m in members {
            let v = obj.get(&m.name).ok_or_else(|| Eip712ParseError::Invalid {
                field: format!("{type_name}.{}", m.name),
                detail: "missing".into(),
            })?;
            fields.insert(m.name.clone(), coerce_value(v, &m.r#type, types)?);
        }
        return Ok(MessageValue::Struct(fields));
    }

    match type_name {
        "address" => {
            let s = json.as_str().ok_or_else(|| Eip712ParseError::Invalid {
                field: type_name.into(),
                detail: "expected string".into(),
            })?;
            Ok(MessageValue::Address(s.parse().map_err(
                |e: <Address as std::str::FromStr>::Err| Eip712ParseError::Invalid {
                    field: type_name.into(),
                    detail: e.to_string(),
                },
            )?))
        }
        "bool" => Ok(MessageValue::Bool(json.as_bool().ok_or_else(|| {
            Eip712ParseError::Invalid {
                field: type_name.into(),
                detail: "expected bool".into(),
            }
        })?)),
        "string" => Ok(MessageValue::String(
            json.as_str()
                .ok_or_else(|| Eip712ParseError::Invalid {
                    field: type_name.into(),
                    detail: "expected string".into(),
                })?
                .to_string(),
        )),
        "bytes" => {
            let s = json.as_str().ok_or_else(|| Eip712ParseError::Invalid {
                field: type_name.into(),
                detail: "expected hex string".into(),
            })?;
            Ok(MessageValue::Bytes(parse_hex_bytes(s).map_err(|e| {
                Eip712ParseError::Invalid {
                    field: type_name.into(),
                    detail: e,
                }
            })?))
        }
        t if t.starts_with("bytes") => {
            let n: usize = t[5..].parse().map_err(|_| Eip712ParseError::Invalid {
                field: t.into(),
                detail: "invalid bytesN width".into(),
            })?;
            if n == 0 || n > 32 {
                return Err(Eip712ParseError::Invalid {
                    field: t.into(),
                    detail: format!("bytes{n} out of range"),
                });
            }
            let s = json.as_str().ok_or_else(|| Eip712ParseError::Invalid {
                field: t.into(),
                detail: "expected hex string".into(),
            })?;
            let bytes = parse_hex_bytes(s).map_err(|e| Eip712ParseError::Invalid {
                field: t.into(),
                detail: e,
            })?;
            if bytes.len() != n {
                return Err(Eip712ParseError::Invalid {
                    field: t.into(),
                    detail: format!("expected {n} bytes, got {}", bytes.len()),
                });
            }
            Ok(MessageValue::BytesFixed(bytes))
        }
        t if t.starts_with("uint") => {
            let bits: u16 = if t == "uint" {
                256
            } else {
                t[4..].parse().map_err(|_| Eip712ParseError::Invalid {
                    field: t.into(),
                    detail: "invalid uint width".into(),
                })?
            };
            parse_uint_value(json, bits).map_err(|e| Eip712ParseError::Invalid {
                field: t.into(),
                detail: e,
            })
        }
        t if t.starts_with("int") => {
            let bits: u16 = if t == "int" {
                256
            } else {
                t[3..].parse().map_err(|_| Eip712ParseError::Invalid {
                    field: t.into(),
                    detail: "invalid int width".into(),
                })?
            };
            parse_int_value(json, bits).map_err(|e| Eip712ParseError::Invalid {
                field: t.into(),
                detail: e,
            })
        }
        other => Err(Eip712ParseError::UnknownType(other.into())),
    }
}

fn parse_uint_value(json: &serde_json::Value, bits: u16) -> Result<MessageValue, String> {
    let value = match json {
        serde_json::Value::String(s) => {
            if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                U256::from_str_radix(stripped, 16).map_err(|e| e.to_string())?
            } else {
                U256::from_str_radix(s, 10).map_err(|e| e.to_string())?
            }
        }
        serde_json::Value::Number(n) => {
            let u = n
                .as_u64()
                .ok_or_else(|| "uint must be non-negative integer".to_string())?;
            U256::from(u)
        }
        _ => return Err("expected string or number".into()),
    };
    Ok(MessageValue::Uint { bits, value })
}

fn parse_int_value(json: &serde_json::Value, bits: u16) -> Result<MessageValue, String> {
    let value = match json {
        serde_json::Value::String(s) => {
            if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                let u = U256::from_str_radix(stripped, 16).map_err(|e| e.to_string())?;
                I256::try_from(u).map_err(|e| e.to_string())?
            } else {
                <I256 as std::str::FromStr>::from_str(s).map_err(|e| e.to_string())?
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                I256::try_from(i).map_err(|e| e.to_string())?
            } else {
                return Err("int must fit in i64 when provided as JSON number".into());
            }
        }
        _ => return Err("expected string or number".into()),
    };
    Ok(MessageValue::Int { bits, value })
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    hex::decode(stripped).map_err(|e| e.to_string())
}

fn parse_bytes32_hex(s: &str) -> Result<[u8; 32], String> {
    let v = parse_hex_bytes(s)?;
    if v.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", v.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const ERC2612_PERMIT_JSON: &str = r#"{
      "domain": {
        "name": "USD Coin",
        "version": "2",
        "chainId": "0x1",
        "verifyingContract": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
      },
      "primaryType": "Permit",
      "types": {
        "EIP712Domain": [
          {"name": "name", "type": "string"},
          {"name": "version", "type": "string"},
          {"name": "chainId", "type": "uint256"},
          {"name": "verifyingContract", "type": "address"}
        ],
        "Permit": [
          {"name": "owner", "type": "address"},
          {"name": "spender", "type": "address"},
          {"name": "value", "type": "uint256"},
          {"name": "nonce", "type": "uint256"},
          {"name": "deadline", "type": "uint256"}
        ]
      },
      "message": {
        "owner": "0x1111111111111111111111111111111111111111",
        "spender": "0x2222222222222222222222222222222222222222",
        "value": "1000000",
        "nonce": "0",
        "deadline": "1900000000"
      }
    }"#;

    #[test]
    fn parses_erc2612_permit() {
        let payload = Eip712Payload::from_json(ERC2612_PERMIT_JSON).unwrap();
        assert_eq!(payload.domain.name.as_deref(), Some("USD Coin"));
        assert_eq!(payload.domain.chain_id, Some(1));
        assert_eq!(payload.primary_type, "Permit");
        assert_eq!(payload.types.get("Permit").unwrap().len(), 5);
        let MessageValue::Struct(ref fields) = payload.message else {
            panic!("expected struct");
        };
        match fields.get("value").unwrap() {
            MessageValue::Uint { bits: 256, value } => {
                assert_eq!(*value, U256::from(1_000_000u64));
            }
            other => panic!("expected uint256, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_chain_id() {
        let mut json: serde_json::Value = serde_json::from_str(ERC2612_PERMIT_JSON).unwrap();
        json["domain"].as_object_mut().unwrap().remove("chainId");
        let err = Eip712Payload::from_json(&json.to_string()).unwrap_err();
        assert!(err.to_string().contains("chainId"));
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let mut json: serde_json::Value = serde_json::from_str(ERC2612_PERMIT_JSON).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!("oops"));
        let err = Eip712Payload::from_json(&json.to_string()).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn accepts_uint_as_decimal_or_hex() {
        let json_dec = ERC2612_PERMIT_JSON.replace("\"value\": \"1000000\"", "\"value\": 1000000");
        let payload = Eip712Payload::from_json(&json_dec).unwrap();
        let MessageValue::Struct(ref fields) = payload.message else {
            unreachable!()
        };
        match fields.get("value").unwrap() {
            MessageValue::Uint { value, .. } => assert_eq!(*value, U256::from(1_000_000u64)),
            _ => panic!(),
        }
    }
}
