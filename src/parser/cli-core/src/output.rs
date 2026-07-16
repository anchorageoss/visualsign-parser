use crate::chains::parse_chain;
use visualsign::registry::TransactionConverterRegistry;
use visualsign::vsptrait::VisualSignOptions;
use visualsign::{SignablePayload, SignablePayloadField};

/// Output format for the transaction visualizer.
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    /// Debug representation of the `SignablePayload` struct.
    Text,
    /// Pretty-printed JSON serialization of the `SignablePayload` struct.
    Json,
    /// Human-readable tree view of the `SignablePayload` struct.
    Human,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "human" => Ok(OutputFormat::Human),
            _ => Err(format!("Invalid output format: {s}")),
        }
    }
}

/// Formats a [`SignablePayload`] as a human-readable tree for terminal display.
pub struct HumanReadableFormatter<'a> {
    payload: &'a SignablePayload,
    condensed_only: bool,
}

impl<'a> HumanReadableFormatter<'a> {
    /// Creates a new formatter for the given payload.
    #[must_use]
    pub fn new(payload: &'a SignablePayload, condensed_only: bool) -> Self {
        Self {
            payload,
            condensed_only,
        }
    }

    fn format_field(
        &self,
        field: &SignablePayloadField,
        writer: &mut dyn std::fmt::Write,
        prefix: &str,
        continuation: &str,
    ) -> std::fmt::Result {
        match field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                writeln!(writer, "{} {}: {}", prefix, common.label, text_v2.text)?;
            }
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                writeln!(writer, "{} {}", prefix, common.label)?;

                if let Some(title) = &preview_layout.title {
                    writeln!(writer, "{}   Title: {}", continuation, title.text)?;
                }
                if let Some(subtitle) = &preview_layout.subtitle {
                    writeln!(writer, "{}   Detail: {}", continuation, subtitle.text)?;
                }

                // Condensed view (if present)
                if let Some(condensed_layout) = &preview_layout.condensed {
                    if !condensed_layout.fields.is_empty() {
                        writeln!(writer, "{continuation}   📋 Condensed View:")?;
                        for (i, nested_field) in condensed_layout.fields.iter().enumerate() {
                            let is_last_nested = i == condensed_layout.fields.len() - 1;
                            let nested_prefix = format!(
                                "{}   {}",
                                continuation,
                                if is_last_nested { "└─" } else { "├─" }
                            );
                            let nested_continuation = format!(
                                "{}   {}",
                                continuation,
                                if is_last_nested { "   " } else { "│  " }
                            );
                            self.format_field(
                                &nested_field.signable_payload_field,
                                writer,
                                &nested_prefix,
                                &nested_continuation,
                            )?;
                        }
                    }
                }

                // Expanded view (if present, only show if not condensed_only)
                if !self.condensed_only {
                    if let Some(expanded_layout) = &preview_layout.expanded {
                        if !expanded_layout.fields.is_empty() {
                            writeln!(writer, "{continuation}   📖 Expanded View:")?;
                            for (i, nested_field) in expanded_layout.fields.iter().enumerate() {
                                let is_last_nested = i == expanded_layout.fields.len() - 1;
                                let nested_prefix = format!(
                                    "{}   {}",
                                    continuation,
                                    if is_last_nested { "└─" } else { "├─" }
                                );
                                let nested_continuation = format!(
                                    "{}   {}",
                                    continuation,
                                    if is_last_nested { "   " } else { "│  " }
                                );
                                self.format_field(
                                    &nested_field.signable_payload_field,
                                    writer,
                                    &nested_prefix,
                                    &nested_continuation,
                                )?;
                            }
                        }
                    }
                }
            }
            SignablePayloadField::AmountV2 { common, amount_v2 } => {
                writeln!(
                    writer,
                    "{} {}: {} {}",
                    prefix,
                    common.label,
                    amount_v2.amount,
                    amount_v2.abbreviation.as_deref().unwrap_or("")
                )?;
            }
            SignablePayloadField::AddressV2 { common, address_v2 } => {
                writeln!(
                    writer,
                    "{} {}: {}",
                    prefix, common.label, address_v2.address
                )?;
            }
            _ => {
                writeln!(writer, "{} Field: {}", prefix, common_label(field))?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for HumanReadableFormatter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "┌─ Transaction: {}", self.payload.title)?;
        if let Some(subtitle) = &self.payload.subtitle {
            writeln!(f, "│  Subtitle: {subtitle}")?;
        }
        writeln!(f, "│  Version: {}", self.payload.version)?;
        if !self.payload.payload_type.is_empty() {
            writeln!(f, "│  Type: {}", self.payload.payload_type)?;
        }
        f.write_str("│\n")?;

        if !self.payload.fields.is_empty() {
            f.write_str("└─ Fields:\n")?;
            for (i, field) in self.payload.fields.iter().enumerate() {
                let is_last = i == self.payload.fields.len() - 1;
                let prefix = if is_last { "   └─" } else { "   ├─" };
                let continuation = if is_last { "      " } else { "   │  " };

                self.format_field(field, f, prefix, continuation)?;
            }
        }

        Ok(())
    }
}

/// Extracts the common label from any field type.
fn common_label(field: &SignablePayloadField) -> String {
    match field {
        SignablePayloadField::TextV2 { common, .. }
        | SignablePayloadField::PreviewLayout { common, .. }
        | SignablePayloadField::AmountV2 { common, .. }
        | SignablePayloadField::AddressV2 { common, .. } => common.label.clone(),
        _ => "Unknown".to_string(),
    }
}

/// Parses a raw transaction and prints it in the requested format.
pub fn parse_and_display(
    chain: &str,
    raw_tx: &str,
    registry: &TransactionConverterRegistry,
    options: VisualSignOptions,
    output_format: OutputFormat,
    condensed_only: bool,
) -> Result<(), String> {
    let registry_chain = parse_chain(chain);
    let conversion = registry
        .convert_transaction(&registry_chain, raw_tx, options)
        .map_err(|err| err.to_string())?;
    let payload = &conversion.payload;
    match output_format {
        OutputFormat::Json => {
            let json_output = serde_json::to_string_pretty(payload)
                .map_err(|err| format!("Failed to serialize output as JSON: {err}"))?;
            println!("{json_output}");
        }
        OutputFormat::Text => {
            println!("{payload:#?}");
        }
        OutputFormat::Human => {
            let formatter = HumanReadableFormatter::new(payload, condensed_only);
            println!("{formatter}");
            if !condensed_only {
                eprintln!(
                    "\nRun with `--condensed-only` to see what users see on hardware wallets"
                );
            }
        }
    }
    // When requested, dump the raw borsh `intermediate_output` bytes as hex.
    // These are the exact bytes signed into `ParsedTransactionPayload` and can
    // be captured verbatim as a downstream (e.g. C++) test fixture. Decoding
    // into a typed schema is chain-specific and intentionally left to the
    // consumer; the CLI stays chain-agnostic and passes the bytes through.
    if let Some(bytes) = &conversion.intermediate_output {
        println!(
            "\n=== intermediate_output ({} bytes, borsh) ===",
            bytes.len()
        );
        println!("{}", hex::encode(bytes));
    }
    Ok(())
}
