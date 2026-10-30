//! Render verified requests as [`SignablePayload`] fields.
//!
//! Values are shown as signed: prices, sizes and decimal amounts are the exact
//! strings from the action. Assets are shown by index: a trading asset is labelled
//! perp, spot or builder-deployed perp from its index range, and other indices
//! (tokens, outcomes) as `#<index>`. Mapping indices to symbols needs exchange
//! metadata and is a later milestone.

use visualsign::field_builders::{create_address_field, create_preview_layout, create_text_field};
use visualsign::time_fmt::format_timestamp_ms;
use visualsign::vsptrait::VisualSignError;
use visualsign::{AnnotatedPayloadField, SignablePayload};

use crate::input::{L1Request, UserSignedRequest, hex_lower};
use crate::l1::*;
use crate::user_signed::{FieldValue, agent_valid_until};

type Fields = Vec<AnnotatedPayloadField>;
type R<T> = Result<T, VisualSignError>;

fn text(label: &str, value: &str) -> R<AnnotatedPayloadField> {
    create_text_field(label, value)
}

fn address(label: &str, value: &str) -> R<AnnotatedPayloadField> {
    create_address_field(label, value, None, None, None, None)
}

fn timestamp(ms: u64) -> String {
    match i64::try_from(ms) {
        Ok(ms_i64) => format!("{} ({ms})", format_timestamp_ms(ms_i64)),
        Err(_) => format!("invalid timestamp ({ms})"),
    }
}

fn asset(index: u64) -> String {
    format!("#{index}")
}

/// A trading asset id, which encodes its market in its range: perps are `index`,
/// spot pairs `10000 + index`, and builder-deployed (HIP-3) perps
/// `100000 + dex * 10000 + index`. The raw id is kept whenever it differs from the
/// index shown, so the label can be checked against the signed action.
fn trading_asset(id: u64) -> String {
    match id {
        0..10_000 => format!("perp #{id}"),
        10_000..100_000 => format!("spot #{} (asset {id})", id - 10_000),
        _ => {
            let rest = id - 100_000;
            format!(
                "builder perp dex {} #{} (asset {id})",
                rest / 10_000,
                rest % 10_000
            )
        }
    }
}

fn yes_no(b: bool) -> &'static str {
    if b { "Yes" } else { "No" }
}

fn side(is_buy: bool) -> &'static str {
    if is_buy { "Buy" } else { "Sell" }
}

/// A 6-decimal USDC integer as a decimal string, sign preserved.
fn usd6(raw: i128) -> String {
    let sign = if raw < 0 { "-" } else { "" };
    let abs = raw.unsigned_abs();
    format!("{sign}{}.{:06}", abs / 1_000_000, abs % 1_000_000)
}

fn finish(title: String, fields: Fields, payload_type: &str) -> SignablePayload {
    SignablePayload::new(
        0,
        title,
        None,
        fields
            .into_iter()
            .map(|f| f.signable_payload_field)
            .collect(),
        payload_type.to_string(),
    )
}

// ---------------------------------------------------------------- L1

pub fn l1_payload(req: &L1Request, title: Option<String>) -> R<SignablePayload> {
    let mut fields: Fields = vec![
        text("Network", if req.mainnet { "Mainnet" } else { "Testnet" })?,
        text("Action", action_label(&req.action))?,
    ];
    fields.extend(action_fields(&req.action)?);
    fields.push(match &req.vault_address {
        Some(vault) => address(
            "Acting For (Vault / Subaccount)",
            &format!("0x{}", hex_lower(vault)),
        )?,
        None => text(
            "Acting For (Vault / Subaccount)",
            "None (signer's own account)",
        )?,
    });
    fields.push(text("Nonce", &timestamp(req.nonce))?);
    fields.push(text(
        "Expires After",
        &req.expires_after
            .map(timestamp)
            .unwrap_or_else(|| "Not set".to_string()),
    )?);
    fields.push(text(
        "Connection ID",
        &format!(
            "0x{} (verified against the action)",
            hex_lower(&req.connection_id)
        ),
    )?);
    let title = title.unwrap_or_else(|| format!("Hyperliquid: {}", action_label(&req.action)));
    Ok(finish(title, fields, "HyperliquidL1Action"))
}

fn action_label(action: &L1Action) -> &'static str {
    match action {
        L1Action::Order(_) => "Place Order",
        L1Action::Cancel(_) => "Cancel Order",
        L1Action::CancelByCloid(_) => "Cancel Order by Client ID",
        L1Action::Modify(_) => "Modify Order",
        L1Action::BatchModify(_) => "Modify Orders",
        L1Action::ScheduleCancel(_) => "Schedule Cancel-All",
        L1Action::UpdateLeverage(_) => "Update Leverage",
        L1Action::UpdateIsolatedMargin(_) => "Update Isolated Margin",
        L1Action::TwapOrder(_) => "Place TWAP Order",
        L1Action::TwapCancel(_) => "Cancel TWAP Order",
        L1Action::VaultTransfer(_) => "Vault Transfer",
        L1Action::SubAccountTransfer(_) => "Subaccount Transfer",
        L1Action::PerpDeploy(_) => "Perp Deploy (HIP-3)",
        L1Action::SpotDeploy(_) => "Spot Deploy (HIP-1/HIP-2)",
        L1Action::OutcomeDeploy(_) => "Outcome Deploy (HIP-4)",
        L1Action::ActivateOutcomeDeployer(_) => "Outcome Deployer Activation (HIP-4)",
        L1Action::Noop(_) => "No-op (Invalidate Nonce)",
    }
}

fn order_type(t: &OrderTypeWire) -> String {
    match (&t.limit, &t.trigger) {
        (Some(limit), _) => format!("Limit ({})", limit.tif),
        (None, Some(trig)) => format!(
            "Trigger {} at {} ({})",
            if trig.tpsl == "tp" {
                "take-profit"
            } else {
                "stop-loss"
            },
            trig.trigger_px,
            if trig.is_market { "market" } else { "limit" }
        ),
        (None, None) => "unknown".to_string(),
    }
}

fn order_fields(o: &OrderWire) -> R<Fields> {
    let mut f = vec![
        text("Asset", &trading_asset(o.a))?,
        text("Side", side(o.b))?,
        text("Limit Price", &o.p)?,
        text("Size", &o.s)?,
        text("Order Type", &order_type(&o.t))?,
        text("Reduce Only", yes_no(o.r))?,
    ];
    if let Some(c) = &o.c {
        f.push(text("Client Order ID", c)?);
    }
    Ok(f)
}

/// The condensed line is all a hardware wallet shows, so it carries the order type
/// and reduce-only flag as well as side, size, asset and price.
fn order_subtitle(o: &OrderWire) -> String {
    format!(
        "{} {} of {} at {}, {}{}",
        side(o.b),
        o.s,
        trading_asset(o.a),
        o.p,
        order_type(&o.t),
        if o.r { ", reduce-only" } else { "" }
    )
}

fn oid(o: &OidOrCloid) -> String {
    match o {
        OidOrCloid::Oid(n) => n.to_string(),
        OidOrCloid::Cloid(c) => format!("client id {c}"),
    }
}

fn pairs(label: &str, items: &[(String, String)]) -> R<AnnotatedPayloadField> {
    let body = if items.is_empty() {
        "(none)".to_string()
    } else {
        items
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    text(label, &body)
}

fn sub_deployers(list: &[SubDeployer]) -> R<Fields> {
    list.iter()
        .enumerate()
        .map(|(i, s)| {
            text(
                &format!("Sub-deployer {}", i + 1),
                &format!(
                    "{} {} for {}",
                    if s.allowed { "Grant" } else { "Revoke" },
                    s.variant,
                    s.user
                ),
            )
        })
        .collect()
}

fn schema_fields(schema: &Option<PerpDexSchema>) -> R<Fields> {
    Ok(match schema {
        None => vec![text("New DEX", "No (existing DEX)")?],
        Some(s) => vec![
            text("New DEX", "Yes")?,
            text("DEX Full Name", &s.full_name)?,
            text("Collateral Token", &asset(s.collateral_token))?,
            text(
                "Oracle Updater",
                s.oracle_updater.as_deref().unwrap_or("deployer"),
            )?,
        ],
    })
}

fn max_gas(g: Option<u64>) -> String {
    g.map(|g| g.to_string())
        .unwrap_or_else(|| "current auction price".to_string())
}

fn settlement_fields(prefix: &str, s: &OutcomeSettlement) -> R<Fields> {
    Ok(vec![
        text(&format!("{prefix}Outcome"), &asset(s.outcome))?,
        text(
            &format!("{prefix}Settle Fraction (YES)"),
            &s.settle_fraction,
        )?,
        text(&format!("{prefix}Name"), &s.name_and_description[0])?,
        text(&format!("{prefix}Description"), &s.name_and_description[1])?,
        text(
            &format!("{prefix}Sides"),
            &format!("{} / {}", s.side_names[0], s.side_names[1]),
        )?,
        text(&format!("{prefix}Details"), &s.details)?,
    ])
}

fn action_fields(action: &L1Action) -> R<Fields> {
    let mut f: Fields = Vec::new();
    match action {
        L1Action::Order(a) => {
            f.push(text("Grouping", &a.grouping)?);
            for (i, o) in a.orders.iter().enumerate() {
                f.push(create_preview_layout(
                    &format!("Order {}", i + 1),
                    order_subtitle(o),
                    order_fields(o)?,
                ));
            }
            if let Some(b) = &a.builder {
                f.push(address("Builder", &b.b)?);
                f.push(text(
                    "Builder Fee",
                    &format!("{} (tenths of a basis point)", b.f),
                )?);
            }
        }
        L1Action::Cancel(a) => {
            for (i, c) in a.cancels.iter().enumerate() {
                f.push(text(
                    &format!("Cancel {}", i + 1),
                    &format!("order {} on {}", c.o, trading_asset(c.a)),
                )?);
            }
            if let Some(fast) = a.f {
                f.push(text("Fast", yes_no(fast))?);
            }
        }
        L1Action::CancelByCloid(a) => {
            for (i, c) in a.cancels.iter().enumerate() {
                f.push(text(
                    &format!("Cancel {}", i + 1),
                    &format!("client id {} on {}", c.cloid, trading_asset(c.asset)),
                )?);
            }
            if let Some(fast) = a.f {
                f.push(text("Fast", yes_no(fast))?);
            }
        }
        L1Action::Modify(m) => {
            f.push(text("Order to Modify", &oid(&m.oid))?);
            f.push(create_preview_layout(
                "New Order",
                order_subtitle(&m.order),
                order_fields(&m.order)?,
            ));
        }
        L1Action::BatchModify(b) => {
            for (i, m) in b.modifies.iter().enumerate() {
                let mut inner = vec![text("Order to Modify", &oid(&m.oid))?];
                inner.extend(order_fields(&m.order)?);
                f.push(create_preview_layout(
                    &format!("Modify {}", i + 1),
                    format!("{} -> {}", oid(&m.oid), order_subtitle(&m.order)),
                    inner,
                ));
            }
        }
        L1Action::ScheduleCancel(s) => f.push(text(
            "Cancel All Orders At",
            &s.time
                .map(timestamp)
                .unwrap_or_else(|| "Unset (clears any scheduled cancel)".to_string()),
        )?),
        L1Action::UpdateLeverage(u) => {
            f.push(text("Asset", &trading_asset(u.asset))?);
            f.push(text("Leverage", &format!("{}x", u.leverage))?);
            f.push(text(
                "Margin Mode",
                if u.is_cross { "Cross" } else { "Isolated" },
            )?);
        }
        L1Action::UpdateIsolatedMargin(u) => {
            f.push(text("Asset", &trading_asset(u.asset))?);
            f.push(text(
                if u.ntli < 0 {
                    "Remove Margin (USDC)"
                } else {
                    "Add Margin (USDC)"
                },
                &usd6(i128::from(u.ntli)),
            )?);
            f.push(text("Is Buy", yes_no(u.is_buy))?);
        }
        L1Action::TwapOrder(t) => {
            let w = &t.twap;
            f.push(text("Asset", &trading_asset(w.a))?);
            f.push(text("Side", side(w.b))?);
            f.push(text("Size", &w.s)?);
            f.push(text("Duration (minutes)", &w.m.to_string())?);
            f.push(text("Randomize", yes_no(w.t))?);
            f.push(text("Reduce Only", yes_no(w.r))?);
        }
        L1Action::TwapCancel(t) => {
            f.push(text("Asset", &trading_asset(t.a))?);
            f.push(text("TWAP ID", &t.t.to_string())?);
        }
        L1Action::VaultTransfer(v) => {
            f.push(text(
                "Direction",
                if v.is_deposit { "Deposit" } else { "Withdraw" },
            )?);
            f.push(address("Vault", &v.vault_address)?);
            f.push(text("Amount (USDC)", &usd6(i128::from(v.usd)))?);
        }
        L1Action::SubAccountTransfer(s) => {
            f.push(text(
                "Direction",
                if s.is_deposit {
                    "Deposit into subaccount"
                } else {
                    "Withdraw from subaccount"
                },
            )?);
            f.push(address("Subaccount", &s.sub_account_user)?);
            f.push(text("Amount (USDC)", &usd6(i128::from(s.usd)))?);
        }
        L1Action::Noop(_) => {}
        L1Action::SpotDeploy(s) => spot_deploy_fields(s, &mut f)?,
        L1Action::PerpDeploy(p) => perp_deploy_fields(p, &mut f)?,
        L1Action::OutcomeDeploy(o) => outcome_deploy_fields(o, &mut f)?,
        L1Action::ActivateOutcomeDeployer(a) => {
            if let Some(v) = &a.activate {
                f.push(text("Operation", "Activate")?);
                f.push(text("Venue Name", &v.venue_name)?);
            } else {
                f.push(text("Operation", "Deactivate (permanent)")?);
            }
        }
    }
    Ok(f)
}

fn spot_deploy_fields(s: &SpotDeployAction, f: &mut Fields) -> R<()> {
    if let Some(r) = &s.register_token2 {
        f.push(text("Operation", "Register Token")?);
        f.push(text("Token Name", &r.spec.name)?);
        f.push(text("Size Decimals", &r.spec.sz_decimals.to_string())?);
        f.push(text("Wei Decimals", &r.spec.wei_decimals.to_string())?);
        f.push(text("Max Gas", &r.max_gas.to_string())?);
        if let Some(n) = &r.full_name {
            f.push(text("Full Name", n)?);
        }
    } else if let Some(g) = &s.user_genesis {
        f.push(text("Operation", "User Genesis")?);
        f.push(text("Token", &asset(g.token))?);
        f.push(pairs("User Allocations (wei)", &g.user_and_wei)?);
        let existing: Vec<(String, String)> = g
            .existing_token_and_wei
            .iter()
            .map(|(t, w)| (asset(*t), w.clone()))
            .collect();
        f.push(pairs("Existing Token Holders (wei)", &existing)?);
        if let Some(b) = &g.blacklist_users {
            let list: Vec<(String, String)> = b
                .iter()
                .map(|(u, bl)| (u.clone(), yes_no(*bl).to_string()))
                .collect();
            f.push(pairs("Blacklist Users", &list)?);
        }
    } else if let Some(g) = &s.genesis {
        f.push(text("Operation", "Genesis")?);
        f.push(text("Token", &asset(g.token))?);
        f.push(text("Max Supply", &g.max_supply)?);
        if let Some(n) = g.no_hyperliquidity {
            f.push(text("No Hyperliquidity", yes_no(n))?);
        }
    } else if let Some(r) = &s.register_spot {
        f.push(text("Operation", "Register Spot Pair")?);
        f.push(text("Base Token", &asset(r.tokens[0]))?);
        f.push(text("Quote Token", &asset(r.tokens[1]))?);
    } else if let Some(r) = &s.register_hyperliquidity {
        f.push(text("Operation", "Register Hyperliquidity")?);
        f.push(text("Spot", &asset(r.spot))?);
        f.push(text("Start Price", &r.start_px)?);
        f.push(text("Order Size", &r.order_sz)?);
        f.push(text("Number of Orders", &r.n_orders.to_string())?);
        if let Some(n) = r.n_seeded_levels {
            f.push(text("Seeded Levels", &n.to_string())?);
        }
    } else if let Some(r) = &s.set_deployer_trading_fee_share {
        f.push(text("Operation", "Set Deployer Trading Fee Share")?);
        f.push(text("Token", &asset(r.token))?);
        f.push(text("Share", &r.share)?);
    } else if let Some(t) = &s.enable_freeze_privilege {
        f.push(text("Operation", "Enable Freeze Privilege")?);
        f.push(text("Token", &asset(t.token))?);
    } else if let Some(u) = &s.freeze_user {
        f.push(text(
            "Operation",
            if u.freeze {
                "Freeze User"
            } else {
                "Unfreeze User"
            },
        )?);
        f.push(text("Token", &asset(u.token))?);
        f.push(address("User", &u.user)?);
    } else if let Some(t) = &s.revoke_freeze_privilege {
        f.push(text("Operation", "Revoke Freeze Privilege")?);
        f.push(text("Token", &asset(t.token))?);
    } else if let Some(t) = &s.enable_quote_token {
        f.push(text("Operation", "Enable Quote Token")?);
        f.push(text("Token", &asset(t.token))?);
    } else if let Some(t) = &s.disable_quote_token {
        f.push(text("Operation", "Disable Quote Token")?);
        f.push(text("Token", &asset(t.token))?);
    }
    Ok(())
}

fn perp_deploy_fields(p: &PerpDeployAction, f: &mut Fields) -> R<()> {
    if let Some(r) = &p.register_asset {
        f.push(text("Operation", "Register Asset")?);
        f.push(text("DEX", &r.dex)?);
        f.push(text("Coin", &r.asset_request.coin)?);
        f.push(text(
            "Size Decimals",
            &r.asset_request.sz_decimals.to_string(),
        )?);
        f.push(text("Oracle Price", &r.asset_request.oracle_px)?);
        f.push(text(
            "Margin Table",
            &r.asset_request.margin_table_id.to_string(),
        )?);
        f.push(text(
            "Isolated Only",
            yes_no(r.asset_request.only_isolated),
        )?);
        f.push(text("Max Gas", &max_gas(r.max_gas))?);
        f.extend(schema_fields(&r.schema)?);
    } else if let Some(r) = &p.register_asset2 {
        f.push(text("Operation", "Register Asset")?);
        f.push(text("DEX", &r.dex)?);
        f.push(text("Coin", &r.asset_request.coin)?);
        f.push(text(
            "Size Decimals",
            &r.asset_request.sz_decimals.to_string(),
        )?);
        f.push(text("Oracle Price", &r.asset_request.oracle_px)?);
        f.push(text(
            "Margin Table",
            &r.asset_request.margin_table_id.to_string(),
        )?);
        f.push(text("Margin Mode", &r.asset_request.margin_mode)?);
        f.push(text("Max Gas", &max_gas(r.max_gas))?);
        f.extend(schema_fields(&r.schema)?);
    } else if let Some(s) = &p.set_oracle {
        f.push(text("Operation", "Set Oracle Prices")?);
        f.push(text("DEX", &s.dex)?);
        f.push(pairs("Oracle Prices", &s.oracle_pxs)?);
        for (i, m) in s.mark_pxs.iter().enumerate() {
            f.push(pairs(&format!("Mark Prices {}", i + 1), m)?);
        }
        f.push(pairs("External Perp Prices", &s.external_perp_pxs)?);
    } else if let Some(m) = &p.set_funding_multipliers {
        f.push(text("Operation", "Set Funding Multipliers")?);
        f.push(pairs("Multipliers", m)?);
    } else if let Some(h) = &p.halt_trading {
        f.push(text(
            "Operation",
            if h.is_halted {
                "Halt Trading"
            } else {
                "Resume Trading"
            },
        )?);
        f.push(text("Coin", &h.coin)?);
    } else if let Some(m) = &p.set_margin_table_ids {
        f.push(text("Operation", "Set Margin Table IDs")?);
        let list: Vec<(String, String)> = m
            .iter()
            .map(|(a, id)| (a.clone(), id.to_string()))
            .collect();
        f.push(pairs("Margin Tables", &list)?);
    } else if let Some(r) = &p.set_fee_recipient {
        f.push(text("Operation", "Set Fee Recipient")?);
        f.push(text("DEX", &r.dex)?);
        f.push(address("Fee Recipient", &r.fee_recipient)?);
    } else if let Some(c) = &p.set_open_interest_caps {
        f.push(text("Operation", "Set Open Interest Caps")?);
        let list: Vec<(String, String)> = c
            .iter()
            .map(|(a, cap)| (a.clone(), cap.to_string()))
            .collect();
        f.push(pairs("Caps (notional)", &list)?);
    } else if let Some(s) = &p.set_sub_deployers {
        f.push(text("Operation", "Set Sub-deployers")?);
        f.push(text("DEX", &s.dex)?);
        f.extend(sub_deployers(&s.sub_deployers)?);
    } else if let Some(d) = &p.disable_dex {
        f.push(text("Operation", "Disable DEX (permanent)")?);
        f.push(text("DEX", d)?);
    }
    Ok(())
}

fn template_fields(prefix: &str, id: &str, kv: &[(String, String)]) -> R<Fields> {
    Ok(vec![
        text(&format!("{prefix}Template"), id)?,
        pairs(&format!("{prefix}Keywords"), kv)?,
    ])
}

fn outcome_deploy_fields(o: &OutcomeDeployAction, f: &mut Fields) -> R<()> {
    f.push(text("Venue", &o.venue)?);
    let op = &o.operation;
    if let Some(t) = &op.register_standalone_outcome_from_template {
        f.push(text("Operation", "Register Standalone Outcome")?);
        f.extend(template_fields("", &t.id, &t.keyword_to_value)?);
        f.push(text("Deployer Fee Scale", &t.deployer_fee_scale)?);
    } else if let Some(q) = &op.register_question_from_template {
        f.push(text("Operation", "Register Question")?);
        let qt = &q.question_template_instance;
        f.extend(template_fields("Question ", &qt.id, &qt.keyword_to_value)?);
        f.push(text("Deployer Fee Scale", &qt.deployer_fee_scale)?);
        for (i, n) in q.named_outcome_template_instances.iter().enumerate() {
            f.extend(template_fields(
                &format!("Outcome {} ", i + 1),
                &n.id,
                &n.keyword_to_value,
            )?);
        }
    } else if let Some(r) = &op.register_and_associate_named_outcome_from_template {
        f.push(text("Operation", "Add Outcome to Question")?);
        f.push(text("Question", &asset(r.question))?);
        let n = &r.named_outcome_template_instance;
        f.extend(template_fields("", &n.id, &n.keyword_to_value)?);
    } else if let Some(s) = &op.settle_outcome {
        f.push(text("Operation", "Settle Outcome")?);
        f.extend(settlement_fields("", s)?);
    } else if let Some(s) = &op.settle_question2 {
        f.push(text("Operation", "Settle Question")?);
        f.push(text("Question", &asset(s.question))?);
        f.push(text("Question Name", &s.name_and_description[0])?);
        f.push(text("Question Description", &s.name_and_description[1])?);
        for (i, st) in s.outcome_settlements.iter().enumerate() {
            f.extend(settlement_fields(&format!("Settlement {} ", i + 1), st)?);
        }
    } else if let Some(list) = &op.set_sub_deployers {
        f.push(text("Operation", "Set Sub-deployers")?);
        f.extend(sub_deployers(list)?);
    }
    Ok(())
}

// ---------------------------------------------------------------- user-signed

fn user_label(name: &str) -> &'static str {
    match name {
        "ApproveAgent" => "Approve API Wallet (Agent)",
        "TokenDelegate" => "Stake Delegation",
        "Withdraw" => "Withdraw to Arbitrum",
        "UsdSend" => "Send USDC",
        "SpotSend" => "Send Spot Token",
        "SendAsset" => "Send Asset",
        "ApproveBuilderFee" => "Approve Builder Fee",
        "UsdClassTransfer" => "Transfer USDC Between Spot and Perp",
        "AcceptTerms" => "Accept Hyperliquid Terms",
        _ => "Unknown",
    }
}

fn field_label(name: &str) -> &'static str {
    match name {
        "agentAddress" => "Agent Address",
        "agentName" => "Agent Name",
        "validator" => "Validator",
        "wei" => "Amount (wei)",
        "isUndelegate" => "Undelegate",
        "destination" => "Destination",
        "amount" => "Amount",
        "token" => "Token",
        "sourceDex" => "Source DEX",
        "destinationDex" => "Destination DEX",
        "fromSubAccount" => "From Subaccount",
        "maxFeeRate" => "Max Fee Rate",
        "builder" => "Builder",
        "toPerp" => "To Perp",
        "time" => "Time",
        "nonce" => "Nonce",
        _ => "Field",
    }
}

/// Approving the zero address removes the agent: Hyperliquid's "Remove API Wallet".
fn is_agent_revoke(req: &UserSignedRequest) -> bool {
    req.spec_name == "ApproveAgent"
        && req.fields.iter().any(|(name, value)| {
            *name == "agentAddress"
                && matches!(value, FieldValue::Address(a) if a[2..].bytes().all(|b| b == b'0'))
        })
}

pub fn user_signed_payload(req: &UserSignedRequest, title: Option<String>) -> R<SignablePayload> {
    let revoke = is_agent_revoke(req);
    let label = if revoke {
        "Revoke API Wallet (Agent)"
    } else {
        user_label(req.spec_name)
    };
    let mut fields: Fields = Vec::new();
    for (name, value) in &req.fields {
        if *name == "hyperliquidChain" {
            if let FieldValue::Str(chain) = value {
                fields.push(text("Network", chain)?);
            }
            fields.push(text("Action", label)?);
            continue;
        }
        let shown = field_label(name);
        match value {
            FieldValue::Address(a) => fields.push(address(shown, a)?),
            FieldValue::Str(s) if *name == "sourceDex" || *name == "destinationDex" => {
                fields.push(text(
                    shown,
                    if s.is_empty() {
                        "(default perp DEX)"
                    } else {
                        s
                    },
                )?)
            }
            FieldValue::Str(s) if *name == "fromSubAccount" => {
                fields.push(text(shown, if s.is_empty() { "(none)" } else { s })?)
            }
            FieldValue::Str(s) => {
                fields.push(text(shown, if s.is_empty() { "(empty)" } else { s })?)
            }
            FieldValue::U64(n) if *name == "time" || *name == "nonce" => {
                fields.push(text(shown, &timestamp(*n))?)
            }
            FieldValue::U64(n) => fields.push(text(shown, &n.to_string())?),
            FieldValue::Bool(b) => fields.push(text(shown, yes_no(*b))?),
        }
        if req.spec_name == "ApproveAgent"
            && !revoke
            && *name == "agentName"
            && let FieldValue::Str(agent_name) = value
        {
            let expiry = agent_valid_until(agent_name).map_err(VisualSignError::ConversionError)?;
            fields.push(match expiry {
                Some(ms) => text("Agent Expiry", &timestamp(ms))?,
                // Hyperliquid may apply its own default lifetime to an agent approved
                // without one, but nothing in the signed request bounds it.
                None => text(
                    "Agent Expiry",
                    "No expiry in the signed request (Hyperliquid may apply its own default)",
                )?,
            });
        }
    }
    fields.push(text("Signature Chain ID", &req.chain_id.to_string())?);
    let title = title.unwrap_or_else(|| format!("Hyperliquid: {label}"));
    Ok(finish(title, fields, "HyperliquidUserSignedAction"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trading_asset_labels_the_market_from_the_index_range() {
        assert_eq!(trading_asset(3), "perp #3");
        assert_eq!(trading_asset(9_999), "perp #9999");
        assert_eq!(trading_asset(10_000), "spot #0 (asset 10000)");
        assert_eq!(trading_asset(11_035), "spot #1035 (asset 11035)");
        assert_eq!(trading_asset(99_999), "spot #89999 (asset 99999)");
        assert_eq!(
            trading_asset(110_000),
            "builder perp dex 1 #0 (asset 110000)"
        );
        assert_eq!(
            trading_asset(120_005),
            "builder perp dex 2 #5 (asset 120005)"
        );
    }
}
