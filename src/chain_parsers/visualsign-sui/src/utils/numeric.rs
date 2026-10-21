//! Helpers for reading Sui `Pure` inputs: `decode_number` decodes primitive
//! numeric types and bool, and `pure_bcs_bytes` returns an untyped input's raw
//! BCS bytes for callers that decode other shapes themselves.
//!
//! Constraints and behavior:
//! - Only `Pure` call args are supported. `Object` and `FundsWithdrawal`
//!   arguments return `DecodeError` from `decode_number` and `MissingData` from
//!   `pure_bcs_bytes`, which also rejects typed values because their JSON form
//!   differs from the BCS encoding.
//! - Supported types implement `FromLeBytes` (`bool`, `u8`, `u16`, `u32`, `u64`, `u128`).
//! - JSON arrays of bytes are converted to little-endian values; type-tagged values are read
//!   from their JSON form (a decimal or `0x` hex string, or a JSON number or bool).
//! - `u256` is not supported; consider splitting into two `u128` or adding explicit support.
//! - Errors include precise messages to aid debugging when fixtures or indices go out of sync.

use crate::types::{SuiCallArg, SuiPureValue, TypeTag};
use std::convert::{TryFrom, TryInto};
use visualsign::errors::VisualSignError;

// TODO: Consider `u256` support and fallback options
pub fn decode_number<T>(call_arg: &SuiCallArg) -> Result<T, VisualSignError>
where
    T: FromLeBytes,
{
    match call_arg {
        SuiCallArg::Object(_) => Err(VisualSignError::DecodeError(
            "Unexpected object in `decode_number`".to_string(),
        )),
        SuiCallArg::FundsWithdrawal(_) => Err(VisualSignError::DecodeError(
            "Unexpected funds withdrawal in `decode_number`".to_string(),
        )),
        SuiCallArg::Pure(value) => match value.value_type() {
            None => {
                let bytes = json_array_to_bytes(&value.value().to_json_value()).map_err(|e| {
                    VisualSignError::DecodeError(format!("Invalid pure value bytes: {e}"))
                })?;
                T::from_le_bytes(&bytes).map_err(VisualSignError::DecodeError)
            }
            Some(_) => T::from_move_value(value),
        },
    }
}

/// Returns the raw BCS bytes of an untyped `Pure` input. Typed values are
/// rejected because their JSON form is no longer the BCS encoding.
pub fn pure_bcs_bytes(call_arg: &SuiCallArg) -> Result<Vec<u8>, VisualSignError> {
    match call_arg {
        SuiCallArg::Object(_) => Err(VisualSignError::MissingData(
            "Unexpected object where pure bytes were expected".to_string(),
        )),
        SuiCallArg::FundsWithdrawal(_) => Err(VisualSignError::MissingData(
            "Unexpected funds withdrawal where pure bytes were expected".to_string(),
        )),
        SuiCallArg::Pure(value) if value.value_type().is_some() => Err(
            VisualSignError::MissingData("Unexpected typed pure value".to_string()),
        ),
        SuiCallArg::Pure(value) => json_array_to_bytes(&value.value().to_json_value())
            .map_err(|e| VisualSignError::DecodeError(format!("Invalid pure value bytes: {e}"))),
    }
}

fn json_array_to_bytes(value: &serde_json::Value) -> Result<Vec<u8>, String> {
    let arr = value
        .as_array()
        .ok_or_else(|| "Expected JSON array for pure bytes".to_string())?;

    arr.iter()
        .enumerate()
        .map(|(i, v)| {
            let n = v
                .as_u64()
                .ok_or_else(|| format!("Non-integer at index {i}"))?;
            u8::try_from(n).map_err(|_| format!("Byte out of range at index {i}: {n}"))
        })
        .collect()
}

pub trait FromLeBytes: Sized {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String>;

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError>;
}

/// The JSON of a pure value typed as `expected`, or the error `decode_number`
/// has always returned for a missing or different type.
fn typed_json(
    value: &SuiPureValue,
    expected: &TypeTag,
    name: &str,
) -> Result<serde_json::Value, VisualSignError> {
    let Some(value_type) = value.value_type() else {
        return Err(VisualSignError::DecodeError(
            "Failed to get value type".into(),
        ));
    };

    if &value_type != expected {
        return Err(VisualSignError::DecodeError(format!(
            "Expected {name} value type"
        )));
    }

    Ok(value.value().to_json_value())
}

/// Reads an unsigned integer the way `SuiJsonValue::to_move_value` does: a
/// decimal or `0x` hex string, or (for `u8`, `u16` and `u32` only) a JSON number.
fn json_to_unsigned<T>(json: &serde_json::Value, accept_number: bool) -> Result<T, VisualSignError>
where
    T: TryFrom<u128>,
{
    let wide: u128 = match json {
        serde_json::Value::Number(n) if accept_number => n
            .as_u64()
            .map(u128::from)
            .ok_or_else(|| VisualSignError::DecodeError(format!("Invalid number: {n}")))?,
        serde_json::Value::String(s) => match s.strip_prefix("0x") {
            Some(hex_digits) => u128::from_str_radix(hex_digits, 16),
            None => s.parse::<u128>(),
        }
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid number {s:?}: {e}")))?,
        other => {
            return Err(VisualSignError::DecodeError(format!(
                "Unexpected JSON for a number: {other}"
            )));
        }
    };
    T::try_from(wide)
        .map_err(|_| VisualSignError::DecodeError(format!("Number out of range: {wide}")))
}

impl FromLeBytes for bool {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() {
            return Err("bytes array is empty in FromLeBytes".to_string());
        }

        match bytes[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(format!("Invalid bool value: {}", bytes[0])),
        }
    }

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
        match typed_json(value, &TypeTag::Bool, "bool")? {
            serde_json::Value::Bool(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected bool value type".to_string(),
            )),
        }
    }
}

macro_rules! impl_from_le_bytes {
    ($($ty:ty => $tag:ident, $accept_number:expr;)*) => {
        $(
            impl FromLeBytes for $ty {
                fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
                    Ok(<$ty>::from_le_bytes(bytes.try_into().map_err(|e| {
                        format!(concat!("Invalid ", stringify!($ty), " value: {}"), e)
                    })?))
                }

                fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
                    let json = typed_json(value, &TypeTag::$tag, stringify!($ty))?;
                    json_to_unsigned(&json, $accept_number)
                }
            }
        )*
    };
}

impl_from_le_bytes! {
    u8 => U8, true;
    u16 => U16, true;
    u32 => U32, true;
    u64 => U64, false;
    u128 => U128, false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SuiJsonValue;

    fn typed(tag: TypeTag, json: serde_json::Value) -> SuiCallArg {
        SuiCallArg::Pure(SuiPureValue::new(Some(tag), SuiJsonValue::new(json)))
    }

    fn untyped(bytes: &[u8]) -> SuiCallArg {
        SuiCallArg::Pure(SuiPureValue::new(
            None,
            SuiJsonValue::new(serde_json::Value::from(bytes.to_vec())),
        ))
    }

    #[test]
    fn decodes_untyped_little_endian_bytes() {
        assert_eq!(
            decode_number::<u64>(&untyped(&1000u64.to_le_bytes())).unwrap(),
            1000
        );
        assert!(decode_number::<u64>(&untyped(&[1, 2])).is_err());
        assert!(decode_number::<bool>(&untyped(&[1])).unwrap());
    }

    #[test]
    fn decodes_typed_u64_string() {
        assert_eq!(
            decode_number::<u64>(&typed(TypeTag::U64, "1000".into())).unwrap(),
            1000
        );
        // A typed u64 whose bytes did not fit stays a byte array and is rejected.
        assert!(decode_number::<u64>(&typed(TypeTag::U64, serde_json::json!([1, 2]))).is_err());
        // The declared type must match the requested one.
        assert!(decode_number::<u32>(&typed(TypeTag::U64, "1000".into())).is_err());
        assert!(pure_bcs_bytes(&typed(TypeTag::U64, "1000".into())).is_err());
    }
}
