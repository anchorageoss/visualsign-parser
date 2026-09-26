mod config;

use config::{
    Config, DepositFunctions, HASHI_CONFIG, HashiDeployment, HashiModules, deployment_for,
};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{decode_number, get_object_value, pure_bcs_bytes};

use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall};
use sui_types::base_types::ObjectID;

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{create_address_field, create_amount_field, create_text_field},
};

const HBTC_SYMBOL: &str = "hBTC";
const SATS_UNIT: &str = "sats";
const SATS_PER_BTC: u64 = 100_000_000;

pub struct HashiVisualizer;

impl CommandVisualizer for HashiVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return Err(VisualSignError::MissingData(
                "Expected a `MoveCall` for Hashi parsing".into(),
            ));
        };

        let deployment = deployment_for(&pwc.package).ok_or_else(|| {
            VisualSignError::MissingData(format!(
                "No Hashi deployment configured for package {}",
                pwc.package
            ))
        })?;

        match pwc.module.as_str().try_into()? {
            HashiModules::Deposit => match pwc.function.as_str().try_into()? {
                DepositFunctions::Deposit => Self::handle_deposit(context, pwc, deployment),
            },
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(HASHI_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Bridge("Hashi")
    }
}

impl HashiVisualizer {
    fn handle_deposit(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
        deployment: &HashiDeployment,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let network = deployment.bitcoin_network;
        let hashi_object = get_object_value(&pwc.arguments, context.inputs(), 0)?;
        let utxo = resolve_utxo(context, deployment, argument_at(&pwc.arguments, 1, "utxo")?)?;

        let amount_btc = format_btc(utxo.amount_sats);
        let title_text = format!("Hashi Deposit {amount_btc} BTC{}", network.title_suffix());
        let outpoint = format!("{}:{}", utxo.txid, utxo.vout);

        let (subtitle_text, summary, recipient_field) = match &utxo.recipient {
            Some(recipient) => (
                format!("To {recipient}"),
                format!(
                    "Request {amount_btc} {HBTC_SYMBOL} for {} transaction output {outpoint}, declared as {amount_btc} BTC. The {HBTC_SYMBOL} is minted to {recipient} after the bridge verifies that output.",
                    network.display_name()
                ),
                create_address_field("hBTC Recipient", recipient, None, None, None, None)?,
            ),
            None => (
                "Warning: no hBTC recipient".to_string(),
                format!(
                    "Warning: this deposit names no recipient. The {} transaction output {outpoint}, declared as {amount_btc} BTC, will not be credited to anyone.",
                    network.display_name()
                ),
                create_text_field("hBTC Recipient", "None: no hBTC will be credited")?,
            ),
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field("Summary", &summary)?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_amount_field("Deposit Amount", &amount_btc, "BTC")?,
                create_amount_field(
                    "Deposit Amount (sats)",
                    &utxo.amount_sats.to_string(),
                    SATS_UNIT,
                )?,
                recipient_field,
                create_text_field("Bitcoin Network", network.display_name())?,
                create_text_field("Bitcoin Transaction", &utxo.txid)?,
                create_text_field("Bitcoin Output Index", &utxo.vout.to_string())?,
                create_address_field(
                    "Sender",
                    &context.sender().to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_address_field(
                    "Bridge Object",
                    &hashi_object.to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
                create_address_field(
                    "Bridge Package",
                    &pwc.package.to_string(),
                    None,
                    None,
                    None,
                    None,
                )?,
            ],
        };

        Ok(vec![preview_field(
            title_text,
            subtitle_text,
            "Hashi Deposit",
            condensed,
            expanded,
        )])
    }
}

struct DepositUtxo {
    txid: String,
    vout: u32,
    amount_sats: u64,
    recipient: Option<String>,
}

/// Follows the `Utxo` argument of `deposit::deposit` back to the
/// `utxo::utxo(utxo_id(txid, vout), amount, derivation_path)` calls that built it.
fn resolve_utxo(
    context: &VisualizerContext,
    deployment: &HashiDeployment,
    utxo_argument: SuiArgument,
) -> Result<DepositUtxo, VisualSignError> {
    let (utxo_index, utxo_call) = producing_hashi_call(
        context,
        context.command_index(),
        utxo_argument,
        deployment,
        "utxo",
        "utxo",
    )?;
    let utxo_id_argument = argument_at(&utxo_call.arguments, 0, "utxo_id")?;
    let (utxo_id_index, utxo_id_call) = producing_hashi_call(
        context,
        utxo_index,
        utxo_id_argument,
        deployment,
        "utxo",
        "utxo_id",
    )?;

    let txid_argument = argument_at(&utxo_id_call.arguments, 0, "txid")?;
    let vout_argument = argument_at(&utxo_id_call.arguments, 1, "vout")?;
    let amount_argument = argument_at(&utxo_call.arguments, 1, "amount")?;
    let derivation_path_argument = argument_at(&utxo_call.arguments, 2, "derivation_path")?;
    for (value, consumer, name) in [
        (utxo_argument, context.command_index(), "utxo"),
        (utxo_id_argument, utxo_index, "utxo_id"),
        (txid_argument, utxo_id_index, "txid"),
        (vout_argument, utxo_id_index, "vout"),
        (amount_argument, utxo_index, "amount"),
        (derivation_path_argument, utxo_index, "derivation_path"),
    ] {
        ensure_unmodified_before(context, deployment, value, consumer, name)?;
    }

    let txid = bitcoin_txid_display(&pure_bcs_bytes(input_at(context, txid_argument, "txid")?)?)?;
    let vout = decode_number::<u32>(input_at(context, vout_argument, "vout")?)?;
    let amount_sats = decode_number::<u64>(input_at(context, amount_argument, "amount")?)?;
    let recipient = decode_optional_address(&pure_bcs_bytes(input_at(
        context,
        derivation_path_argument,
        "derivation_path",
    )?)?)?;

    Ok(DepositUtxo {
        txid,
        vout,
        amount_sats,
        recipient,
    })
}

/// Any command can take a pure input or a result by `&mut` and hand a rewritten
/// value to later commands, so a decoded value is only what executes when no
/// command before its consumer may have modified it. A use after the consumer
/// cannot change what the consumer already received.
fn ensure_unmodified_before(
    context: &VisualizerContext,
    deployment: &HashiDeployment,
    value: SuiArgument,
    consumer: usize,
    name: &str,
) -> Result<(), VisualSignError> {
    let modifier = context
        .commands()
        .iter()
        .take(consumer)
        .position(|command| {
            possibly_mutable_arguments(command, deployment)
                .into_iter()
                .any(|argument| refers_to(argument, value))
        });
    match modifier {
        None => Ok(()),
        Some(index) => Err(VisualSignError::MissingData(format!(
            "`{name}` is passed to command {index}, which may modify it before command {consumer} consumes it"
        ))),
    }
}

/// `SplitCoins` amounts and `MakeMoveVec` elements are taken by value. The parser
/// cannot see third-party Move signatures, so every other command argument,
/// including those of any call outside `is_read_only_call`, may be `&mut`.
fn possibly_mutable_arguments(
    command: &SuiCommand,
    deployment: &HashiDeployment,
) -> Vec<SuiArgument> {
    match command {
        SuiCommand::SplitCoins(coin, _) => vec![*coin],
        SuiCommand::MakeMoveVec(_, _) => Vec::new(),
        SuiCommand::MoveCall(call) if is_read_only_call(call, deployment) => Vec::new(),
        other => command_arguments(other),
    }
}

/// `coin::value` and `balance::value` borrow immutably, and Hashi's
/// `utxo::utxo_id` and `utxo::utxo` take every argument by value.
fn is_read_only_call(call: &SuiProgrammableMoveCall, deployment: &HashiDeployment) -> bool {
    let framework_value = call.package == ObjectID::from_single_byte(2)
        && matches!(call.module.as_str(), "coin" | "balance")
        && call.function == "value";
    let hashi_constructor = deployment.owns_package(&call.package)
        && call.module == "utxo"
        && matches!(call.function.as_str(), "utxo_id" | "utxo");
    framework_value || hashi_constructor
}

fn command_arguments(command: &SuiCommand) -> Vec<SuiArgument> {
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

fn refers_to(argument: SuiArgument, value: SuiArgument) -> bool {
    match (argument, value) {
        (SuiArgument::Input(argument), SuiArgument::Input(value)) => argument == value,
        (SuiArgument::GasCoin, SuiArgument::GasCoin) => true,
        _ => result_slot(argument).is_some_and(|slot| Some(slot) == result_slot(value)),
    }
}

/// `Result(n)` and `NestedResult(n, 0)` name the same single-output value.
fn result_slot(argument: SuiArgument) -> Option<(u16, u16)> {
    match argument {
        SuiArgument::Result(command) => Some((command, 0)),
        SuiArgument::NestedResult(command, index) => Some((command, index)),
        _ => None,
    }
}

fn producing_hashi_call<'a>(
    context: &'a VisualizerContext,
    consumer: usize,
    argument: SuiArgument,
    deployment: &HashiDeployment,
    module: &str,
    function: &str,
) -> Result<(usize, &'a SuiProgrammableMoveCall), VisualSignError> {
    match producing_command(context, consumer, argument, function)? {
        (index, SuiCommand::MoveCall(call))
            if deployment.owns_package(&call.package)
                && call.module == module
                && call.function == function =>
        {
            Ok((index, call))
        }
        _ => Err(VisualSignError::MissingData(format!(
            "Expected argument to come from Hashi `{module}::{function}`"
        ))),
    }
}

/// Resolves `argument` to the earlier command that produced it. A reference to
/// the consumer itself or a later command is malformed and rejected.
fn producing_command<'a>(
    context: &'a VisualizerContext,
    consumer: usize,
    argument: SuiArgument,
    name: &str,
) -> Result<(usize, &'a SuiCommand), VisualSignError> {
    let (index, _) = result_reference(argument, name)?;
    if index >= consumer {
        return Err(VisualSignError::MissingData(format!(
            "`{name}` in command {consumer} refers to the result of command {index}, which does not precede it"
        )));
    }
    context
        .commands()
        .get(index)
        .map(|command| (index, command))
        .ok_or_else(|| {
            VisualSignError::MissingData(format!("`{name}` refers to missing command {index}"))
        })
}

fn result_reference(argument: SuiArgument, name: &str) -> Result<(usize, usize), VisualSignError> {
    result_slot(argument)
        .map(|(command, index)| (usize::from(command), usize::from(index)))
        .ok_or_else(|| {
            VisualSignError::MissingData(format!(
                "`{name}` is not the result of a previous command"
            ))
        })
}

fn argument_at(
    arguments: &[SuiArgument],
    index: usize,
    name: &str,
) -> Result<SuiArgument, VisualSignError> {
    arguments
        .get(index)
        .copied()
        .ok_or_else(|| VisualSignError::MissingData(format!("Argument `{name}` not found")))
}

fn input_at<'a>(
    context: &'a VisualizerContext,
    argument: SuiArgument,
    name: &str,
) -> Result<&'a SuiCallArg, VisualSignError> {
    let SuiArgument::Input(index) = argument else {
        return Err(VisualSignError::MissingData(format!(
            "Argument `{name}` is not a transaction input"
        )));
    };
    context
        .inputs()
        .get(usize::from(index))
        .ok_or_else(|| VisualSignError::MissingData(format!("Input for `{name}` not found")))
}

/// Bitcoin displays txids byte-reversed relative to their internal order,
/// which is the order Hashi stores in the Move `address`.
fn bitcoin_txid_display(bcs_bytes: &[u8]) -> Result<String, VisualSignError> {
    let txid: [u8; 32] = bcs_bytes.try_into().map_err(|_| {
        VisualSignError::DecodeError(format!(
            "Bitcoin txid must be 32 bytes, got {}",
            bcs_bytes.len()
        ))
    })?;
    let mut display_order = txid;
    display_order.reverse();
    Ok(hex::encode(display_order))
}

fn decode_optional_address(bcs_bytes: &[u8]) -> Result<Option<String>, VisualSignError> {
    let path: Option<[u8; 32]> = bcs::from_bytes(bcs_bytes).map_err(|e| {
        VisualSignError::DecodeError(format!("Invalid derivation_path encoding: {e}"))
    })?;
    Ok(path.map(|address| ObjectID::new(address).to_string()))
}

fn format_btc(sats: u64) -> String {
    let whole = sats / SATS_PER_BTC;
    let fraction = sats % SATS_PER_BTC;
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction_digits = format!("{fraction:08}");
    format!("{whole}.{}", fraction_digits.trim_end_matches('0'))
}

fn preview_field(
    title_text: String,
    subtitle_text: String,
    label: &str,
    condensed: SignablePayloadFieldListLayout,
    expanded: SignablePayloadFieldListLayout,
) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: title_text.clone(),
                label: label.to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 { text: title_text }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: subtitle_text,
                }),
                condensed: Some(condensed),
                expanded: Some(expanded),
            },
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
