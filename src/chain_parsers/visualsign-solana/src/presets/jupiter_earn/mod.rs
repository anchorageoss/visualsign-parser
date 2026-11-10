//! Jupiter Lend Earn preset: a tokenized vault rendered by `core::receipt_token`.
//! `deposit`/`withdraw`/`mint`/`redeem` and their bounded variants get the semantic view;
//! everything else keeps the generic IDL rendering.

mod config;

use crate::core::receipt_token::{self, Action, Flow, ReceiptTokenProgram, RecipientRule};
use crate::core::{
    InstructionVisualizer, SolanaIntegrationConfig, TransactionSummary, VisualizerContext,
    VisualizerKind,
};
use config::JupiterEarnConfig;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;

pub(crate) const JUPITER_EARN_PROGRAM_ID: &str = "jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9";

const JUPITER_EARN_DISPLAY_NAME: &str = "Jupiter Lend Earn";

/// `withdraw(u64::MAX)` burns the whole fToken balance, verified on mainnet for plain `withdraw`
/// only (<https://solscan.io/tx/296YJsTYWiNgJ5b5LENoL6prJeASneukmHofteMdWTvv8DhLNEY19uAEvZYjxQmAWJuCemP4wtzgxtvDXxSS8sXs>).
const WITHDRAW_ALL_TITLE: &str = "Redeem full {receipt} position via {program}";

const DEPOSIT_TITLE: &str = "Supply {asset} to {program}";
const MINT_TITLE: &str = "Mint {receipt} on {program}";
const WITHDRAW_TITLE: &str = "Redeem {receipt} for {asset} via {program}";
const REDEEM_TITLE: &str = "Redeem {receipt} via {program}";

const RECIPIENT: RecipientRule = RecipientRule::SignerTokenAccount("recipient_token_account");

/// Product name used in titles; the subtitle and Program rows keep the on-chain program name.
pub(crate) static JUPITER_EARN: ReceiptTokenProgram = ReceiptTokenProgram {
    program_id: JUPITER_EARN_PROGRAM_ID,
    display_name: JUPITER_EARN_DISPLAY_NAME,
    title_name: "Jupiter Lend",
    signer: "signer",
    asset_mint: "mint",
    receipt_mint: "f_token_mint",
    token_program: "token_program",
    estimated_legs: true,
    actions: &[
        Action::new(
            "deposit",
            Flow::Deposit,
            "Deposit",
            DEPOSIT_TITLE,
            "assets",
            RECIPIENT,
        ),
        Action::new(
            "deposit_with_min_amount_out",
            Flow::Deposit,
            "Deposit",
            DEPOSIT_TITLE,
            "assets",
            RECIPIENT,
        )
        .with_bound("min_amount_out"),
        Action::new("mint", Flow::Mint, "Mint", MINT_TITLE, "shares", RECIPIENT),
        Action::new(
            "mint_with_max_assets",
            Flow::Mint,
            "Mint",
            MINT_TITLE,
            "shares",
            RECIPIENT,
        )
        .with_bound("max_assets"),
        Action::new(
            "withdraw",
            Flow::Withdraw,
            "Withdraw",
            WITHDRAW_TITLE,
            "amount",
            RECIPIENT,
        )
        .with_full_position_title(WITHDRAW_ALL_TITLE),
        Action::new(
            "withdraw_with_max_shares_burn",
            Flow::Withdraw,
            "Withdraw",
            WITHDRAW_TITLE,
            "amount",
            RECIPIENT,
        )
        .with_bound("max_shares_burn"),
        Action::new(
            "redeem",
            Flow::Redeem,
            "Redeem",
            REDEEM_TITLE,
            "shares",
            RECIPIENT,
        ),
        Action::new(
            "redeem_with_min_amount_out",
            Flow::Redeem,
            "Redeem",
            REDEEM_TITLE,
            "shares",
            RECIPIENT,
        )
        .with_bound("min_amount_out"),
    ],
};

static JUPITER_EARN_CONFIG: JupiterEarnConfig = JupiterEarnConfig;

pub struct JupiterEarnVisualizer;

impl InstructionVisualizer for JupiterEarnVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        receipt_token::visualize(std::slice::from_ref(&JUPITER_EARN), context)
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&JUPITER_EARN_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(JUPITER_EARN_DISPLAY_NAME)
    }

    fn transaction_summary(&self, context: &VisualizerContext) -> Option<TransactionSummary> {
        receipt_token::transaction_summary(std::slice::from_ref(&JUPITER_EARN), context)
    }

    fn visualize_with_summary(
        &self,
        context: &VisualizerContext,
    ) -> Result<(AnnotatedPayloadField, Option<TransactionSummary>), VisualSignError> {
        receipt_token::visualize_with_summary(std::slice::from_ref(&JUPITER_EARN), context)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::core::receipt_token::UserAction;
    use crate::core::receipt_token::test_support::assert_profile_matches_idl;
    mod fixture_test;
    mod summary_test;

    /// A `*_with_*` variant with an undecodable bound must not render like the unbounded
    /// instruction: it is a parse failure and keeps the generic view.
    #[test]
    fn test_missing_bound_on_with_variant_is_a_parse_failure() {
        let instruction = fixture_test::synthetic_instruction(
            "deposit_with_min_amount_out",
            &[10_000_000, 9_000_000],
            &[],
        );
        let accounts: Vec<String> = instruction
            .accounts
            .iter()
            .map(|a| a.pubkey.to_string())
            .collect();
        let mut parsed = JUPITER_EARN.parse(&instruction.data, &accounts).unwrap();
        assert!(UserAction::from_parsed(&JUPITER_EARN, &parsed).is_some());

        parsed.args.remove("min_amount_out");
        assert!(UserAction::from_parsed(&JUPITER_EARN, &parsed).is_none());
    }

    #[test]
    fn test_jupiter_earn_idl_loads() {
        let idl = JUPITER_EARN
            .idl()
            .expect("Jupiter Lend Earn IDL should load");
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_jupiter_earn_idl_has_discriminators() {
        let idl = JUPITER_EARN.idl().unwrap();
        for instruction in &idl.instructions {
            let disc = instruction.discriminator.as_ref().unwrap_or_else(|| {
                panic!("instruction {} missing discriminator", instruction.name)
            });
            assert_eq!(
                disc.len(),
                8,
                "instruction {} discriminator",
                instruction.name
            );
        }
    }

    #[test]
    fn test_jupiter_earn_profile_matches_its_idl() {
        assert_profile_matches_idl(&JUPITER_EARN);
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage_data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let result = JUPITER_EARN.parse(&garbage_data, &[]);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let result = JUPITER_EARN.parse(&short_data, &[]);
        assert!(result.is_err(), "Short data should return error");
    }
}
