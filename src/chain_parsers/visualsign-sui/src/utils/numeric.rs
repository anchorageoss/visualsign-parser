use move_core_types::annotated_value::MoveTypeLayout;
use move_core_types::language_storage::TypeTag;
use move_core_types::runtime_value::MoveValue;
use std::convert::TryInto;
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
                let bytes = json_array_to_bytes(&value.value().to_json_value()).ok_or(
                    VisualSignError::DecodeError(
                        "Failed to convert call argument to JSON value".into(),
                    ),
                )?;
                T::from_le_bytes(&bytes).ok_or(VisualSignError::DecodeError(
                    "Failed to decode number".into(),
                ))
            }
            Some(_) => T::from_move_value(value),
        },
    }
}

fn json_array_to_bytes(value: &serde_json::Value) -> Option<Vec<u8>> {
    value.as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect()
    })
}

pub trait FromLeBytes: Sized {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self>;

    fn from_move_value(value: &SuiPureValue) -> Result<Self, VisualSignError>;
}

impl FromLeBytes for u8 {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(bytes[0])
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
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u16::from_le_bytes(bytes.try_into().ok()?))
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
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
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
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
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
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u128::from_le_bytes(bytes.try_into().ok()?))
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
