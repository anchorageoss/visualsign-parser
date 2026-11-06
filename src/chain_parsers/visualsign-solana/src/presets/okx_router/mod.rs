//! OKX Router preset implementation for Solana

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::OkxRouterConfig;
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

pub(crate) const OKX_ROUTER_PROGRAM_ID: &str = "proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u";

const DISPLAY_NAME: &str = "OKX Router";

const IDL_JSON: &str = include_str!("okx_router.json");

static CONFIG: OkxRouterConfig = OkxRouterConfig;

pub struct OkxRouterVisualizer;

impl InstructionVisualizer for OkxRouterVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let view = InstructionView::from_context(context);
        let data = context.data();

        if data.len() < 8 {
            return Err(VisualSignError::DecodeError(
                "Instruction data too short for Anchor discriminator".to_string(),
            ));
        }

        let idl = load_idl()?;
        let parsed = parse_instruction_with_idl(data, OKX_ROUTER_PROGRAM_ID, &idl)
            .map_err(|e| VisualSignError::DecodeError(format!("IDL parse failed: {e}")))?;

        let accounts = build_named_accounts(&idl, data, &view.accounts);

        build_visualization(context, &view, data, &parsed, &accounts)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Dex(DISPLAY_NAME)
    }
}

fn load_idl() -> Result<Idl, VisualSignError> {
    decode_idl_data(IDL_JSON)
        .map_err(|e| VisualSignError::DecodeError(format!("Failed to decode bundled IDL: {e}")))
}

/// The instruction's accounts, named from the IDL by position.
///
/// Anchor reads accounts by position too: an omitted optional account is
/// passed as the program's own ID, so naming by position shows exactly what
/// the program will use. Such placeholders are labelled "Not provided" next
/// to the address. Accounts past the IDL's list (the pools and vaults a route
/// passes through) are kept, in order, as remaining accounts, so no signed
/// account is left out of the display.
struct InstructionAccounts {
    named: BTreeMap<String, String>,
    remaining: Vec<String>,
}

fn build_named_accounts(idl: &Idl, instruction_data: &[u8], accounts: &[String]) -> InstructionAccounts {
    let mut result = InstructionAccounts {
        named: BTreeMap::new(),
        remaining: Vec::new(),
    };
    let Some(discriminator) = instruction_data.get(..8) else {
        result.remaining = accounts.to_vec();
        return result;
    };
    let Some(idl_instruction) = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| discriminator == disc.as_slice())
    }) else {
        result.remaining = accounts.to_vec();
        return result;
    };
    for (index, account) in accounts.iter().enumerate() {
        match idl_instruction.accounts.get(index) {
            Some(idl_account) => {
                let shown = if idl_account.is_optional && account == OKX_ROUTER_PROGRAM_ID {
                    format!("Not provided ({account})")
                } else {
                    account.clone()
                };
                result.named.insert(idl_account.name.clone(), shown);
            }
            None => result.remaining.push(account.clone()),
        }
    }
    result
}

fn format_arg_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

fn build_visualization(
    context: &VisualizerContext,
    view: &InstructionView,
    data: &[u8],
    parsed: &SolanaParsedInstructionData,
    accounts: &InstructionAccounts,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id = view.program_id.clone();
    let title = format!("{DISPLAY_NAME}: {}", parsed.instruction_name);

    let condensed_fields = vec![create_text_field("Instruction", &parsed.instruction_name)?];

    let mut expanded_fields = vec![
        create_text_field("Program", DISPLAY_NAME)?,
        create_text_field("Program ID", &program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    for (name, address) in &accounts.named {
        expanded_fields.push(create_text_field(name, address)?);
    }
    for (index, address) in accounts.remaining.iter().enumerate() {
        expanded_fields.push(create_text_field(
            &format!("Remaining Account {}", index + 1),
            address,
        )?);
    }

    for (key, value) in &parsed.program_call_args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    expanded_fields.push(create_raw_data_field(data, Some(hex::encode(data)))?);

    let condensed = SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title.clone(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: String::new(),
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    let fallback_text = format!("Program ID: {program_id}\nData: {}", hex::encode(data));

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_parser::solana::structs::IdlInstruction;

    /// `swap_tob_v3` names 14 accounts; slots 5-11 are optional.
    fn swap_tob_v3(idl: &Idl) -> (Vec<u8>, &IdlInstruction) {
        let instruction = idl
            .instructions
            .iter()
            .find(|inst| inst.name == "swap_tob_v3")
            .expect("swap_tob_v3 is in the IDL");
        let mut data = instruction.discriminator.clone().expect("discriminator");
        data.extend_from_slice(&[0u8; 4]);
        (data, instruction)
    }

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|_| solana_pubkey::Pubkey::new_unique().to_string()).collect()
    }

    #[test]
    fn test_accounts_past_the_idl_list_are_kept_in_order() {
        let idl = load_idl().expect("IDL must load");
        let (data, instruction) = swap_tob_v3(&idl);
        let declared = instruction.accounts.len();
        let accounts = keys(declared + 3);

        let result = build_named_accounts(&idl, &data, &accounts);

        assert_eq!(result.named.len(), declared);
        assert_eq!(result.remaining, accounts[declared..].to_vec());
    }

    #[test]
    fn test_unused_optional_account_reads_not_provided() {
        let idl = load_idl().expect("IDL must load");
        let (data, instruction) = swap_tob_v3(&idl);
        let optional_index = instruction
            .accounts
            .iter()
            .position(|account| account.is_optional)
            .expect("swap_tob_v3 has optional accounts");
        let required_index = instruction
            .accounts
            .iter()
            .position(|account| !account.is_optional)
            .expect("swap_tob_v3 has required accounts");
        let mut accounts = keys(instruction.accounts.len());
        accounts[optional_index] = OKX_ROUTER_PROGRAM_ID.to_string();
        // The program ID in a required slot is just an address, not a placeholder.
        accounts[required_index] = OKX_ROUTER_PROGRAM_ID.to_string();

        let result = build_named_accounts(&idl, &data, &accounts);

        assert_eq!(
            result.named[&instruction.accounts[optional_index].name],
            format!("Not provided ({OKX_ROUTER_PROGRAM_ID})")
        );
        assert_eq!(
            result.named[&instruction.accounts[required_index].name],
            OKX_ROUTER_PROGRAM_ID
        );
    }

    #[test]
    fn test_unknown_or_short_data_keeps_every_account() {
        let idl = load_idl().expect("IDL must load");
        let accounts = keys(4);
        for data in [&[0xFFu8; 12][..], &[1, 2, 3][..]] {
            let result = build_named_accounts(&idl, data, &accounts);
            assert!(result.named.is_empty());
            assert_eq!(result.remaining, accounts);
        }
    }

    #[test]
    fn test_okx_router_idl_loads() {
        let idl = load_idl().expect("IDL must load");
        assert!(
            !idl.instructions.is_empty(),
            "IDL should declare at least one instruction"
        );
    }

    #[test]
    fn test_okx_router_idl_has_discriminators() {
        let idl = load_idl().expect("IDL must load");
        for instruction in &idl.instructions {
            let discriminator = instruction
                .discriminator
                .as_ref()
                .expect("decode_idl_data must populate every discriminator");
            assert_eq!(
                discriminator.len(),
                8,
                "discriminator for `{}` should be 8 bytes, got {}",
                instruction.name,
                discriminator.len()
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let idl = load_idl().expect("IDL must load");
        let bogus = [0xFFu8; 9];
        let result = parse_instruction_with_idl(&bogus, OKX_ROUTER_PROGRAM_ID, &idl);
        assert!(
            result.is_err(),
            "unknown discriminator should fail to parse"
        );
    }

    #[test]
    fn test_short_data_returns_error() {
        let idl = load_idl().expect("IDL must load");
        let short = [0x01u8, 0x02, 0x03];
        let result = parse_instruction_with_idl(&short, OKX_ROUTER_PROGRAM_ID, &idl);
        assert!(result.is_err(), "data shorter than 8 bytes should error");
    }
}
