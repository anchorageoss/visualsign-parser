use std::collections::HashMap;

use ::visualsign::AnnotatedPayloadField;
use ::visualsign::errors::VisualSignError;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::Instruction;

mod instructions;
mod visualsign;

pub use instructions::*;
pub use visualsign::*;

/// Identifier for which visualizer handled a command, categorized by dApp type. - Copied from Sui chain_parser
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualizerKind {
    /// Decentralized exchange protocols (e.g., AMMs, DEX aggregators)
    Dex(&'static str),
    /// Lending/borrowing protocols
    Lending(&'static str),
    /// Validator or pooled staking without liquid derivative tokens
    StakingPools(&'static str),
    /// Payment and simple transfer-related operations
    Payments(&'static str),
}

/// Context for visualizing a Solana instruction.
///
/// Holds all necessary information to visualize a specific command
/// within a transaction.
#[derive(Debug, Clone)]
pub struct VisualizerContext<'a> {
    /// The address sending the transaction.
    sender: &'a SolanaAccount,
    /// Index of the instruction to visualize.
    instruction_index: usize,
    /// All instruction in the transaction.
    /// Instruction struct contains data
    instructions: &'a Vec<Instruction>,
}

impl<'a> VisualizerContext<'a> {
    /// Creates a new `VisualizerContext`.
    pub fn new(
        sender: &'a SolanaAccount,
        instruction_index: usize,
        instructions: &'a Vec<Instruction>,
    ) -> Self {
        Self {
            sender,
            instruction_index,
            instructions,
        }
    }

    /// Returns the sender address.
    pub fn sender(&self) -> &SolanaAccount {
        self.sender
    }

    /// Returns the instruction index.
    pub fn instruction_index(&self) -> usize {
        self.instruction_index
    }

    /// Returns a reference to all instructions.
    pub fn instructions(&self) -> &Vec<Instruction> {
        self.instructions
    }

    /// Returns the current instruction being visualized.
    pub fn current_instruction(&self) -> Option<&Instruction> {
        self.instructions.get(self.instruction_index)
    }
}

pub struct SolanaIntegrationConfigData {
    pub programs: HashMap<&'static str, HashMap<&'static str, Vec<&'static str>>>,
}
pub trait SolanaIntegrationConfig {
    fn new() -> Self
    where
        Self: Sized;

    fn data(&self) -> &SolanaIntegrationConfigData;

    fn can_handle(&self, program_id: &str, _instruction: &Instruction) -> bool {
        // For now, just check if we support the program_id
        // You can extend this to parse instruction_data for specific instruction types
        self.data()
            .programs
            .get(program_id)
            .map(|_supported_instructions| true) // Can be refined to check specific instruction types
            .unwrap_or(false)
    }
}

// Trait for visualizing Solana Instructions - Copied from Sui chain_parser
pub trait InstructionVisualizer {
    /// Visualizes a specific instruction in a transaction.
    ///
    /// Returns `Some(SignablePayloadField)` if the instruction can be visualized,
    /// or `None` if the instruction is not supported by this visualizer.
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError>;

    /// Returns the config for the visualizer.
    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig>;

    /// The identifier of this visualizer.
    fn kind(&self) -> VisualizerKind;

    /// Checks if this visualizer can handle the given instruction.
    fn can_handle(&self, context: &VisualizerContext) -> bool {
        let Some(config) = self.get_config() else {
            return false;
        };

        let Some(instruction) = context.current_instruction() else {
            return false;
        };

        // Use Solana's program_id and instruction data
        let program_id = instruction.program_id.to_string();
        config.can_handle(&program_id, instruction)
    }
}

/// Result of a successful visualization attempt, including which visualizer handled it.
#[derive(Debug, Clone)]
pub struct VisualizeResult {
    pub field: AnnotatedPayloadField,
    pub kind: VisualizerKind,
}

/// Tries multiple visualizers in order, returning the first successful visualization.
///
/// # Arguments
/// * `visualizers` - Slice of visualizer trait objects.
/// * `context` - The visualization context.
///
/// # Returns
/// * `Some(VisualizeResult)` if any visualizer can handle the command, including which one.
/// * `None` if none can handle it.
pub fn visualize_with_any(
    visualizers: &[&dyn InstructionVisualizer],
    context: &VisualizerContext,
) -> Option<Result<VisualizeResult, VisualSignError>> {
    visualizers.iter().find_map(|v| {
        if !v.can_handle(context) {
            return None;
        }

        eprintln!(
            "Handling instruction {} with visualizer {:?}",
            context.instruction_index(),
            v.kind()
        );

        Some(
            v.visualize_tx_commands(context)
                .map(|field| VisualizeResult {
                    field,
                    kind: v.kind(),
                }),
        )
    })
}
