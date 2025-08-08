use std::convert::TryInto;

use sui_json_rpc_types::SuiCallArg;

use move_core_types::annotated_value::MoveTypeLayout;

#[allow(dead_code)]
pub fn decode_bool(call_arg: &SuiCallArg) -> Option<bool> {
    match call_arg.pure()?.to_bcs_bytes(&MoveTypeLayout::Bool).ok()?[0] {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

// TODO: think about u256 and fallback options
pub fn decode_number<T>(call_arg: &SuiCallArg) -> Option<T>
where
    T: FromLeBytes,
{
    T::from_le_bytes(&json_array_to_bytes(&call_arg.pure()?.to_json_value())?)
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
}

impl FromLeBytes for u8 {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(bytes[0])
    }
}

impl FromLeBytes for u16 {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u16::from_le_bytes(bytes.try_into().ok()?))
    }
}

impl FromLeBytes for u32 {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
    }
}

impl FromLeBytes for u64 {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
    }
}

impl FromLeBytes for u128 {
    fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        Some(u128::from_le_bytes(bytes.try_into().ok()?))
    }
}
