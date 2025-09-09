mod chain_config;
mod commands;
mod helper;
mod transaction;
mod visualsign;

use std::collections::HashMap;

use sui_json_rpc_types::{SuiCallArg, SuiCommand};
use sui_types::base_types::SuiAddress;

use ::visualsign::AnnotatedPayloadField;
use ::visualsign::errors::VisualSignError;
pub use visualsign::{
    SuiTransactionWrapper, SuiVisualSignConverter, transaction_string_to_visual_sign,
    transaction_to_visual_sign,
};

/// Identifier for which visualizer handled a command, categorized by dApp type.
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
            .is_some_and(|functions| functions.contains(&function))
    }
}

/// Context for visualizing a Sui transaction command.
///
/// Holds the information required to visualize a specific command within a transaction.
#[derive(Debug, Clone)]
pub struct VisualizerContext<'a> {
    /// The address sending the transaction.
    sender: &'a SuiAddress,
    /// Index of the command to visualize.
    command_index: usize,
    /// All commands in the transaction.
    commands: &'a [SuiCommand],
    /// All input arguments for the transaction.
    inputs: &'a [SuiCallArg],
}

impl<'a> VisualizerContext<'a> {
    /// Creates a new `VisualizerContext`.
    pub fn new(
        sender: &'a SuiAddress,
        command_index: usize,
        commands: &'a [SuiCommand],
        inputs: &'a [SuiCallArg],
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
    pub fn commands(&self) -> &[SuiCommand] {
        self.commands
    }

    /// Returns a reference to all inputs.
    pub fn inputs(&self) -> &[SuiCallArg] {
        self.inputs
    }
}

/// Trait for visualizing Sui transaction commands.
pub trait CommandVisualizer {
    /// Visualizes a specific command in a transaction.
    ///
    /// Returns `Vec<AnnotatedPayloadField>` if the command can be visualized,
    /// or `None` if the command is not supported by this visualizer.
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError>;

    /// Returns the config for the visualizer.
    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig>;

    /// The identifier for this visualizer.
    fn kind(&self) -> VisualizerKind;

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

/// Result of a successful visualization attempt, including which visualizer handled it.
#[derive(Debug, Clone)]
pub struct VisualizeResult {
    pub field: Vec<AnnotatedPayloadField>,
    pub kind: VisualizerKind,
}

/// Tries multiple visualizers in order and returns the first successful visualization.
///
/// # Arguments
/// * `visualizers` - Slice of visualizer trait objects.
/// * `context` - The visualization context.
///
/// # Returns
/// * `Some(VisualizeResult)` if any visualizer can handle the command, including which one.
/// * `None` if none can handle it.
pub fn visualize_with_any(
    visualizers: &[&dyn CommandVisualizer],
    context: &VisualizerContext,
) -> Option<Result<VisualizeResult, VisualSignError>> {
    visualizers.iter().find_map(|v| {
        if !v.can_handle(context) {
            return None;
        }

        tracing::debug!(
            "Handling command {:?} with visualizer {:?}",
            context
                .commands()
                .get(context.command_index())
                .map(std::string::ToString::to_string),
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
