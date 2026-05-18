//! ERC-7730 descriptor model, matching, and path resolution.

pub mod embedded;
pub mod path;
pub mod registry;
pub mod schema;

use alloy_primitives::Address;
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

/// Treat explicit `null` JSON values as `Default::default()`. Some ERC-7730
/// descriptors use `null` instead of omitting a field, e.g., `"excluded": null`.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Erc7730Descriptor {
    #[serde(default)]
    pub context: DescriptorContext,
    /// Display formats. Optional because some descriptors only contribute metadata
    /// or constants and inherit display from an `includes`-referenced descriptor.
    #[serde(default)]
    pub display: DescriptorDisplay,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DescriptorContext {
    #[serde(default)]
    pub eip712: Option<Eip712Context>,
    #[serde(default)]
    pub contract: Option<ContractContext>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Eip712Context {
    #[serde(default)]
    pub schemas: Vec<Eip712Schema>,
    #[serde(default)]
    pub deployments: Vec<Deployment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContractContext {
    #[serde(default)]
    pub deployments: Vec<Deployment>,
    #[serde(default)]
    pub abi: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Eip712Schema {
    pub types: BTreeMap<String, Vec<SchemaTypeMember>>,
    #[serde(rename = "primaryType")]
    pub primary_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchemaTypeMember {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deployment {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub address: Address,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DescriptorDisplay {
    #[serde(default)]
    pub formats: BTreeMap<String, DisplayFormat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DisplayFormat {
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub fields: Vec<DescriptorField>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub excluded: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub required: Vec<String>,
    #[serde(default)]
    pub screens: serde_json::Value, // ignored
}

#[derive(Debug, Clone, Deserialize)]
pub struct DescriptorField {
    /// Path into the message tree. Present for reference-style fields.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    /// Literal value for constant fields (some descriptors use this instead of `path`).
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Nested fields for group-style entries (label + fields).
    #[serde(default)]
    pub fields: Option<Vec<DescriptorField>>,
}

/// Read a file from the (optional) `static/eip7730` tree. Returns `None` when the
/// file isn't present so CI checkouts without the vendored registry still pass.
#[cfg(test)]
pub(crate) fn read_optional_static(relative: &str) -> Option<String> {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("static/eip7730");
    path.push(relative);
    std::fs::read_to_string(&path).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn parses_erc2612_descriptor() {
        let Some(text) = read_optional_static("ercs/eip712-erc2612-permit.json") else {
            eprintln!("skip: static/eip7730 not present");
            return;
        };
        let d: Erc7730Descriptor = serde_json::from_str(&text).unwrap();
        let ctx = d.context.eip712.expect("expected eip712 context");
        assert_eq!(ctx.schemas[0].primary_type, "Permit");
        let permit_fmt = d.display.formats.get("Permit").unwrap();
        assert_eq!(permit_fmt.fields.len(), 3);
        assert_eq!(permit_fmt.fields[0].format.as_deref(), Some("raw"));
        assert_eq!(permit_fmt.fields[1].format.as_deref(), Some("tokenAmount"));
        assert_eq!(permit_fmt.fields[2].format.as_deref(), Some("date"));
        assert_eq!(permit_fmt.excluded, vec!["owner", "nonce"]);
    }
}
