//! Priority fee committed by compute-budget instructions, as the runtime charges it:
//! `ceil(unit_price * unit_limit / 1_000_000)` lamports on the requested limit.

use crate::utils::format_token_amount;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_amount_field;

/// Runtime ceiling; bounds the fee when no limit is requested (the default is version-dependent).
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

const MICRO_LAMPORTS_PER_LAMPORT: u128 = 1_000_000;

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
            Some(limit) => PriorityFeeEstimate::Exact(lamports(price, limit)),
            None => PriorityFeeEstimate::UpperBound(lamports(price, MAX_COMPUTE_UNIT_LIMIT)),
        })
    }
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
    fn test_fee_saturates_instead_of_overflowing() {
        assert_eq!(lamports(u64::MAX, MAX_COMPUTE_UNIT_LIMIT), u64::MAX);
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
