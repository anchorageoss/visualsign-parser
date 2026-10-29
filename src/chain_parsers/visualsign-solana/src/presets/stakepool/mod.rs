//! Stakepool program preset for Solana

mod config;

#[cfg(feature = "stake-pool")]
use crate::core::resolve_program_display;
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, VisualizerContext, VisualizerKind,
};
use config::StakepoolConfig;
#[cfg(feature = "stake-pool")]
use spl_stake_pool::instruction::StakePoolInstruction;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;
#[cfg(feature = "stake-pool")]
use visualsign::field_builders::create_text_field;
#[cfg(feature = "stake-pool")]
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon};

// Create a static instance that we can reference
static STAKEPOOL_CONFIG: StakepoolConfig = StakepoolConfig;

pub struct StakepoolVisualizer;

impl InstructionVisualizer for StakepoolVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        #[cfg(feature = "stake-pool")]
        {
            let stakepool_instruction = parse_stake_pool_instruction(context.data())?;
            create_stakepool_preview_layout(&stakepool_instruction, context)
        }
        #[cfg(not(feature = "stake-pool"))]
        {
            let _ = context;
            Err(VisualSignError::DecodeError(
                "stake pool decoding needs the `stake-pool` feature".to_string(),
            ))
        }
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        // Without `stake-pool` the instruction type is not compiled in: decline,
        // so the unknown-program visualizer shows the instruction instead.
        if cfg!(feature = "stake-pool") {
            Some(&STAKEPOOL_CONFIG)
        } else {
            None
        }
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::StakingPools("StakePool")
    }
}

#[cfg(feature = "stake-pool")]
fn create_stakepool_preview_layout(
    instruction: &StakePoolInstruction,
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let program_id_str = resolve_program_display(context);
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
                    program_id_str,
                    hex::encode(context.data())
                ),
            },
            preview_layout,
        },
    })
}

#[cfg(feature = "stake-pool")]
fn parse_stake_pool_instruction(data: &[u8]) -> Result<StakePoolInstruction, VisualSignError> {
    use borsh::de::BorshDeserialize;

    StakePoolInstruction::try_from_slice(data).map_err(|e| {
        VisualSignError::DecodeError(format!("Failed to parse stakepool instruction: {e}"))
    })
}

#[cfg(feature = "stake-pool")]
fn format_stake_pool_instruction(instruction: &StakePoolInstruction) -> String {
    format!(
        "Stake Pool Instruction: {}",
        get_stake_pool_instruction_name(instruction)
    )
}

#[cfg(feature = "stake-pool")]
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
