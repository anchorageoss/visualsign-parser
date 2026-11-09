//! Marinade Finance preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::MarinadeFinanceConfig;
use solana_parser::solana::structs::IdlAccount;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use std::collections::BTreeMap;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_raw_data_field, create_text_field};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const MARINADE_FINANCE_PROGRAM_ID: &str = "MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD";

const MARINADE_FINANCE_DISPLAY_NAME: &str = "Marinade Finance";

const MARINADE_FINANCE_IDL_JSON: &str = include_str!("marinade_finance.json");

static MARINADE_FINANCE_CONFIG: MarinadeFinanceConfig = MarinadeFinanceConfig;

pub struct MarinadeFinanceVisualizer;

impl InstructionVisualizer for MarinadeFinanceVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        // InstructionView, not resolve_accounts()?: an account from an address
        // lookup table shows as an unresolved(N) placeholder instead of failing
        // the whole transaction.
        let view = InstructionView::from_context(context);
        let program_id_str = view.program_id.clone();
        let data = context.data();

        let parsed_result = parse_marinade_finance_instruction(data);

        let (condensed_fields, expanded_fields, title_text) = match &parsed_result {
            Ok(parsed) => {
                let accounts = build_named_accounts(&parsed.instruction_name, &view.accounts);
                (
                    build_condensed_fields(&parsed.instruction_name)?,
                    build_parsed_fields(&program_id_str, parsed, &accounts, data)?,
                    format!(
                        "{MARINADE_FINANCE_DISPLAY_NAME}: {}",
                        parsed.instruction_name
                    ),
                )
            }
            Err(_) => (
                build_fallback_condensed_fields()?,
                build_fallback_fields(&program_id_str, data)?,
                format!("{MARINADE_FINANCE_DISPLAY_NAME}: Unknown Instruction"),
            ),
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 { text: title_text }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(SignablePayloadFieldListLayout {
                fields: condensed_fields,
            }),
            expanded: Some(SignablePayloadFieldListLayout {
                fields: expanded_fields,
            }),
        };

        let fallback_text = format!("Program ID: {program_id_str}\nData: {}", hex::encode(data));

        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {}", context.instruction_index() + 1),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&MARINADE_FINANCE_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::StakingPools(MARINADE_FINANCE_DISPLAY_NAME)
    }
}

/// Load and cache the bundled Marinade Finance IDL.
fn load_marinade_finance_idl() -> Option<&'static Idl> {
    static IDL: std::sync::OnceLock<Option<Idl>> = std::sync::OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(MARINADE_FINANCE_IDL_JSON).ok())
        .as_ref()
}

/// Parse an instruction's data using the bundled IDL.
fn parse_marinade_finance_instruction(
    data: &[u8],
) -> Result<SolanaParsedInstructionData, VisualSignError> {
    if data.len() < 8 {
        return Err(VisualSignError::DecodeError(
            "instruction data shorter than 8-byte discriminator".into(),
        ));
    }

    let idl = load_marinade_finance_idl().ok_or_else(|| {
        VisualSignError::DecodeError("failed to load Marinade Finance IDL".into())
    })?;

    parse_instruction_with_idl(data, MARINADE_FINANCE_PROGRAM_ID, idl)
        .map_err(|e| VisualSignError::DecodeError(e.to_string()))
}

/// Each instruction's account names in the order the program reads them.
///
/// Marinade's IDL is in the legacy Anchor format, where an account can be a
/// group of accounts (`updateActive`'s `common`, `initialize`'s `liqPool`) that
/// the transaction carries as its members, inline. The decoded IDL keeps a
/// group's members in `IdlAccount::accounts`; flatten them in order and name
/// each `group.member`, the same names solana_parser's `create_accounts_map`
/// gives the intermediate output (pinned by a parity test below).
fn flattened_account_names() -> &'static BTreeMap<String, Vec<String>> {
    static NAMES: std::sync::OnceLock<BTreeMap<String, Vec<String>>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        fn flatten(prefix: &str, accounts: &[IdlAccount], out: &mut Vec<String>) {
            for account in accounts {
                let name = format!("{prefix}{}", account.name);
                if account.accounts.is_empty() {
                    out.push(name);
                } else {
                    flatten(&format!("{name}."), &account.accounts, out);
                }
            }
        }
        let mut names = BTreeMap::new();
        for instruction in load_marinade_finance_idl()
            .map(|idl| idl.instructions.as_slice())
            .unwrap_or_default()
        {
            let mut flat = Vec::new();
            flatten("", &instruction.accounts, &mut flat);
            names.insert(instruction.name.clone(), flat);
        }
        names
    })
}

/// The instruction's accounts, named by position, plus any past the IDL's list.
struct InstructionAccounts {
    named: BTreeMap<String, String>,
    remaining: Vec<String>,
}

fn build_named_accounts(instruction_name: &str, accounts: &[String]) -> InstructionAccounts {
    let names = flattened_account_names()
        .get(instruction_name)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut result = InstructionAccounts {
        named: BTreeMap::new(),
        remaining: Vec::new(),
    };
    for (index, account) in accounts.iter().enumerate() {
        match names.get(index) {
            Some(name) => {
                result.named.insert(name.clone(), account.clone());
            }
            None => result.remaining.push(account.clone()),
        }
    }
    result
}

/// Build the expanded fields shown when the IDL parsed successfully.
fn build_parsed_fields(
    program_id: &str,
    parsed: &SolanaParsedInstructionData,
    accounts: &InstructionAccounts,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    for (name, address) in &accounts.named {
        fields.push(create_text_field(name, address)?);
    }
    for (index, address) in accounts.remaining.iter().enumerate() {
        fields.push(create_text_field(
            &format!("Remaining Account {}", index + 1),
            address,
        )?);
    }

    let mut arg_entries: Vec<(&String, &serde_json::Value)> =
        parsed.program_call_args.iter().collect();
    arg_entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in arg_entries {
        fields.push(create_text_field(name, &format_arg_value(value))?);
    }

    append_raw_data(&mut fields, data)?;

    Ok(fields)
}

/// Build the expanded fields shown when IDL parsing failed.
fn build_fallback_fields(
    program_id: &str,
    data: &[u8],
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Status", "IDL parsing failed - showing raw data")?,
    ];
    append_raw_data(&mut fields, data)?;
    Ok(fields)
}

/// Build the condensed fields when parsing succeeded.
fn build_condensed_fields(
    instruction_name: &str,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok(vec![
        create_text_field("Program", MARINADE_FINANCE_DISPLAY_NAME)?,
        create_text_field("Instruction", instruction_name)?,
    ])
}

/// Build the condensed fields when parsing failed.
fn build_fallback_condensed_fields() -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    Ok(vec![create_text_field(
        "Program",
        MARINADE_FINANCE_DISPLAY_NAME,
    )?])
}

fn append_raw_data(
    fields: &mut Vec<AnnotatedPayloadField>,
    data: &[u8],
) -> Result<(), VisualSignError> {
    fields.push(create_raw_data_field(data, None)?);
    Ok(())
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_parser::solana::idl_parser::create_accounts_map;
    use solana_parser::solana::structs::{AccountAddress, SolanaAccount};

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|index| format!("Account{index:02}")).collect()
    }

    #[test]
    fn test_update_deactivated_names_follow_flattened_groups() {
        // `common` is a group of 13 accounts; the decoded IDL would treat it as
        // one, which labelled common's second member "operationalSolAccount".
        let names = &flattened_account_names()["updateDeactivated"];
        assert_eq!(names.len(), 15);
        assert_eq!(names[0], "common.state");
        assert_eq!(names[13], "operationalSolAccount");
        assert_eq!(names[14], "systemProgram");

        let accounts = keys(15);
        let result = build_named_accounts("updateDeactivated", &accounts);
        assert_eq!(result.named["common.state"], "Account00");
        assert_eq!(result.named["operationalSolAccount"], "Account13");
        assert_eq!(result.named["systemProgram"], "Account14");
        assert!(result.remaining.is_empty());
    }

    #[test]
    fn test_names_match_solana_parsers_create_accounts_map() {
        // The preset and the intermediate output must name every account the
        // same way, nested groups included.
        let idl = load_marinade_finance_idl().expect("IDL");
        for instruction in &idl.instructions {
            let names = &flattened_account_names()[&instruction.name];
            let keys = keys(names.len());
            let addresses: Vec<AccountAddress> = keys
                .iter()
                .map(|key| {
                    AccountAddress::Static(SolanaAccount {
                        account_key: key.clone(),
                        signer: false,
                        writable: false,
                    })
                })
                .collect();
            let upstream: BTreeMap<String, String> = create_accounts_map(&addresses, instruction)
                .expect("enough accounts")
                .into_iter()
                .collect();
            let ours = build_named_accounts(&instruction.name, &keys);
            assert_eq!(ours.named, upstream, "{}", instruction.name);
            assert!(ours.remaining.is_empty(), "{}", instruction.name);
        }
    }

    #[test]
    fn test_every_instruction_has_flattened_names() {
        let idl = load_marinade_finance_idl().expect("IDL");
        for instruction in &idl.instructions {
            let names = flattened_account_names()
                .get(&instruction.name)
                .unwrap_or_else(|| panic!("no names for {}", instruction.name));
            assert!(names.len() >= instruction.accounts.len());
        }
    }

    #[test]
    fn test_accounts_past_the_idl_list_are_kept_in_order() {
        let declared = flattened_account_names()["deposit"].len();
        let accounts = keys(declared + 2);
        let result = build_named_accounts("deposit", &accounts);
        assert_eq!(result.named.len(), declared);
        assert_eq!(result.remaining, accounts[declared..].to_vec());
    }

    #[test]
    fn test_marinade_finance_idl_loads() {
        let idl = load_marinade_finance_idl().expect("IDL should load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL should declare at least one instruction"
        );
    }

    #[test]
    fn test_marinade_finance_idl_has_discriminators() {
        let idl = load_marinade_finance_idl().expect("IDL should load");
        for instruction in &idl.instructions {
            let discriminator = instruction.discriminator.as_ref().unwrap_or_else(|| {
                panic!("instruction '{}' missing discriminator", instruction.name)
            });
            assert_eq!(
                discriminator.len(),
                8,
                "instruction '{}' has non-8-byte discriminator",
                instruction.name
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage = [0xFFu8; 9];
        let result = parse_marinade_finance_instruction(&garbage);
        assert!(
            result.is_err(),
            "garbage discriminator should not match any instruction"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let too_short = [0x01u8, 0x02, 0x03];
        let result = parse_marinade_finance_instruction(&too_short);
        assert!(
            result.is_err(),
            "data shorter than discriminator should return error"
        );
    }
}
