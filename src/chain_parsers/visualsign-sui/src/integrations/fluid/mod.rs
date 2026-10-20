//! Previews for calls into [Fluid](https://fluid.io) on Sui: borrowing against
//! collateral in a vault (`vaults::user`) and lending for interest
//! (`lending::user`).
//!
//! Fluid's Sui sources are not public, so the call semantics below are read
//! from the bytecode of the published testnet packages listed in
//! [`config::DEPLOYMENTS`].

mod config;
mod ptb;

use config::{
    Config, FLUID_CONFIG, FluidDeployment, KNOWN_COINS, KnownCoin, LendingFunctions,
    VaultFunctions, deployment_for,
};
use ptb::{
    argument_at, ensure_unmodified_before, first_type_argument_is, input_at, is_call, missing,
    only_consumer, output, producing_command, pure_address, refers_to, result_slot, type_matches,
};

use crate::core::{
    CommandVisualizer, SuiIntegrationConfig, TransactionSummary, VisualizerContext, VisualizerKind,
};
use crate::utils::decode_number;

use move_core_types::language_storage::TypeTag;
use sui_json_rpc_types::{
    SuiArgument, SuiCallArg, SuiCommand, SuiProgrammableMoveCall, SuiReservation, SuiWithdrawFrom,
    SuiWithdrawalTypeArg,
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

// `close_position(market, vault_registry, position: Position, cap: PositionOwnerCap, ctx)`.
const CLOSE_MARKET_ARG: usize = 0;
const CLOSE_POSITION_ARG: usize = 2;

// `deposit(registry, state, liquidity_registry, reserve, assets: Balance<T>,
// min_shares: u64, clock, ctx) -> Balance<FToken<T>>`.
// `withdraw(.., assets: u64, max_shares: Balance<FToken<T>>, clock, ctx)
// -> (Balance<T>, Balance<FToken<T>>)`.
// `redeem(.., shares: Balance<FToken<T>>, min_assets: u64, clock, ctx) -> Balance<T>`.
const LENDING_STATE_ARG: usize = 1;
const LENDING_FIRST_ARG: usize = 4;
const LENDING_SECOND_ARG: usize = 5;

pub struct FluidVisualizer;

impl CommandVisualizer for FluidVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let (pwc, deployment) = fluid_call(context)?;
        let (preview, details) = preview(context, pwc, deployment)?;
        let details = std::iter::once(signer_field(context)?)
            .chain(details)
            .collect();
        Ok(vec![preview_field(preview, details)])
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(FLUID_CONFIG.get_or_init(Config::new))
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(FLUID_DISPLAY_NAME)
    }

    fn transaction_summary(&self, context: &VisualizerContext) -> Option<TransactionSummary> {
        let (pwc, deployment) = fluid_call(context).ok()?;
        let (preview, _) = preview(context, pwc, deployment).ok()?;
        only_supports_this_call(context, deployment).then_some(preview.summary)
    }
}

fn fluid_call<'a>(
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
    Ok((pwc, deployment))
}

/// The payload's title and subtitle and the card's label, header and key rows
/// all come from here, so they cannot disagree.
struct Preview {
    label: &'static str,
    summary: TransactionSummary,
    key_rows: Vec<AnnotatedPayloadField>,
}

fn preview(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
) -> Result<(Preview, Vec<AnnotatedPayloadField>), VisualSignError> {
    if deployment.is_vaults_package(&pwc.package) {
        match pwc.function.as_str().try_into()? {
            VaultFunctions::Operate => operate_preview(context, pwc, deployment),
            VaultFunctions::ClosePosition => close_position_preview(context, pwc, deployment),
        }
    } else {
        let coin = Coin::from_type_argument(pwc, 0)?;
        match pwc.function.as_str().try_into()? {
            LendingFunctions::Deposit => lending_deposit_preview(context, pwc, deployment, &coin),
            LendingFunctions::Withdraw => lending_withdraw_preview(context, pwc, deployment, &coin),
            LendingFunctions::Redeem => lending_redeem_preview(context, pwc, deployment, &coin),
        }
    }
}

fn title(action: &str, deployment: &FluidDeployment) -> String {
    match deployment.sui_network {
        config::SuiNetwork::Mainnet => format!("{action} on {FLUID_DISPLAY_NAME}"),
        network @ config::SuiNetwork::Testnet => format!(
            "{action} on {FLUID_DISPLAY_NAME} ({})",
            network.display_name()
        ),
    }
}

/// A coin type argument, with its symbol and decimals when it is a
/// [`KNOWN_COINS`] entry.
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
        Ok(Self::new(coin_type))
    }

    fn new(coin_type: &str) -> Self {
        Self {
            coin_type: coin_type.to_string(),
            known: KNOWN_COINS
                .iter()
                .find(|known| type_matches(coin_type, known.coin_type)),
        }
    }

    /// The same coin's Fluid lending share, `FToken<T>`. Fluid registers it
    /// with the underlying coin's decimals and an `f`-prefixed symbol
    /// (`create_internal` in the lending package's `lending_state` module).
    fn f_token(&self, deployment: &FluidDeployment) -> Result<FToken<'_>, VisualSignError> {
        let prefix = deployment
            .f_token_type_prefix()
            .ok_or_else(|| missing("Fluid deployment has an invalid lending package id"))?;
        Ok(FToken {
            coin_type: format!("{prefix}{}>", self.coin_type),
            underlying: self,
        })
    }

    fn symbol(&self) -> String {
        self.known
            .map_or_else(|| self.coin_type.clone(), |known| known.symbol.to_string())
    }

    /// The amount in whole coins with the coin's symbol, or in base units with
    /// the full coin type when its decimals are unknown.
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

struct FToken<'a> {
    coin_type: String,
    underlying: &'a Coin,
}

impl FToken<'_> {
    fn amount_field(
        &self,
        label: &str,
        base_units: u64,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        match self.underlying.known {
            Some(known) => create_amount_field(
                label,
                &format_units(base_units, known.decimals),
                &format!("f{}", known.symbol),
            ),
            None => create_amount_field(
                label,
                &base_units.to_string(),
                &format!("base units of {}", self.coin_type),
            ),
        }
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

/// An `OperateAmount`, built by `structs::{none, positive, negative, max}`.
/// `positive(0)` and `negative(0)` build `none`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperateAmount {
    None,
    Positive(u64),
    Negative(u64),
    Max,
}

/// What `operate` does to the collateral (`col_amount`) or the debt
/// (`debt_amount`). A positive collateral amount is a deposit and a negative
/// one a withdrawal; a positive debt amount is a borrow and a negative one a
/// repayment. `max` is negative on both: it withdraws all collateral or repays
/// all debt (`get_new_position_info` in the vaults package's `position`
/// module). `operate` aborts when both amounts are `none`.
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

    fn name(self) -> &'static str {
        match self {
            Side::Collateral => "col_amount",
            Side::Debt => "debt_amount",
        }
    }

    /// The action phrase for the title, or `None` when this side is unchanged.
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
            (Side::Collateral, OperateAmount::Max) => Some(format!("Withdraw all {symbol} collateral")),
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

    /// True when `operate` pays this side's coin out to the caller, rather than
    /// only returning what was supplied and not used.
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

/// Vault structs constructors take every argument by value.
fn is_amount_constructor(call: &SuiProgrammableMoveCall, deployment: &FluidDeployment) -> bool {
    deployment.is_vaults_package(&call.package)
        && call.module == "structs"
        && matches!(
            call.function.as_str(),
            "none" | "positive" | "negative" | "max"
        )
}

/// Calls that take every argument by value, so cannot modify a value passed to
/// them and hand it on.
fn takes_by_value(call: &SuiProgrammableMoveCall, deployment: &FluidDeployment) -> bool {
    let framework = call.package == ObjectID::from_single_byte(SUI_FRAMEWORK)
        && matches!(call.module.as_str(), "coin" | "balance")
        && matches!(
            call.function.as_str(),
            "value" | "zero" | "redeem_funds" | "send_funds"
        );
    let option = call.package == ObjectID::from_single_byte(STDLIB)
        && call.module == "option"
        && matches!(call.function.as_str(), "some" | "destroy_some");
    framework || option || is_amount_constructor(call, deployment)
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

/// Where a `Balance<T>` output goes: it must be passed, alone, to exactly one
/// `0x2::balance::send_funds<T>`, whose recipient no earlier command may have
/// modified.
fn funds_recipient(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    producer: usize,
    slot: u16,
    coin_type: &str,
    name: &str,
) -> Result<SuiAddress, VisualSignError> {
    let value = output(producer, slot)?;
    let (index, consumer) = only_consumer(context, producer, value, name)?;
    match consumer {
        SuiCommand::MoveCall(call)
            if is_call(call, SUI_FRAMEWORK, "balance", "send_funds")
                && first_type_argument_is(call, coin_type)
                && call
                    .arguments
                    .first()
                    .is_some_and(|argument| refers_to(*argument, value)) =>
        {
            let recipient = argument_at(&call.arguments, 1, "recipient")?;
            ensure_unmodified_before(context, recipient, index, "recipient", |call| {
                takes_by_value(call, deployment)
            })?;
            pure_address(input_at(context, recipient, "recipient")?)
        }
        _ => Err(missing(format!(
            "The {name} from command {producer} is not sent with 0x2::balance::send_funds<{coin_type}>"
        ))),
    }
}

/// Where the position owner cap that `operate` hands back goes: through
/// `0x1::option::destroy_some` to a `TransferObjects` of that cap alone.
/// Whoever holds the cap can withdraw the collateral and borrow against it.
fn cap_recipient(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    operate_index: usize,
) -> Result<SuiAddress, VisualSignError> {
    let unsupported =
        || missing("The position owner cap is not returned through 0x1::option::destroy_some and TransferObjects");
    let option = output(operate_index, OPERATE_CAP_OUTPUT)?;
    let (destroy_index, destroy) =
        only_consumer(context, operate_index, option, "position owner cap option")?;
    let SuiCommand::MoveCall(destroy) = destroy else {
        return Err(unsupported());
    };
    if !is_call(destroy, STDLIB, "option", "destroy_some")
        || !destroy
            .arguments
            .first()
            .is_some_and(|argument| refers_to(*argument, option))
    {
        return Err(unsupported());
    }
    let cap = output(destroy_index, 0)?;
    let (transfer_index, transfer) =
        only_consumer(context, destroy_index, cap, "position owner cap")?;
    let SuiCommand::TransferObjects(objects, recipient) = transfer else {
        return Err(unsupported());
    };
    if !matches!(objects.as_slice(), [object] if refers_to(*object, cap)) {
        return Err(unsupported());
    }
    ensure_unmodified_before(context, *recipient, transfer_index, "recipient", |call| {
        takes_by_value(call, deployment)
    })?;
    pure_address(input_at(context, *recipient, "recipient")?)
}

enum Position {
    New,
    Existing(ObjectID),
}

fn resolve_position(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
    argument_index: usize,
) -> Result<Position, VisualSignError> {
    let argument = argument_at(&pwc.arguments, argument_index, "position")?;
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

fn position_row(position: &Position) -> Result<AnnotatedPayloadField, VisualSignError> {
    match position {
        Position::New => create_text_field("Position", "New position"),
        Position::Existing(id) => {
            create_address_field("Position", &id.to_string(), None, None, None, None)
        }
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

fn operate_preview(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
) -> Result<(Preview, Vec<AnnotatedPayloadField>), VisualSignError> {
    let operate_index = context.command_index();
    let collateral = Coin::from_type_argument(pwc, 0)?;
    let debt = Coin::from_type_argument(pwc, 1)?;
    let col_amount = resolve_operate_amount(context, pwc, deployment, Side::Collateral)?;
    let debt_amount = resolve_operate_amount(context, pwc, deployment, Side::Debt)?;
    let actions: Vec<String> = [
        Side::Collateral.action(col_amount, &collateral),
        Side::Debt.action(debt_amount, &debt),
    ]
    .into_iter()
    .flatten()
    .collect();
    let action = match actions.as_slice() {
        [] => {
            return Err(missing(
                "Fluid operate with neither a collateral nor a debt change aborts",
            ));
        }
        [only] => only.clone(),
        [first, second, ..] => format!("{first} and {}", lowercase_first(second)),
    };

    let position = resolve_position(context, pwc, deployment, OPERATE_POSITION_ARG)?;
    let col_recipient = funds_recipient(
        context,
        deployment,
        operate_index,
        OPERATE_COL_OUTPUT,
        &collateral.coin_type,
        "collateral balance",
    )?;
    let debt_recipient = funds_recipient(
        context,
        deployment,
        operate_index,
        OPERATE_DEBT_OUTPUT,
        &debt.coin_type,
        "debt balance",
    )?;
    let owner = cap_recipient(context, deployment, operate_index)?;
    let market = object_argument(context, pwc, OPERATE_MARKET_ARG, "market")?;

    let mut key_rows: Vec<AnnotatedPayloadField> = [
        Side::Collateral.row(col_amount, &collateral)?,
        Side::Debt.row(debt_amount, &debt)?,
    ]
    .into_iter()
    .flatten()
    .collect();
    key_rows.push(position_row(&position)?);
    // `operate` returns what it pays out plus any supplied coin it did not use,
    // so a recipient is shown whenever the coin is paid out or goes to someone
    // other than the signer.
    for (side, amount, coin, recipient) in [
        (Side::Collateral, col_amount, &collateral, col_recipient),
        (Side::Debt, debt_amount, &debt, debt_recipient),
    ] {
        if side.pays_out(amount) || recipient != *context.sender() {
            key_rows.push(recipient_field(
                context,
                &format!("{} Recipient", coin.symbol()),
                recipient,
            )?);
        }
    }
    key_rows.push(recipient_field(context, "Position Owner", owner)?);
    key_rows.push(create_text_field(
        "Network",
        deployment.sui_network.display_name(),
    )?);

    let subtitle = match position {
        Position::New => "New position".to_string(),
        Position::Existing(id) => format!("Position {id}"),
    };
    let details = vec![
        create_address_field(
            "Vault Market",
            &market.to_string(),
            None,
            None,
            None,
            None,
        )?,
        create_text_field("Collateral Coin", &collateral.coin_type)?,
        create_text_field("Debt Coin", &debt.coin_type)?,
        package_field("Vaults Package", &pwc.package)?,
    ];
    Ok((
        Preview {
            label: "Fluid Vault",
            summary: TransactionSummary {
                title: title(&action, deployment),
                subtitle: Some(subtitle),
            },
            key_rows,
        },
        details,
    ))
}

fn close_position_preview(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
) -> Result<(Preview, Vec<AnnotatedPayloadField>), VisualSignError> {
    let Position::Existing(position) =
        resolve_position(context, pwc, deployment, CLOSE_POSITION_ARG)?
    else {
        return Err(missing("Closing a position opened in the same transaction"));
    };
    let market = object_argument(context, pwc, CLOSE_MARKET_ARG, "market")?;
    Ok((
        Preview {
            label: "Fluid Vault",
            summary: TransactionSummary {
                title: title("Close vault position", deployment),
                subtitle: Some(format!("Position {position}")),
            },
            key_rows: vec![
                position_row(&Position::Existing(position))?,
                create_text_field("Network", deployment.sui_network.display_name())?,
            ],
        },
        vec![
            create_address_field(
                "Vault Market",
                &market.to_string(),
                None,
                None,
                None,
                None,
            )?,
            package_field("Vaults Package", &pwc.package)?,
        ],
    ))
}

/// The amount a `Balance<T>` argument carries: it must come from a
/// `0x2::balance::redeem_funds<T>` of the signer's own address balance,
/// which withdraws exactly the reserved amount. Any other funding is reported
/// as undecodable rather than guessed.
fn redeemed_amount(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    consumer: usize,
    balance_argument: SuiArgument,
    coin_type: &str,
    name: &str,
) -> Result<u64, VisualSignError> {
    let unsupported = || {
        missing(format!(
            "`{name}` is not a 0x2::balance::redeem_funds<{coin_type}> of the signer's address balance"
        ))
    };
    let (redeem_index, redeem) = match producing_command(context, consumer, balance_argument, name)? {
        (index, SuiCommand::MoveCall(call))
            if is_call(call, SUI_FRAMEWORK, "balance", "redeem_funds")
                && first_type_argument_is(call, coin_type) =>
        {
            (index, call)
        }
        _ => return Err(unsupported()),
    };
    ensure_unmodified_before(context, balance_argument, consumer, name, |call| {
        takes_by_value(call, deployment)
    })?;
    let withdrawal_argument = argument_at(&redeem.arguments, 0, "withdrawal")?;
    let SuiCallArg::FundsWithdrawal(withdrawal) =
        input_at(context, withdrawal_argument, "withdrawal")?
    else {
        return Err(unsupported());
    };
    if withdrawal.withdraw_from != SuiWithdrawFrom::Sender {
        return Err(unsupported());
    }
    let SuiWithdrawalTypeArg::Balance(balance_type) = &withdrawal.type_arg;
    let balance_type: TypeTag = balance_type
        .clone()
        .try_into()
        .map_err(|e| VisualSignError::DecodeError(format!("Invalid funds withdrawal type: {e}")))?;
    if !type_matches(&balance_type.to_canonical_string(true), coin_type) {
        return Err(unsupported());
    }
    // `withdrawal_split` and `withdrawal_join` change a withdrawal's limit.
    ensure_unmodified_before(context, withdrawal_argument, redeem_index, "withdrawal", |call| {
        takes_by_value(call, deployment)
    })?;
    let SuiReservation::MaxAmountU64(amount) = withdrawal.reservation;
    Ok(amount)
}

fn pure_u64(
    context: &VisualizerContext,
    deployment: &FluidDeployment,
    pwc: &SuiProgrammableMoveCall,
    index: usize,
    name: &str,
) -> Result<u64, VisualSignError> {
    let argument = argument_at(&pwc.arguments, index, name)?;
    ensure_unmodified_before(context, argument, context.command_index(), name, |call| {
        takes_by_value(call, deployment)
    })?;
    decode_number::<u64>(input_at(context, argument, name)?)
}

fn lending_details(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    coin: &Coin,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let pool = object_argument(context, pwc, LENDING_STATE_ARG, "lending state")?;
    Ok(vec![
        create_address_field(
            "Lending Pool",
            &pool.to_string(),
            None,
            None,
            None,
            None,
        )?,
        create_text_field("Coin", &coin.coin_type)?,
        package_field("Lending Package", &pwc.package)?,
    ])
}

fn lending_deposit_preview(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
    coin: &Coin,
) -> Result<(Preview, Vec<AnnotatedPayloadField>), VisualSignError> {
    let index = context.command_index();
    let f_token = coin.f_token(deployment)?;
    let amount = redeemed_amount(
        context,
        deployment,
        index,
        argument_at(&pwc.arguments, LENDING_FIRST_ARG, "assets")?,
        &coin.coin_type,
        "assets",
    )?;
    let min_shares = pure_u64(context, deployment, pwc, LENDING_SECOND_ARG, "min_shares")?;
    let share_recipient = funds_recipient(
        context,
        deployment,
        index,
        0,
        &f_token.coin_type,
        "fToken balance",
    )?;
    Ok((
        Preview {
            label: "Fluid Lending",
            summary: TransactionSummary {
                title: title(&format!("Lend {}", coin.display(amount)), deployment),
                subtitle: Some(format!("fTokens to {share_recipient}")),
            },
            key_rows: vec![
                coin.amount_field("Amount", amount)?,
                f_token.amount_field("Minimum fTokens Received", min_shares)?,
                recipient_field(context, "fToken Recipient", share_recipient)?,
                create_text_field("Network", deployment.sui_network.display_name())?,
            ],
        },
        lending_details(context, pwc, coin)?,
    ))
}

fn lending_withdraw_preview(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
    coin: &Coin,
) -> Result<(Preview, Vec<AnnotatedPayloadField>), VisualSignError> {
    let index = context.command_index();
    let f_token = coin.f_token(deployment)?;
    let amount = pure_u64(context, deployment, pwc, LENDING_FIRST_ARG, "assets")?;
    let max_shares = redeemed_amount(
        context,
        deployment,
        index,
        argument_at(&pwc.arguments, LENDING_SECOND_ARG, "max_shares")?,
        &f_token.coin_type,
        "max_shares",
    )?;
    let recipient = funds_recipient(
        context,
        deployment,
        index,
        0,
        &coin.coin_type,
        "withdrawn balance",
    )?;
    let change_recipient = funds_recipient(
        context,
        deployment,
        index,
        1,
        &f_token.coin_type,
        "unused fToken balance",
    )?;
    let mut key_rows = vec![
        coin.amount_field("Amount", amount)?,
        f_token.amount_field("Maximum fTokens Burned", max_shares)?,
        recipient_field(context, &format!("{} Recipient", coin.symbol()), recipient)?,
    ];
    if change_recipient != *context.sender() {
        key_rows.push(recipient_field(
            context,
            "Unused fToken Recipient",
            change_recipient,
        )?);
    }
    key_rows.push(create_text_field(
        "Network",
        deployment.sui_network.display_name(),
    )?);
    Ok((
        Preview {
            label: "Fluid Lending",
            summary: TransactionSummary {
                title: title(
                    &format!("Withdraw {} from lending", coin.display(amount)),
                    deployment,
                ),
                subtitle: Some(format!("To {recipient}")),
            },
            key_rows,
        },
        lending_details(context, pwc, coin)?,
    ))
}

fn lending_redeem_preview(
    context: &VisualizerContext,
    pwc: &SuiProgrammableMoveCall,
    deployment: &FluidDeployment,
    coin: &Coin,
) -> Result<(Preview, Vec<AnnotatedPayloadField>), VisualSignError> {
    let index = context.command_index();
    let f_token = coin.f_token(deployment)?;
    let shares = redeemed_amount(
        context,
        deployment,
        index,
        argument_at(&pwc.arguments, LENDING_FIRST_ARG, "shares")?,
        &f_token.coin_type,
        "shares",
    )?;
    let min_assets = pure_u64(context, deployment, pwc, LENDING_SECOND_ARG, "min_assets")?;
    let recipient = funds_recipient(
        context,
        deployment,
        index,
        0,
        &coin.coin_type,
        "redeemed balance",
    )?;
    Ok((
        Preview {
            label: "Fluid Lending",
            summary: TransactionSummary {
                title: title(
                    &format!("Withdraw {} from lending", coin.symbol()),
                    deployment,
                ),
                subtitle: Some(format!("To {recipient}")),
            },
            key_rows: vec![
                f_token.amount_field("fTokens Redeemed", shares)?,
                coin.amount_field("Minimum Received", min_assets)?,
                recipient_field(context, &format!("{} Recipient", coin.symbol()), recipient)?,
                create_text_field("Network", deployment.sui_network.display_name())?,
            ],
        },
        lending_details(context, pwc, coin)?,
    ))
}

/// True when every other command is a step the Fluid previews account for:
/// building amounts, reading prices, opening and sharing the position, funding
/// the call from the signer's address balance, and sending its outputs where
/// the card shows. Anything else is an action the summary would not describe.
fn only_supports_this_call(context: &VisualizerContext, deployment: &FluidDeployment) -> bool {
    let main = context.command_index();
    let feeds_main = |index: usize| {
        output(index, 0).is_ok_and(|value| {
            context.commands().get(main).is_some_and(|command| match command {
                SuiCommand::MoveCall(call) => call
                    .arguments
                    .iter()
                    .any(|argument| refers_to(*argument, value)),
                _ => false,
            })
        })
    };
    let from_main = |argument: Option<&SuiArgument>| {
        argument
            .and_then(|argument| result_slot(*argument))
            .is_some_and(|(command, _)| usize::from(command) == main)
    };
    context
        .commands()
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != main)
        .all(|(index, command)| match command {
            SuiCommand::MoveCall(call) => {
                is_amount_constructor(call, deployment)
                    || (deployment.is_oracle_package(&call.package)
                        && call.module == "view"
                        && call.function == "get_both_exchange_rate")
                    || (deployment.is_vaults_package(&call.package)
                        && ((call.module == "user" && call.function == "init_position")
                            || (call.module == "position" && call.function == "share")))
                    || is_call(call, SUI_FRAMEWORK, "balance", "zero")
                    || is_call(call, STDLIB, "option", "some")
                    || (is_call(call, STDLIB, "option", "destroy_some")
                        && from_main(call.arguments.first()))
                    || (is_call(call, SUI_FRAMEWORK, "balance", "redeem_funds")
                        && feeds_main(index))
                    || (is_call(call, SUI_FRAMEWORK, "balance", "send_funds")
                        && from_main(call.arguments.first()))
            }
            // The position owner cap's transfer, which the card shows.
            SuiCommand::TransferObjects(_, _) => true,
            _ => false,
        })
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}

fn recipient_name(context: &VisualizerContext, recipient: SuiAddress) -> &'static str {
    if recipient == *context.sender() {
        "Signer's address"
    } else {
        "Not the signer's address"
    }
}

fn recipient_field(
    context: &VisualizerContext,
    label: &str,
    recipient: SuiAddress,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    create_address_field(
        label,
        &recipient.to_string(),
        Some(recipient_name(context, recipient)),
        None,
        None,
        None,
    )
}

fn signer_field(context: &VisualizerContext) -> Result<AnnotatedPayloadField, VisualSignError> {
    create_address_field(
        "Signer",
        &context.sender().to_string(),
        None,
        None,
        None,
        None,
    )
}

fn package_field(label: &str, package: &ObjectID) -> Result<AnnotatedPayloadField, VisualSignError> {
    create_address_field(
        label,
        &package.to_string(),
        Some(FLUID_DISPLAY_NAME),
        None,
        None,
        None,
    )
}

fn preview_field(preview: Preview, details: Vec<AnnotatedPayloadField>) -> AnnotatedPayloadField {
    let Preview {
        label,
        summary,
        key_rows,
    } = preview;
    // Expanded is the full field list (docs/field-types.mdx), so it repeats the key rows.
    let every_row = key_rows.iter().cloned().chain(details).collect();
    AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.title.clone(),
                label: label.to_string(),
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
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
