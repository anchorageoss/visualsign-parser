//! ERC-7730 path resolver.
//!
//! Paths look like `#.spender`, `@.to`, `#.permitted.[].amount`, `$.chainId`.
//! - `#` roots at the message
//! - `@` is the verifyingContract from the domain
//! - `$` is the domain itself

use crate::eip712::payload::{Domain, MessageValue};
use alloy_primitives::U256;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("invalid path syntax: {0}")]
    InvalidSyntax(String),
    #[error("missing field at path: {0}")]
    MissingField(String),
    #[error("value at '{0}' is not indexable")]
    NotIndexable(String),
}

#[derive(Debug, Clone, Copy)]
pub enum PathRoot {
    Message,           // #
    VerifyingContract, // @
    Domain,            // $
}

#[derive(Debug, Clone)]
pub struct ParsedPath {
    pub root: PathRoot,
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, Clone)]
pub enum PathSegment {
    Field(String),
    ArrayIter,         // [] — applies to each element
    ArrayIndex(usize), // [n] — specific index
}

pub fn parse(path: &str) -> Result<ParsedPath, PathError> {
    if path.is_empty() {
        return Err(PathError::InvalidSyntax(
            "path must start with #, @ or $".into(),
        ));
    }
    // Some ERC-7730 descriptors use bare paths (no leading root) — treat as `#.<rest>`.
    let (root, rest) = match path.chars().next() {
        Some('#') => (PathRoot::Message, &path[1..]),
        Some('@') => (PathRoot::VerifyingContract, &path[1..]),
        Some('$') => (PathRoot::Domain, &path[1..]),
        _ => (PathRoot::Message, path),
    };
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    if rest.is_empty() {
        return Ok(ParsedPath {
            root,
            segments: vec![],
        });
    }
    let mut segments = Vec::new();
    for raw in rest.split('.') {
        if raw == "[]" {
            segments.push(PathSegment::ArrayIter);
        } else if let Some(inner) = raw.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let idx: usize = inner
                .parse()
                .map_err(|_| PathError::InvalidSyntax(format!("bad index: {raw}")))?;
            segments.push(PathSegment::ArrayIndex(idx));
        } else if raw.is_empty() {
            return Err(PathError::InvalidSyntax(format!(
                "empty segment in path: {path}"
            )));
        } else {
            segments.push(PathSegment::Field(raw.to_string()));
        }
    }
    Ok(ParsedPath { root, segments })
}

/// Resolve a path against a message. Returns one value, or multiple if the path includes `[]` iterators.
pub fn resolve(
    path: &ParsedPath,
    message: &MessageValue,
    domain: &Domain,
) -> Result<Vec<MessageValue>, PathError> {
    let starts: Vec<MessageValue> = match path.root {
        PathRoot::Message => vec![message.clone()],
        PathRoot::Domain => {
            let mut fields = std::collections::BTreeMap::new();
            if let Some(c) = domain.chain_id {
                fields.insert(
                    "chainId".into(),
                    MessageValue::Uint {
                        bits: 256,
                        value: U256::from(c),
                    },
                );
            }
            if let Some(a) = domain.verifying_contract {
                fields.insert("verifyingContract".into(), MessageValue::Address(a));
            }
            if let Some(n) = &domain.name {
                fields.insert("name".into(), MessageValue::String(n.clone()));
            }
            if let Some(v) = &domain.version {
                fields.insert("version".into(), MessageValue::String(v.clone()));
            }
            vec![MessageValue::Struct(fields)]
        }
        PathRoot::VerifyingContract => {
            let a = domain.verifying_contract.ok_or_else(|| {
                PathError::MissingField("@: domain.verifyingContract missing".into())
            })?;
            // `@` is the verifyingContract address. Some descriptors index it with `.to`
            // (ERC-7730 idiom: "the contract being signed against acts as the to/token
            // address"). Synthesize a struct view exposing those aliases so paths like
            // `@.to`, `@.token`, `@.address` all resolve to the contract address.
            if path.segments.is_empty() {
                vec![MessageValue::Address(a)]
            } else {
                let mut fields = std::collections::BTreeMap::new();
                let addr = MessageValue::Address(a);
                fields.insert("to".into(), addr.clone());
                fields.insert("token".into(), addr.clone());
                fields.insert("address".into(), addr);
                vec![MessageValue::Struct(fields)]
            }
        }
    };

    let mut current = starts;
    for (i, seg) in path.segments.iter().enumerate() {
        let mut next = Vec::with_capacity(current.len());
        for val in current {
            match (seg, &val) {
                (PathSegment::Field(name), MessageValue::Struct(fields)) => {
                    let v = fields
                        .get(name)
                        .ok_or_else(|| PathError::MissingField(format!("segment {i}: '{name}'")))?;
                    next.push(v.clone());
                }
                (PathSegment::ArrayIter, MessageValue::Array(items)) => {
                    next.extend(items.iter().cloned());
                }
                (PathSegment::ArrayIndex(idx), MessageValue::Array(items)) => {
                    let v = items.get(*idx).ok_or_else(|| {
                        PathError::MissingField(format!("index {idx} out of bounds"))
                    })?;
                    next.push(v.clone());
                }
                _ => {
                    return Err(PathError::NotIndexable(format!(
                        "segment {i}: type does not support this operation"
                    )));
                }
            }
        }
        current = next;
    }
    Ok(current)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::eip712::payload::Domain;
    use alloy_primitives::Address;
    use std::collections::BTreeMap;

    fn permit_message() -> MessageValue {
        let mut fields = BTreeMap::new();
        fields.insert(
            "spender".into(),
            MessageValue::Address(Address::with_last_byte(0x22)),
        );
        fields.insert(
            "value".into(),
            MessageValue::Uint {
                bits: 256,
                value: U256::from(1_000_000u64),
            },
        );
        MessageValue::Struct(fields)
    }

    fn domain() -> Domain {
        Domain {
            name: Some("USD Coin".into()),
            version: Some("2".into()),
            chain_id: Some(1),
            verifying_contract: Some(Address::with_last_byte(0xa0)),
            salt: None,
        }
    }

    #[test]
    fn resolves_message_field() {
        let p = parse("#.spender").unwrap();
        let r = resolve(&p, &permit_message(), &domain()).unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(r[0], MessageValue::Address(_)));
    }

    #[test]
    fn resolves_verifying_contract() {
        let p = parse("@").unwrap();
        let r = resolve(&p, &permit_message(), &domain()).unwrap();
        assert!(matches!(r[0], MessageValue::Address(_)));
    }

    #[test]
    fn resolves_domain_field() {
        let p = parse("$.chainId").unwrap();
        let r = resolve(&p, &permit_message(), &domain()).unwrap();
        match &r[0] {
            MessageValue::Uint { value, .. } => assert_eq!(*value, U256::from(1u64)),
            _ => panic!(),
        }
    }

    #[test]
    fn iterates_array() {
        let arr = MessageValue::Array(vec![
            MessageValue::Uint {
                bits: 256,
                value: U256::from(1u64),
            },
            MessageValue::Uint {
                bits: 256,
                value: U256::from(2u64),
            },
        ]);
        let mut fields = BTreeMap::new();
        fields.insert("items".into(), arr);
        let msg = MessageValue::Struct(fields);
        let p = parse("#.items.[]").unwrap();
        let r = resolve(&p, &msg, &domain()).unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn bare_path_treated_as_message_relative() {
        // Some ERC-7730 descriptors use bare paths (no `#` prefix) — they refer to
        // the message root by convention.
        let p = parse("spender").unwrap();
        assert!(matches!(p.root, PathRoot::Message));
        assert_eq!(p.segments.len(), 1);
    }

    #[test]
    fn empty_path_rejected() {
        assert!(parse("").is_err());
    }
}
