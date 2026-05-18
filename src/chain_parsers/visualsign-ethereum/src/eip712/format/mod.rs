//! ERC-7730 field-format renderers.

pub mod address_name;
pub mod amount;
pub mod calldata;
pub mod date;
pub mod duration;
pub mod enum_format;
pub mod nft;
pub mod raw;
pub mod token_amount;
pub mod unit;

use crate::eip712::descriptor::DescriptorField;
use crate::eip712::descriptor::path;
use crate::eip712::payload::{Domain, MessageValue};
use crate::registry::ContractRegistry;
use visualsign::SignablePayloadField;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("unsupported format: {0}")]
    Unsupported(String),
    #[error("path resolution failed: {0}")]
    Path(String),
    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
    #[error("missing param '{0}'")]
    MissingParam(&'static str),
    #[error("invalid param '{name}': {detail}")]
    InvalidParam { name: String, detail: String },
    #[error("descriptor field missing required key '{0}'")]
    DescriptorMissing(&'static str),
    #[error(transparent)]
    Visualsign(#[from] visualsign::vsptrait::VisualSignError),
}

pub struct RenderContext<'a> {
    pub chain_id: u64,
    pub registry: Option<&'a ContractRegistry>,
    pub domain: &'a Domain,
    pub message: &'a MessageValue,
}

/// Render one descriptor field against the message tree. May produce multiple
/// output fields if the field's path resolves to multiple values (array iterator).
pub fn render_field(
    field: &DescriptorField,
    ctx: &RenderContext,
) -> Result<Vec<SignablePayloadField>, RenderError> {
    let label = field.label.as_deref().unwrap_or("");
    let format = field
        .format
        .as_deref()
        .ok_or(RenderError::DescriptorMissing("format"))?;
    let path_str = field
        .path
        .as_deref()
        .ok_or(RenderError::DescriptorMissing("path"))?;

    let parsed = path::parse(path_str).map_err(|e| RenderError::Path(e.to_string()))?;
    let values = path::resolve(&parsed, ctx.message, ctx.domain)
        .map_err(|e| RenderError::Path(e.to_string()))?;

    let mut out = Vec::with_capacity(values.len());
    for v in &values {
        let f = match format {
            "raw" => raw::render(label, v)?,
            "amount" => amount::render(label, v, &field.params, ctx)?,
            "tokenAmount" => token_amount::render(label, v, &field.params, ctx)?,
            "addressName" => address_name::render(label, v, &field.params, ctx)?,
            "date" => date::render(label, v, &field.params)?,
            "duration" => duration::render(label, v)?,
            "unit" => unit::render(label, v, &field.params)?,
            "enum" => enum_format::render(label, v, &field.params)?,
            "nft" => nft::render(label, v, &field.params, ctx)?,
            "calldata" => calldata::render(label, v, &field.params, ctx)?,
            other => return Err(RenderError::Unsupported(other.into())),
        };
        out.push(f);
    }
    Ok(out)
}

/// Convert a non-empty decimal scaled by N decimals into a human-readable string,
/// trimming trailing zeros. `format_decimal(1_500_000, 6)` returns `"1.5"`.
pub(crate) fn format_decimal(amount: u128, decimals: u8) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let divisor = 10u128.pow(decimals as u32);
    let whole = amount / divisor;
    let frac = amount % divisor;
    if frac == 0 {
        whole.to_string()
    } else {
        let frac_str = format!("{frac:0width$}", width = decimals as usize);
        let trimmed = frac_str.trim_end_matches('0');
        format!("{whole}.{trimmed}")
    }
}
