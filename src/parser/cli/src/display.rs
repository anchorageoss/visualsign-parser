use visualsign::{SignablePayload, SignablePayloadField};

/// Output format for CLI display.
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    /// Rust debug format
    Text,
    /// Pretty-printed JSON
    Json,
    /// Human-readable tree
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

/// Renders a [`SignablePayload`] as a human-readable tree.
pub struct HumanReadableFormatter<'a> {
    payload: &'a SignablePayload,
    condensed_only: bool,
}

impl<'a> HumanReadableFormatter<'a> {
    /// Create a new formatter.
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

fn common_label(field: &SignablePayloadField) -> String {
    match field {
        SignablePayloadField::TextV2 { common, .. }
        | SignablePayloadField::PreviewLayout { common, .. }
        | SignablePayloadField::AmountV2 { common, .. }
        | SignablePayloadField::AddressV2 { common, .. } => common.label.clone(),
        _ => "Unknown".to_string(),
    }
}

/// Print a [`SignablePayload`] using the chosen [`OutputFormat`].
pub fn print_payload(payload: &SignablePayload, format: OutputFormat, condensed_only: bool) {
    match format {
        OutputFormat::Json => match serde_json::to_string_pretty(payload) {
            Ok(s) => println!("{s}"),
            Err(_) => eprintln!("Error: Failed to serialize output as JSON"),
        },
        OutputFormat::Text => println!("{payload:#?}"),
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
}
