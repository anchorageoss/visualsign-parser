//! Abra: USDC mints USDaf 1:1 (backed-token program); USDaf stakes into sUSDaf (liquid-staking
//! program). Each exit is a request, then a claim from a per-signer escrow after a cooldown.
//!
//! IDLs: the issuer's generic 0.1.0 builds with `address` set to the deployed program ID. Of the
//! fixtures, only mint, cancel_unmint, stake and request_unstake postdate the Nov 2026 upgrades.

mod config;

use crate::core::receipt_token::{
    self, Action, Denomination, Flow, ReceiptTokenProgram, RecipientRule, Seed,
};
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, TransactionSummary, VisualizerContext,
    VisualizerKind,
};
use config::AbraConfig;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;

pub(crate) const ABRA_BACKED_TOKEN_PROGRAM_ID: &str =
    "H9Cyw2RRHStwtKTAgGPm3nU48xwQ2Scy8u2gjg82dyQQ";
pub(crate) const ABRA_LIQUID_STAKING_PROGRAM_ID: &str =
    "9cYNMhVvGC2LhgJYZavKVWYZ8T1oqSHYt5ehTddP51yC";

const ABRA_DISPLAY_NAME: &str = "Abra";
pub(crate) const ABRA_BACKED_TOKEN_DISPLAY_NAME: &str = "Abra Backed Token";
pub(crate) const ABRA_LIQUID_STAKING_DISPLAY_NAME: &str = "Abra Liquid Staking";

const COLLECT_TITLE: &str = "Collect {asset} from {program}";

/// The per-signer request account that owns an exit's escrow, as the IDL derives it.
const UNMINT_REQUEST_SEEDS: &[Seed] = &[
    Seed::Literal(b"unmint_details"),
    Seed::Account("user"),
    Seed::Account("claim_token_mint"),
];
const UNSTAKE_REQUEST_SEEDS: &[Seed] = &[
    Seed::Literal(b"user_unstake_request"),
    Seed::Account("user"),
];

/// `mint` pays USDC into Abra's treasury and mints USDaf 1:1. The exit escrows USDaf in
/// the signer's request until `claim_unmint` burns it for USDC or `cancel_unmint` returns it.
const ABRA_BACKED_TOKEN: ReceiptTokenProgram = ReceiptTokenProgram {
    program_id: ABRA_BACKED_TOKEN_PROGRAM_ID,
    display_name: ABRA_BACKED_TOKEN_DISPLAY_NAME,
    title_name: ABRA_DISPLAY_NAME,
    signer: "user",
    asset_mint: "claim_token_mint",
    receipt_mint: "abrafi_backed_token_mint",
    token_program: "token_program",
    estimated_legs: false,
    actions: &[
        Action::new(
            "mint",
            Flow::Deposit,
            "Mint",
            "Supply {asset} to Mint {receipt}",
            "amount",
            RecipientRule::SignerTokenAccount("user_abrafi_backed_token_account"),
        )
        .with_asset_mint("collateral_token_mint")
        .with_account_rows(&[("Paid to", "treasury_token_account")]),
        Action::new(
            "request_unmint",
            Flow::RequestRedeem,
            "Request unmint",
            "Request Unmint {receipt}",
            "amount",
            RecipientRule::SignerEscrow {
                account: "escrow_token_account",
                request: "user_unmint_details",
                seeds: UNMINT_REQUEST_SEEDS,
                holds: Denomination::Receipt,
            },
        ),
        Action::new(
            "claim_unmint",
            Flow::Claim {
                exact: Denomination::Receipt,
            },
            "Collect",
            COLLECT_TITLE,
            "claim_amount",
            RecipientRule::SignerTokenAccount("user_claim_token_account"),
        )
        .with_account_rows(&[
            ("Burned from", "escrow_token_account"),
            ("Paid from", "vault_token_account"),
        ]),
        Action::new(
            "cancel_unmint",
            Flow::CancelRequest,
            "Cancel unmint",
            "Cancel Unmint {receipt}",
            "cancel_amount",
            RecipientRule::SignerTokenAccount("user_abrafi_backed_token_account"),
        )
        .with_account_rows(&[("Returned from", "escrow_token_account")]),
    ],
};

/// `stake` moves USDaf into the staking vault and mints sUSDaf at the current rate. The exit
/// burns sUSDaf and escrows USDaf in the signer's request until `claim_unstake`.
const ABRA_LIQUID_STAKING: ReceiptTokenProgram = ReceiptTokenProgram {
    program_id: ABRA_LIQUID_STAKING_PROGRAM_ID,
    display_name: ABRA_LIQUID_STAKING_DISPLAY_NAME,
    title_name: ABRA_DISPLAY_NAME,
    signer: "user",
    asset_mint: "underlying_token_mint",
    receipt_mint: "liquid_staking_token_mint",
    token_program: "token_program",
    estimated_legs: false,
    actions: &[
        Action::new(
            "stake",
            Flow::Deposit,
            "Stake",
            "Stake {asset} to Mint {receipt}",
            "underlying_amount",
            RecipientRule::SignerTokenAccount("user_liquid_staking_token_account"),
        )
        .with_account_rows(&[("Staked into", "vault_token_account")]),
        Action::new(
            "request_unstake",
            Flow::RequestRedeem,
            "Request unstake",
            "Request Unstake {receipt}",
            "liquid_staking_amount",
            RecipientRule::SignerEscrow {
                account: "escrow_token_account",
                request: "user_unstake_request",
                seeds: UNSTAKE_REQUEST_SEEDS,
                holds: Denomination::Asset,
            },
        ),
        Action::new(
            "claim_unstake",
            Flow::Claim {
                exact: Denomination::Asset,
            },
            "Collect",
            COLLECT_TITLE,
            "claim_underlying_amount",
            RecipientRule::SignerTokenAccount("user_underlying_token_account"),
        )
        .with_account_rows(&[("Collected from", "escrow_token_account")]),
    ],
};

pub(crate) static ABRA_PROGRAMS: [ReceiptTokenProgram; 2] =
    [ABRA_BACKED_TOKEN, ABRA_LIQUID_STAKING];

static ABRA_CONFIG: AbraConfig = AbraConfig;

pub struct AbraVisualizer;

impl InstructionVisualizer for AbraVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        receipt_token::visualize(&ABRA_PROGRAMS, context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&ABRA_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(ABRA_DISPLAY_NAME)
    }

    fn transaction_summary(&self, context: &VisualizerContext) -> Option<TransactionSummary> {
        receipt_token::transaction_summary(&ABRA_PROGRAMS, context)
    }

    fn visualize_with_summary(
        &self,
        context: &VisualizerContext,
    ) -> Result<(AnnotatedPayloadField, Option<TransactionSummary>), VisualSignError> {
        receipt_token::visualize_with_summary(&ABRA_PROGRAMS, context)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::core::receipt_token::test_support::assert_profile_matches_idl;
    mod fixture_test;
    mod summary_test;

    #[test]
    fn test_abra_idls_load_with_8_byte_discriminators() {
        for program in &ABRA_PROGRAMS {
            let idl = program
                .idl()
                .unwrap_or_else(|| panic!("{} IDL should load", program.display_name));
            assert!(!idl.instructions.is_empty(), "IDL should have instructions");
            for instruction in &idl.instructions {
                let disc = instruction.discriminator.as_ref().unwrap_or_else(|| {
                    panic!("instruction {} missing discriminator", instruction.name)
                });
                assert_eq!(disc.len(), 8, "instruction {}", instruction.name);
            }
        }
    }

    #[test]
    fn test_abra_profiles_match_their_idls() {
        for program in &ABRA_PROGRAMS {
            assert_profile_matches_idl(program);
        }
    }

    /// Only the seven allowlisted operations get the semantic view.
    #[test]
    fn test_abra_actions_are_the_seven_operations() {
        let names: Vec<&str> = ABRA_PROGRAMS
            .iter()
            .flat_map(|program| program.actions.iter().map(|action| action.instruction))
            .collect();
        assert_eq!(
            names,
            [
                "mint",
                "request_unmint",
                "claim_unmint",
                "cancel_unmint",
                "stake",
                "request_unstake",
                "claim_unstake",
            ]
        );
    }

    #[test]
    fn test_unknown_discriminator_and_short_data_return_errors() {
        let garbage_data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let short_data = [0x01, 0x02, 0x03];
        for program in &ABRA_PROGRAMS {
            assert!(program.parse(&garbage_data, &[]).is_err());
            assert!(program.parse(&short_data, &[]).is_err());
        }
    }
}
