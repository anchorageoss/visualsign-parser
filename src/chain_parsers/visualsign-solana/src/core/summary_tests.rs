//! Program-independent tests for the transaction-summary framework: the
//! contract between [`InstructionVisualizer::transaction_summary`] /
//! [`InstructionVisualizer::is_infrastructure`], [`visualize_with_any`] and
//! [`SummaryAccumulator`]. A fake visualizer stands in for any preset, so these
//! tests pin the gate rules without depending on a real program's decoder.

use super::*;
use ::visualsign::field_builders::create_text_field;
use solana_sdk::instruction::CompiledInstruction;

/// A visualizer for one program that renders a fixed row and reports whatever
/// summary and infrastructure flag the test configures.
struct FakeVisualizer {
    program: Pubkey,
    label: &'static str,
    summary: Option<TransactionSummary>,
    infrastructure: bool,
    fails: bool,
}

impl FakeVisualizer {
    fn plain(program: Pubkey, label: &'static str) -> Self {
        Self {
            program,
            label,
            summary: None,
            infrastructure: false,
            fails: false,
        }
    }

    fn infrastructure(program: Pubkey, label: &'static str) -> Self {
        Self {
            infrastructure: true,
            ..Self::plain(program, label)
        }
    }

    fn proposing(program: Pubkey, label: &'static str, title: &str) -> Self {
        Self {
            summary: Some(summary(title)),
            ..Self::plain(program, label)
        }
    }

    fn failing(program: Pubkey, label: &'static str) -> Self {
        Self {
            fails: true,
            ..Self::plain(program, label)
        }
    }
}

impl InstructionVisualizer for FakeVisualizer {
    fn visualize_tx_commands(
        &self,
        _context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        if self.fails {
            return Err(VisualSignError::DecodeError(format!(
                "{} cannot decode",
                self.label
            )));
        }
        create_text_field(self.label, "rendered")
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        None
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Payments("test")
    }

    fn transaction_summary(&self, _context: &VisualizerContext) -> Option<TransactionSummary> {
        self.summary.clone()
    }

    fn is_infrastructure(&self, _context: &VisualizerContext) -> bool {
        self.infrastructure
    }

    fn can_handle(&self, context: &VisualizerContext) -> bool {
        matches!(context.program_id(), ProgramRef::Resolved(pk) if *pk == self.program)
    }
}

fn summary(title: &str) -> TransactionSummary {
    TransactionSummary {
        title: title.to_string(),
        subtitle: Some("Test Program".to_string()),
        fields: vec![create_text_field("Amount", "1").expect("field")],
    }
}

/// One instruction per program key; `programs[i]` is the program of instruction `i`.
/// `account_keys[0]` is the fee payer and never a program.
fn instructions_for(programs: &[Pubkey]) -> (Vec<Pubkey>, Vec<CompiledInstruction>) {
    let mut account_keys = vec![Pubkey::new_unique()];
    account_keys.extend_from_slice(programs);
    let instructions = programs
        .iter()
        .enumerate()
        .map(|(i, _)| CompiledInstruction {
            program_id_index: u8::try_from(i + 1).expect("small index"),
            accounts: vec![0],
            data: vec![0x01],
        })
        .collect();
    (account_keys, instructions)
}

/// Mirrors the decode loops: dispatch each instruction, fold its result into
/// the accumulator, block on anything that fails or has no visualizer.
fn fold(
    visualizers: &[&dyn InstructionVisualizer],
    account_keys: &[Pubkey],
    instructions: &[CompiledInstruction],
) -> (Vec<AnnotatedPayloadField>, Option<TransactionSummary>) {
    let registry = crate::idl::IdlRegistry::new();
    let sender = SolanaAccount {
        account_key: account_keys[0].to_string(),
        signer: false,
        writable: false,
    };
    let mut fields = Vec::new();
    let mut accumulator = SummaryAccumulator::default();
    for (i, ci) in instructions.iter().enumerate() {
        let context = VisualizerContext::new(&sender, ci, account_keys, &registry, i);
        match visualize_with_any(visualizers, &context) {
            Some(Ok(mut result)) => {
                accumulator.observe(&mut result);
                fields.push(result.field);
            }
            Some(Err(_)) | None => accumulator.block(),
        }
    }
    (fields, accumulator.finish())
}

fn labels(fields: &[AnnotatedPayloadField]) -> Vec<String> {
    fields
        .iter()
        .map(|f| f.signable_payload_field.label().clone())
        .collect()
}

#[test]
fn visualize_with_any_carries_summary_and_infrastructure_from_the_visualizer() {
    let program = Pubkey::new_unique();
    let (keys, instructions) = instructions_for(&[program]);
    let registry = crate::idl::IdlRegistry::new();
    let sender = SolanaAccount {
        account_key: keys[0].to_string(),
        signer: false,
        writable: false,
    };
    let context = VisualizerContext::new(&sender, &instructions[0], &keys, &registry, 0);

    let proposer = FakeVisualizer::proposing(program, "Action", "Do the thing");
    let result = visualize_with_any(&[&proposer], &context)
        .expect("handled")
        .expect("rendered");
    assert_eq!(
        result.summary.as_ref().map(|s| s.title.as_str()),
        Some("Do the thing")
    );
    assert!(!result.infrastructure);

    let infra = FakeVisualizer::infrastructure(program, "Setup");
    let result = visualize_with_any(&[&infra], &context)
        .expect("handled")
        .expect("rendered");
    assert!(result.summary.is_none());
    assert!(result.infrastructure);
}

#[test]
fn visualize_with_any_returns_none_when_no_visualizer_handles_the_program() {
    let (keys, instructions) = instructions_for(&[Pubkey::new_unique()]);
    let registry = crate::idl::IdlRegistry::new();
    let sender = SolanaAccount {
        account_key: keys[0].to_string(),
        signer: false,
        writable: false,
    };
    let context = VisualizerContext::new(&sender, &instructions[0], &keys, &registry, 0);
    let other = FakeVisualizer::plain(Pubkey::new_unique(), "Other");
    assert!(visualize_with_any(&[&other], &context).is_none());
}

#[test]
fn accumulator_adopts_a_single_proposal() {
    let mut accumulator = SummaryAccumulator::default();
    let mut result = VisualizeResult {
        field: create_text_field("Action", "rendered").expect("field"),
        kind: VisualizerKind::Payments("test"),
        summary: Some(summary("Do the thing")),
        infrastructure: false,
    };
    accumulator.observe(&mut result);
    assert!(
        result.summary.is_none(),
        "observe takes the proposal out of the result"
    );
    assert_eq!(
        accumulator.finish().map(|s| s.title),
        Some("Do the thing".to_string())
    );
}

#[test]
fn accumulator_yields_nothing_without_a_proposal() {
    assert!(SummaryAccumulator::default().finish().is_none());

    let mut accumulator = SummaryAccumulator::default();
    let mut infra = VisualizeResult {
        field: create_text_field("Setup", "rendered").expect("field"),
        kind: VisualizerKind::Payments("test"),
        summary: None,
        infrastructure: true,
    };
    accumulator.observe(&mut infra);
    assert!(
        accumulator.finish().is_none(),
        "infrastructure alone proposes no summary"
    );
}

/// The gate: exactly one proposer, and every other instruction infrastructure.
#[test]
fn fold_adopts_one_proposal_surrounded_by_infrastructure() {
    let (setup, action, nonce) = (
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
    );
    let (keys, instructions) = instructions_for(&[setup, action, nonce]);
    let visualizers: [&dyn InstructionVisualizer; 3] = [
        &FakeVisualizer::infrastructure(setup, "Setup"),
        &FakeVisualizer::proposing(action, "Action", "Do the thing"),
        &FakeVisualizer::infrastructure(nonce, "Nonce"),
    ];

    let (fields, adopted) = fold(&visualizers, &keys, &instructions);

    assert_eq!(labels(&fields), ["Setup", "Action", "Nonce"]);
    let adopted = adopted.expect("summary adopted");
    assert_eq!(adopted.title, "Do the thing");
    assert_eq!(adopted.subtitle.as_deref(), Some("Test Program"));
    assert_eq!(labels(&adopted.fields), ["Amount"]);
}

#[test]
fn fold_rejects_two_proposals() {
    let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
    let (keys, instructions) = instructions_for(&[a, b]);
    let visualizers: [&dyn InstructionVisualizer; 2] = [
        &FakeVisualizer::proposing(a, "First", "First action"),
        &FakeVisualizer::proposing(b, "Second", "Second action"),
    ];
    let (fields, adopted) = fold(&visualizers, &keys, &instructions);
    assert_eq!(fields.len(), 2, "both instructions still render");
    assert!(adopted.is_none());
}

/// A rendered instruction that is neither the proposer nor infrastructure, such
/// as a transfer, means the transaction does more than the proposal says.
#[test]
fn fold_rejects_a_proposal_next_to_a_non_infrastructure_instruction() {
    let (action, transfer) = (Pubkey::new_unique(), Pubkey::new_unique());
    let (keys, instructions) = instructions_for(&[action, transfer]);
    let visualizers: [&dyn InstructionVisualizer; 2] = [
        &FakeVisualizer::proposing(action, "Action", "Do the thing"),
        &FakeVisualizer::plain(transfer, "Transfer"),
    ];
    let (fields, adopted) = fold(&visualizers, &keys, &instructions);
    assert_eq!(labels(&fields), ["Action", "Transfer"]);
    assert!(adopted.is_none());
}

/// An instruction no visualizer handles is unknown value movement: it blocks.
#[test]
fn fold_rejects_a_proposal_next_to_an_unhandled_instruction() {
    let (action, unknown) = (Pubkey::new_unique(), Pubkey::new_unique());
    let (keys, instructions) = instructions_for(&[action, unknown]);
    let visualizers: [&dyn InstructionVisualizer; 1] =
        [&FakeVisualizer::proposing(action, "Action", "Do the thing")];
    let (fields, adopted) = fold(&visualizers, &keys, &instructions);
    assert_eq!(labels(&fields), ["Action"]);
    assert!(adopted.is_none());
}

/// A visualizer that fails to render blocks the summary even when it would
/// otherwise have been infrastructure.
#[test]
fn fold_rejects_a_proposal_next_to_a_failed_instruction() {
    let (action, broken) = (Pubkey::new_unique(), Pubkey::new_unique());
    let (keys, instructions) = instructions_for(&[action, broken]);
    let visualizers: [&dyn InstructionVisualizer; 2] = [
        &FakeVisualizer::proposing(action, "Action", "Do the thing"),
        &FakeVisualizer::failing(broken, "Broken"),
    ];
    let (fields, adopted) = fold(&visualizers, &keys, &instructions);
    assert_eq!(labels(&fields), ["Action"]);
    assert!(adopted.is_none());
}

/// Order does not matter: a blocker before the proposer blocks just the same.
#[test]
fn fold_blocks_regardless_of_instruction_order() {
    let (transfer, action) = (Pubkey::new_unique(), Pubkey::new_unique());
    let (keys, instructions) = instructions_for(&[transfer, action]);
    let visualizers: [&dyn InstructionVisualizer; 2] = [
        &FakeVisualizer::plain(transfer, "Transfer"),
        &FakeVisualizer::proposing(action, "Action", "Do the thing"),
    ];
    let (_, adopted) = fold(&visualizers, &keys, &instructions);
    assert!(adopted.is_none());
}
