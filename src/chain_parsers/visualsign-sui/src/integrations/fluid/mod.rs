//! Previews for Fluid vault `user::operate` calls on Sui. Fluid's Sui sources are not
//! public, so the semantics below are read from the published vaults package's bytecode.

mod config;
mod ptb;

use config::{
    Config, FLUID_CONFIG, FluidDeployment, KNOWN_COINS, KnownCoin, VaultFunctions, deployment_for,
};
use ptb::{
    argument_at, ensure_unmodified_before, first_type_argument_is, input_at, is_call, missing,
    only_consumer, output, producing_command, pure_address, refers_to, result_slot, type_matches,
    uses_after,
};

use crate::core::{
    CommandVisualizer, SuiIntegrationConfig, TransactionSummary, VisualizerContext, VisualizerKind,
};
use crate::utils::decode_number;

use sui_json_rpc_types::{
    SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall, SuiReservation, SuiWithdrawFrom,
};
use sui_types::base_types::{ObjectID, SuiAddress};

use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
    errors::VisualSignError,
    field_builders::{create_address_field, create_amount_field, create_text_field},
};

const FLUID_DISPLAY_NAME: &str = "Fluid";

const STDLIB: u8 = 1;
const SUI_FRAMEWORK: u8 = 2;

// `operate(registry, market, vault_registry, col_reserve, debt_reserve, position,
// cap: Option<PositionOwnerCap>, price, col: Balance<C>, debt: Balance<D>,
// col_amount: OperateAmount, debt_amount: OperateAmount, clock, ctx)
// -> (Balance<C>, Balance<D>, Option<PositionOwnerCap>)`.
const OPERATE_MARKET_ARG: usize = 1;
const OPERATE_POSITION_ARG: usize = 5;
const OPERATE_COL_AMOUNT_ARG: usize = 10;
const OPERATE_DEBT_AMOUNT_ARG: usize = 11;
const OPERATE_COL_OUTPUT: u16 = 0;
const OPERATE_DEBT_OUTPUT: u16 = 1;
const OPERATE_CAP_OUTPUT: u16 = 2;

pub struct FluidVisualizer;

impl CommandVisualizer for FluidVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (pwc, deployment) = resolve_fluid_call(context)?;
        let operation = resolve_operate(context, pwc, deployment)?;
        Ok(vec![preview_field(context, &operation)?])
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(FLUID_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(FLUID_DISPLAY_NAME)
    }
}

fn resolve_fluid_call<'a>(
    context: &'a VisualizerContext,
) -> Result<(&'a SuiProgrammableMoveCall, &'static FluidDeployment), VisualSignError> {
    let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index()) else {
        return Err(missing("Expected a `MoveCall` for Fluid parsing"));
    };
    let deployment = deployment_for(&pwc.package).ok_or_else(|| {
        missing(format!(
            "No Fluid deployment configured for package {}",
            pwc.package
        ))
    })?;
    match pwc.function.as_str().try_into()? {
        VaultFunctions::Operate => Ok((pwc, deployment)),
    }
}

struct Coin {
    coin_type: String,
    known: Option<&'static KnownCoin>,
}

impl Coin {
    fn from_type_argument(
        call: &SuiProgrammableMoveCall,
        index: usize,
    ) -> Result<Self, VisualSignError> {
        let coin_type = call
            .type_arguments
            .get(index)
            .ok_or_else(|| missing(format!("Type argument {index} not found")))?;
        Ok(Self {
            coin_type: coin_type.clone(),
            known: KNOWN_COINS
                .iter()
                .find(|known| type_matches(coin_type, known.coin_type)),
        })
    }

    fn sui() -> Self {
        let coin_type = "0x2::sui::SUI";
        Self {
            coin_type: coin_type.to_string(),
            known: KNOWN_COINS
                .iter()
                .find(|known| type_matches(coin_type, known.coin_type)),
        }
    }

    fn symbol(&self) -> String {
        self.known
            .map_or_else(|| self.coin_type.clone(), |known| known.symbol.to_string())
    }

    fn amount(&self, base_units: u64) -> (String, String) {
        match self.known {
            Some(known) => (
                format_units(base_units, known.decimals),
                known.symbol.to_string(),
            ),
            None => (
                base_units.to_string(),
                format!("base units of {}", self.coin_type),
            ),
        }
    }

    fn display(&self, base_units: u64) -> String {
        let (amount, unit) = self.amount(base_units);
        format!("{amount} {unit}")
    }

    fn amount_field(
        &self,
        label: &str,
        base_units: u64,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let (amount, unit) = self.amount(base_units);
        create_amount_field(label, &amount, &unit)
    }
}

fn format_units(base_units: u64, decimals: u32) -> String {
    let scale = 10_u64.pow(decimals);
    let whole = base_units / scale;
    let fraction = base_units % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction_digits = format!("{fraction:0width$}", width = decimals as usize);
    format!("{whole}.{}", fraction_digits.trim_end_matches('0'))
}

/// `positive(0)` and `negative(0)` build `none`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperateAmount {
    None,
    Positive(u64),
    Negative(u64),
    Max,
}

/// `max` withdraws all collateral or repays all debt (`position::get_new_position_info`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Collateral,
    Debt,
}

impl Side {
    fn arg_index(self) -> usize {
        match self {
            Side::Collateral => OPERATE_COL_AMOUNT_ARG,
            Side::Debt => OPERATE_DEBT_AMOUNT_ARG,
        }
    }

    fn output_slot(self) -> u16 {
        match self {
            Side::Collateral => OPERATE_COL_OUTPUT,
            Side::Debt => OPERATE_DEBT_OUTPUT,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Side::Collateral => "col_amount",
            Side::Debt => "debt_amount",
        }
    }

    fn balance_name(self) -> &'static str {
        match self {
            Side::Collateral => "collateral balance",
            Side::Debt => "debt balance",
        }
    }

    fn action(self, amount: OperateAmount, coin: &Coin) -> Option<String> {
        let symbol = coin.symbol();
        match (self, amount) {
            (_, OperateAmount::None) => None,
            (Side::Collateral, OperateAmount::Positive(value)) => {
                Some(format!("Deposit {} collateral", coin.display(value)))
            }
            (Side::Collateral, OperateAmount::Negative(value)) => {
                Some(format!("Withdraw {} collateral", coin.display(value)))
            }
            (Side::Collateral, OperateAmount::Max) => {
                Some(format!("Withdraw all {symbol} collateral"))
            }
            (Side::Debt, OperateAmount::Positive(value)) => {
                Some(format!("Borrow {}", coin.display(value)))
            }
            (Side::Debt, OperateAmount::Negative(value)) => {
                Some(format!("Repay {}", coin.display(value)))
            }
            (Side::Debt, OperateAmount::Max) => Some(format!("Repay all {symbol} debt")),
        }
    }

    fn row(
        self,
        amount: OperateAmount,
        coin: &Coin,
    ) -> Result<Option<AnnotatedPayloadField>, VisualSignError> {
        let symbol = coin.symbol();
        let row = match (self, amount) {
            (_, OperateAmount::None) => return Ok(None),
            (Side::Collateral, OperateAmount::Positive(value)) => {
                coin.amount_field("Collateral Deposit", value)?
            }
            (Side::Collateral, OperateAmount::Negative(value)) => {
                coin.amount_field("Collateral Withdrawal", value)?
            }
            (Side::Collateral, OperateAmount::Max) => {
                create_text_field("Collateral Withdrawal", &format!("All {symbol} collateral"))?
            }
            (Side::Debt, OperateAmount::Positive(value)) => coin.amount_field("Borrow", value)?,
            (Side::Debt, OperateAmount::Negative(value)) => coin.amount_field("Repayment", value)?,
            (Side::Debt, OperateAmount::Max) => {
                create_text_field("Repayment", &format!("All {symbol} debt"))?
            }
        };
        Ok(Some(row))
    }

    fn pays_out(self, amount: OperateAmount) -> bool {
        matches!(
            (self, amount),
            (
                Side::Collateral,
                OperateAmount::Negative(_) | OperateAmount::Max
            ) | (Side::Debt, OperateAmount::Positive(_))
        )
    }
}

fn is_amount_constructor(call: &SuiProgrammableMoveCall, deployment: &FluidDeployment) -> bool {
    deployment.is_vaults_package(&call.package)
        && call.module == "structs"
        && matches!(
            call.function.as_str(),
            "none" | "positive" | "negative" | "max"
        )
}

fn takes_by_value(call: &SuiProgrammableMoveCall, deployment: &FluidDeployment) -> bool {
    let framework = call.package == ObjectID::from_single_byte(SUI_FRAMEWORK)
        && matches!(call.module.as_str(), "coin" | "balance")
        && matches!(
            call.function.as_str(),
            "value"
                | "zero"
                | "redeem_funds"
                | "send_funds"
                | "from_balance"
                | "into_balance"
                | "destroy_zero"
        );
    let transfer = is_call(call, SUI_FRAMEWORK, "transfer", "public_transfer");
    let option = call.package == ObjectID::from_single_byte(STDLIB)
        && call.module == "option"
        && matches!(
            call.function.as_str(),
            "some" | "none" | "destroy_some" | "destroy_none"
        );
    framework || transfer || option || is_amount_constructor(call, deployment)
}

fn resolve_operate_amount(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
    side: Side,
) -> Result<OperateAmount, VisualSignError> {
    let name = side.name();
    let operate_index = context.command_index();
    let argument = argument_at(&pwc.arguments, side.arg_index(), name)?;
    let (constructor_index, constructor) =
        match producing_command(context, operate_index, argument, name)? {
            (index, SuiCommand::MoveCall(call)) if is_amount_constructor(call, deployment) => {
                (index, call)
            }
            _ => {
                return Err(missing(format!(
                    "`{name}` is not built by Fluid's structs::{{none, positive, negative, max}}"
                )));
            }
        };
    ensure_unmodified_before(context, argument, operate_index, name, |call| {
        takes_by_value(call, deployment)
    })?;
    // A value computed on chain, such as a swap output, isn't in the transaction.
    let value = |context: &VisualizerContext| -> Result<u64, VisualSignError> {
        let value_argument = argument_at(&constructor.arguments, 0, name)?;
        ensure_unmodified_before(context, value_argument, constructor_index, name, |call| {
            takes_by_value(call, deployment)
        })?;
        decode_number::<u64>(input_at(context, value_argument, name)?)
    };
    Ok(match constructor.function.as_str() {
        "positive" => match value(context)? {
            0 => OperateAmount::None,
            value => OperateAmount::Positive(value),
        },
        "negative" => match value(context)? {
            0 => OperateAmount::None,
            value => OperateAmount::Negative(value),
        },
        "max" => OperateAmount::Max,
        _ => OperateAmount::None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Destination {
    /// `also` lists other funds or objects that reach the recipient with this output.
    Address {
        recipient: SuiAddress,
        also: Vec<String>,
    },
    /// `destroy_zero` aborts unless the balance is empty.
    Destroyed,
}

/// A `TransferObjects` command and the traced object in it.
type TransferClaim = (usize, SuiArgument);

/// Follows a `Balance<T>` to its recipient, noting any other funds joined in along the way.
fn trace_funds(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    producer: usize,
    slot: u16,
    coin: &Coin,
    name: &str,
) -> Result<(Destination, Option<TransferClaim>), VisualSignError> {
    let unsupported = || {
        missing(format!(
            "The {name} from command {producer} is not sent to an address through 0x2::balance, 0x2::coin or 0x2::transfer"
        ))
    };
    let used_once = || {
        missing(format!(
            "The {name} from command {producer} must be used by exactly one later command"
        ))
    };
    let coin_type = coin.coin_type.as_str();
    let mut value = output(producer, slot)?;
    let mut after = producer;
    let mut is_coin = false;
    let mut also = Vec::new();
    loop {
        let Some((index, command)) =
            find_next_move(context, after, value, is_coin, coin, &mut also)?
        else {
            return Err(used_once());
        };
        let first_is_value = |call: &SuiProgrammableMoveCall| {
            call.arguments
                .first()
                .is_some_and(|first| refers_to(*first, value))
        };
        let send = |arguments: &[SuiArgument], at: usize, also: Vec<String>| {
            resolve_recipient(context, deployment, arguments, at, index)
                .map(|recipient| Destination::Address { recipient, also })
        };
        match command {
            SuiCommand::MoveCall(call)
                if !is_coin
                    && is_call(call, SUI_FRAMEWORK, "balance", "send_funds")
                    && first_type_argument_is(call, coin_type)
                    && first_is_value(call) =>
            {
                return Ok((send(&call.arguments, 1, also)?, None));
            }
            SuiCommand::MoveCall(call)
                if is_coin
                    && (is_call(call, SUI_FRAMEWORK, "coin", "send_funds")
                        || is_call(call, SUI_FRAMEWORK, "transfer", "public_transfer"))
                    && first_is_value(call) =>
            {
                return Ok((send(&call.arguments, 1, also)?, None));
            }
            SuiCommand::MoveCall(call)
                if !is_coin
                    && is_call(call, SUI_FRAMEWORK, "balance", "destroy_zero")
                    && first_type_argument_is(call, coin_type)
                    && first_is_value(call) =>
            {
                return Ok((Destination::Destroyed, None));
            }
            SuiCommand::MoveCall(call)
                if !is_coin
                    && is_call(call, SUI_FRAMEWORK, "coin", "from_balance")
                    && first_type_argument_is(call, coin_type)
                    && first_is_value(call) =>
            {
                value = output(index, 0)?;
                after = index;
                is_coin = true;
            }
            SuiCommand::MoveCall(call)
                if !is_coin
                    && is_call(call, SUI_FRAMEWORK, "balance", "join")
                    && first_type_argument_is(call, coin_type)
                    && call
                        .arguments
                        .get(1)
                        .is_some_and(|second| refers_to(*second, value)) =>
            {
                let into = argument_at(&call.arguments, 0, "balance")?;
                if result_slot(into).is_none() {
                    return Err(unsupported());
                }
                if !is_untouched_zero(context, into, index, coin_type) {
                    also.extend(describe_balance(context, into, index, coin, 0));
                }
                value = into;
                after = index;
            }
            SuiCommand::TransferObjects(objects, to)
                if is_coin && objects.iter().any(|object| refers_to(*object, value)) =>
            {
                return Ok((send(&[*to], 0, also)?, Some((index, value))));
            }
            _ => return Err(unsupported()),
        }
    }
}

/// `None` when `value` moves more or less than once.
fn find_next_move<'a>(
    context: &'a VisualizerContext,
    after: usize,
    value: SuiArgument,
    is_coin: bool,
    coin: &Coin,
    also: &mut Vec<String>,
) -> Result<Option<(usize, &'a SuiCommand)>, VisualSignError> {
    let mut moves = Vec::new();
    for (index, command) in uses_after(context, after, value) {
        match command {
            SuiCommand::MoveCall(call)
                if !is_coin
                    && is_call(call, SUI_FRAMEWORK, "balance", "join")
                    && call
                        .arguments
                        .first()
                        .is_some_and(|first| refers_to(*first, value)) =>
            {
                let other = argument_at(&call.arguments, 1, "balance")?;
                also.extend(describe_balance(context, other, index, coin, 0));
            }
            _ => moves.push((index, command)),
        }
    }
    Ok(match moves.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// True when `target` is a `balance::zero<T>` no command used before `join_index`.
fn is_untouched_zero(
    context: &VisualizerContext,
    target: SuiArgument,
    join_index: usize,
    coin_type: &str,
) -> bool {
    let Ok((zero_index, SuiCommand::MoveCall(zero))) =
        producing_command(context, join_index, target, "balance")
    else {
        return false;
    };
    is_call(zero, SUI_FRAMEWORK, "balance", "zero")
        && first_type_argument_is(zero, coin_type)
        && uses_after(context, zero_index, target)
            .first()
            .is_some_and(|(first, _)| *first == join_index)
}

const DESCRIBE_DEPTH: usize = 8;
const AMOUNT_NOT_IN_TRANSACTION: &str = "amount not in transaction";

/// `None` for an untouched `balance::zero`, which adds nothing.
fn describe_balance(
    context: &VisualizerContext,
    balance: SuiArgument,
    consumer: usize,
    coin: &Coin,
    depth: usize,
) -> Option<String> {
    let symbol = coin.symbol();
    let unknown = || format!("other {symbol} ({AMOUNT_NOT_IN_TRANSACTION})");
    if depth > DESCRIBE_DEPTH {
        return Some(unknown());
    }
    let Ok((index, SuiCommand::MoveCall(call))) =
        producing_command(context, consumer, balance, "balance")
    else {
        return Some(unknown());
    };
    let changes = find_changes(context, balance, index, consumer, Some(coin), depth);
    let base = match (call.module.as_str(), call.function.as_str()) {
        ("balance", "zero") if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK) => {
            return match changes {
                Changes::Unchanged | Changes::Reduced => None,
                Changes::Added(added) => Some(added.join(" plus ")),
                Changes::Unknown => Some(unknown()),
            };
        }
        ("coin", "into_balance") if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK) => {
            let source = argument_at(&call.arguments, 0, "coin").ok()?;
            describe_coin(context, source, index, Some(coin), depth + 1)
        }
        ("balance", "redeem_funds") if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK) => {
            describe_redeemed(context, call, coin)
        }
        ("balance", "split") if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK) => {
            let amount = call
                .arguments
                .get(1)
                .and_then(|amount| decode_pure_amount(context, *amount, index));
            match amount {
                Some(amount) => format!("{} split from another balance", coin.display(amount)),
                None => unknown(),
            }
        }
        (module, function) => format!(
            "{symbol} returned by {module}::{function} at command {index} ({AMOUNT_NOT_IN_TRANSACTION})"
        ),
    };
    Some(changes.qualify(base))
}

/// How a coin or balance changed between the command that made it and `consumer`.
enum Changes {
    Unchanged,
    Reduced,
    Added(Vec<String>),
    Unknown,
}

impl Changes {
    fn qualify(self, base: String) -> String {
        match self {
            Changes::Unchanged => base,
            Changes::Reduced => format!("what remains of {base}"),
            Changes::Added(added) => std::iter::once(base)
                .chain(added)
                .collect::<Vec<_>>()
                .join(" plus "),
            Changes::Unknown => {
                format!("{base}, changed by later commands ({AMOUNT_NOT_IN_TRANSACTION})")
            }
        }
    }
}

/// A mix of reductions and additions, or any other command, leaves the amount unknown.
fn find_changes(
    context: &VisualizerContext,
    value: SuiArgument,
    producer: usize,
    consumer: usize,
    coin: Option<&Coin>,
    depth: usize,
) -> Changes {
    let mut reduced = false;
    let mut added = Vec::new();
    for (index, command) in uses_after(context, producer, value) {
        if index >= consumer {
            break;
        }
        match command {
            SuiCommand::SplitCoins(source, _) if refers_to(*source, value) => reduced = true,
            SuiCommand::MergeCoins(target, others) if refers_to(*target, value) => {
                added.extend(
                    others
                        .iter()
                        .map(|other| describe_coin(context, *other, index, coin, depth + 1)),
                );
            }
            SuiCommand::MoveCall(call)
                if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK)
                    && matches!(call.module.as_str(), "coin" | "balance")
                    && call
                        .arguments
                        .first()
                        .is_some_and(|first| refers_to(*first, value)) =>
            {
                match (call.module.as_str(), call.function.as_str(), call.arguments.get(1)) {
                    (_, "value", _) => {}
                    (_, "split", _) => reduced = true,
                    ("balance", "join", Some(other)) => match coin {
                        Some(coin) => {
                            added.extend(describe_balance(context, *other, index, coin, depth + 1));
                        }
                        None => return Changes::Unknown,
                    },
                    ("coin", "join", Some(other)) => {
                        added.push(describe_coin(context, *other, index, coin, depth + 1));
                    }
                    _ => return Changes::Unknown,
                }
            }
            _ => return Changes::Unknown,
        }
    }
    match (reduced, added.is_empty()) {
        (false, true) => Changes::Unchanged,
        (true, true) => Changes::Reduced,
        (false, false) => Changes::Added(added),
        (true, false) => Changes::Unknown,
    }
}

fn describe_coin(
    context: &VisualizerContext,
    object: SuiArgument,
    consumer: usize,
    coin: Option<&Coin>,
    depth: usize,
) -> String {
    let sui = Coin::sui();
    match object {
        SuiArgument::GasCoin => "the gas coin's whole balance".to_string(),
        SuiArgument::Input(_) => match input_at(context, object, "object").ok().and_then(SuiCallArg::object) {
            Some(id) => format!("object {id} ({AMOUNT_NOT_IN_TRANSACTION})"),
            None => format!("an input ({AMOUNT_NOT_IN_TRANSACTION})"),
        },
        SuiArgument::Result(_) | SuiArgument::NestedResult(_, _) => {
            let Ok((index, producer)) = producing_command(context, consumer, object, "object") else {
                return format!("an object ({AMOUNT_NOT_IN_TRANSACTION})");
            };
            let changes_coin = match producer {
                SuiCommand::SplitCoins(SuiArgument::GasCoin, _) => Some(&sui),
                _ => coin,
            };
            let changes = find_changes(context, object, index, consumer, changes_coin, depth);
            let base = match producer {
                SuiCommand::SplitCoins(source, amounts) => {
                    let amount = result_slot(object)
                        .and_then(|(_, slot)| amounts.get(usize::from(slot)))
                        .and_then(|amount| decode_pure_amount(context, *amount, index));
                    let (from, source_coin) = match source {
                        SuiArgument::GasCoin => ("the gas coin".to_string(), Some(&sui)),
                        SuiArgument::Input(_) => (
                            input_at(context, *source, "coin")
                                .ok()
                                .and_then(SuiCallArg::object)
                                .map_or_else(|| "a coin".to_string(), |id| format!("coin {id}")),
                            coin,
                        ),
                        _ => ("another coin".to_string(), coin),
                    };
                    match (amount, source_coin) {
                        (Some(amount), Some(source_coin)) => {
                            format!("{} split from {from}", source_coin.display(amount))
                        }
                        (Some(amount), None) => format!("{amount} base units split from {from}"),
                        (None, _) => format!("a coin split from {from} ({AMOUNT_NOT_IN_TRANSACTION})"),
                    }
                }
                SuiCommand::MoveCall(call) if depth <= DESCRIBE_DEPTH => {
                    match (call.module.as_str(), call.function.as_str()) {
                        ("coin", "from_balance") if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK) => {
                            let balance_coin = Coin::from_type_argument(call, 0).ok();
                            let balance_coin = balance_coin.as_ref().or(coin);
                            match (call.arguments.first(), balance_coin) {
                                (Some(balance), Some(balance_coin)) => {
                                    describe_balance(context, *balance, index, balance_coin, depth + 1)
                                        .unwrap_or_else(|| "an empty coin".to_string())
                                }
                                _ => format!("a coin ({AMOUNT_NOT_IN_TRANSACTION})"),
                            }
                        }
                        ("coin", "redeem_funds") if call.package == ObjectID::from_single_byte(SUI_FRAMEWORK) => {
                            let redeemed = Coin::from_type_argument(call, 0).ok();
                            match redeemed.as_ref().or(coin) {
                                Some(redeemed) => describe_redeemed(context, call, redeemed),
                                None => format!("a coin from an address balance ({AMOUNT_NOT_IN_TRANSACTION})"),
                            }
                        }
                        (module, function) => format!(
                            "an object returned by {module}::{function} at command {index}"
                        ),
                    }
                }
                _ => format!("an object from command {index}"),
            };
            changes.qualify(base)
        }
    }
}

/// The reserved amount of a `redeem_funds`, which withdraws exactly that much.
fn describe_redeemed(context: &VisualizerContext, call: &SuiProgrammableMoveCall, coin: &Coin) -> String {
    let withdrawal = call
        .arguments
        .first()
        .and_then(|argument| input_at(context, *argument, "withdrawal").ok());
    match withdrawal {
        Some(SuiCallArg::FundsWithdrawal(withdrawal)) => {
            let SuiReservation::MaxAmountU64(amount) = withdrawal.reservation;
            let owner = match withdrawal.withdraw_from {
                SuiWithdrawFrom::Sender => "the signer's",
                SuiWithdrawFrom::Sponsor => "the sponsor's",
            };
            format!("{} from {owner} address balance", coin.display(amount))
        }
        _ => format!("{} from an address balance ({AMOUNT_NOT_IN_TRANSACTION})", coin.symbol()),
    }
}

/// A pure `u64` input no command before `consumer` could have modified.
fn decode_pure_amount(context: &VisualizerContext, argument: SuiArgument, consumer: usize) -> Option<u64> {
    ensure_unmodified_before(context, argument, consumer, "amount", |call| {
        call.package == ObjectID::from_single_byte(SUI_FRAMEWORK)
            && matches!(call.module.as_str(), "coin" | "balance")
    })
    .ok()?;
    decode_number::<u64>(input_at(context, argument, "amount").ok()?).ok()
}

fn resolve_recipient(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    arguments: &[SuiArgument],
    index: usize,
    consumer: usize,
) -> Result<SuiAddress, VisualSignError> {
    let argument = argument_at(arguments, index, "recipient")?;
    ensure_unmodified_before(context, argument, consumer, "recipient", |call| {
        takes_by_value(call, deployment)
    })?;
    pure_address(input_at(context, argument, "recipient")?)
}

/// Whoever holds the position owner cap can withdraw the collateral and borrow against it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CapDestination {
    NotPassed,
    Address {
        recipient: SuiAddress,
        also: Vec<String>,
    },
}

fn trace_cap(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    operate_index: usize,
) -> Result<(CapDestination, Option<TransferClaim>), VisualSignError> {
    let unsupported = || {
        missing(
            "The position owner cap is not returned through 0x1::option::destroy_some and a transfer, or 0x1::option::destroy_none",
        )
    };
    let option = output(operate_index, OPERATE_CAP_OUTPUT)?;
    let (unwrap_index, unwrap) =
        only_consumer(context, operate_index, option, "position owner cap option")?;
    let SuiCommand::MoveCall(unwrap) = unwrap else {
        return Err(unsupported());
    };
    if !unwrap
        .arguments
        .first()
        .is_some_and(|argument| refers_to(*argument, option))
    {
        return Err(unsupported());
    }
    if is_call(unwrap, STDLIB, "option", "destroy_none") {
        return Ok((CapDestination::NotPassed, None));
    }
    if !is_call(unwrap, STDLIB, "option", "destroy_some") {
        return Err(unsupported());
    }
    let cap = output(unwrap_index, 0)?;
    let (transfer_index, transfer) =
        only_consumer(context, unwrap_index, cap, "position owner cap")?;
    let (arguments, at, claim) = match transfer {
        SuiCommand::TransferObjects(_, to) => (vec![*to], 0, Some((transfer_index, cap))),
        SuiCommand::MoveCall(call)
            if is_call(call, SUI_FRAMEWORK, "transfer", "public_transfer")
                && call
                    .arguments
                    .first()
                    .is_some_and(|first| refers_to(*first, cap)) =>
        {
            (call.arguments.clone(), 1, None)
        }
        _ => return Err(unsupported()),
    };
    let recipient = resolve_recipient(context, deployment, &arguments, at, transfer_index)?;
    Ok((
        CapDestination::Address {
            recipient,
            also: Vec::new(),
        },
        claim,
    ))
}

/// The objects in a claimed `TransferObjects` that no trace of this `operate` reached.
fn describe_untraced_objects(
    context: &VisualizerContext,
    claims: &[TransferClaim],
    index: usize,
) -> Vec<String> {
    let Some(SuiCommand::TransferObjects(objects, _)) = context.commands().get(index) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter(|object| {
            !claims
                .iter()
                .any(|(claimed, value)| *claimed == index && refers_to(**object, *value))
        })
        .map(|object| describe_coin(context, *object, index, None, 0))
        .collect()
}

enum Position {
    New,
    Existing(ObjectID),
}

fn resolve_position(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
) -> Result<Position, VisualSignError> {
    let argument = argument_at(&pwc.arguments, OPERATE_POSITION_ARG, "position")?;
    if let SuiArgument::Input(_) = argument {
        return input_at(context, argument, "position")?
            .object()
            .map(|id| Position::Existing(*id))
            .ok_or_else(|| missing("Position is not an object input"));
    }
    match producing_command(context, context.command_index(), argument, "position")? {
        (_, SuiCommand::MoveCall(call))
            if deployment.is_vaults_package(&call.package)
                && call.module == "user"
                && call.function == "init_position"
                && result_slot(argument).is_some_and(|(_, slot)| slot == 0) =>
        {
            Ok(Position::New)
        }
        _ => Err(missing(
            "Position is neither an object input nor opened by Fluid's user::init_position",
        )),
    }
}

fn object_argument(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    index: usize,
    name: &str,
) -> Result<ObjectID, VisualSignError> {
    let argument = argument_at(&pwc.arguments, index, name)?;
    input_at(context, argument, name)?
        .object()
        .copied()
        .ok_or_else(|| missing(format!("`{name}` is not an object input")))
}

struct Operate<'a> {
    deployment: &'a FluidDeployment,
    package: ObjectID,
    market: ObjectID,
    position: Position,
    collateral: Coin,
    debt: Coin,
    col_amount: OperateAmount,
    debt_amount: OperateAmount,
    col_destination: Destination,
    debt_destination: Destination,
    cap: CapDestination,
}

fn resolve_operate<'a>(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &'a FluidDeployment,
) -> Result<Operate<'a>, VisualSignError> {
    let operate_index = context.command_index();
    let collateral = Coin::from_type_argument(pwc, 0)?;
    let debt = Coin::from_type_argument(pwc, 1)?;
    let col_amount = resolve_operate_amount(context, pwc, deployment, Side::Collateral)?;
    let debt_amount = resolve_operate_amount(context, pwc, deployment, Side::Debt)?;
    if col_amount == OperateAmount::None && debt_amount == OperateAmount::None {
        return Err(missing(
            "Fluid operate with neither a collateral nor a debt change aborts",
        ));
    }
    let destination = |side: Side, coin: &Coin| {
        trace_funds(
            context,
            deployment,
            operate_index,
            side.output_slot(),
            coin,
            side.balance_name(),
        )
    };
    let (mut col_destination, col_claim) = destination(Side::Collateral, &collateral)?;
    let (mut debt_destination, debt_claim) = destination(Side::Debt, &debt)?;
    let (mut cap, cap_claim) = trace_cap(context, deployment, operate_index)?;
    // A shared `TransferObjects` is listed once, on the first output that claims it.
    let claims: Vec<TransferClaim> = [col_claim, debt_claim, cap_claim]
        .into_iter()
        .flatten()
        .collect();
    let mut listed = Vec::new();
    for (claim, also) in [
        (col_claim, match &mut col_destination {
            Destination::Address { also, .. } => Some(also),
            Destination::Destroyed => None,
        }),
        (debt_claim, match &mut debt_destination {
            Destination::Address { also, .. } => Some(also),
            Destination::Destroyed => None,
        }),
        (cap_claim, match &mut cap {
            CapDestination::Address { also, .. } => Some(also),
            CapDestination::NotPassed => None,
        }),
    ] {
        if let (Some((index, _)), Some(also)) = (claim, also)
            && !listed.contains(&index)
        {
            listed.push(index);
            also.extend(describe_untraced_objects(context, &claims, index));
        }
    }
    Ok(Operate {
        deployment,
        package: pwc.package,
        market: object_argument(context, pwc, OPERATE_MARKET_ARG, "market")?,
        position: resolve_position(context, pwc, deployment)?,
        col_destination,
        debt_destination,
        cap,
        collateral,
        debt,
        col_amount,
        debt_amount,
    })
}

impl Operate<'_> {
    fn sides(&self) -> [(Side, OperateAmount, &Coin, &Destination); 2] {
        [
            (
                Side::Collateral,
                self.col_amount,
                &self.collateral,
                &self.col_destination,
            ),
            (
                Side::Debt,
                self.debt_amount,
                &self.debt,
                &self.debt_destination,
            ),
        ]
    }

    fn summary(&self) -> TransactionSummary {
        let actions: Vec<String> = self
            .sides()
            .into_iter()
            .filter_map(|(side, amount, coin, _)| side.action(amount, coin))
            .collect();
        let action = match actions.as_slice() {
            [first, second, ..] => format!("{first} and {}", lowercase_first(second)),
            [only] => only.clone(),
            [] => String::new(),
        };
        TransactionSummary {
            title: format!("{action} on {FLUID_DISPLAY_NAME}"),
            subtitle: Some(match self.position {
                Position::New => "New position".to_string(),
                Position::Existing(id) => format!("Position {id}"),
            }),
        }
    }
}

fn preview_field(
    context: &VisualizerContext,
    operation: &Operate<'_>,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let mut key_rows: Vec<AnnotatedPayloadField> = Vec::new();
    for (side, amount, coin, _) in operation.sides() {
        key_rows.extend(side.row(amount, coin)?);
    }
    key_rows.push(match operation.position {
        Position::New => create_text_field("Position", "New position")?,
        Position::Existing(id) => {
            create_address_field("Position", &id.to_string(), None, None, None, None)?
        }
    });
    // Outputs also return any supplied coin `operate` didn't use.
    for (side, amount, coin, destination) in operation.sides() {
        if let Destination::Address { recipient, also } = destination
            && (side.pays_out(amount) || recipient != context.sender())
        {
            let label = format!("{} Recipient", coin.symbol());
            key_rows.push(recipient_field(context, &label, *recipient)?);
            key_rows.extend(create_also_field(context, &label, *recipient, also)?);
        }
    }
    if let CapDestination::Address { recipient, also } = &operation.cap {
        key_rows.push(recipient_field(context, "Position Owner", *recipient)?);
        key_rows.extend(create_also_field(context, "Position Owner", *recipient, also)?);
    }
    key_rows.push(create_text_field(
        "Network",
        operation.deployment.sui_network.display_name(),
    )?);

    let details = vec![
        create_address_field(
            "Signer",
            &context.sender().to_string(),
            None,
            None,
            None,
            None,
        )?,
        create_address_field(
            "Vault Market",
            &operation.market.to_string(),
            None,
            None,
            None,
            None,
        )?,
        create_text_field("Collateral Coin", &operation.collateral.coin_type)?,
        create_text_field("Debt Coin", &operation.debt.coin_type)?,
        create_address_field(
            "Vaults Package",
            &operation.package.to_string(),
            Some(FLUID_DISPLAY_NAME),
            None,
            None,
            None,
        )?,
    ];

    let summary = operation.summary();
    // Expanded is the full field list (docs/field-types.mdx), so it repeats the key rows.
    let every_row = key_rows.iter().cloned().chain(details).collect();
    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.title.clone(),
                label: "Fluid Vault".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: summary.title,
                }),
                subtitle: summary.subtitle.map(|text| SignablePayloadFieldTextV2 { text }),
                condensed: Some(SignablePayloadFieldListLayout { fields: key_rows }),
                expanded: Some(SignablePayloadFieldListLayout { fields: every_row }),
            },
        },
    })
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

/// Shown only when the funds leave the signer.
fn create_also_field(
    context: &VisualizerContext,
    label: &str,
    recipient: SuiAddress,
    also: &[String],
) -> Result<Option<AnnotatedPayloadField>, VisualSignError> {
    if also.is_empty() || recipient == *context.sender() {
        return Ok(None);
    }
    create_text_field(&format!("{label} Also Receives"), &also.join("; ")).map(Some)
}

fn recipient_field(
    context: &VisualizerContext,
    label: &str,
    recipient: SuiAddress,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    let name = if recipient == *context.sender() {
        "Signer's address"
    } else {
        "Not the signer's address"
    };
    create_address_field(
        label,
        &recipient.to_string(),
        Some(name),
        None,
        None,
        None,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
