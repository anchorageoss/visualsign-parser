//! Display structs for allowlisted L1 actions.
//!
//! Shapes follow hyperliquid-python-sdk 0.24.0 (`exchange.py`, `utils/signing.py`) and
//! the official exchange-endpoint, HIP-1/2, HIP-3 and HIP-4 API docs. Every struct
//! denies unknown fields, and fields are declared in wire order: after parsing, the
//! struct is re-encoded to msgpack and must equal the transcoded raw bytes
//! byte-for-byte (see [`crate::input`]), so a field order, an omitted-vs-null choice or
//! an integer type that differs from what was hashed is refused rather than shown.
//!
//! Optional keys the SDK omits when unset use `skip_serializing_if`; keys the SDK sends
//! as `null` (for example `maxGas` and `schema` in `registerAsset`) do not.
//!
//! Sub-operation families (`spotDeploy`, `perpDeploy`, `outcomeDeploy`,
//! `activateOutcomeDeployer`) are modelled as one struct per family with one optional
//! field per variant; [`L1Action::validate`] requires exactly one to be present.

use serde::{Deserialize, Deserializer, Serialize};

/// The `type` values this parser accepts. Anything else is refused before typed parsing.
pub const ALLOWED_TYPES: &[&str] = &[
    "order",
    "cancel",
    "cancelByCloid",
    "modify",
    "batchModify",
    "scheduleCancel",
    "updateLeverage",
    "updateIsolatedMargin",
    "twapOrder",
    "twapCancel",
    "vaultTransfer",
    "subAccountTransfer",
    "perpDeploy",
    "spotDeploy",
    "outcomeDeploy",
    "activateOutcomeDeployer",
    "noop",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum L1Action {
    #[serde(rename = "order")]
    Order(OrderAction),
    #[serde(rename = "cancel")]
    Cancel(CancelAction),
    #[serde(rename = "cancelByCloid")]
    CancelByCloid(CancelByCloidAction),
    #[serde(rename = "modify")]
    Modify(ModifyWire),
    #[serde(rename = "batchModify")]
    BatchModify(BatchModifyAction),
    #[serde(rename = "scheduleCancel")]
    ScheduleCancel(ScheduleCancelAction),
    #[serde(rename = "updateLeverage")]
    UpdateLeverage(UpdateLeverageAction),
    #[serde(rename = "updateIsolatedMargin")]
    UpdateIsolatedMargin(UpdateIsolatedMarginAction),
    #[serde(rename = "twapOrder")]
    TwapOrder(TwapOrderAction),
    #[serde(rename = "twapCancel")]
    TwapCancel(TwapCancelAction),
    #[serde(rename = "vaultTransfer")]
    VaultTransfer(VaultTransferAction),
    #[serde(rename = "subAccountTransfer")]
    SubAccountTransfer(SubAccountTransferAction),
    #[serde(rename = "perpDeploy")]
    PerpDeploy(PerpDeployAction),
    #[serde(rename = "spotDeploy")]
    SpotDeploy(SpotDeployAction),
    #[serde(rename = "outcomeDeploy")]
    OutcomeDeploy(OutcomeDeployAction),
    #[serde(rename = "activateOutcomeDeployer")]
    ActivateOutcomeDeployer(ActivateOutcomeDeployerAction),
    #[serde(rename = "noop")]
    Noop(NoopAction),
}

// ---------------------------------------------------------------- trading

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderAction {
    pub orders: Vec<OrderWire>,
    pub grouping: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder: Option<BuilderInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuilderInfo {
    /// Builder address that receives the fee.
    pub b: String,
    /// Fee in tenths of a basis point.
    pub f: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderWire {
    /// Asset index.
    pub a: u64,
    /// Is buy.
    pub b: bool,
    /// Limit price, as signed.
    pub p: String,
    /// Size, as signed.
    pub s: String,
    /// Reduce only.
    pub r: bool,
    /// Order type.
    pub t: OrderTypeWire,
    /// Client order id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c: Option<String>,
}

/// `{"limit": {...}}` or `{"trigger": {...}}`; exactly one is present.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderTypeWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<LimitWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitWire {
    pub tif: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TriggerWire {
    pub is_market: bool,
    pub trigger_px: String,
    pub tpsl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelAction {
    pub cancels: Vec<CancelWire>,
    /// "fast" flag from the API docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelWire {
    pub a: u64,
    pub o: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelByCloidAction {
    pub cancels: Vec<CancelByCloidWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelByCloidWire {
    pub asset: u64,
    pub cloid: String,
}

/// An order id or a client order id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OidOrCloid {
    Oid(u64),
    Cloid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifyWire {
    pub oid: OidOrCloid,
    pub order: OrderWire,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchModifyAction {
    pub modifies: Vec<ModifyWire>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCancelAction {
    /// The SDK omits the key (not `null`) to unset the scheduled cancel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateLeverageAction {
    pub asset: u64,
    pub is_cross: bool,
    pub leverage: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateIsolatedMarginAction {
    pub asset: u64,
    pub is_buy: bool,
    /// Signed USDC amount with 6 decimals.
    pub ntli: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwapOrderAction {
    pub twap: TwapWire,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwapWire {
    /// Asset index.
    pub a: u64,
    /// Is buy.
    pub b: bool,
    /// Size, as signed.
    pub s: String,
    /// Reduce only.
    pub r: bool,
    /// Duration in minutes.
    pub m: u64,
    /// Randomize.
    pub t: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwapCancelAction {
    /// Asset index.
    pub a: u64,
    /// TWAP id.
    pub t: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VaultTransferAction {
    pub vault_address: String,
    pub is_deposit: bool,
    /// USDC with 6 decimals.
    pub usd: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubAccountTransferAction {
    pub sub_account_user: String,
    pub is_deposit: bool,
    /// USDC with 6 decimals.
    pub usd: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoopAction {}

// ---------------------------------------------------------------- spotDeploy (HIP-1/HIP-2)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SpotDeployAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_token2: Option<RegisterToken2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_genesis: Option<UserGenesis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genesis: Option<Genesis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_spot: Option<RegisterSpot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_hyperliquidity: Option<RegisterHyperliquidity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_deployer_trading_fee_share: Option<SetDeployerTradingFeeShare>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_freeze_privilege: Option<TokenRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_user: Option<FreezeUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoke_freeze_privilege: Option<TokenRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_quote_token: Option<TokenRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_quote_token: Option<TokenRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterToken2 {
    pub spec: TokenSpec,
    pub max_gas: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TokenSpec {
    pub name: String,
    pub sz_decimals: u64,
    pub wei_decimals: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UserGenesis {
    pub token: u64,
    pub user_and_wei: Vec<(String, String)>,
    pub existing_token_and_wei: Vec<(u64, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blacklist_users: Option<Vec<(String, bool)>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Genesis {
    pub token: u64,
    pub max_supply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_hyperliquidity: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterSpot {
    /// `[base token, quote token]`.
    pub tokens: [u64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterHyperliquidity {
    pub spot: u64,
    pub start_px: String,
    pub order_sz: String,
    pub n_orders: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_seeded_levels: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDeployerTradingFeeShare {
    pub token: u64,
    pub share: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRef {
    pub token: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreezeUser {
    pub token: u64,
    pub user: String,
    pub freeze: bool,
}

// ---------------------------------------------------------------- perpDeploy (HIP-3)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PerpDeployAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_asset: Option<RegisterAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_asset2: Option<RegisterAsset2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_oracle: Option<SetOracle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_funding_multipliers: Option<Vec<(String, String)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halt_trading: Option<HaltTrading>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_margin_table_ids: Option<Vec<(String, u64)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_fee_recipient: Option<SetFeeRecipient>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_open_interest_caps: Option<Vec<(String, u64)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_sub_deployers: Option<PerpSetSubDeployers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_dex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterAsset {
    /// Sent as `null` when unset: the exchange hashes it that way.
    pub max_gas: Option<u64>,
    pub asset_request: RegisterAssetRequest,
    pub dex: String,
    /// Sent as `null` unless the DEX is being created.
    pub schema: Option<PerpDexSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterAssetRequest {
    pub coin: String,
    pub sz_decimals: u64,
    pub oracle_px: String,
    pub margin_table_id: u64,
    pub only_isolated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterAsset2 {
    pub max_gas: Option<u64>,
    pub asset_request: RegisterAssetRequest2,
    pub dex: String,
    pub schema: Option<PerpDexSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterAssetRequest2 {
    pub coin: String,
    pub sz_decimals: u64,
    pub oracle_px: String,
    pub margin_table_id: u64,
    pub margin_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PerpDexSchema {
    pub full_name: String,
    pub collateral_token: u64,
    /// `null` means the deployer.
    pub oracle_updater: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetOracle {
    pub dex: String,
    pub oracle_pxs: Vec<(String, String)>,
    pub mark_pxs: Vec<Vec<(String, String)>>,
    pub external_perp_pxs: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HaltTrading {
    pub coin: String,
    pub is_halted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SetFeeRecipient {
    pub dex: String,
    pub fee_recipient: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PerpSetSubDeployers {
    pub dex: String,
    pub sub_deployers: Vec<SubDeployer>,
}

/// One sub-deployer permission change. `variant` names the operation granted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubDeployer {
    pub variant: String,
    pub user: String,
    pub allowed: bool,
}

// ---------------------------------------------------------------- outcomeDeploy (HIP-4)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeDeployAction {
    pub venue: String,
    pub operation: OutcomeOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OutcomeOperation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_standalone_outcome_from_template: Option<TemplateInstance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_question_from_template: Option<RegisterQuestionFromTemplate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_and_associate_named_outcome_from_template:
        Option<RegisterAndAssociateNamedOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_outcome: Option<OutcomeSettlement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_question2: Option<SettleQuestion2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_sub_deployers: Option<Vec<SubDeployer>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TemplateInstance {
    pub id: String,
    pub keyword_to_value: Vec<(String, String)>,
    pub deployer_fee_scale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NamedOutcomeTemplateInstance {
    pub id: String,
    pub keyword_to_value: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterQuestionFromTemplate {
    pub question_template_instance: TemplateInstance,
    pub named_outcome_template_instances: Vec<NamedOutcomeTemplateInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RegisterAndAssociateNamedOutcome {
    pub question: u64,
    pub named_outcome_template_instance: NamedOutcomeTemplateInstance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OutcomeSettlement {
    pub outcome: u64,
    pub settle_fraction: String,
    pub details: String,
    pub name_and_description: [String; 2],
    pub side_names: [String; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettleQuestion2 {
    pub question: u64,
    pub outcome_settlements: Vec<OutcomeSettlement>,
    pub name_and_description: [String; 2],
}

/// `{"activate": {"venueName": ..}}` or `{"deactivate": null}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateOutcomeDeployerAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activate: Option<OutcomeVenue>,
    /// `Some(())` when the key is present (its value must be `null`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_null"
    )]
    pub deactivate: Option<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OutcomeVenue {
    pub venue_name: String,
}

/// A key whose value must be `null`: present means `Some(())`.
fn present_null<'de, D: Deserializer<'de>>(d: D) -> Result<Option<()>, D::Error> {
    <()>::deserialize(d).map(Some)
}

// ---------------------------------------------------------------- validation

fn exactly_one(family: &str, present: &[bool]) -> Result<(), String> {
    match present.iter().filter(|p| **p).count() {
        1 => Ok(()),
        0 => Err(format!("{family} carries no supported operation")),
        n => Err(format!(
            "{family} carries {n} operations; expected exactly one"
        )),
    }
}

fn order_type_ok(order: &OrderWire) -> Result<(), String> {
    match (&order.t.limit, &order.t.trigger) {
        (Some(limit), None) => match limit.tif.as_str() {
            "Alo" | "Ioc" | "Gtc" => Ok(()),
            other => Err(format!("unsupported time-in-force '{other}'")),
        },
        (None, Some(trigger)) => match trigger.tpsl.as_str() {
            "tp" | "sl" => Ok(()),
            other => Err(format!("unsupported trigger kind '{other}'")),
        },
        _ => Err("order type must be exactly one of limit or trigger".to_string()),
    }
}

impl L1Action {
    /// Semantic checks serde cannot express: closed string enums and
    /// one-operation-per-family.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Order(a) => {
                match a.grouping.as_str() {
                    "na" | "normalTpsl" | "positionTpsl" => {}
                    other => return Err(format!("unsupported order grouping '{other}'")),
                }
                a.orders.iter().try_for_each(order_type_ok)
            }
            Self::Modify(m) => order_type_ok(&m.order),
            Self::BatchModify(b) => b.modifies.iter().try_for_each(|m| order_type_ok(&m.order)),
            Self::SpotDeploy(s) => exactly_one(
                "spotDeploy",
                &[
                    s.register_token2.is_some(),
                    s.user_genesis.is_some(),
                    s.genesis.is_some(),
                    s.register_spot.is_some(),
                    s.register_hyperliquidity.is_some(),
                    s.set_deployer_trading_fee_share.is_some(),
                    s.enable_freeze_privilege.is_some(),
                    s.freeze_user.is_some(),
                    s.revoke_freeze_privilege.is_some(),
                    s.enable_quote_token.is_some(),
                    s.disable_quote_token.is_some(),
                ],
            ),
            Self::PerpDeploy(p) => exactly_one(
                "perpDeploy",
                &[
                    p.register_asset.is_some(),
                    p.register_asset2.is_some(),
                    p.set_oracle.is_some(),
                    p.set_funding_multipliers.is_some(),
                    p.halt_trading.is_some(),
                    p.set_margin_table_ids.is_some(),
                    p.set_fee_recipient.is_some(),
                    p.set_open_interest_caps.is_some(),
                    p.set_sub_deployers.is_some(),
                    p.disable_dex.is_some(),
                ],
            ),
            Self::OutcomeDeploy(o) => {
                let op = &o.operation;
                exactly_one(
                    "outcomeDeploy",
                    &[
                        op.register_standalone_outcome_from_template.is_some(),
                        op.register_question_from_template.is_some(),
                        op.register_and_associate_named_outcome_from_template
                            .is_some(),
                        op.settle_outcome.is_some(),
                        op.settle_question2.is_some(),
                        op.set_sub_deployers.is_some(),
                    ],
                )
            }
            Self::ActivateOutcomeDeployer(a) => exactly_one(
                "activateOutcomeDeployer",
                &[a.activate.is_some(), a.deactivate.is_some()],
            ),
            _ => Ok(()),
        }
    }
}
