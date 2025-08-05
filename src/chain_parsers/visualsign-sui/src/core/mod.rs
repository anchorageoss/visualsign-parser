mod commands;
mod helper;
mod transaction;
mod visualsign;

use ::visualsign::SignablePayloadField;
use sui_json_rpc_types::{SuiCallArg, SuiCommand};
use sui_types::base_types::SuiAddress;

pub use visualsign::{SuiTransactionWrapper, SuiVisualSignConverter};

/// Context for visualizing a Sui transaction command.
///
/// Holds all necessary information to visualize a specific command
/// within a transaction.
#[derive(Debug, Clone)]
pub struct VisualizerContext<'a> {
    /// The address sending the transaction.
    sender: &'a SuiAddress,
    /// Index of the command to visualize.
    command_index: usize,
    /// All commands in the transaction.
    commands: &'a Vec<SuiCommand>,
    /// All input arguments for the transaction.
    inputs: &'a Vec<SuiCallArg>,
}

impl<'a> VisualizerContext<'a> {
    /// Creates a new `VisualizerContext`.
    pub fn new(
        sender: &'a SuiAddress,
        command_index: usize,
        commands: &'a Vec<SuiCommand>,
        inputs: &'a Vec<SuiCallArg>,
    ) -> Self {
        Self {
            sender,
            command_index,
            commands,
            inputs,
        }
    }

    /// Returns the sender address.
    pub fn sender(&self) -> &SuiAddress {
        self.sender
    }

    /// Returns the command index.
    pub fn command_index(&self) -> usize {
        self.command_index
    }

    /// Returns a reference to all commands.
    pub fn commands(&self) -> &Vec<SuiCommand> {
        self.commands
    }

    /// Returns a reference to all inputs.
    pub fn inputs(&self) -> &Vec<SuiCallArg> {
        self.inputs
    }
}

/// Trait for visualizing Sui transaction commands.
pub trait CommandVisualizer {
    /// Visualizes a specific command in a transaction.
    ///
    /// Returns `Some(SignablePayloadField)` if the command can be visualized,
    /// or `None` if the command is not supported by this visualizer.
    fn visualize_tx_commands(&self, context: &VisualizerContext) -> Option<SignablePayloadField>;

    /// Checks if this visualizer can handle the given command.
    fn can_handle(&self, context: &VisualizerContext) -> bool;
}

/// Tries multiple visualizers in order, returning the first successful visualization.
///
/// # Arguments
/// * `visualizers` - Slice of visualizer trait objects.
/// * `context` - The visualization context.
///
/// # Returns
/// * `Some(SignablePayloadField)` if any visualizer can handle the command.
/// * `None` if none can handle it.
pub fn visualize_with_any(
    visualizers: &[&dyn CommandVisualizer],
    context: &VisualizerContext,
) -> Option<SignablePayloadField> {
    visualizers
        .iter()
        .find(|v| v.can_handle(context))
        .and_then(|v| v.visualize_tx_commands(context))
}
