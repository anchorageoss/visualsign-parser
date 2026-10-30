//! JSON form of `Pure` inputs, matching `sui_json::SuiJsonValue` for the
//! values this parser produces.

use std::str::FromStr;

use super::base::{AddressParseError, SuiAddress};

/// A JSON-rendered `Pure` input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiJsonValue(serde_json::Value);

impl SuiJsonValue {
    #[must_use]
    pub fn new(value: serde_json::Value) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        self.0.clone()
    }

    /// Reads an address from a `0x` hex string or a JSON array of 32 bytes.
    ///
    /// # Errors
    /// Returns an error for any other shape or length.
    pub fn to_sui_address(&self) -> Result<SuiAddress, AddressParseError> {
        match &self.0 {
            serde_json::Value::String(s) => {
                let s = s.trim().to_lowercase();
                if !s.starts_with("0x") {
                    return Err(AddressParseError::new(
                        "Address hex string must start with 0x.",
                    ));
                }
                SuiAddress::from_str(&s)
            }
            serde_json::Value::Array(values) => {
                let bytes = values
                    .iter()
                    .map(|v| v.as_u64().and_then(|b| u8::try_from(b).ok()))
                    .collect::<Option<Vec<u8>>>()
                    .ok_or_else(|| {
                        AddressParseError::new("Invalid input: Cannot parse input into SuiAddress.")
                    })?;
                SuiAddress::from_bytes(bytes)
            }
            other => Err(AddressParseError::new(&format!(
                "Unexpected arg {other} for expected type address"
            ))),
        }
    }
}
