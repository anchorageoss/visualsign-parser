use move_core_types::annotated_value::MoveTypeLayout;
use move_core_types::language_storage::TypeTag;
use move_core_types::runtime_value::MoveValue;
use std::convert::{TryFrom, TryInto};
use sui_json::SuiJsonValue;
use sui_json_rpc_types::{SuiCallArg, SuiPureValue};
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
        SuiCallArg::Pure(value) => match value.value_type() {
            None => {
                let bytes = json_array_to_bytes(&value.value().to_json_value()).map_err(|e| {
                    VisualSignError::DecodeError(format!("Invalid pure value bytes: {e}"))
                })?;
                T::from_le_bytes(&bytes).map_err(|e| VisualSignError::DecodeError(e.to_string()))
            }
            Some(_) => T::from_move_value(value),
        },
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
        let Some(value_type) = value.value_type() else {
            return Err(VisualSignError::DecodeError(
                "Failed to get value type".into(),
            ));
        };

        if value_type != TypeTag::Bool {
            return Err(VisualSignError::DecodeError(
                "Expected bool value type".into(),
            ));
        }

        let move_value =
            SuiJsonValue::to_move_value(&value.value().to_json_value(), &MoveTypeLayout::Bool)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        match move_value {
            MoveValue::Bool(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected bool value type".to_string(),
            )),
        }
    }
}

impl FromLeBytes for u8 {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(u8::from_le_bytes(
            bytes
                .try_into()
                .map_err(|e| format!("Invalid u8 value: {e}"))?,
        ))
    }

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
        let Some(value_type) = value.value_type() else {
            return Err(VisualSignError::DecodeError(
                "Failed to get value type".into(),
            ));
        };

        if value_type != TypeTag::U8 {
            return Err(VisualSignError::DecodeError(
                "Expected u8 value type".into(),
            ));
        }

        let move_value =
            SuiJsonValue::to_move_value(&value.value().to_json_value(), &MoveTypeLayout::U8)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        match move_value {
            MoveValue::U8(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected u8 value type".to_string(),
            )),
        }
    }
}

impl FromLeBytes for u16 {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(u16::from_le_bytes(
            bytes
                .try_into()
                .map_err(|e| format!("Invalid u16 value: {e}"))?,
        ))
    }

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
        let Some(value_type) = value.value_type() else {
            return Err(VisualSignError::DecodeError(
                "Failed to get value type".into(),
            ));
        };

        if value_type != TypeTag::U16 {
            return Err(VisualSignError::DecodeError(
                "Expected u16 value type".into(),
            ));
        }

        let move_value =
            SuiJsonValue::to_move_value(&value.value().to_json_value(), &MoveTypeLayout::U16)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        match move_value {
            MoveValue::U16(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected u16 value type".to_string(),
            )),
        }
    }
}

impl FromLeBytes for u32 {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(u32::from_le_bytes(
            bytes
                .try_into()
                .map_err(|e| format!("Invalid u32 value: {e}"))?,
        ))
    }

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
        let Some(value_type) = value.value_type() else {
            return Err(VisualSignError::DecodeError(
                "Failed to get value type".into(),
            ));
        };

        if value_type != TypeTag::U32 {
            return Err(VisualSignError::DecodeError(
                "Expected u32 value type".into(),
            ));
        }

        let move_value =
            SuiJsonValue::to_move_value(&value.value().to_json_value(), &MoveTypeLayout::U32)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        match move_value {
            MoveValue::U32(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected u32 value type".to_string(),
            )),
        }
    }
}

impl FromLeBytes for u64 {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(u64::from_le_bytes(
            bytes
                .try_into()
                .map_err(|e| format!("Invalid u64 value: {e}"))?,
        ))
    }

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
        let Some(value_type) = value.value_type() else {
            return Err(VisualSignError::DecodeError(
                "Failed to get value type".into(),
            ));
        };

        if value_type != TypeTag::U64 {
            return Err(VisualSignError::DecodeError(
                "Expected u64 value type".into(),
            ));
        }

        let move_value =
            SuiJsonValue::to_move_value(&value.value().to_json_value(), &MoveTypeLayout::U64)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        match move_value {
            MoveValue::U64(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected u64 value type".to_string(),
            )),
        }
    }
}

impl FromLeBytes for u128 {
    fn from_le_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(u128::from_le_bytes(
            bytes
                .try_into()
                .map_err(|e| format!("Invalid u128 value: {e}"))?,
        ))
    }

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError> {
        let Some(value_type) = value.value_type() else {
            return Err(VisualSignError::DecodeError(
                "Failed to get value type".into(),
            ));
        };

        if value_type != TypeTag::U128 {
            return Err(VisualSignError::DecodeError(
                "Expected u128 value type".into(),
            ));
        }

        let move_value =
            SuiJsonValue::to_move_value(&value.value().to_json_value(), &MoveTypeLayout::U128)
                .map_err(|e| VisualSignError::DecodeError(e.to_string()))?;

        match move_value {
            MoveValue::U128(value) => Ok(value),
            _ => Err(VisualSignError::DecodeError(
                "Expected u128 value type".to_string(),
            )),
        }
    }
}
