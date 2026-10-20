//! Dataflow checks over a PTB's commands: which command produced a value,
//! which single command consumes it, and whether any command in between may
//! have modified it.

use crate::core::VisualizerContext;
use crate::utils::pure_bcs_bytes;

use move_core_types::language_storage::TypeTag;
use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall};
use sui_types::base_types::{ObjectID, SuiAddress};
use visualsign::errors::VisualSignError;

pub(super) fn missing(message: impl Into<String>) -> VisualSignError {
    VisualSignError::MissingData(message.into())
}

pub(super) fn is_call(
    call: &SuiProgrammableMoveCall,
    package: u8,
    module: &str,
    function: &str,
) -> bool {
    call.package == ObjectID::from_single_byte(package)
        && call.module == module
        && call.function == function
}

/// True when `actual` and `expected` name the same Move type, however each
/// spells its addresses.
pub(super) fn type_matches(actual: &str, expected: &str) -> bool {
    match (
        sui_types::parse_sui_type_tag(actual),
        sui_types::parse_sui_type_tag(expected),
    ) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => false,
    }
}

pub(super) fn first_type_argument_is(call: &SuiProgrammableMoveCall, expected: &str) -> bool {
    call.type_arguments
        .first()
        .is_some_and(|actual| type_matches(actual, expected))
}

pub(super) fn argument_at(
    arguments: &[SuiArgument],
    index: usize,
    name: &str,
) -> Result<SuiArgument, VisualSignError> {
    arguments
        .get(index)
        .copied()
        .ok_or_else(|| missing(format!("Argument `{name}` not found")))
}

pub(super) fn input_at<'a>(
    context: &'a VisualizerContext,
    argument: SuiArgument,
    name: &str,
) -> Result<&'a SuiCallArg, VisualSignError> {
    let SuiArgument::Input(index) = argument else {
        return Err(missing(format!(
            "Argument `{name}` is not a transaction input"
        )));
    };
    context
        .inputs()
        .get(usize::from(index))
        .ok_or_else(|| missing(format!("Input for `{name}` not found")))
}

/// Decodes an address input, whether the RPC view typed it as `address` or
/// left it as raw BCS bytes.
pub(super) fn pure_address(input: &SuiCallArg) -> Result<SuiAddress, VisualSignError> {
    if let SuiCallArg::Pure(value) = input
        && value.value_type() == Some(TypeTag::Address)
    {
        return match value.value().to_json_value() {
            serde_json::Value::String(address) => address
                .parse::<SuiAddress>()
                .map_err(|e| VisualSignError::DecodeError(format!("Invalid address: {e}"))),
            _ => Err(VisualSignError::DecodeError(
                "Typed address input is not a string".into(),
            )),
        };
    }
    bcs::from_bytes::<SuiAddress>(&pure_bcs_bytes(input)?)
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid address: {e}")))
}

/// `Result(n)` and `NestedResult(n, 0)` name the same single-output value.
pub(super) fn result_slot(argument: SuiArgument) -> Option<(u16, u16)> {
    match argument {
        SuiArgument::Result(command) => Some((command, 0)),
        SuiArgument::NestedResult(command, index) => Some((command, index)),
        _ => None,
    }
}

pub(super) fn refers_to(argument: SuiArgument, value: SuiArgument) -> bool {
    match (argument, value) {
        (SuiArgument::Input(argument), SuiArgument::Input(value)) => argument == value,
        (SuiArgument::GasCoin, SuiArgument::GasCoin) => true,
        _ => result_slot(argument).is_some_and(|slot| Some(slot) == result_slot(value)),
    }
}

/// The value at output `slot` of command `producer`.
pub(super) fn output(producer: usize, slot: u16) -> Result<SuiArgument, VisualSignError> {
    u16::try_from(producer)
        .map(|producer| SuiArgument::NestedResult(producer, slot))
        .map_err(|_| missing(format!("Command index {producer} is out of range")))
}

/// Resolves `argument` to the earlier command that produced it. A reference to
/// the consumer itself or a later command is malformed and rejected.
pub(super) fn producing_command<'a>(
    context: &'a VisualizerContext,
    consumer: usize,
    argument: SuiArgument,
    name: &str,
) -> Result<(usize, &'a SuiCommand), VisualSignError> {
    let (index, _) = result_slot(argument)
        .map(|(command, slot)| (usize::from(command), slot))
        .ok_or_else(|| missing(format!("`{name}` is not the result of a previous command")))?;
    if index >= consumer {
        return Err(missing(format!(
            "`{name}` in command {consumer} refers to the result of command {index}, which does not precede it"
        )));
    }
    context
        .commands()
        .get(index)
        .map(|command| (index, command))
        .ok_or_else(|| missing(format!("`{name}` refers to missing command {index}")))
}

/// The one command after `producer` that uses `value`. A value used twice, or
/// never, is rejected: the preview could not say where it ends up.
pub(super) fn only_consumer<'a>(
    context: &'a VisualizerContext,
    producer: usize,
    value: SuiArgument,
    name: &str,
) -> Result<(usize, &'a SuiCommand), VisualSignError> {
    let mut consumers = context
        .commands()
        .iter()
        .enumerate()
        .skip(producer + 1)
        .filter(|(_, command)| {
            command_arguments(command)
                .into_iter()
                .any(|argument| refers_to(argument, value))
        });
    match (consumers.next(), consumers.next()) {
        (Some(consumer), None) => Ok(consumer),
        _ => Err(missing(format!(
            "The {name} from command {producer} must be used by exactly one later command"
        ))),
    }
}

/// Any command can take a pure input or a result by `&mut` and hand a rewritten
/// value to later commands, so a decoded value is only what executes when no
/// command before its consumer may have modified it. A use after the consumer
/// cannot change what the consumer already received.
pub(super) fn ensure_unmodified_before(
    context: &VisualizerContext,
    value: SuiArgument,
    consumer: usize,
    name: &str,
    takes_by_value: impl Fn(&SuiProgrammableMoveCall) -> bool,
) -> Result<(), VisualSignError> {
    let modifier = context
        .commands()
        .iter()
        .take(consumer)
        .position(|command| {
            possibly_mutable_arguments(command, &takes_by_value)
                .into_iter()
                .any(|argument| refers_to(argument, value))
        });
    match modifier {
        None => Ok(()),
        Some(index) => Err(missing(format!(
            "`{name}` is passed to command {index}, which may modify it before command {consumer} consumes it"
        ))),
    }
}

/// `SplitCoins` amounts, `MakeMoveVec` elements and the objects and recipient
/// of `TransferObjects` are taken by value. The parser cannot see third-party
/// Move signatures, so every other command argument, including those of any
/// call `takes_by_value` does not vouch for, may be `&mut`.
fn possibly_mutable_arguments(
    command: &SuiCommand,
    takes_by_value: &impl Fn(&SuiProgrammableMoveCall) -> bool,
) -> Vec<SuiArgument> {
    match command {
        SuiCommand::SplitCoins(coin, _) => vec![*coin],
        SuiCommand::MakeMoveVec(_, _) | SuiCommand::TransferObjects(_, _) => Vec::new(),
        SuiCommand::MoveCall(call) if takes_by_value(call) => Vec::new(),
        other => command_arguments(other),
    }
}

pub(super) fn command_arguments(command: &SuiCommand) -> Vec<SuiArgument> {
    match command {
        SuiCommand::MoveCall(call) => call.arguments.clone(),
        SuiCommand::TransferObjects(objects, recipient) => {
            objects.iter().copied().chain([*recipient]).collect()
        }
        SuiCommand::SplitCoins(coin, amounts) | SuiCommand::MergeCoins(coin, amounts) => {
            std::iter::once(*coin)
                .chain(amounts.iter().copied())
                .collect()
        }
        SuiCommand::Upgrade(_, _, ticket) => vec![*ticket],
        SuiCommand::MakeMoveVec(_, elements) => elements.clone(),
        SuiCommand::Publish(_) => Vec::new(),
    }
}
