//! Fee a Solana message commits to, as the runtime charges it: the base fee per signature plus
//! `ceil(unit_price * unit_limit / 1_000_000)` lamports of priority fee on the unit limit.

use crate::utils::format_token_amount;
use borsh::BorshDeserialize;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_pubkey::Pubkey;
use visualsign::AnnotatedPayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_amount_field;

/// Runtime ceiling on the compute unit limit, requested or defaulted.
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

/// Default units the runtime allots an instruction of a program it does not run natively.
const DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT: u32 = 200_000;

/// Default units the runtime allots an instruction of a builtin program (SIMD-0170).
const MAX_BUILTIN_ALLOCATION_COMPUTE_UNIT_LIMIT: u32 = 3_000;

/// Heap frame sizes the runtime accepts: multiples of 1024 in this range; anything else is invalid.
const MIN_HEAP_FRAME_BYTES: u32 = 32 * 1024;
const MAX_HEAP_FRAME_BYTES: u32 = 256 * 1024;

const MICRO_LAMPORTS_PER_LAMPORT: u128 = 1_000_000;

/// Base fee the runtime charges per signature, in lamports; see [`charged_signatures`].
pub const BASE_FEE_LAMPORTS_PER_SIGNATURE: u64 = 5_000;

/// Fee cap applied when the caller supplies none: 0.1 SOL, well above the fee any transaction
/// legitimately commits to. A caller's `max_fee_lamports` replaces it.
pub const DEFAULT_MAX_FEE_LAMPORTS: u64 = 100_000_000;

/// The signature-verification precompiles, whose verifies the runtime charges as signatures.
fn is_signature_precompile(program_id: &Pubkey) -> bool {
    solana_sdk_ids::ed25519_program::check_id(program_id)
        || solana_sdk_ids::secp256k1_program::check_id(program_id)
        || solana_sdk_ids::secp256r1_program::check_id(program_id)
}

/// Builtins that stay native, so the runtime allots them the builtin default. A builtin that
/// may move to BPF (vote) counts as any other program, which never undercounts the fee.
fn is_native_builtin(program_id: &Pubkey) -> bool {
    solana_sdk_ids::system_program::check_id(program_id)
        || solana_sdk_ids::compute_budget::check_id(program_id)
        || solana_sdk_ids::bpf_loader::check_id(program_id)
        || solana_sdk_ids::bpf_loader_deprecated::check_id(program_id)
        || solana_sdk_ids::bpf_loader_upgradeable::check_id(program_id)
        || solana_sdk_ids::loader_v4::check_id(program_id)
        || solana_sdk_ids::ed25519_program::check_id(program_id)
        || solana_sdk_ids::secp256k1_program::check_id(program_id)
}

/// Decodes a compute-budget request as the runtime does: trailing bytes are ignored.
pub fn decode_compute_budget_request(data: &[u8]) -> borsh::io::Result<ComputeBudgetInstruction> {
    ComputeBudgetInstruction::deserialize(&mut &data[..])
}

const SOL_DECIMALS: u8 = 9;

/// The compute-budget requests of a message, and the unit limit the runtime applies without one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PriorityFee {
    unit_price: Option<u64>,
    unit_limit: Option<u32>,
    heap_frame_requested: bool,
    loaded_accounts_data_size_requested: bool,
    /// Why the runtime rejects the transaction: a repeated, out-of-range, or unused request.
    invalid: Option<&'static str>,
    /// The runtime's default unit limit for this message's instructions.
    default_unit_limit: u32,
}

/// The fee in lamports. `UpperBound` when no unit limit was requested: the fee at the runtime's
/// default limit, which only a builtin still run natively could make lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityFeeEstimate {
    Exact(u64),
    UpperBound(u64),
}

impl PriorityFee {
    /// Records one request as the runtime does: a repeat, an out-of-range size, or the unused
    /// variant makes the transaction invalid.
    pub fn record(&mut self, request: &ComputeBudgetInstruction) {
        let fault = match request {
            ComputeBudgetInstruction::SetComputeUnitPrice(price) => self
                .unit_price
                .replace(*price)
                .is_some()
                .then_some("repeated compute unit price"),
            ComputeBudgetInstruction::SetComputeUnitLimit(limit) => self
                .unit_limit
                .replace(*limit)
                .is_some()
                .then_some("repeated compute unit limit"),
            ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
                if std::mem::replace(&mut self.heap_frame_requested, true) {
                    Some("repeated heap frame request")
                } else if !(MIN_HEAP_FRAME_BYTES..=MAX_HEAP_FRAME_BYTES).contains(bytes)
                    || !bytes.is_multiple_of(1024)
                {
                    Some("heap frame size out of range")
                } else {
                    None
                }
            }
            ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
                if std::mem::replace(&mut self.loaded_accounts_data_size_requested, true) {
                    Some("repeated loaded accounts data size limit")
                } else if *bytes == 0 {
                    Some("loaded accounts data size limit of zero")
                } else {
                    None
                }
            }
            ComputeBudgetInstruction::Unused => Some("unused compute budget request"),
        };
        if let Some(fault) = fault {
            self.invalid.get_or_insert(fault);
        }
    }

    /// The requests in the compute-budget instructions, decoded as the runtime decodes them, plus
    /// the default unit limit for the whole message. An undecodable request leaves the fee unknown.
    pub fn from_instructions(
        instructions: &[CompiledInstruction],
        static_account_keys: &[Pubkey],
    ) -> Result<Self, VisualSignError> {
        let mut fee = Self::default();
        for (index, instruction) in instructions.iter().enumerate() {
            let program_id = static_account_keys.get(usize::from(instruction.program_id_index));
            let allotment = if program_id.is_some_and(is_native_builtin) {
                MAX_BUILTIN_ALLOCATION_COMPUTE_UNIT_LIMIT
            } else {
                DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT
            };
            fee.default_unit_limit = fee.default_unit_limit.saturating_add(allotment);
            if !program_id.is_some_and(solana_sdk_ids::compute_budget::check_id) {
                continue;
            }
            let request = decode_compute_budget_request(&instruction.data).map_err(|e| {
                VisualSignError::ValidationError(format!(
                    "the fee this transaction commits to cannot be determined: compute budget \
                     instruction {index} did not decode: {e}"
                ))
            })?;
            fee.record(&request);
        }
        Ok(fee)
    }

    /// Why the runtime rejects the transaction, if a request repeats, is out of range, or unused.
    pub fn invalid(&self) -> Option<&'static str> {
        self.invalid
    }

    /// `None` when no price was requested or the price is zero: only the base fee is paid.
    pub fn estimate(&self) -> Option<PriorityFeeEstimate> {
        let price = self.unit_price.filter(|price| *price > 0)?;
        Some(match self.unit_limit {
            Some(limit) => {
                PriorityFeeEstimate::Exact(lamports(price, limit.min(MAX_COMPUTE_UNIT_LIMIT)))
            }
            None => PriorityFeeEstimate::UpperBound(lamports(
                price,
                self.default_unit_limit.min(MAX_COMPUTE_UNIT_LIMIT),
            )),
        })
    }

    /// The fee the transaction commits to: the base fee for the [`charged_signatures`] plus the
    /// priority fee. Fails closed when the runtime would reject the compute-budget requests.
    pub fn committed(&self, signatures: u64) -> Result<CommittedFee, VisualSignError> {
        if let Some(fault) = self.invalid() {
            return Err(VisualSignError::ValidationError(format!(
                "the runtime rejects this transaction: {fault}"
            )));
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

/// The fee the message commits to, from its compiled form alone: the base fee for the
/// [`charged_signatures`] plus the priority fee its compute-budget instructions request.
pub fn committed_fee(
    num_required_signatures: u8,
    instructions: &[CompiledInstruction],
    static_account_keys: &[Pubkey],
) -> Result<CommittedFee, VisualSignError> {
    PriorityFee::from_instructions(instructions, static_account_keys)?.committed(
        charged_signatures(num_required_signatures, instructions, static_account_keys),
    )
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

    /// The priority fee part, which the adopted summary hoists as its own row.
    pub fn priority(self) -> Option<PriorityFeeEstimate> {
        self.priority
    }

    /// Rejects a fee cap below the committed fee, so a "Max Fee" row is never lower than what
    /// the transaction pays.
    pub fn check_cap(self, max_fee_lamports: u64) -> Result<(), VisualSignError> {
        let total = self.lamports();
        if max_fee_lamports >= total {
            return Ok(());
        }
        let base = self.base;
        let priority = match self.priority {
            None => "no priority fee".to_string(),
            Some(PriorityFeeEstimate::Exact(lamports)) => format!("{lamports} priority fee"),
            Some(PriorityFeeEstimate::UpperBound(lamports)) => {
                format!("up to {lamports} priority fee at the default compute unit limit")
            }
        };
        Err(VisualSignError::ValidationError(format!(
            "max fee of {max_fee_lamports} lamports is below the fee this transaction commits to: \
             {total} lamports ({base} base fee plus {priority})"
        )))
    }
}

/// The fee cap in force: the caller's `max_fee_lamports`, or the parser default without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeCap {
    pub lamports: u64,
    pub caller_supplied: bool,
}

impl FeeCap {
    pub fn resolve(caller_cap: Option<u64>) -> Self {
        match caller_cap {
            Some(lamports) => Self {
                lamports,
                caller_supplied: true,
            },
            None => Self {
                lamports: DEFAULT_MAX_FEE_LAMPORTS,
                caller_supplied: false,
            },
        }
    }

    /// Top-level row, rendered once [`CommittedFee::check_cap`] passed. The label says when the
    /// value is the parser default rather than a cap chosen for this transaction.
    pub fn field(self) -> Result<AnnotatedPayloadField, VisualSignError> {
        let label = if self.caller_supplied {
            "Max Fee"
        } else {
            "Max Fee (default)"
        };
        create_amount_field(
            label,
            &format_token_amount(self.lamports, SOL_DECIMALS),
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

    fn compiled(program_id_index: u8, data: Vec<u8>) -> CompiledInstruction {
        CompiledInstruction {
            program_id_index,
            accounts: vec![],
            data,
        }
    }

    /// `requests` compiled as compute-budget instructions, then one data-free instruction per
    /// program in `others`. Key 0 is the compute budget program.
    fn message(
        requests: &[ComputeBudgetInstruction],
        others: &[Pubkey],
    ) -> (Vec<Pubkey>, Vec<CompiledInstruction>) {
        let mut keys = vec![solana_sdk_ids::compute_budget::id()];
        let mut instructions: Vec<CompiledInstruction> = requests
            .iter()
            .map(|request| compiled(0, borsh::to_vec(request).unwrap()))
            .collect();
        for program in others {
            keys.push(*program);
            instructions.push(compiled(
                u8::try_from(keys.len() - 1).unwrap(),
                vec![0xff; 4],
            ));
        }
        (keys, instructions)
    }

    fn fee_with(requests: &[ComputeBudgetInstruction]) -> PriorityFee {
        let (keys, instructions) = message(requests, &[]);
        PriorityFee::from_instructions(&instructions, &keys).unwrap()
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

    /// Without a limit the runtime allots each instruction a default: the builtin allotment for
    /// the compute budget request itself, the full default for another program.
    #[test]
    fn test_price_without_limit_uses_the_runtime_default_limit() {
        let price = ComputeBudgetInstruction::SetComputeUnitPrice(1_000_000);
        assert_eq!(
            fee_with(std::slice::from_ref(&price)).estimate(),
            Some(PriorityFeeEstimate::UpperBound(3_000))
        );
        let (keys, instructions) = message(&[price], &[Pubkey::new_unique()]);
        let fee = PriorityFee::from_instructions(&instructions, &keys).unwrap();
        assert_eq!(
            fee.estimate(),
            Some(PriorityFeeEstimate::UpperBound(203_000))
        );
    }

    /// Native builtins get 3_000 units each; a builtin that may move to BPF (vote), any other
    /// program, and an unresolvable program index get 200_000, so the fee is never undercounted.
    #[test]
    fn test_default_limit_counts_builtins_at_the_runtime_allotment() {
        let (keys, mut instructions) = message(
            &[ComputeBudgetInstruction::SetComputeUnitPrice(1_000_000)],
            &[
                solana_sdk_ids::system_program::id(),
                solana_sdk_ids::ed25519_program::id(),
                Pubkey::new_unique(),
                solana_sdk_ids::vote::id(),
            ],
        );
        instructions.push(compiled(9, vec![]));
        let fee = PriorityFee::from_instructions(&instructions, &keys).unwrap();
        assert_eq!(
            fee.estimate(),
            Some(PriorityFeeEstimate::UpperBound(
                3_000 + 3_000 + 3_000 + 200_000 + 200_000 + 200_000
            ))
        );
    }

    #[test]
    fn test_default_limit_is_clamped_to_the_runtime_maximum() {
        let others: Vec<Pubkey> = (0..8).map(|_| Pubkey::new_unique()).collect();
        let (keys, instructions) = message(
            &[ComputeBudgetInstruction::SetComputeUnitPrice(1_000_000)],
            &others,
        );
        let fee = PriorityFee::from_instructions(&instructions, &keys).unwrap();
        assert_eq!(
            fee.estimate(),
            Some(PriorityFeeEstimate::UpperBound(u64::from(
                MAX_COMPUTE_UNIT_LIMIT
            )))
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
    fn test_valid_other_requests_do_not_affect_the_fee() {
        let fee = fee_with(&[
            ComputeBudgetInstruction::RequestHeapFrame(64 * 1024),
            ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(1 << 20),
        ]);
        assert_eq!(fee.estimate(), None);
        assert_eq!(fee.invalid(), None);
    }

    /// Every request the runtime rejects marks the transaction invalid, and the first fault wins.
    #[test]
    fn test_requests_the_runtime_rejects_are_invalid() {
        use ComputeBudgetInstruction::*;
        let cases: &[(&[ComputeBudgetInstruction], &str)] = &[
            (
                &[SetComputeUnitPrice(1), SetComputeUnitPrice(2)],
                "repeated compute unit price",
            ),
            (
                &[SetComputeUnitLimit(1), SetComputeUnitLimit(2)],
                "repeated compute unit limit",
            ),
            (
                &[RequestHeapFrame(64 * 1024), RequestHeapFrame(64 * 1024)],
                "repeated heap frame request",
            ),
            (
                &[
                    SetLoadedAccountsDataSizeLimit(1 << 20),
                    SetLoadedAccountsDataSizeLimit(1 << 20),
                ],
                "repeated loaded accounts data size limit",
            ),
            (&[RequestHeapFrame(1_000)], "heap frame size out of range"),
            (
                &[RequestHeapFrame(32 * 1024 + 1)],
                "heap frame size out of range",
            ),
            (
                &[RequestHeapFrame(257 * 1024)],
                "heap frame size out of range",
            ),
            (
                &[SetLoadedAccountsDataSizeLimit(0)],
                "loaded accounts data size limit of zero",
            ),
            (&[Unused], "unused compute budget request"),
            (
                &[SetComputeUnitPrice(1), SetComputeUnitPrice(2), Unused],
                "repeated compute unit price",
            ),
        ];
        for (requests, fault) in cases {
            let fee = fee_with(requests);
            assert_eq!(fee.invalid(), Some(*fault), "{requests:?}");
            let Err(VisualSignError::ValidationError(message)) = fee.committed(1) else {
                panic!("{requests:?} must fail closed");
            };
            assert_eq!(
                message,
                format!("the runtime rejects this transaction: {fault}")
            );
        }
    }

    #[test]
    fn test_fee_cap_field_renders_lamports_as_sol() {
        let field = FeeCap::resolve(Some(5_105_000)).field().unwrap();
        let SignablePayloadField::AmountV2 { amount_v2, common } = field.signable_payload_field
        else {
            panic!("expected amount_v2");
        };
        assert_eq!(common.label, "Max Fee");
        assert_eq!(common.fallback_text, "0.005105 SOL");
        assert_eq!(amount_v2.amount, "0.005105");
        assert_eq!(amount_v2.abbreviation.as_deref(), Some("SOL"));

        let zero = FeeCap::resolve(Some(0)).field().unwrap();
        assert_eq!(zero.signable_payload_field.fallback_text(), "0 SOL");
    }

    /// The default is a parser-wide ceiling, not a cap chosen for this transaction, and the
    /// label says so.
    #[test]
    fn test_default_cap_is_labelled_as_the_default() {
        let cap = FeeCap::resolve(None);
        assert_eq!(cap.lamports, DEFAULT_MAX_FEE_LAMPORTS);
        assert!(!cap.caller_supplied);
        let field = cap.field().unwrap();
        assert_eq!(field.signable_payload_field.label(), "Max Fee (default)");
        assert_eq!(field.signable_payload_field.fallback_text(), "0.1 SOL");

        let same_value = FeeCap::resolve(Some(DEFAULT_MAX_FEE_LAMPORTS));
        assert!(same_value.caller_supplied);
        assert_eq!(
            same_value.field().unwrap().signable_payload_field.label(),
            "Max Fee"
        );
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

        // One compute budget instruction: 3_000 default units at 1_000 micro-lamports each.
        let bound = fee_with(&[ComputeBudgetInstruction::SetComputeUnitPrice(1_000)])
            .committed(1)
            .unwrap();
        assert_eq!(bound.lamports(), 5_000 + 3);
    }

    #[test]
    fn test_charged_signatures_add_the_verifies_of_precompile_instructions() {
        let keys = [
            Pubkey::new_unique(),
            solana_sdk_ids::ed25519_program::id(),
            solana_sdk_ids::secp256k1_program::id(),
            solana_sdk_ids::secp256r1_program::id(),
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
    fn test_from_instructions_reads_only_compute_budget_requests() {
        let keys = [Pubkey::new_unique(), solana_sdk_ids::compute_budget::id()];
        let limit = borsh::to_vec(&ComputeBudgetInstruction::SetComputeUnitLimit(300_000)).unwrap();
        let price = borsh::to_vec(&ComputeBudgetInstruction::SetComputeUnitPrice(50_000)).unwrap();
        let instructions = [
            compiled(1, limit.clone()),
            compiled(0, vec![0xff; 4]), // another program: never decoded
            compiled(1, price),
            compiled(7, limit), // program index out of range
        ];
        let fee = PriorityFee::from_instructions(&instructions, &keys).unwrap();
        assert_eq!(fee.estimate(), Some(PriorityFeeEstimate::Exact(15_000)));
        assert_eq!(
            committed_fee(2, &instructions, &keys).unwrap().lamports(),
            10_000 + 15_000
        );
    }

    /// The runtime reads a request with `try_from_slice_unchecked`, so bytes after it are
    /// ignored and the request is still charged.
    #[test]
    fn test_from_instructions_ignores_trailing_bytes_like_the_runtime() {
        let keys = [solana_sdk_ids::compute_budget::id()];
        let instructions = [compiled(0, vec![3, 0x40, 0x42, 0x0f, 0, 0, 0, 0, 0, 0xff])];
        let fee = PriorityFee::from_instructions(&instructions, &keys).unwrap();
        assert_eq!(fee.estimate(), Some(PriorityFeeEstimate::UpperBound(3_000)));
    }

    /// Cut short or an unknown variant: the runtime rejects the transaction, the parser
    /// cannot read the request, and the fee stays unknown.
    #[test]
    fn test_from_instructions_fails_closed_on_an_undecodable_request() {
        let keys = [solana_sdk_ids::compute_budget::id()];
        for data in [vec![3, 0x40, 0x42], vec![9]] {
            let instructions = [compiled(0, data)];
            let Err(VisualSignError::ValidationError(message)) =
                PriorityFee::from_instructions(&instructions, &keys)
            else {
                panic!("an undecodable request must leave the fee unknown");
            };
            assert!(
                message.contains("compute budget instruction 0 did not decode"),
                "{message}"
            );
            assert!(committed_fee(1, &instructions, &keys).is_err());
        }
    }

    #[test]
    fn test_committed_fee_saturates() {
        let huge = fee_with(&[
            ComputeBudgetInstruction::SetComputeUnitLimit(MAX_COMPUTE_UNIT_LIMIT),
            ComputeBudgetInstruction::SetComputeUnitPrice(u64::MAX),
        ]);
        assert_eq!(huge.committed(1).unwrap().lamports(), u64::MAX);
        let many_signatures = PriorityFee::default().committed(u64::MAX).unwrap();
        assert_eq!(many_signatures.lamports(), u64::MAX);
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
        let Err(VisualSignError::ValidationError(message)) = bound.check_cap(5_002) else {
            panic!("a cap below the bound must be rejected");
        };
        assert!(
            message.ends_with(
                "5003 lamports (5000 base fee plus up to 3 priority fee at the default compute \
                 unit limit)"
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
