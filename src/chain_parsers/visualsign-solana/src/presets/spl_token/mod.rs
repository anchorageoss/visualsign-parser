//! SPL Token preset implementation for Solana
//! Handles the Token Program (TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)

mod config;

use crate::core::{
    AccountRef, InstructionVisualizer, ProgramRef, SolanaIntegrationConfig, VisualizerContext,
    VisualizerKind,
};
use config::SplTokenConfig;
use solana_program::program_option::COption;
use spl_token::instruction::{AuthorityType, TokenInstruction};
use visualsign::errors::VisualSignError;
use visualsign::field_builders::*;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

// Create a static instance that we can reference
static SPL_TOKEN_CONFIG: SplTokenConfig = SplTokenConfig;

pub struct SplTokenVisualizer;

impl InstructionVisualizer for SplTokenVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = InstructionView::from_context(context);
        // Catch-all "partial rendering" contract (see `VisualizerContext` docs):
        // never abort the whole transaction. Malformed instruction bytes
        // (including attacker-controlled input) degrade to a raw program_id +
        // data layout instead of returning `Err`, which the non-diagnostics
        // dispatch turns into a whole-transaction failure.
        match TokenInstruction::unpack(&instruction.data) {
            Ok(token_instruction) => {
                create_token_preview_layout(&token_instruction, &instruction, context)
            }
            Err(_) => create_unparsable_preview_layout(&instruction, context),
        }
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&SPL_TOKEN_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("SplToken")
    }
}

/// A display-resolved view of the instruction built from the post-#228
/// wire-data context.
///
/// The per-variant branches reference `instruction.program_id` /
/// `instruction.accounts` / `instruction.data`; this builds that view at the
/// entry point with every program and account index already resolved to a
/// display string. Following the catch-all "partial rendering" contract
/// documented on `VisualizerContext`, indices that need an address-table lookup
/// (v0 transactions) render as `unresolved(N)` placeholders rather than aborting
/// the whole transaction or being substituted with `Pubkey::default()` (which
/// would render as a valid-looking address).
struct InstructionView {
    program_id: String,
    accounts: Vec<String>,
    data: Vec<u8>,
}

impl InstructionView {
    fn from_context(context: &VisualizerContext) -> Self {
        let program_id = match context.program_id() {
            ProgramRef::Resolved(pk) => pk.to_string(),
            ProgramRef::Unresolved { raw_index } => format!("unresolved({raw_index})"),
        };
        let accounts = (0..context.num_accounts())
            .map(|i| match context.account(i) {
                Some(AccountRef::Resolved(pk)) => pk.to_string(),
                Some(AccountRef::Unresolved { raw_index }) => format!("unresolved({raw_index})"),
                None => "unknown".to_string(),
            })
            .collect();
        Self {
            program_id,
            accounts,
            data: context.data().to_vec(),
        }
    }
}

/// Graceful fallback when the instruction bytes do not unpack into a known SPL
/// Token instruction. Mirrors the unknown-program raw layout: surface the
/// program and raw data rather than erroring (which would erase the whole tx).
fn create_unparsable_preview_layout(
    instruction: &InstructionView,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let title = "SPL Token (unparsed)";

    let condensed_fields = vec![
        create_text_field("Instruction", title)?,
        create_text_field("Program", "SPL Token")?,
    ];

    let expanded_fields = vec![
        create_text_field("Instruction", title)?,
        create_text_field("Program", "SPL Token")?,
        create_text_field("Program ID", &instruction.program_id)?,
        create_text_field("Raw Data", &hex::encode(&instruction.data))?,
    ];

    create_preview_layout_field(
        title,
        condensed_fields,
        expanded_fields,
        instruction,
        context,
    )
}

fn format_authority_type(authority_type: &AuthorityType) -> &'static str {
    match authority_type {
        AuthorityType::MintTokens => "Mint Tokens",
        AuthorityType::FreezeAccount => "Freeze Account",
        AuthorityType::AccountOwner => "Account Owner",
        AuthorityType::CloseAccount => "Close Account",
    }
}

fn format_coption_pubkey(coption: &COption<solana_sdk::pubkey::Pubkey>) -> String {
    match coption {
        COption::Some(pubkey) => pubkey.to_string(),
        COption::None => "None".to_string(),
    }
}

fn create_token_preview_layout(
    token_instruction: &TokenInstruction,
    instruction: &InstructionView,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    match token_instruction {
        TokenInstruction::MintTo { amount } => {
            let instruction_name = format!("Mint To: {amount}");

            let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", "Mint To")?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // MintTo accounts: [0] mint, [1] destination account, [2] mint authority
            if let Some(mint_account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("mint", mint_account)?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("account", destination)?);
            }
            if let Some(authority) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("mintAuthority", authority)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::MintToChecked { amount, decimals } => {
            let instruction_name = format!("Mint To: {amount} (decimals: {decimals})");

            let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", "Mint To (Checked)")?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // MintToChecked accounts: [0] mint, [1] destination account, [2] mint authority
            if let Some(mint_account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("mint", mint_account)?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("account", destination)?);
            }
            if let Some(authority) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("mintAuthority", authority)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::SetAuthority {
            authority_type,
            new_authority,
        } => {
            let authority_type_str = format_authority_type(authority_type);
            let new_authority_str = format_coption_pubkey(new_authority);
            let instruction_name = format!("Set Authority: {authority_type_str}");

            let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", "Set Authority")?,
                create_text_field("Authority Type", authority_type_str)?,
                create_text_field("New Authority", &new_authority_str)?,
            ];

            // SetAuthority accounts: [0] account whose authority is being set, [1] current authority
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(current_authority) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Current Authority", current_authority)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Transfer { amount } => {
            let instruction_name = "Transfer";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // Transfer accounts: [0] source account, [1] destination account, [2] owner.
            // The mint is not in the instruction — it lives inside the source/destination
            // token accounts on-chain. Signal the absence via tracing rather than a visible
            // field so wallet UIs stay clean; observers can still flag the deprecated
            // unchecked variant.
            tracing::debug!(
                "spl_token: unchecked Transfer omits mint from instruction; use TransferChecked to verify"
            );
            if let Some(source) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Source", source)?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Destination", destination)?);
            }
            if let Some(owner) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::TransferChecked { amount, decimals } => {
            let instruction_name = "Transfer (Checked)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // TransferChecked accounts: [0] source account, [1] mint, [2] destination account, [3] owner
            if let Some(source) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Source", source)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(destination) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Destination", destination)?);
            }
            if let Some(owner) = instruction.accounts.get(3) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Burn { amount } => {
            let instruction_name = "Burn";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // Burn accounts: [0] token account to burn from, [1] mint, [2] owner
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(owner) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::BurnChecked { amount, decimals } => {
            let instruction_name = "Burn (Checked)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // BurnChecked accounts: [0] token account to burn from, [1] mint, [2] owner
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(owner) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Approve { amount } => {
            let instruction_name = "Approve";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
            ];

            // Approve accounts: [0] source account, [1] delegate, [2] owner.
            // Mint is not in the instruction — signal via tracing, not a visible field.
            tracing::debug!(
                "spl_token: unchecked Approve omits mint from instruction; use ApproveChecked to verify"
            );
            if let Some(source) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Source", source)?);
            }
            if let Some(delegate) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Delegate", delegate)?);
            }
            if let Some(owner) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::ApproveChecked { amount, decimals } => {
            let instruction_name = "Approve (Checked)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Amount", &amount.to_string())?,
                create_text_field("Decimals", &decimals.to_string())?,
            ];

            // ApproveChecked accounts: [0] source account, [1] mint, [2] delegate, [3] owner
            if let Some(source) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Source", source)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(delegate) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Delegate", delegate)?);
            }
            if let Some(owner) = instruction.accounts.get(3) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            let expanded_fields = expanded_fields;

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::InitializeMint {
            decimals,
            mint_authority,
            freeze_authority,
        } => {
            let instruction_name = "Initialize Mint";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Decimals", &decimals.to_string())?,
                create_text_field("Mint Authority", &mint_authority.to_string())?,
                create_text_field("Freeze Authority", &format_coption_pubkey(freeze_authority))?,
            ];

            // InitializeMint accounts: [0] mint, [1] rent sysvar
            if let Some(mint) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::InitializeMint2 {
            decimals,
            mint_authority,
            freeze_authority,
        } => {
            let instruction_name = "Initialize Mint (v2)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Decimals", &decimals.to_string())?,
                create_text_field("Mint Authority", &mint_authority.to_string())?,
                create_text_field("Freeze Authority", &format_coption_pubkey(freeze_authority))?,
            ];

            // InitializeMint2 accounts: [0] mint
            if let Some(mint) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::InitializeAccount => {
            let instruction_name = "Initialize Token Account";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
            ];

            // InitializeAccount accounts: [0] account, [1] mint, [2] owner, [3] rent
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(owner) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::InitializeAccount2 { owner } => {
            let instruction_name = "Initialize Token Account (v2)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Owner", &owner.to_string())?,
            ];

            // InitializeAccount2 accounts: [0] account, [1] mint, [2] rent (owner is in
            // instruction data, not the account list)
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::InitializeAccount3 { owner } => {
            let instruction_name = "Initialize Token Account (v3)";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
                create_text_field("Owner", &owner.to_string())?,
            ];

            // InitializeAccount3 accounts: [0] account, [1] mint (owner is in instruction data)
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::FreezeAccount => {
            let instruction_name = "Freeze Account";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
            ];

            // FreezeAccount accounts: [0] account to freeze, [1] mint, [2] freeze authority
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(authority) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Freeze Authority", authority)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::ThawAccount => {
            let instruction_name = "Thaw Account";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
            ];

            // ThawAccount accounts: [0] account to thaw, [1] mint, [2] freeze authority
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(mint) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Token Mint", mint)?);
            }
            if let Some(authority) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Freeze Authority", authority)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::CloseAccount => {
            let instruction_name = "Close Account";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
            ];

            // CloseAccount accounts: [0] account to close, [1] lamport destination, [2] owner.
            // The mint is not in the instruction; it lives inside the closed token account.
            tracing::debug!(
                "spl_token: CloseAccount omits mint from instruction (derived from token account state)"
            );
            if let Some(account) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Account", account)?);
            }
            if let Some(destination) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Destination", destination)?);
            }
            if let Some(owner) = instruction.accounts.get(2) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        TokenInstruction::Revoke => {
            let instruction_name = "Revoke";

            let condensed_fields = vec![create_text_field("Instruction", instruction_name)?];

            let mut expanded_fields = vec![
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Instruction", instruction_name)?,
            ];

            // Revoke accounts: [0] source, [1] owner. The mint is not in the instruction.
            tracing::debug!(
                "spl_token: Revoke omits mint from instruction (derived from source token account state)"
            );
            if let Some(source) = instruction.accounts.first() {
                expanded_fields.push(create_text_field("Source", source)?);
            }
            if let Some(owner) = instruction.accounts.get(1) {
                expanded_fields.push(create_text_field("Owner", owner)?);
            }

            expanded_fields.push(create_text_field(
                "Raw Data",
                &hex::encode(&instruction.data),
            )?);

            create_preview_layout_field(
                instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
        _ => {
            // Fallback for the remaining instructions (InitializeMultisig{,2},
            // InitializeImmutableOwner, SyncNative, GetAccountDataSize, AmountToUiAmount,
            // UiAmountToAmount) — these are either rare in user-facing flows or do not
            // reference a mint/token account in a way worth surfacing as named fields.
            let instruction_name = format_token_instruction(token_instruction);

            let condensed_fields = vec![
                create_text_field("Instruction", &instruction_name)?,
                create_text_field("Program", "SPL Token")?,
            ];

            let expanded_fields = vec![
                create_text_field("Instruction", &instruction_name)?,
                create_text_field("Program", "SPL Token")?,
                create_text_field("Program ID", &instruction.program_id)?,
                create_text_field("Raw Data", &hex::encode(&instruction.data))?,
            ];

            create_preview_layout_field(
                &instruction_name,
                condensed_fields,
                expanded_fields,
                instruction,
                context,
            )
        }
    }
}

fn create_preview_layout_field(
    title: &str,
    condensed_fields: Vec<AnnotatedPayloadField>,
    expanded_fields: Vec<AnnotatedPayloadField>,
    instruction: &InstructionView,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let condensed = SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = SignablePayloadFieldPreviewLayout {
        title: Some(SignablePayloadFieldTextV2 {
            text: title.to_string(),
        }),
        subtitle: Some(SignablePayloadFieldTextV2 {
            text: String::new(),
        }),
        condensed: Some(condensed),
        expanded: Some(expanded),
    };

    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: format!("Instruction {}", context.instruction_index() + 1),
                fallback_text: format!(
                    "Program ID: {}\nData: {}",
                    instruction.program_id,
                    hex::encode(&instruction.data)
                ),
            },
            preview_layout,
        },
    })
}

fn format_token_instruction(instruction: &TokenInstruction) -> String {
    match instruction {
        TokenInstruction::InitializeMint { .. } => "Initialize Mint".to_string(),
        TokenInstruction::InitializeMint2 { .. } => "Initialize Mint (v2)".to_string(),
        TokenInstruction::InitializeAccount => "Initialize Token Account".to_string(),
        TokenInstruction::InitializeAccount2 { .. } => "Initialize Token Account (v2)".to_string(),
        TokenInstruction::InitializeAccount3 { .. } => "Initialize Token Account (v3)".to_string(),
        TokenInstruction::InitializeMultisig { .. } => "Initialize Multisig".to_string(),
        TokenInstruction::InitializeMultisig2 { .. } => "Initialize Multisig (v2)".to_string(),
        TokenInstruction::Transfer { .. } => "Transfer".to_string(),
        TokenInstruction::TransferChecked { .. } => "Transfer (Checked)".to_string(),
        TokenInstruction::Approve { .. } => "Approve".to_string(),
        TokenInstruction::ApproveChecked { .. } => "Approve (Checked)".to_string(),
        TokenInstruction::Revoke => "Revoke".to_string(),
        TokenInstruction::SetAuthority { .. } => "Set Authority".to_string(),
        // Note: MintTo and MintToChecked are handled specially in create_token_preview_layout
        // and never reach this function, so they are intentionally omitted here
        TokenInstruction::Burn { .. } => "Burn".to_string(),
        TokenInstruction::BurnChecked { .. } => "Burn (Checked)".to_string(),
        TokenInstruction::CloseAccount => "Close Account".to_string(),
        TokenInstruction::FreezeAccount => "Freeze Account".to_string(),
        TokenInstruction::ThawAccount => "Thaw Account".to_string(),
        TokenInstruction::SyncNative => "Sync Native".to_string(),
        TokenInstruction::GetAccountDataSize { .. } => "Get Account Data Size".to_string(),
        TokenInstruction::InitializeImmutableOwner => "Initialize Immutable Owner".to_string(),
        TokenInstruction::AmountToUiAmount { .. } => "Amount To UI Amount".to_string(),
        TokenInstruction::UiAmountToAmount { .. } => "UI Amount To Amount".to_string(),
        // These cases are handled specially above and should never reach here
        TokenInstruction::MintTo { .. } | TokenInstruction::MintToChecked { .. } => {
            unreachable!("MintTo instructions are handled specially in create_token_preview_layout")
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
