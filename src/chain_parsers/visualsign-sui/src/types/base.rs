//! 32-byte Sui identifiers with the string forms the presets rely on.
//!
//! These mirror the formatting of `sui_types::base_types::{SuiAddress, ObjectID}`
//! exactly, so rendered payloads stay byte-identical:
//! - `Display` is `0x` followed by all 64 lowercase hex digits.
//! - `ObjectID::to_hex_literal` is the short Move form (`0x2`).
//! - `ObjectID::to_hex` is the 64 hex digits without a prefix.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

const LENGTH: usize = 32;

/// Error returned when a string or byte slice is not a 32-byte Sui address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressParseError(String);

impl fmt::Display for AddressParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AddressParseError {}

impl AddressParseError {
    pub(crate) fn new(message: &str) -> Self {
        Self(message.to_string())
    }
}

fn decode_hex_32(hex_digits: &str) -> Result<[u8; LENGTH], AddressParseError> {
    let bytes = hex::decode(hex_digits).map_err(|e| AddressParseError(e.to_string()))?;
    <[u8; LENGTH]>::try_from(bytes.as_slice()).map_err(|_| {
        AddressParseError(format!(
            "Expected {LENGTH} bytes, got {} bytes",
            bytes.len()
        ))
    })
}

/// Lowercase hex with leading zeros trimmed (`"0"` for the zero address).
fn short_hex(bytes: &[u8; LENGTH]) -> String {
    let full = hex::encode(bytes);
    let trimmed = full.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Parses `0x`-prefixed hex, left-padding short literals such as `0x2`.
fn from_hex_literal(literal: &str) -> Result<[u8; LENGTH], AddressParseError> {
    let digits = literal
        .strip_prefix("0x")
        .ok_or_else(|| AddressParseError("Hex literal must start with 0x".to_string()))?;
    if digits.len() < LENGTH * 2 {
        decode_hex_32(&format!("{digits:0>width$}", width = LENGTH * 2))
    } else {
        decode_hex_32(digits)
    }
}

macro_rules! sui_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
        pub struct $name([u8; LENGTH]);

        impl $name {
            pub const ZERO: Self = Self([0; LENGTH]);

            #[must_use]
            pub const fn new(bytes: [u8; LENGTH]) -> Self {
                Self(bytes)
            }

            /// Accepts exactly 32 bytes.
            ///
            /// # Errors
            /// Returns an error for any other length.
            pub fn from_bytes<T: AsRef<[u8]>>(bytes: T) -> Result<Self, AddressParseError> {
                <[u8; LENGTH]>::try_from(bytes.as_ref())
                    .map(Self)
                    .map_err(|_| {
                        AddressParseError(format!(
                            "Expected {LENGTH} bytes, got {} bytes",
                            bytes.as_ref().len()
                        ))
                    })
            }

            #[must_use]
            pub const fn into_bytes(self) -> [u8; LENGTH] {
                self.0
            }

            /// Parses a `0x`-prefixed literal, left-padding short forms (`0x2`).
            ///
            /// # Errors
            /// Returns an error without the `0x` prefix, for non-hex digits, or for
            /// more than 64 digits.
            pub fn from_hex_literal(literal: &str) -> Result<Self, AddressParseError> {
                from_hex_literal(literal).map(Self)
            }

            /// Short Move form: `0x` plus hex with leading zeros trimmed.
            #[must_use]
            pub fn to_hex_literal(self) -> String {
                format!("0x{}", short_hex(&self.0))
            }

            /// All 64 hex digits, no prefix.
            #[must_use]
            pub fn to_hex(self) -> String {
                hex::encode(self.0)
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "0x{}", hex::encode(self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "0x{}", hex::encode(self.0))
            }
        }

        impl From<sui_sdk_types::Address> for $name {
            fn from(address: sui_sdk_types::Address) -> Self {
                Self(address.into_inner())
            }
        }

        impl From<$name> for sui_sdk_types::Address {
            fn from(id: $name) -> Self {
                sui_sdk_types::Address::new(id.0)
            }
        }
    };
}

sui_id!(
    /// A Sui account address.
    SuiAddress
);

sui_id!(
    /// A Sui object or package ID.
    ObjectID
);

impl ObjectID {
    /// The ID whose last byte is `byte` and the rest zero (`0x2` for 2).
    #[must_use]
    pub const fn from_single_byte(byte: u8) -> Self {
        let mut bytes = [0; LENGTH];
        bytes[LENGTH - 1] = byte;
        Self(bytes)
    }
}

impl From<ObjectID> for SuiAddress {
    fn from(id: ObjectID) -> Self {
        Self(id.0)
    }
}

impl From<SuiAddress> for ObjectID {
    fn from(address: SuiAddress) -> Self {
        Self(address.0)
    }
}

impl FromStr for SuiAddress {
    type Err = AddressParseError;

    /// Exactly 64 hex digits, with or without a `0x` prefix.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode_hex_32(s.strip_prefix("0x").unwrap_or(s)).map(Self)
    }
}

impl FromStr for ObjectID {
    type Err = AddressParseError;

    /// 64 hex digits with or without `0x`, or a short `0x` literal.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        decode_hex_32(s.strip_prefix("0x").unwrap_or(s))
            .or_else(|_| from_hex_literal(s))
            .map(Self)
    }
}

/// An object version.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    #[cfg(test)]
    #[must_use]
    pub const fn new() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    #[cfg(test)]
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// An object digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ObjectDigest([u8; LENGTH]);

impl ObjectDigest {
    #[cfg(test)]
    #[must_use]
    pub const fn inner(&self) -> &[u8; LENGTH] {
        &self.0
    }
}

impl From<sui_sdk_types::Digest> for ObjectDigest {
    fn from(digest: sui_sdk_types::Digest) -> Self {
        Self(digest.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_hex_forms_match_sui_types() {
        let mut bytes = [0u8; LENGTH];
        bytes[LENGTH - 1] = 2;
        let id = ObjectID::new(bytes);
        assert_eq!(
            id.to_string(),
            "0x0000000000000000000000000000000000000000000000000000000000000002"
        );
        assert_eq!(id.to_hex_literal(), "0x2");
        assert_eq!(
            id.to_hex(),
            "0000000000000000000000000000000000000000000000000000000000000002"
        );
        assert_eq!(ObjectID::ZERO.to_hex_literal(), "0x0");
        assert_eq!(SuiAddress::new(bytes).to_string(), id.to_string());
    }

    #[test]
    fn hex_literal_parsing() {
        assert_eq!(
            ObjectID::from_hex_literal("0x2").unwrap().to_hex_literal(),
            "0x2"
        );
        assert!(ObjectID::from_hex_literal("2").is_err());
        assert!(ObjectID::from_hex_literal(&format!("0x{}", "1".repeat(65))).is_err());
        let full = format!("0x{}", "ab".repeat(32));
        assert_eq!(ObjectID::from_hex_literal(&full).unwrap().to_string(), full);
        assert_eq!(full.parse::<SuiAddress>().unwrap().to_string(), full);
        assert_eq!(full[2..].parse::<SuiAddress>().unwrap().to_string(), full);
        assert!("0x2".parse::<SuiAddress>().is_err());
        assert_eq!("0x2".parse::<ObjectID>().unwrap().to_hex_literal(), "0x2");
    }

    #[test]
    fn bcs_is_32_raw_bytes() {
        let address = SuiAddress::new([0xab; LENGTH]);
        let encoded = bcs::to_bytes(&address).unwrap();
        assert_eq!(encoded, vec![0xab; LENGTH]);
        assert_eq!(bcs::from_bytes::<SuiAddress>(&encoded).unwrap(), address);
    }
}
