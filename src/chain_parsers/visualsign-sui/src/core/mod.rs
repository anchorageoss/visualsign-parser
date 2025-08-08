mod commands;
mod helper;
mod transaction;
mod visualsign;

use std::collections::HashMap;

use sui_json_rpc_types::{SuiCallArg, SuiCommand};
use sui_types::base_types::SuiAddress;

use ::visualsign::SignablePayloadField;
pub use visualsign::{
    SuiTransactionWrapper, SuiVisualSignConverter, transaction_string_to_visual_sign,
    transaction_to_visual_sign,
};

pub struct SuiIntegrationConfigData {
    pub packages: HashMap<&'static str, HashMap<&'static str, Vec<&'static str>>>,
}

pub trait SuiIntegrationConfig {
    fn new() -> Self
    where
        Self: Sized;

    fn data(&self) -> &SuiIntegrationConfigData;

    fn can_handle(&self, package: &str, module: &str, function: &str) -> bool {
        self.data()
            .packages
            .get(package)
            .and_then(|modules| modules.get(module))
            .map(|functions| functions.contains(&function))
            .unwrap_or(false)
    }
}

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

    /// Returns the config for the visualizer.
    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig>;

    /// Checks if this visualizer can handle the given command.
    fn can_handle(&self, context: &VisualizerContext) -> bool {
        let Some(config) = self.get_config() else {
            return false;
        };

        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return false;
        };

        config.can_handle(&pwc.package.to_hex_literal(), &pwc.module, &pwc.function)
    }
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
