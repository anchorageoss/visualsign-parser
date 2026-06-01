//! Proto based Rust types.

// We don't want to run clippy on generated code.
#![allow(clippy::all)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Allow unused imports as they may be used by generated code -- currently it is required for post::Message and google::rpc::Status to work
#![allow(unused_imports)]

// Re-export prost to ensure crate users don't have to have the burden of keeping
// their own prost dep in sync with this crate.
pub use prost;
pub use prost_types;
pub use qos_hex;

use google::rpc::Status;
use prost::Message;

// Tonic version needs to be in sync with prost, so we re-export it here as well.
#[cfg(feature = "tonic_types")]
pub use tonic;
#[cfg(feature = "tonic_types")]
pub use tonic_reflection;

include!("generated/_include.rs");

/// Serde adapter that represents the `parser.Abi.abi_type` enum field as its
/// protobuf string name (e.g. `"ABI_TYPE_PROXY"`) over JSON instead of the raw
/// i32 discriminant. Referenced via `#[serde(with = "crate::abi_type_serde")]`
/// on the generated field. An unset/unknown value serializes as `null`; an
/// unrecognized string fails deserialization.
#[cfg(feature = "serde_derive")]
pub mod abi_type_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::parser::AbiType;

    pub fn serialize<S>(value: &Option<i32>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value.and_then(AbiType::from_i32) {
            Some(kind) => serializer.serialize_some(kind.as_str_name()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = Option::<String>::deserialize(deserializer)?;
        match name {
            Some(name) => {
                let kind = AbiType::from_str_name(&name).ok_or_else(|| {
                    serde::de::Error::custom(format!("unknown AbiType variant: {name}"))
                })?;
                Ok(Some(kind as i32))
            }
            None => Ok(None),
        }
    }
}

// Necessary to enable reflection on gRPC server
#[cfg(feature = "tonic_types")]
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("generated/descriptor.bin");

#[cfg(feature = "tonic_types")]
impl From<Status> for tonic::Status {
    fn from(status: Status) -> Self {
        Self::with_details(
            tonic::Code::from_i32(status.code),
            status.message.clone(),
            status.encode_to_vec().into(),
        )
    }
}
