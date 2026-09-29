mod config;

use config::{
    BitcoinNetwork, Config, DepositFunctions, HASHI_CONFIG, HashiDeployment, HashiModules,
    WithdrawFunctions, deployment_for,
};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext, VisualizerKind};
use crate::utils::{decode_number, get_object_value, pure_bcs_bytes};

use bech32::{ToBase32, Variant, u5};
use move_core_types::language_storage::TypeTag;
use sui_json_rpc_types::{
    SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall, SuiReservation, SuiWithdrawFrom,
    SuiWithdrawalTypeArg,
};
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

const P2WPKH_PROGRAM_LEN: usize = 20;
const P2TR_PROGRAM_LEN: usize = 32;

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
            HashiModules::Withdraw => match pwc.function.as_str().try_into()? {
                WithdrawFunctions::RequestWithdrawal => {
                    Self::handle_request_withdrawal(context, pwc, deployment)
                }
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

    fn handle_request_withdrawal(
        context: &VisualizerContext,
        pwc: &SuiProgrammableMoveCall,
        deployment: &HashiDeployment,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let network = deployment.bitcoin_network;
        let hashi_object = get_object_value(&pwc.arguments, context.inputs(), 0)?;
        let amount_sats =
            resolve_balance_amount(context, deployment, argument_at(&pwc.arguments, 2, "btc")?)?;
        let address_argument = argument_at(&pwc.arguments, 3, "bitcoin_address")?;
        ensure_unmodified_before(
            context,
            deployment,
            address_argument,
            context.command_index(),
            "bitcoin_address",
        )?;
        let address_bytes =
            pure_bcs_bytes(input_at(context, address_argument, "bitcoin_address")?)?;
        let bitcoin_address = encode_bitcoin_address(&address_bytes, network)?;

        let amount_btc = format_btc(amount_sats);
        let title_text = format!(
            "Hashi Withdraw {amount_btc} {HBTC_SYMBOL}{}",
            network.title_suffix()
        );
        let subtitle_text = format!("To {bitcoin_address}");

        let condensed = SignablePayloadFieldListLayout {
            fields: vec![create_text_field(
                "Summary",
                &format!(
                    "Request a withdrawal of {amount_btc} {HBTC_SYMBOL} to {bitcoin_address} on {}. The bridge sends the BTC minus the Bitcoin miner fee.",
                    network.display_name()
                ),
            )?],
        };

        let expanded = SignablePayloadFieldListLayout {
            fields: vec![
                create_amount_field("Withdrawal Amount", &amount_btc, HBTC_SYMBOL)?,
                create_amount_field(
                    "Withdrawal Amount (sats)",
                    &amount_sats.to_string(),
                    SATS_UNIT,
                )?,
                create_address_field(
                    "Bitcoin Recipient",
                    &bitcoin_address,
                    None,
                    None,
                    None,
                    None,
                )?,
                create_text_field("Bitcoin Network", network.display_name())?,
                create_text_field(
                    "Network Fee",
                    "The Bitcoin miner fee is subtracted from the withdrawal amount",
                )?,
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
            "Hashi Withdrawal",
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

enum BalanceFunding {
    Coin,
    AddressBalance,
}

/// Follows the `Balance<BTC>` argument of `withdraw::request_withdrawal` back to
/// the amount it carries: through `coin::into_balance<BTC>` to the `SplitCoins`
/// amount the coin was cut from or the reservation of the `coin::redeem_funds`
/// that produced it, or through `balance::redeem_funds<BTC>` to the reserved
/// amount of the address-balance withdrawal. Any other funding shape, such as
/// passing a whole coin to `into_balance`, is reported as undecodable rather
/// than guessed.
fn resolve_balance_amount(
    context: &VisualizerContext,
    deployment: &HashiDeployment,
    balance_argument: SuiArgument,
) -> Result<u64, VisualSignError> {
    let (producer_index, producer, funding) =
        match producing_command(context, context.command_index(), balance_argument, "btc")? {
            (index, SuiCommand::MoveCall(call))
                if call.package == ObjectID::from_single_byte(2) =>
            {
                let funding = match (call.module.as_str(), call.function.as_str()) {
                    ("coin", "into_balance") => BalanceFunding::Coin,
                    ("balance", "redeem_funds") => BalanceFunding::AddressBalance,
                    _ => return Err(unsupported_funding()),
                };
                (index, call, funding)
            }
            _ => return Err(unsupported_funding()),
        };

    let expected_coin_type = hbtc_coin_type(deployment)?;
    if !producer
        .type_arguments
        .first()
        .is_some_and(|coin_type| coin_type_matches(coin_type, &expected_coin_type))
    {
        return Err(VisualSignError::MissingData(format!(
            "Withdrawal balance is not a Balance<{expected_coin_type}>"
        )));
    }
    ensure_unmodified_before(
        context,
        deployment,
        balance_argument,
        context.command_index(),
        "btc",
    )?;

    match funding {
        BalanceFunding::Coin => resolve_coin_amount(
            context,
            deployment,
            producer_index,
            producer,
            &expected_coin_type,
        ),
        BalanceFunding::AddressBalance => resolve_redeemed_amount(
            context,
            deployment,
            producer_index,
            producer,
            &expected_coin_type,
        ),
    }
}

fn unsupported_funding() -> VisualSignError {
    VisualSignError::MissingData(
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds"
            .into(),
    )
}

fn hbtc_coin_type(deployment: &HashiDeployment) -> Result<String, VisualSignError> {
    let type_origin = deployment.type_origin().ok_or_else(|| {
        VisualSignError::MissingData("Hashi deployment has an invalid type origin id".into())
    })?;
    Ok(format!("{}::btc::BTC", type_origin.to_hex_literal()))
}

/// `redeem_funds` withdraws exactly the withdrawal's limit, which starts as the
/// signed reservation amount. `withdrawal_split` and `withdrawal_join` change
/// that limit, so any earlier command that may modify the input is rejected.
fn resolve_redeemed_amount(
    context: &VisualizerContext,
    deployment: &HashiDeployment,
    redeem_index: usize,
    redeem: &SuiProgrammableMoveCall,
    expected_coin_type: &str,
) -> Result<u64, VisualSignError> {
    let withdrawal_argument = argument_at(&redeem.arguments, 0, "withdrawal")?;
    let SuiCallArg::FundsWithdrawal(withdrawal) =
        input_at(context, withdrawal_argument, "withdrawal")?
    else {
        return Err(VisualSignError::MissingData(
            "Redeemed withdrawal is not a funds withdrawal input".into(),
        ));
    };
    let SuiWithdrawalTypeArg::Balance(balance_type) = &withdrawal.type_arg;
    let balance_type: TypeTag = balance_type
        .clone()
        .try_into()
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid funds withdrawal type: {e}")))?;
    if !coin_type_matches(&balance_type.to_string(), expected_coin_type) {
        return Err(VisualSignError::MissingData(format!(
            "Funds withdrawal is not denominated in {expected_coin_type}"
        )));
    }
    if withdrawal.withdraw_from != SuiWithdrawFrom::Sender {
        return Err(VisualSignError::MissingData(
            "Withdrawing from the sponsor's address balance is not supported".into(),
        ));
    }
    ensure_unmodified_before(
        context,
        deployment,
        withdrawal_argument,
        redeem_index,
        "withdrawal",
    )?;
    let SuiReservation::MaxAmountU64(amount) = withdrawal.reservation;
    Ok(amount)
}

fn resolve_coin_amount(
    context: &VisualizerContext,
    deployment: &HashiDeployment,
    into_balance_index: usize,
    into_balance: &SuiProgrammableMoveCall,
    expected_coin_type: &str,
) -> Result<u64, VisualSignError> {
    let coin_argument = argument_at(&into_balance.arguments, 0, "coin")?;
    match coin_argument {
        SuiArgument::GasCoin => {
            return Err(VisualSignError::MissingData(
                "The gas coin cannot fund an hBTC withdrawal".into(),
            ));
        }
        SuiArgument::Input(_) => {
            return Err(VisualSignError::MissingData(
                "Withdrawing a whole coin is not supported: its amount is not part of the transaction"
                    .into(),
            ));
        }
        SuiArgument::Result(_) | SuiArgument::NestedResult(_, _) => {}
    }
    let (coin_producer_index, coin_producer) = producing_command(
        context,
        into_balance_index,
        coin_argument,
        "withdrawal coin",
    )?;
    ensure_unmodified_before(
        context,
        deployment,
        coin_argument,
        into_balance_index,
        "withdrawal coin",
    )?;
    let (split_source, amounts) = match coin_producer {
        SuiCommand::SplitCoins(split_source, amounts) => (split_source, amounts),
        SuiCommand::MoveCall(redeem)
            if redeem.package == ObjectID::from_single_byte(2)
                && redeem.module == "coin"
                && redeem.function == "redeem_funds" =>
        {
            return resolve_redeemed_amount(
                context,
                deployment,
                coin_producer_index,
                redeem,
                expected_coin_type,
            );
        }
        _ => {
            return Err(VisualSignError::MissingData(
                "Withdrawal coin is not produced by SplitCoins or 0x2::coin::redeem_funds".into(),
            ));
        }
    };
    if split_from_gas_coin(context, coin_producer_index, *split_source)? {
        return Err(VisualSignError::MissingData(
            "The gas coin cannot fund an hBTC withdrawal".into(),
        ));
    }
    let (_, result_index) = result_reference(coin_argument, "withdrawal coin")?;
    let amount_argument = *amounts.get(result_index).ok_or_else(|| {
        VisualSignError::MissingData("SplitCoins amount for the withdrawal coin not found".into())
    })?;
    ensure_unmodified_before(
        context,
        deployment,
        amount_argument,
        coin_producer_index,
        "split amount",
    )?;

    decode_number::<u64>(input_at(context, amount_argument, "split amount")?)
}

/// A split keeps its source's coin type, so a coin split, directly or through
/// further splits, from the gas coin is `Coin<SUI>`.
fn split_from_gas_coin(
    context: &VisualizerContext,
    split_index: usize,
    split_source: SuiArgument,
) -> Result<bool, VisualSignError> {
    let (mut consumer, mut source) = (split_index, split_source);
    while result_slot(source).is_some() {
        let (index, command) = producing_command(context, consumer, source, "split source")?;
        let SuiCommand::SplitCoins(parent_source, _) = command else {
            return Ok(false);
        };
        (consumer, source) = (index, *parent_source);
    }
    Ok(matches!(source, SuiArgument::GasCoin))
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

fn coin_type_matches(actual: &str, expected: &str) -> bool {
    let normalize = |coin_type: &str| {
        coin_type.split_once("::").and_then(|(address, rest)| {
            ObjectID::from_hex_literal(address)
                .ok()
                .map(|id| (id, rest.to_string()))
        })
    };
    normalize(actual).is_some_and(|actual| Some(actual) == normalize(expected))
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

/// Matches how the Hashi committee builds the payout script
/// (`script_pubkey_from_witness_program` in the Hashi repo): 20 bytes is a v0
/// P2WPKH (bech32) and 32 bytes is always a v1 P2TR (bech32m), never P2WSH.
fn encode_bitcoin_address(
    bcs_bytes: &[u8],
    network: BitcoinNetwork,
) -> Result<String, VisualSignError> {
    let program: Vec<u8> = bcs::from_bytes(bcs_bytes).map_err(|e| {
        VisualSignError::DecodeError(format!("Invalid bitcoin_address encoding: {e}"))
    })?;
    let (witness_version, variant) = match program.len() {
        P2WPKH_PROGRAM_LEN => (0, Variant::Bech32),
        P2TR_PROGRAM_LEN => (1, Variant::Bech32m),
        other => {
            return Err(VisualSignError::DecodeError(format!(
                "Bitcoin address must be a 20-byte P2WPKH or 32-byte P2TR program, got {other} bytes"
            )));
        }
    };
    let version = u5::try_from_u8(witness_version)
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid witness version: {e}")))?;
    let data: Vec<u5> = std::iter::once(version)
        .chain(program.to_base32())
        .collect();
    bech32::encode(network.bech32_hrp(), data, variant)
        .map_err(|e| VisualSignError::DecodeError(format!("Bitcoin address encoding failed: {e}")))
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
