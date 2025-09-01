//! Stakepool program preset for Solana

mod config;

use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::StakepoolConfig;
use spl_stake_pool::instruction::StakePoolInstruction;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_text_field;
use visualsign::{AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon};

// Create a static instance that we can reference
static STAKEPOOL_CONFIG: StakepoolConfig = StakepoolConfig;

pub struct StakepoolVisualizer;

impl InstructionVisualizer for StakepoolVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let instruction = context
            .current_instruction()
            .ok_or_else(|| VisualSignError::MissingData("No instruction found".into()))?;

        // Try to parse as stakepool instruction
        let stakepool_instruction = parse_stake_pool_instruction(&instruction.data)?;

        // Generate proper preview layout
        create_stakepool_preview_layout(&stakepool_instruction, instruction, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&STAKEPOOL_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::StakingPools("StakePool")
    }
}

fn create_stakepool_preview_layout(
    instruction: &StakePoolInstruction,
    solana_instruction: &solana_sdk::instruction::Instruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let instruction_name = format_stake_pool_instruction(instruction);

    let condensed_fields = vec![create_text_field("Instruction", &instruction_name)?];

    let expanded_fields = vec![create_text_field(
        "Stake Pool Instruction",
        &instruction_name,
    )?];

    let condensed = visualsign::SignablePayloadFieldListLayout {
        fields: condensed_fields,
    };
    let expanded = visualsign::SignablePayloadFieldListLayout {
        fields: expanded_fields,
    };

    let preview_layout = visualsign::SignablePayloadFieldPreviewLayout {
        title: Some(visualsign::SignablePayloadFieldTextV2 {
            text: instruction_name.clone(),
        }),
        subtitle: Some(visualsign::SignablePayloadFieldTextV2 {
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
                    solana_instruction.program_id,
                    hex::encode(&solana_instruction.data)
                ),
            },
            preview_layout,
        },
    })
}

fn parse_stake_pool_instruction(data: &[u8]) -> Result<StakePoolInstruction, VisualSignError> {
    use borsh::de::BorshDeserialize;

    StakePoolInstruction::try_from_slice(data).map_err(|e| {
        VisualSignError::DecodeError(format!("Failed to parse stakepool instruction: {}", e))
    })
}

fn format_stake_pool_instruction(instruction: &StakePoolInstruction) -> String {
    format!(
        "Stake Pool Instruction: {}",
        get_stake_pool_instruction_name(instruction)
    )
}

fn get_stake_pool_instruction_name(instruction: &StakePoolInstruction) -> &'static str {
    match instruction {
        StakePoolInstruction::Initialize { .. } => "Initialize",
        StakePoolInstruction::AddValidatorToPool(_) => "Add Validator to Pool",
        StakePoolInstruction::RemoveValidatorFromPool => "Remove Validator from Pool",
        StakePoolInstruction::DecreaseValidatorStake { .. } => "Decrease Validator Stake",
        StakePoolInstruction::IncreaseValidatorStake { .. } => "Increase Validator Stake",
        StakePoolInstruction::SetPreferredValidator { .. } => "Set Preferred Validator",
        StakePoolInstruction::UpdateValidatorListBalance { .. } => "Update Validator List Balance",
        StakePoolInstruction::UpdateStakePoolBalance => "Update Stake Pool Balance",
        StakePoolInstruction::CleanupRemovedValidatorEntries => "Cleanup Removed Validator Entries",
        StakePoolInstruction::DepositStake => "Deposit Stake",
        StakePoolInstruction::WithdrawStake(_) => "Withdraw Stake",
        StakePoolInstruction::SetManager => "Set Manager",
        StakePoolInstruction::SetFee { .. } => "Set Fee",
        StakePoolInstruction::SetStaker => "Set Staker",
        StakePoolInstruction::DepositSol(_) => "Deposit SOL",
        StakePoolInstruction::SetFundingAuthority(_) => "Set Funding Authority",
        StakePoolInstruction::WithdrawSol(_) => "Withdraw SOL",
        StakePoolInstruction::IncreaseAdditionalValidatorStake { .. } => {
            "Increase Additional Validator Stake"
        }
        StakePoolInstruction::DecreaseAdditionalValidatorStake { .. } => {
            "Decrease Additional Validator Stake"
        }
        StakePoolInstruction::DecreaseValidatorStakeWithReserve { .. } => {
            "Decrease Validator Stake with Reserve"
        }
        StakePoolInstruction::CreateTokenMetadata { .. } => "Create Token Metadata",
        StakePoolInstruction::UpdateTokenMetadata { .. } => "Update Token Metadata",
        StakePoolInstruction::DepositStakeWithSlippage { .. } => "Deposit Stake with Slippage",
        StakePoolInstruction::WithdrawStakeWithSlippage { .. } => "Withdraw Stake with Slippage",
        StakePoolInstruction::DepositSolWithSlippage { .. } => "Deposit SOL with Slippage",
        StakePoolInstruction::WithdrawSolWithSlippage { .. } => "Withdraw SOL with Slippage",
        #[allow(deprecated)]
        StakePoolInstruction::Redelegate { .. } => "Redelegate",
    }
}
