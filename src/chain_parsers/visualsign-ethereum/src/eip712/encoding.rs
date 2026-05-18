//! EIP-712 encoding primitives per <https://eips.ethereum.org/EIPS/eip-712>.

use crate::eip712::payload::{Domain, Eip712Payload, MessageValue, TypeMember};
use alloy_primitives::{B256, U256, keccak256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum EncodingError {
    #[error("unknown type '{0}' in schema")]
    UnknownType(String),
    #[error("value at '{path}' does not match declared type '{ty}'")]
    TypeMismatch { path: String, ty: String },
}

/// `keccak256(encodeType(name, types))`.
pub fn type_hash(
    name: &str,
    types: &BTreeMap<String, Vec<TypeMember>>,
) -> Result<B256, EncodingError> {
    Ok(keccak256(encode_type(name, types)?.as_bytes()))
}

/// `encodeType` per EIP-712: primary type first, then referenced structs alphabetically.
pub fn encode_type(
    name: &str,
    types: &BTreeMap<String, Vec<TypeMember>>,
) -> Result<String, EncodingError> {
    let mut deps = BTreeSet::new();
    collect_deps(name, types, &mut deps)?;
    deps.remove(name);

    let mut out = String::new();
    out.push_str(&format_struct(name, types)?);
    for dep in deps {
        out.push_str(&format_struct(&dep, types)?);
    }
    Ok(out)
}

fn collect_deps(
    name: &str,
    types: &BTreeMap<String, Vec<TypeMember>>,
    out: &mut BTreeSet<String>,
) -> Result<(), EncodingError> {
    if !out.insert(name.to_string()) {
        return Ok(());
    }
    let members = types
        .get(name)
        .ok_or_else(|| EncodingError::UnknownType(name.into()))?;
    for m in members {
        let base = m.r#type.trim_end_matches("[]");
        if types.contains_key(base) {
            collect_deps(base, types, out)?;
        }
    }
    Ok(())
}

fn format_struct(
    name: &str,
    types: &BTreeMap<String, Vec<TypeMember>>,
) -> Result<String, EncodingError> {
    let members = types
        .get(name)
        .ok_or_else(|| EncodingError::UnknownType(name.into()))?;
    let mut s = String::with_capacity(64);
    s.push_str(name);
    s.push('(');
    for (i, m) in members.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&m.r#type);
        s.push(' ');
        s.push_str(&m.name);
    }
    s.push(')');
    Ok(s)
}

/// `keccak256(typeHash || encodeData(value))`.
pub fn struct_hash(
    name: &str,
    value: &MessageValue,
    types: &BTreeMap<String, Vec<TypeMember>>,
) -> Result<B256, EncodingError> {
    let mut buf = Vec::with_capacity(32 * 8);
    buf.extend_from_slice(type_hash(name, types)?.as_slice());
    encode_data(name, value, types, &mut buf)?;
    Ok(keccak256(&buf))
}

fn encode_data(
    type_name: &str,
    value: &MessageValue,
    types: &BTreeMap<String, Vec<TypeMember>>,
    buf: &mut Vec<u8>,
) -> Result<(), EncodingError> {
    let members = types
        .get(type_name)
        .ok_or_else(|| EncodingError::UnknownType(type_name.into()))?;
    let MessageValue::Struct(fields) = value else {
        return Err(EncodingError::TypeMismatch {
            path: type_name.into(),
            ty: type_name.into(),
        });
    };
    for m in members {
        let v = fields
            .get(&m.name)
            .ok_or_else(|| EncodingError::TypeMismatch {
                path: format!("{type_name}.{}", m.name),
                ty: m.r#type.clone(),
            })?;
        encode_field(&m.r#type, v, types, buf)?;
    }
    Ok(())
}

fn encode_field(
    ty: &str,
    value: &MessageValue,
    types: &BTreeMap<String, Vec<TypeMember>>,
    buf: &mut Vec<u8>,
) -> Result<(), EncodingError> {
    // Arrays: keccak256(concat(encode(item) for item in arr))
    if let Some(inner) = ty.strip_suffix("[]") {
        let MessageValue::Array(items) = value else {
            return Err(EncodingError::TypeMismatch {
                path: ty.into(),
                ty: ty.into(),
            });
        };
        let mut inner_buf = Vec::with_capacity(32 * items.len());
        for item in items {
            encode_field(inner, item, types, &mut inner_buf)?;
        }
        buf.extend_from_slice(keccak256(&inner_buf).as_slice());
        return Ok(());
    }

    // Struct: structHash recursively
    if types.contains_key(ty) {
        let h = struct_hash(ty, value, types)?;
        buf.extend_from_slice(h.as_slice());
        return Ok(());
    }

    // Primitives
    match (ty, value) {
        ("address", MessageValue::Address(a)) => {
            buf.extend_from_slice(&[0u8; 12]);
            buf.extend_from_slice(a.as_slice());
        }
        ("bool", MessageValue::Bool(b)) => {
            buf.extend_from_slice(&[0u8; 31]);
            buf.push(if *b { 1 } else { 0 });
        }
        ("bytes", MessageValue::Bytes(b)) => {
            buf.extend_from_slice(keccak256(b).as_slice());
        }
        ("string", MessageValue::String(s)) => {
            buf.extend_from_slice(keccak256(s.as_bytes()).as_slice());
        }
        (t, MessageValue::BytesFixed(b)) if t.starts_with("bytes") => {
            let mut padded = [0u8; 32];
            padded[..b.len()].copy_from_slice(b);
            buf.extend_from_slice(&padded);
        }
        (t, MessageValue::Uint { value, .. }) if t.starts_with("uint") => {
            buf.extend_from_slice(&value.to_be_bytes::<32>());
        }
        (t, MessageValue::Int { value, .. }) if t.starts_with("int") => {
            buf.extend_from_slice(&value.to_be_bytes::<32>());
        }
        _ => {
            return Err(EncodingError::TypeMismatch {
                path: ty.into(),
                ty: ty.into(),
            });
        }
    }
    Ok(())
}

/// The standard EIP-712 domain separator.
pub fn domain_separator(domain: &Domain) -> Result<B256, EncodingError> {
    let mut members: Vec<TypeMember> = Vec::new();
    if domain.name.is_some() {
        members.push(TypeMember {
            name: "name".into(),
            r#type: "string".into(),
        });
    }
    if domain.version.is_some() {
        members.push(TypeMember {
            name: "version".into(),
            r#type: "string".into(),
        });
    }
    if domain.chain_id.is_some() {
        members.push(TypeMember {
            name: "chainId".into(),
            r#type: "uint256".into(),
        });
    }
    if domain.verifying_contract.is_some() {
        members.push(TypeMember {
            name: "verifyingContract".into(),
            r#type: "address".into(),
        });
    }
    if domain.salt.is_some() {
        members.push(TypeMember {
            name: "salt".into(),
            r#type: "bytes32".into(),
        });
    }

    let mut types = BTreeMap::new();
    types.insert("EIP712Domain".to_string(), members);

    let mut fields = BTreeMap::new();
    if let Some(n) = &domain.name {
        fields.insert("name".into(), MessageValue::String(n.clone()));
    }
    if let Some(v) = &domain.version {
        fields.insert("version".into(), MessageValue::String(v.clone()));
    }
    if let Some(c) = domain.chain_id {
        fields.insert(
            "chainId".into(),
            MessageValue::Uint {
                bits: 256,
                value: U256::from(c),
            },
        );
    }
    if let Some(vc) = domain.verifying_contract {
        fields.insert("verifyingContract".into(), MessageValue::Address(vc));
    }
    if let Some(s) = domain.salt {
        fields.insert("salt".into(), MessageValue::BytesFixed(s.to_vec()));
    }
    struct_hash("EIP712Domain", &MessageValue::Struct(fields), &types)
}

/// `signing_hash = keccak256(0x1901 || domainSeparator || structHash(primaryType, message))`.
pub fn signing_hash(payload: &Eip712Payload) -> Result<B256, EncodingError> {
    let domain_sep = domain_separator(&payload.domain)?;
    let msg_hash = struct_hash(&payload.primary_type, &payload.message, &payload.types)?;
    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.push(0x19);
    buf.push(0x01);
    buf.extend_from_slice(domain_sep.as_slice());
    buf.extend_from_slice(msg_hash.as_slice());
    Ok(keccak256(&buf))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // Canonical "Bob/Alice mail" example from EIP-712 spec.
    const SPEC_PAYLOAD: &str = r#"{
      "types": {
        "EIP712Domain": [
          {"name": "name", "type": "string"},
          {"name": "version", "type": "string"},
          {"name": "chainId", "type": "uint256"},
          {"name": "verifyingContract", "type": "address"}
        ],
        "Person": [
          {"name": "name", "type": "string"},
          {"name": "wallet", "type": "address"}
        ],
        "Mail": [
          {"name": "from", "type": "Person"},
          {"name": "to", "type": "Person"},
          {"name": "contents", "type": "string"}
        ]
      },
      "primaryType": "Mail",
      "domain": {
        "name": "Ether Mail",
        "version": "1",
        "chainId": "0x1",
        "verifyingContract": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"
      },
      "message": {
        "from": {"name": "Cow", "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},
        "to":   {"name": "Bob", "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},
        "contents": "Hello, Bob!"
      }
    }"#;

    #[test]
    fn type_hash_matches_spec() {
        let payload = Eip712Payload::from_json(SPEC_PAYLOAD).unwrap();
        let encoded = encode_type("Mail", &payload.types).unwrap();
        assert_eq!(
            encoded,
            "Mail(Person from,Person to,string contents)Person(string name,address wallet)"
        );
    }

    #[test]
    fn domain_separator_matches_spec() {
        let payload = Eip712Payload::from_json(SPEC_PAYLOAD).unwrap();
        let h = domain_separator(&payload.domain).unwrap();
        assert_eq!(
            hex::encode(h.as_slice()),
            "f2cee375fa42b42143804025fc449deafd50cc031ca257e0b194a650a912090f"
        );
    }

    #[test]
    fn signing_hash_matches_spec() {
        let payload = Eip712Payload::from_json(SPEC_PAYLOAD).unwrap();
        let h = signing_hash(&payload).unwrap();
        assert_eq!(
            hex::encode(h.as_slice()),
            "be609aee343fb3c4b28e1df9e632fca64fcfaede20f02e86244efddf30957bd2"
        );
    }
}
