use crate::chains::parse_chain;
use clap::Parser;
use visualsign::registry::{Chain, TransactionConverterRegistry};
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
use visualsign::{SignablePayload, SignablePayloadField};

#[derive(Parser, Debug)]
#[command(name = "visualsign-parser")]
#[command(version = "1.0")]
#[command(about = "Converts raw transactions to visual signing properties")]
pub(crate) struct Args {
    #[arg(short, long, help = "Chain type")]
    pub(crate) chain: String,

    #[arg(
        short,
        long,
        value_name = "RAW_TX",
        help = "Raw transaction string. Prefix with '@' to read from a file \
                (e.g. '@/path/to/tx.hex'), or use '@-' to read from stdin."
    )]
    transaction: String,

    #[arg(short, long, default_value = "text", help = "Output format")]
    output: OutputFormat,

    #[arg(
        long,
        help = "Show only condensed view (what hardware wallets display)"
    )]
    condensed_only: bool,

    #[arg(
        long,
        help = "Also pretty-print the chain-specific intermediate output \
                (used by Turnkey's Solana policy engine). Currently produced \
                for Solana only; ignored for other chains."
    )]
    with_intermediate: bool,

    #[arg(
        long,
        short = 'n',
        value_name = "NETWORK",
        help = "Network identifier - supports:\n\
                Chain ID: 1, 137, 42161, etc.\n\
                Canonical name: ETHEREUM_MAINNET, POLYGON_MAINNET, ARBITRUM_MAINNET, etc."
    )]
    pub(crate) network: Option<String>,

    #[cfg(feature = "ethereum")]
    #[command(flatten)]
    pub(crate) ethereum: crate::ethereum::EthereumArgs,

    #[cfg(feature = "solana")]
    #[command(flatten)]
    pub(crate) solana: crate::solana::SolanaArgs,
}

#[derive(Debug, Clone, Copy)]
enum OutputFormat {
    Text,
    Json,
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

struct HumanReadableFormatter<'a> {
    payload: &'a SignablePayload,
    condensed_only: bool,
}

impl<'a> HumanReadableFormatter<'a> {
    fn new(payload: &'a SignablePayload, condensed_only: bool) -> Self {
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

/// Helper to extract common label from any field type
fn common_label(field: &SignablePayloadField) -> String {
    match field {
        SignablePayloadField::TextV2 { common, .. }
        | SignablePayloadField::PreviewLayout { common, .. }
        | SignablePayloadField::AmountV2 { common, .. }
        | SignablePayloadField::AddressV2 { common, .. } => common.label.clone(),
        _ => "Unknown".to_string(),
    }
}

fn parse_and_display(
    chain: &str,
    raw_tx: &str,
    registry: &TransactionConverterRegistry,
    options: VisualSignOptions,
    output_format: OutputFormat,
    condensed_only: bool,
    with_intermediate: bool,
) -> Result<(), String> {
    let registry_chain = parse_chain(chain);
    let conversion = registry
        .convert_transaction(&registry_chain, raw_tx, options)
        .map_err(|err| err.to_string())?;
    let intermediate_bytes = conversion.intermediate_output.clone();
    let payload = conversion.payload;
    match output_format {
        OutputFormat::Json => {
            let json_output = serde_json::to_string_pretty(&payload)
                .map_err(|err| format!("Failed to serialize output as JSON: {err}"))?;
            println!("{json_output}");
        }
        OutputFormat::Text => {
            println!("{payload:#?}");
        }
        OutputFormat::Human => {
            let formatter = HumanReadableFormatter::new(&payload, condensed_only);
            println!("{formatter}");
            if !condensed_only {
                eprintln!(
                    "\nRun with `--condensed-only` to see what users see on hardware wallets"
                );
            }
        }
    }
    if with_intermediate {
        print_intermediate_output(
            &registry_chain,
            intermediate_bytes.as_deref(),
            output_format,
        )?;
    }
    Ok(())
}

fn print_intermediate_output(
    chain: &Chain,
    bytes: Option<&[u8]>,
    output_format: OutputFormat,
) -> Result<(), String> {
    let Some(bytes) = bytes else {
        eprintln!(
            "\n(no intermediate output produced for chain {})",
            chain.as_str()
        );
        return Ok(());
    };

    match chain {
        #[cfg(feature = "solana")]
        Chain::Solana => {
            use visualsign_solana::intermediate::SolanaIntermediateOutput;
            let parsed: SolanaIntermediateOutput = borsh::from_slice(bytes)
                .map_err(|err| format!("Failed to borsh-decode SolanaIntermediateOutput: {err}"))?;
            println!("\n=== Intermediate Output (Solana, policy schema) ===");
            match output_format {
                OutputFormat::Json => {
                    let json =
                        serde_json::to_string_pretty(&serialize_solana_intermediate(&parsed))
                            .map_err(|err| {
                                format!("Failed to serialize intermediate output as JSON: {err}")
                            })?;
                    println!("{json}");
                }
                _ => {
                    println!("{parsed:#?}");
                }
            }
        }
        _ => {
            eprintln!(
                "\n(intermediate output present ({} bytes) but no decoder for chain {})",
                bytes.len(),
                chain.as_str()
            );
        }
    }
    Ok(())
}

#[cfg(feature = "solana")]
fn serialize_solana_intermediate(
    output: &visualsign_solana::intermediate::SolanaIntermediateOutput,
) -> serde_json::Value {
    use serde_json::json;
    use visualsign_solana::intermediate::{
        SolTransfer, SolanaAccount, SolanaAddressTableLookup, SolanaIntermediateInstruction,
        SolanaParsedInstructionDataIo, SolanaSingleAddressTableLookup, SplTransfer,
    };

    fn account(a: &SolanaAccount) -> serde_json::Value {
        json!({
            "account_key": a.account_key,
            "signer": a.signer,
            "writable": a.writable,
        })
    }

    fn single_lookup(lk: &SolanaSingleAddressTableLookup) -> serde_json::Value {
        json!({
            "address_table_key": lk.address_table_key,
            "index": lk.index,
            "writable": lk.writable,
        })
    }

    fn lookup(lk: &SolanaAddressTableLookup) -> serde_json::Value {
        json!({
            "address_table_key": lk.address_table_key,
            "writable_indexes": lk.writable_indexes,
            "readonly_indexes": lk.readonly_indexes,
        })
    }

    fn parsed(pid: &SolanaParsedInstructionDataIo) -> serde_json::Value {
        let args: serde_json::Value = serde_json::from_str(&pid.program_call_args_json)
            .unwrap_or_else(|_| json!(pid.program_call_args_json));
        json!({
            "instruction_name": pid.instruction_name,
            "discriminator": pid.discriminator,
            "named_accounts": pid.named_accounts,
            "program_call_args": args,
            "idl_source": pid.idl_source,
            "idl_hash": pid.idl_hash,
        })
    }

    fn instruction(i: &SolanaIntermediateInstruction) -> serde_json::Value {
        json!({
            "program_key": i.program_key,
            "accounts": i.accounts.iter().map(account).collect::<Vec<_>>(),
            "instruction_data_hex": i.instruction_data_hex,
            "address_table_lookups": i.address_table_lookups.iter().map(single_lookup).collect::<Vec<_>>(),
            "parsed_instruction_data": i.parsed_instruction_data.as_ref().map(parsed),
        })
    }

    fn sol_transfer(t: &SolTransfer) -> serde_json::Value {
        json!({"from": t.from, "to": t.to, "amount": t.amount})
    }

    fn spl_transfer(t: &SplTransfer) -> serde_json::Value {
        json!({
            "from": t.from,
            "to": t.to,
            "amount": t.amount,
            "owner": t.owner,
            "signers": t.signers,
            "token_mint": t.token_mint,
            "decimals": t.decimals,
            "fee": t.fee,
        })
    }

    json!({
        "account_keys": output.account_keys,
        "program_keys": output.program_keys,
        "instructions": output.instructions.iter().map(instruction).collect::<Vec<_>>(),
        "transfers": output.transfers.iter().map(sol_transfer).collect::<Vec<_>>(),
        "spl_transfers": output.spl_transfers.iter().map(spl_transfer).collect::<Vec<_>>(),
        "recent_blockhash": output.recent_blockhash,
        "address_table_lookups": output.address_table_lookups.iter().map(lookup).collect::<Vec<_>>(),
    })
}

/// app cli
pub struct Cli;
impl Cli {
    /// start the parser cli
    pub fn execute() -> Result<(), String> {
        let args = Args::parse();
        let chain = parse_chain(&args.chain);
        let plugins = crate::build_plugins(&args);

        let mut registry = TransactionConverterRegistry::new();
        for plugin in &plugins {
            plugin.register(&mut registry);
        }

        let plugin = plugins.iter().find(|p| p.chain() == chain).ok_or_else(|| {
            let supported: Vec<String> = plugins
                .iter()
                .map(|p| p.chain().as_str().to_lowercase())
                .collect();
            let supported_str = if supported.is_empty() {
                "none".to_string()
            } else {
                supported.join(", ")
            };
            if chain == Chain::Unspecified {
                format!(
                    "unrecognized chain '{}'.\nSupported chains: {supported_str}",
                    args.chain,
                )
            } else {
                format!(
                    "chain '{}' is not supported by this CLI build.\n\
                     Supported chains: {supported_str}",
                    args.chain,
                )
            }
        })?;

        let chain_metadata = plugin.create_metadata(args.network.clone())?;

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: chain_metadata,
            developer_config: Some(DeveloperConfig {
                allow_signed_transactions: true,
            }),
        };

        let raw_tx = match crate::tx_input::resolve_transaction_input(&args.transaction) {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        };

        parse_and_display(
            &args.chain,
            &raw_tx,
            &registry,
            options,
            args.output,
            args.condensed_only,
            args.with_intermediate,
        )
    }
}
