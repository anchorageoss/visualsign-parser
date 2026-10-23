//! Priority fee committed by compute-budget instructions, as the runtime charges it:
//! `ceil(unit_price * unit_limit / 1_000_000)` lamports on the requested limit.

use crate::utils::format_token_amount;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::pubkey::Pubkey;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_amount_field;

/// Runtime ceiling: a requested limit is clamped to it, and it bounds the fee when no limit is
/// requested (the default is version-dependent).
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

const MICRO_LAMPORTS_PER_LAMPORT: u128 = 1_000_000;

/// Base fee the runtime charges per signature, in lamports; see [`charged_signatures`].
pub const BASE_FEE_LAMPORTS_PER_SIGNATURE: u64 = 5_000;

/// Not re-exported by `solana_sdk` yet; the id from `solana_sdk_ids::secp256r1_program`.
mod secp256r1_program {
    solana_sdk::declare_id!("Secp256r1SigVerify1111111111111111111111111");
}

/// The signature-verification precompiles, whose verifies the runtime charges as signatures.
fn is_signature_precompile(program_id: &Pubkey) -> bool {
    solana_sdk::ed25519_program::check_id(program_id)
        || solana_sdk::secp256k1_program::check_id(program_id)
        || secp256r1_program::check_id(program_id)
}

const SOL_DECIMALS: u8 = 9;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PriorityFee {
    unit_price: Option<u64>,
    unit_limit: Option<u32>,
    duplicate: bool,
}

/// The fee in lamports. `UpperBound` when no unit limit was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityFeeEstimate {
    Exact(u64),
    UpperBound(u64),
}

impl PriorityFee {
    pub fn record(&mut self, request: &ComputeBudgetInstruction) {
        match request {
            ComputeBudgetInstruction::SetComputeUnitPrice(price) => {
                self.duplicate |= self.unit_price.replace(*price).is_some();
            }
            ComputeBudgetInstruction::SetComputeUnitLimit(limit) => {
                self.duplicate |= self.unit_limit.replace(*limit).is_some();
            }
            ComputeBudgetInstruction::RequestHeapFrame(_)
            | ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(_)
            | ComputeBudgetInstruction::Unused => {}
        }
    }

    /// A repeated price or limit: the runtime rejects the transaction.
    pub fn is_invalid(&self) -> bool {
        self.duplicate
    }

    /// `None` when no price was requested or the price is zero: only the base fee is paid.
    pub fn estimate(&self) -> Option<PriorityFeeEstimate> {
        let price = self.unit_price.filter(|price| *price > 0)?;
        Some(match self.unit_limit {
            Some(limit) => {
                PriorityFeeEstimate::Exact(lamports(price, limit.min(MAX_COMPUTE_UNIT_LIMIT)))
            }
            None => PriorityFeeEstimate::UpperBound(lamports(price, MAX_COMPUTE_UNIT_LIMIT)),
        })
    }

    /// The fee the transaction commits to: the base fee for the [`charged_signatures`] plus the
    /// priority fee. Fails closed on a repeated price or limit, which the runtime rejects.
    pub fn committed(&self, signatures: u64) -> Result<CommittedFee, VisualSignError> {
        if self.is_invalid() {
            return Err(VisualSignError::ValidationError(
                "the fee this transaction commits to cannot be determined: repeated compute unit \
                 price or limit"
                    .to_string(),
            ));
        }
        Ok(CommittedFee {
            base: BASE_FEE_LAMPORTS_PER_SIGNATURE.saturating_mul(signatures),
            priority: self.estimate(),
        })
    }
}

/// Signatures the runtime charges for: the header's plus the ones each precompile instruction
/// verifies (its first data byte), as `SanitizedMessage::get_signature_details` counts them.
pub fn charged_signatures(
    num_required_signatures: u8,
    instructions: &[CompiledInstruction],
    static_account_keys: &[Pubkey],
) -> u64 {
    instructions
        .iter()
        .filter(|instruction| {
            static_account_keys
                .get(usize::from(instruction.program_id_index))
                .is_some_and(is_signature_precompile)
        })
        .filter_map(|instruction| instruction.data.first())
        .fold(u64::from(num_required_signatures), |total, verifies| {
            total.saturating_add(u64::from(*verifies))
        })
}

impl PriorityFeeEstimate {
    pub fn lamports(self) -> u64 {
        match self {
            Self::Exact(lamports) | Self::UpperBound(lamports) => lamports,
        }
    }

    pub fn field(self) -> Result<AnnotatedPayloadField, VisualSignError> {
        let label = match self {
            Self::Exact(_) => "Priority fee",
            Self::UpperBound(_) => "Maximum priority fee",
        };
        create_amount_field(
            label,
            &format_token_amount(self.lamports(), SOL_DECIMALS),
            "SOL",
        )
    }
}

/// Base fee plus priority fee, in lamports: the most the transaction pays if it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommittedFee {
    base: u64,
    priority: Option<PriorityFeeEstimate>,
}

impl CommittedFee {
    pub fn lamports(self) -> u64 {
        self.base
            .saturating_add(self.priority.map_or(0, PriorityFeeEstimate::lamports))
    }

    /// Rejects a fee cap below the committed fee, so a "Max Fee" row is never lower than what
    /// the transaction pays.
    pub fn check_cap(self, max_fee_lamports: u64) -> Result<(), VisualSignError> {
        let total = self.lamports();
        if max_fee_lamports >= total {
            return Ok(());
        }
        let priority = match self.priority {
            None => "no priority fee".to_string(),
            Some(PriorityFeeEstimate::Exact(lamports)) => format!("{lamports} priority fee"),
            Some(PriorityFeeEstimate::UpperBound(lamports)) => {
                format!("up to {lamports} priority fee with no compute unit limit")
            }
        };
        Err(VisualSignError::ValidationError(format!(
            "max fee of {max_fee_lamports} lamports is below the fee this transaction commits to: \
             {total} lamports ({base} base fee plus {priority})",
            base = self.base
        )))
    }
}

/// Top-level "Max Fee" row. Rendered only once [`CommittedFee::check_cap`] passes, so the value
/// is a true upper bound on what the transaction pays rather than the caller's bare claim.
pub fn max_fee_field(lamports: u64) -> Result<AnnotatedPayloadField, VisualSignError> {
    create_amount_field(
        "Max Fee",
        &format_token_amount(lamports, SOL_DECIMALS),
        "SOL",
    )
}

/// Rounded up, saturating at `u64::MAX` as the runtime does.
fn lamports(unit_price: u64, unit_limit: u32) -> u64 {
    let micro_lamports = u128::from(unit_price) * u128::from(unit_limit);
    u64::try_from(micro_lamports.div_ceil(MICRO_LAMPORTS_PER_LAMPORT)).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use visualsign::SignablePayloadField;

    fn fee_with(requests: &[ComputeBudgetInstruction]) -> PriorityFee {
        let mut fee = PriorityFee::default();
        for request in requests {
            fee.record(request);
        }
        fee
    }

    #[test]
    fn test_price_and_limit_give_an_exact_fee() {
        let fee = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(300_000),
            ComputeBudgetInstruction::SetComputeUnitPrice(50_000),
        ]);
        assert_eq!(fee.estimate(), Some(PriorityFeeEstimate::Exact(15_000)));
    }

    #[test]
    fn test_fee_rounds_up_to_the_next_lamport() {
        let fee = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(1),
            ComputeBudgetInstruction::SetComputeUnitPrice(1),
        ]);
        assert_eq!(fee.estimate(), Some(PriorityFeeEstimate::Exact(1)));
    }

    #[test]
    fn test_price_without_limit_is_bounded_by_the_runtime_maximum() {
        let fee = fee_with(&[ComputeBudgetInstruction::SetComputeUnitPrice(1_000)]);
        assert_eq!(
            fee.estimate(),
            Some(PriorityFeeEstimate::UpperBound(1_400_000_000 / 1_000_000))
        );
    }

    #[test]
    fn test_requested_limit_is_clamped_to_the_runtime_maximum() {
        let fee = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(10_000_000),
            ComputeBudgetInstruction::SetComputeUnitPrice(1_000),
        ]);
        assert_eq!(fee.estimate(), Some(PriorityFeeEstimate::Exact(1_400)));
    }

    #[test]
    fn test_no_price_or_zero_price_means_no_priority_fee() {
        assert_eq!(PriorityFee::default().estimate(), None);
        let limit_only = fee_with(&[ComputeBudgetInstruction::SetComputeUnitLimit(200_000)]);
        assert_eq!(limit_only.estimate(), None);
        let zero_price = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(200_000),
            ComputeBudgetInstruction::SetComputeUnitPrice(0),
        ]);
        assert_eq!(zero_price.estimate(), None);
    }

    #[test]
    fn test_other_requests_do_not_affect_the_fee() {
        let fee = fee_with(&[
            ComputeBudgetInstruction::RequestHeapFrame(64 * 1024),
            ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(1 << 20),
            ComputeBudgetInstruction::Unused,
        ]);
        assert_eq!(fee.estimate(), None);
        assert!(!fee.is_invalid());
    }

    #[test]
    fn test_duplicate_price_or_limit_is_invalid() {
        let twice_priced = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitPrice(1),
            ComputeBudgetInstruction::SetComputeUnitPrice(2),
        ]);
        assert!(twice_priced.is_invalid());
        let twice_limited = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(1),
            ComputeBudgetInstruction::SetComputeUnitLimit(2),
        ]);
        assert!(twice_limited.is_invalid());
    }

    #[test]
    fn test_max_fee_field_renders_lamports_as_sol() {
        let field = max_fee_field(5_105_000).unwrap();
        let SignablePayloadField::AmountV2 { amount_v2, common } = field.signable_payload_field
        else {
            panic!("expected amount_v2");
        };
        assert_eq!(common.label, "Max Fee");
        assert_eq!(common.fallback_text, "0.005105 SOL");
        assert_eq!(amount_v2.amount, "0.005105");
        assert_eq!(amount_v2.abbreviation.as_deref(), Some("SOL"));

        let zero = max_fee_field(0).unwrap();
        assert_eq!(zero.signable_payload_field.fallback_text(), "0 SOL");
    }

    #[test]
    fn test_fee_saturates_instead_of_overflowing() {
        assert_eq!(lamports(u64::MAX, MAX_COMPUTE_UNIT_LIMIT), u64::MAX);
    }

    #[test]
    fn test_committed_fee_adds_the_base_fee_per_signature() {
        let base_only = PriorityFee::default().committed(2).unwrap();
        assert_eq!(base_only.lamports(), 10_000);

        let exact = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(300_000),
            ComputeBudgetInstruction::SetComputeUnitPrice(50_000),
        ])
        .committed(1)
        .unwrap();
        assert_eq!(exact.lamports(), 20_000);

        let bound = fee_with(&[ComputeBudgetInstruction::SetComputeUnitPrice(1_000)])
            .committed(1)
            .unwrap();
        assert_eq!(bound.lamports(), 5_000 + 1_400);
    }

    fn compiled(program_id_index: u8, data: Vec<u8>) -> CompiledInstruction {
        CompiledInstruction {
            program_id_index,
            accounts: vec![],
            data,
        }
    }

    #[test]
    fn test_charged_signatures_add_the_verifies_of_precompile_instructions() {
        let keys = [
            Pubkey::new_unique(),
            solana_sdk::ed25519_program::id(),
            solana_sdk::secp256k1_program::id(),
            secp256r1_program::id(),
        ];
        let instructions = [
            compiled(1, vec![2, 0, 0]), // ed25519 verifies two signatures
            compiled(2, vec![1]),       // secp256k1 verifies one
            compiled(3, vec![3]),       // secp256r1 verifies three
            compiled(1, vec![]),        // no count byte
            compiled(0, vec![9]),       // not a precompile
            compiled(7, vec![9]),       // program index out of range
        ];
        assert_eq!(charged_signatures(1, &instructions, &keys), 1 + 2 + 1 + 3);
        assert_eq!(charged_signatures(2, &[], &keys), 2);
    }

    #[test]
    fn test_committed_fee_saturates_and_fails_closed_on_duplicates() {
        let huge = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(MAX_COMPUTE_UNIT_LIMIT),
            ComputeBudgetInstruction::SetComputeUnitPrice(u64::MAX),
        ]);
        assert_eq!(huge.committed(1).unwrap().lamports(), u64::MAX);
        let many_signatures = PriorityFee::default().committed(u64::MAX).unwrap();
        assert_eq!(many_signatures.lamports(), u64::MAX);

        let twice_priced = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitPrice(1),
            ComputeBudgetInstruction::SetComputeUnitPrice(2),
        ]);
        assert!(matches!(
            twice_priced.committed(1),
            Err(VisualSignError::ValidationError(_))
        ));
    }

    #[test]
    fn test_cap_must_cover_the_committed_fee() {
        let fee = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(300_000),
            ComputeBudgetInstruction::SetComputeUnitPrice(50_000),
        ])
        .committed(1)
        .unwrap();
        fee.check_cap(20_000).unwrap();
        fee.check_cap(u64::MAX).unwrap();
        let Err(VisualSignError::ValidationError(message)) = fee.check_cap(19_999) else {
            panic!("a cap below the committed fee must be rejected");
        };
        assert_eq!(
            message,
            "max fee of 19999 lamports is below the fee this transaction commits to: \
             20000 lamports (5000 base fee plus 15000 priority fee)"
        );
    }

    #[test]
    fn test_cap_error_names_the_priority_fee_shape() {
        let bound = fee_with(&[ComputeBudgetInstruction::SetComputeUnitPrice(1_000)])
            .committed(1)
            .unwrap();
        let Err(VisualSignError::ValidationError(message)) = bound.check_cap(6_399) else {
            panic!("a cap below the bound must be rejected");
        };
        assert!(
            message.ends_with(
                "6400 lamports (5000 base fee plus up to 1400 priority fee with no compute unit \
                 limit)"
            ),
            "{message}"
        );

        let base_only = PriorityFee::default().committed(1).unwrap();
        let Err(VisualSignError::ValidationError(message)) = base_only.check_cap(0) else {
            panic!("a zero cap must be rejected");
        };
        assert!(
            message.ends_with("5000 lamports (5000 base fee plus no priority fee)"),
            "{message}"
        );
    }

    #[test]
    fn test_field_renders_sol_with_the_bound_in_the_label() {
        let exact = PriorityFeeEstimate::Exact(15_000).field().unwrap();
        let SignablePayloadField::AmountV2 { amount_v2, common } = exact.signable_payload_field
        else {
            panic!("expected amount_v2");
        };
        assert_eq!(common.label, "Priority fee");
        assert_eq!(amount_v2.amount, "0.000015");
        assert_eq!(amount_v2.abbreviation.as_deref(), Some("SOL"));

        let bound = PriorityFeeEstimate::UpperBound(1_400_000_000)
            .field()
            .unwrap();
        assert_eq!(bound.signable_payload_field.label(), "Maximum priority fee");
        assert_eq!(bound.signable_payload_field.fallback_text(), "1.4 SOL");
    }
}
