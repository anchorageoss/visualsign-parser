//! Jupiter Lend Earn preset implementation for Solana.
//!
//! Renders the Jupiter Lend Earn (lending) program `jup3YeL8...`. The
//! user-facing instructions -- the `deposit`, `withdraw`, `mint` and `redeem`
//! families -- get a semantic view: an action title, the amount normalized with
//! the token's decimals and symbol, the asset and receipt-token mints as named
//! addresses, and the signing wallet. Every other instruction keeps the generic
//! IDL rendering (program, instruction, named accounts, raw args).
//!
//! Only the instruction's own bytes and accounts feed the view. Symbols and
//! decimals come from the shared `utils` token table, keyed by mint. A mint
//! outside that table, or one that sits in an address lookup table and cannot
//! be resolved statically, is never guessed: unknown mints render raw units and
//! a truncated address, unresolved mints fall back to the generic view.

mod config;

use crate::core::{
    InstructionView, InstructionVisualizer, SolanaIntegrationConfig, TransactionSummary,
    VisualizerContext, VisualizerKind, format_arg_value, is_unresolved_placeholder,
};
use crate::utils::{TokenInfo, format_token_amount, lookup_token, truncate_address};
use config::JupiterEarnConfig;
use solana_parser::{
    Idl, SolanaParsedInstructionData, decode_idl_data, parse_instruction_with_idl,
};
use solana_sdk::pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::OnceLock;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_raw_data_field, create_text_field,
};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

pub(crate) const JUPITER_EARN_PROGRAM_ID: &str = "jup3YeL8QhtSx1e253b2FDvsMNC87fDrgQZivbrndc9";

const JUPITER_EARN_DISPLAY_NAME: &str = "Jupiter Lend Earn";

const JUPITER_EARN_IDL_JSON: &str = include_str!("jupiter_earn.json");

/// `withdraw(u64::MAX)` is the program's "withdraw everything" sentinel: it
/// burns the signer's whole fToken balance and pays out its value, instead of
/// failing on an amount above the balance. Observed on mainnet, e.g.
/// <https://solscan.io/tx/296YJsTYWiNgJ5b5LENoL6prJeASneukmHofteMdWTvv8DhLNEY19uAEvZYjxQmAWJuCemP4wtzgxtvDXxSS8sXs>
/// (`withdraw(18446744073709551615)` burned 187,120.685353 jlUSDG and paid out
/// 199,999.391176 USDG).
const WITHDRAW_ALL_AMOUNT: u64 = u64::MAX;

const ESTIMATED: &str = "(estimated, rate at execution)";

static JUPITER_EARN_CONFIG: JupiterEarnConfig = JupiterEarnConfig;

pub struct JupiterEarnVisualizer;

impl InstructionVisualizer for JupiterEarnVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let view = InstructionView::from_context(context);
        let data = context.data();

        let instruction_data_hex = hex::encode(data);
        let fallback_text = format!(
            "Program ID: {}\nData: {instruction_data_hex}",
            view.program_id
        );

        let parsed = parse_jupiter_earn_instruction(data, &view.accounts);

        let (title, condensed_fields, mut expanded_fields) = match parsed {
            Ok(parsed) => build_parsed_fields(&parsed, &view.program_id)?,
            Err(e) => {
                let index = context.instruction_index();
                tracing::warn!(
                    "Failed to parse Jupiter Lend Earn instruction {index} with IDL: {e}"
                );
                build_fallback_fields(&view.program_id)?
            }
        };

        let condensed = SignablePayloadFieldListLayout {
            fields: condensed_fields,
        };
        expanded_fields.push(create_raw_data_field(data, Some(instruction_data_hex))?);
        let expanded = SignablePayloadFieldListLayout {
            fields: expanded_fields,
        };

        let preview_layout = SignablePayloadFieldPreviewLayout {
            title: Some(SignablePayloadFieldTextV2 { text: title }),
            subtitle: Some(SignablePayloadFieldTextV2 {
                text: String::new(),
            }),
            condensed: Some(condensed),
            expanded: Some(expanded),
        };

        let index = context.instruction_index() + 1;
        Ok(AnnotatedPayloadField {
            static_annotation: None,
            dynamic_annotation: None,
            signable_payload_field: SignablePayloadField::PreviewLayout {
                common: SignablePayloadFieldCommon {
                    label: format!("Instruction {index}"),
                    fallback_text,
                },
                preview_layout,
            },
        })
    }

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig> {
        Some(&JUPITER_EARN_CONFIG)
    }

    fn kind(&self) -> VisualizerKind {
        VisualizerKind::Lending(JUPITER_EARN_DISPLAY_NAME)
    }

    /// A single recognized user action names the whole transaction and hoists
    /// its key rows to the top level, e.g. "Deposit 414.122446 USDC to Jupiter
    /// Lend Earn" with Program / Amount / Instruction / Receive rows.
    ///
    /// Proposed only when the instruction's signer is the transaction's fee
    /// payer: the hoisted From row names the fee payer, so a relayed
    /// transaction must not read as if the relayer were the depositor. Admin
    /// instructions and undecodable data propose nothing.
    fn transaction_summary(&self, context: &VisualizerContext) -> Option<TransactionSummary> {
        let view = InstructionView::from_context(context);
        let parsed = parse_jupiter_earn_instruction(context.data(), &view.accounts).ok()?;
        if parsed.named_accounts.get("signer")? != &context.sender().account_key {
            return None;
        }
        let action = UserAction::from_parsed(&parsed)?;
        let fields = action
            .summary_fields(&view.program_id, &parsed.parsed.instruction_name)
            .ok()?;
        Some(TransactionSummary {
            title: action.title(),
            subtitle: Some(JUPITER_EARN_DISPLAY_NAME.to_string()),
            fields,
        })
    }
}

fn get_jupiter_earn_idl() -> Option<&'static Idl> {
    static IDL: OnceLock<Option<Idl>> = OnceLock::new();
    IDL.get_or_init(|| decode_idl_data(JUPITER_EARN_IDL_JSON).ok())
        .as_ref()
}

fn parse_jupiter_earn_instruction(
    data: &[u8],
    accounts: &[String],
) -> Result<JupiterEarnParsedInstruction, Box<dyn std::error::Error>> {
    if data.len() < 8 {
        return Err("Invalid instruction data length".into());
    }

    let idl = get_jupiter_earn_idl().ok_or("Jupiter Lend Earn IDL not available")?;
    let parsed = parse_instruction_with_idl(data, JUPITER_EARN_PROGRAM_ID, idl)?;

    let named_accounts = build_named_accounts(data, idl, accounts);

    Ok(JupiterEarnParsedInstruction {
        parsed,
        named_accounts,
    })
}

fn build_named_accounts(data: &[u8], idl: &Idl, accounts: &[String]) -> BTreeMap<String, String> {
    let mut named_accounts = BTreeMap::new();

    let idl_instruction = idl.instructions.iter().find(|inst| {
        inst.discriminator
            .as_ref()
            .is_some_and(|disc| data.get(..disc.len()) == Some(disc.as_slice()))
    });

    if let Some(idl_instruction) = idl_instruction {
        for (index, account_str) in accounts.iter().enumerate() {
            if let Some(idl_account) = idl_instruction.accounts.get(index) {
                named_accounts.insert(idl_account.name.clone(), account_str.clone());
            }
        }
    }

    named_accounts
}

struct JupiterEarnParsedInstruction {
    parsed: SolanaParsedInstructionData,
    named_accounts: BTreeMap<String, String>,
}

/// Which side of a lending position an amount is denominated in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Denomination {
    /// The underlying asset (USDC, USDG, ...): the instruction's `mint`.
    Asset,
    /// The receipt token (jlUSDC, jlUSDG, ...): the instruction's `f_token_mint`.
    Receipt,
}

/// The two mints of a Jupiter Lend Earn position with whatever the shared
/// token table knows about them. `None` info means "unknown mint": amounts in
/// that denomination render as raw units and the mint as a truncated address.
struct LendAsset {
    mint: String,
    info: Option<TokenInfo>,
    receipt_mint: String,
    receipt_info: Option<TokenInfo>,
}

impl LendAsset {
    /// `None` when either mint is missing or is an unresolved lookup-table
    /// placeholder: a placeholder is not an address and must not be shown as
    /// one, so the instruction falls back to the generic view.
    fn from_named_accounts(named_accounts: &BTreeMap<String, String>) -> Option<Self> {
        let mint = named_accounts.get("mint")?;
        let receipt_mint = named_accounts.get("f_token_mint")?;
        if is_unresolved_placeholder(mint) || is_unresolved_placeholder(receipt_mint) {
            return None;
        }
        Some(Self {
            mint: mint.clone(),
            info: lookup_token(mint),
            receipt_mint: receipt_mint.clone(),
            receipt_info: lookup_token(receipt_mint),
        })
    }

    fn mint(&self, denomination: Denomination) -> &str {
        match denomination {
            Denomination::Asset => &self.mint,
            Denomination::Receipt => &self.receipt_mint,
        }
    }

    fn info(&self, denomination: Denomination) -> Option<&TokenInfo> {
        match denomination {
            Denomination::Asset => self.info.as_ref(),
            Denomination::Receipt => self.receipt_info.as_ref(),
        }
    }

    fn symbol(&self, denomination: Denomination) -> String {
        match self.info(denomination) {
            Some(info) => info.symbol.to_string(),
            None => truncate_address(self.mint(denomination)),
        }
    }

    /// One exact amount in `denomination`. Title phrase and row are derived
    /// from the same classification so they can never disagree.
    fn amount(&self, raw: u64, denomination: Denomination) -> Amount<'_> {
        Amount {
            asset: self,
            raw,
            denomination,
        }
    }

    /// A user-set cap (`max_assets`, `max_shares_burn`): `u64::MAX` means the
    /// cap is not in effect. Never used for a minimum: a `u64::MAX` floor is an
    /// unsatisfiable requirement, so minimums render the literal via `amount`.
    fn max_bound_phrase(&self, raw: u64, denomination: Denomination) -> String {
        if raw == u64::MAX {
            "no limit".to_string()
        } else {
            self.amount(raw, denomination).phrase()
        }
    }
}

/// How an exact amount is shown. `u64::MAX` is a verified sentinel only for
/// `withdraw` (`WITHDRAW_ALL_AMOUNT`, handled by `UserAction` before it gets
/// here); for every other instruction the program's behaviour is unverified,
/// so the literal is shown, flagged, and no meaning is claimed.
enum AmountKind {
    /// Known decimals: a normalized number.
    Decimal(String),
    /// Unknown mint: the raw integer, labelled as raw units.
    RawUnits,
    /// `u64::MAX` outside the verified withdraw sentinel.
    UnverifiedMax,
}

struct Amount<'a> {
    asset: &'a LendAsset,
    raw: u64,
    denomination: Denomination,
}

impl Amount<'_> {
    fn kind(&self) -> AmountKind {
        if self.raw == u64::MAX {
            return AmountKind::UnverifiedMax;
        }
        match self.asset.info(self.denomination) {
            Some(info) => AmountKind::Decimal(format_token_amount(self.raw, info.decimals)),
            None => AmountKind::RawUnits,
        }
    }

    /// Lower-case phrase for titles and bound rows: "12.5 USDC",
    /// "12500000 raw units of So11...1112",
    /// "18446744073709551615 raw units of USDC (u64::MAX)".
    fn phrase(&self) -> String {
        let symbol = self.asset.symbol(self.denomination);
        match self.kind() {
            AmountKind::Decimal(amount) => format!("{amount} {symbol}"),
            AmountKind::RawUnits => format!("{} raw units of {symbol}", self.raw),
            AmountKind::UnverifiedMax => format!("{} raw units of {symbol} (u64::MAX)", self.raw),
        }
    }

    /// The same amount as a row: `amount_v2` when it is a number, text for the
    /// flagged literal.
    fn field(&self, label: &str) -> Result<AnnotatedPayloadField, VisualSignError> {
        let symbol = self.asset.symbol(self.denomination);
        match self.kind() {
            AmountKind::Decimal(amount) => create_amount_field(label, &amount, &symbol),
            AmountKind::RawUnits => create_amount_field(
                &format!("{label} (raw units)"),
                &self.raw.to_string(),
                &symbol,
            ),
            AmountKind::UnverifiedMax => create_text_field(label, &self.phrase()),
        }
    }
}

/// Whose token account receives the action's output. Decided statically from
/// the signer, the mint and the token program: the signer's associated token
/// account is a pure function of those three. Nothing else is knowable without
/// chain state, so `Other` only proves "not the signer's associated token
/// account". A signer-owned auxiliary token account is also flagged; that
/// false positive is accepted because a missed third party is the worse error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RecipientOwnership {
    /// The signer's own associated token account.
    Signer,
    /// Not the signer's associated token account: a third party, or a
    /// signer-owned auxiliary account (flagged anyway, see above).
    Other,
    /// An input was unresolved or not a valid pubkey; nothing is claimed.
    Unknown,
}

/// The `recipient_token_account`: where a deposit or mint sends receipt
/// tokens, and where a withdraw or redeem pays out assets. The IDL puts no
/// constraint on it, so it must be shown; it is the one account that decides
/// whether the signer or someone else ends up with the value.
struct Recipient {
    account: String,
    ownership: RecipientOwnership,
}

impl Recipient {
    fn from_named_accounts(
        named_accounts: &BTreeMap<String, String>,
        received_mint: &str,
    ) -> Option<Self> {
        let account = named_accounts.get("recipient_token_account")?.clone();
        let ownership = match (
            named_accounts.get("signer"),
            named_accounts.get("token_program"),
        ) {
            (Some(signer), Some(token_program)) => {
                match (
                    Pubkey::from_str(signer),
                    Pubkey::from_str(received_mint),
                    Pubkey::from_str(token_program),
                    Pubkey::from_str(&account),
                ) {
                    (Ok(signer), Ok(mint), Ok(token_program), Ok(account)) => {
                        let signer_ata = get_associated_token_address_with_program_id(
                            &signer,
                            &mint,
                            &token_program,
                        );
                        if account == signer_ata {
                            RecipientOwnership::Signer
                        } else {
                            RecipientOwnership::Other
                        }
                    }
                    _ => RecipientOwnership::Unknown,
                }
            }
            _ => RecipientOwnership::Unknown,
        };
        Some(Self { account, ownership })
    }

    fn field(&self) -> Result<AnnotatedPayloadField, VisualSignError> {
        let (name, badge) = match self.ownership {
            RecipientOwnership::Signer => (Some("Signer's associated token account"), None),
            RecipientOwnership::Other => (
                Some("Not the signer's associated token account"),
                Some("THIRD PARTY"),
            ),
            RecipientOwnership::Unknown => (None, None),
        };
        create_address_field("Recipient", &self.account, name, None, None, badge)
    }
}

/// A user-facing Jupiter Lend Earn action, decoded from a parsed instruction.
struct UserAction {
    asset: LendAsset,
    recipient: Option<Recipient>,
    kind: UserActionKind,
}

enum UserActionKind {
    /// `deposit` / `deposit_with_min_amount_out`: exact assets in, receipt out.
    Deposit {
        assets: u64,
        min_receipt_out: Option<u64>,
    },
    /// `mint` / `mint_with_max_assets`: exact receipt out, assets in.
    Mint { shares: u64, max_assets: Option<u64> },
    /// `withdraw` / `withdraw_with_max_shares_burn`: exact assets out, receipt burned.
    Withdraw {
        amount: u64,
        max_shares_burn: Option<u64>,
    },
    /// `redeem` / `redeem_with_min_amount_out`: exact receipt burned, assets out.
    Redeem {
        shares: u64,
        min_assets_out: Option<u64>,
    },
}

impl UserAction {
    fn from_parsed(instruction: &JupiterEarnParsedInstruction) -> Option<Self> {
        let args = &instruction.parsed.program_call_args;
        let kind = match instruction.parsed.instruction_name.as_str() {
            "deposit" | "deposit_with_min_amount_out" => UserActionKind::Deposit {
                assets: u64_arg(args, "assets")?,
                min_receipt_out: u64_arg(args, "min_amount_out"),
            },
            "mint" | "mint_with_max_assets" => UserActionKind::Mint {
                shares: u64_arg(args, "shares")?,
                max_assets: u64_arg(args, "max_assets"),
            },
            "withdraw" | "withdraw_with_max_shares_burn" => UserActionKind::Withdraw {
                amount: u64_arg(args, "amount")?,
                max_shares_burn: u64_arg(args, "max_shares_burn"),
            },
            "redeem" | "redeem_with_min_amount_out" => UserActionKind::Redeem {
                shares: u64_arg(args, "shares")?,
                min_assets_out: u64_arg(args, "min_amount_out"),
            },
            _ => return None,
        };
        let asset = LendAsset::from_named_accounts(&instruction.named_accounts)?;
        // Deposit and mint send receipt tokens to the recipient; withdraw and
        // redeem pay assets out to it.
        let received = match kind {
            UserActionKind::Deposit { .. } | UserActionKind::Mint { .. } => Denomination::Receipt,
            UserActionKind::Withdraw { .. } | UserActionKind::Redeem { .. } => Denomination::Asset,
        };
        let recipient =
            Recipient::from_named_accounts(&instruction.named_accounts, asset.mint(received));
        Some(Self {
            asset,
            recipient,
            kind,
        })
    }

    fn action_name(&self) -> &'static str {
        match self.kind {
            UserActionKind::Deposit { .. } => "Deposit",
            UserActionKind::Mint { .. } => "Mint",
            UserActionKind::Withdraw { .. } => "Withdraw",
            UserActionKind::Redeem { .. } => "Redeem",
        }
    }

    fn title(&self) -> String {
        let asset = &self.asset;
        match self.kind {
            UserActionKind::Deposit { assets, .. } => format!(
                "Deposit {} to {JUPITER_EARN_DISPLAY_NAME}",
                asset.amount(assets, Denomination::Asset).phrase()
            ),
            UserActionKind::Mint { shares, .. } => format!(
                "Mint {} on {JUPITER_EARN_DISPLAY_NAME}",
                asset.amount(shares, Denomination::Receipt).phrase()
            ),
            UserActionKind::Withdraw {
                amount: WITHDRAW_ALL_AMOUNT,
                ..
            } => format!(
                "Withdraw full {} position from {JUPITER_EARN_DISPLAY_NAME}",
                asset.symbol(Denomination::Asset)
            ),
            UserActionKind::Withdraw { amount, .. } => format!(
                "Withdraw {} from {JUPITER_EARN_DISPLAY_NAME}",
                asset.amount(amount, Denomination::Asset).phrase()
            ),
            UserActionKind::Redeem { shares, .. } => format!(
                "Redeem {} from {JUPITER_EARN_DISPLAY_NAME}",
                asset.amount(shares, Denomination::Receipt).phrase()
            ),
        }
    }

    /// The exact amount the user signs, as one `amount_v2` row (text for the
    /// full-position sentinel and for the flagged `u64::MAX` literal).
    fn amount_field(&self) -> Result<AnnotatedPayloadField, VisualSignError> {
        let asset = &self.asset;
        let (raw, denomination) = match self.kind {
            UserActionKind::Deposit { assets, .. } => (assets, Denomination::Asset),
            UserActionKind::Mint { shares, .. } => (shares, Denomination::Receipt),
            UserActionKind::Withdraw {
                amount: WITHDRAW_ALL_AMOUNT,
                ..
            } => {
                return create_text_field(
                    "Amount",
                    &format!(
                        "Full {} position (all {} burned)",
                        asset.symbol(Denomination::Asset),
                        asset.symbol(Denomination::Receipt)
                    ),
                );
            }
            UserActionKind::Withdraw { amount, .. } => (amount, Denomination::Asset),
            UserActionKind::Redeem { shares, .. } => (shares, Denomination::Receipt),
        };
        asset.amount(raw, denomination).field("Amount")
    }

    /// The other leg of the position (receipt minted or burned, asset paid or
    /// received), which the program settles at its execution-time rate, plus
    /// any user-set bound on it.
    fn leg_fields(&self) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let asset = &self.asset;
        let mut fields = Vec::new();
        let asset_symbol = asset.symbol(Denomination::Asset);
        let receipt_symbol = asset.symbol(Denomination::Receipt);

        match self.kind {
            UserActionKind::Deposit {
                min_receipt_out, ..
            } => {
                fields.push(create_text_field(
                    "Receive",
                    &format!("{receipt_symbol} {ESTIMATED}"),
                )?);
                if let Some(min) = min_receipt_out {
                    fields.push(create_text_field(
                        "Minimum received",
                        &asset.amount(min, Denomination::Receipt).phrase(),
                    )?);
                }
            }
            UserActionKind::Mint { max_assets, .. } => {
                fields.push(create_text_field(
                    "Pay",
                    &format!("{asset_symbol} {ESTIMATED}"),
                )?);
                if let Some(max) = max_assets {
                    fields.push(create_text_field(
                        "Maximum paid",
                        &asset.max_bound_phrase(max, Denomination::Asset),
                    )?);
                }
            }
            UserActionKind::Withdraw {
                amount,
                max_shares_burn,
            } => {
                if amount != WITHDRAW_ALL_AMOUNT {
                    fields.push(create_text_field(
                        "Burn",
                        &format!("{receipt_symbol} {ESTIMATED}"),
                    )?);
                }
                if let Some(max) = max_shares_burn {
                    fields.push(create_text_field(
                        "Maximum burned",
                        &asset.max_bound_phrase(max, Denomination::Receipt),
                    )?);
                }
            }
            UserActionKind::Redeem {
                min_assets_out, ..
            } => {
                fields.push(create_text_field(
                    "Receive",
                    &format!("{asset_symbol} {ESTIMATED}"),
                )?);
                if let Some(min) = min_assets_out {
                    fields.push(create_text_field(
                        "Minimum received",
                        &asset.amount(min, Denomination::Asset).phrase(),
                    )?);
                }
            }
        }
        Ok(fields)
    }

    /// Rows hoisted to the top level of the payload for a single-action
    /// transaction: the program as a named address, the exact amount, who
    /// receives the output, the instruction, then the estimated other leg. The
    /// signer is omitted because the From row already names it (see
    /// `transaction_summary`).
    fn summary_fields(
        &self,
        program_id: &str,
        instruction_name: &str,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let mut fields = vec![
            create_address_field(
                "Program",
                program_id,
                Some(JUPITER_EARN_DISPLAY_NAME),
                None,
                None,
                None,
            )?,
            self.amount_field()?,
        ];
        if let Some(recipient) = &self.recipient {
            fields.push(recipient.field()?);
        }
        fields.push(create_text_field("Instruction", instruction_name)?);
        fields.extend(self.leg_fields()?);
        Ok(fields)
    }

    /// A mint as a named address row: the table's name and symbol when known,
    /// the bare address otherwise.
    fn mint_field(
        &self,
        label: &str,
        denomination: Denomination,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let asset = &self.asset;
        let info = asset.info(denomination);
        create_address_field(
            label,
            asset.mint(denomination),
            info.map(|i| i.name),
            None,
            info.map(|i| i.symbol),
            None,
        )
    }
}

fn u64_arg(args: &serde_json::Map<String, serde_json::Value>, name: &str) -> Option<u64> {
    args.get(name).and_then(|v| v.as_u64())
}

type PreviewParts = (
    String,
    Vec<AnnotatedPayloadField>,
    Vec<AnnotatedPayloadField>,
);

fn build_parsed_fields(
    instruction: &JupiterEarnParsedInstruction,
    program_id: &str,
) -> Result<PreviewParts, VisualSignError> {
    let parsed = &instruction.parsed;

    let (title, condensed_fields) = match UserAction::from_parsed(instruction) {
        Some(action) => (
            action.title(),
            build_user_action_condensed(&action, instruction, program_id)?,
        ),
        None => (
            format!("{JUPITER_EARN_DISPLAY_NAME}: {}", parsed.instruction_name),
            build_generic_condensed(parsed)?,
        ),
    };

    let mut expanded_fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
        create_text_field("Discriminator", &parsed.discriminator)?,
    ];

    for (account_name, account_address) in &instruction.named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    for (key, value) in &parsed.program_call_args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    Ok((title, condensed_fields, expanded_fields))
}

/// Condensed rows for a user action, ordered by what an approver must confirm
/// first: program, action, signing wallet, who receives the output, asset,
/// amount, then the other leg.
fn build_user_action_condensed(
    action: &UserAction,
    instruction: &JupiterEarnParsedInstruction,
    program_id: &str,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_address_field(
            "Program",
            program_id,
            Some(JUPITER_EARN_DISPLAY_NAME),
            None,
            None,
            None,
        )?,
        create_text_field("Action", action.action_name())?,
    ];

    if let Some(signer) = instruction.named_accounts.get("signer") {
        fields.push(create_address_field("Signer", signer, None, None, None, None)?);
    }
    if let Some(recipient) = &action.recipient {
        fields.push(recipient.field()?);
    }

    fields.push(action.mint_field("Asset", Denomination::Asset)?);
    fields.push(action.amount_field()?);
    fields.extend(action.leg_fields()?);
    fields.push(action.mint_field("Receipt token", Denomination::Receipt)?);

    Ok(fields)
}

fn build_generic_condensed(
    parsed: &SolanaParsedInstructionData,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program", JUPITER_EARN_DISPLAY_NAME)?,
        create_text_field("Instruction", &parsed.instruction_name)?,
    ];
    for (key, value) in &parsed.program_call_args {
        fields.push(create_text_field(key, &format_arg_value(value))?);
    }
    Ok(fields)
}

fn build_fallback_fields(program_id: &str) -> Result<PreviewParts, VisualSignError> {
    let title = format!("{JUPITER_EARN_DISPLAY_NAME}: Unknown Instruction");

    let condensed_fields = vec![
        create_text_field("Program", JUPITER_EARN_DISPLAY_NAME)?,
        create_text_field("Status", "Unknown instruction type")?,
    ];

    let expanded_fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Status", "Unknown instruction type")?,
    ];

    Ok((title, condensed_fields, expanded_fields))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    mod fixture_test;
    mod summary_test;

    #[test]
    fn test_jupiter_earn_idl_loads() {
        let idl = get_jupiter_earn_idl().expect("Jupiter Lend Earn IDL should load");
        assert!(!idl.instructions.is_empty(), "IDL should have instructions");
    }

    #[test]
    fn test_jupiter_earn_idl_has_discriminators() {
        let idl = get_jupiter_earn_idl().unwrap();
        for instruction in &idl.instructions {
            let disc = instruction
                .discriminator
                .as_ref()
                .unwrap_or_else(|| panic!("instruction {} missing discriminator", instruction.name));
            assert_eq!(disc.len(), 8, "instruction {} discriminator", instruction.name);
        }
    }

    /// Every instruction this preset renders semantically must exist in the
    /// bundled IDL under exactly this name, or it would silently fall back to
    /// the generic view.
    #[test]
    fn test_jupiter_earn_idl_covers_user_facing_instructions() {
        let idl = get_jupiter_earn_idl().unwrap();
        for name in [
            "deposit",
            "deposit_with_min_amount_out",
            "mint",
            "mint_with_max_assets",
            "withdraw",
            "withdraw_with_max_shares_burn",
            "redeem",
            "redeem_with_min_amount_out",
        ] {
            assert!(
                idl.instructions.iter().any(|i| i.name == name),
                "IDL is missing instruction {name}"
            );
        }
    }

    #[test]
    fn test_unknown_discriminator_returns_error() {
        let garbage_data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let result = parse_jupiter_earn_instruction(&garbage_data, &[]);
        assert!(result.is_err(), "Unknown discriminator should return error");
    }

    #[test]
    fn test_short_data_returns_error() {
        let short_data = [0x01, 0x02, 0x03];
        let result = parse_jupiter_earn_instruction(&short_data, &[]);
        assert!(result.is_err(), "Short data should return error");
    }

    fn accounts(mint: &str, receipt: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("mint".to_string(), mint.to_string()),
            ("f_token_mint".to_string(), receipt.to_string()),
        ])
    }

    #[test]
    fn test_unknown_mints_render_raw_units_without_guessing() {
        let asset = LendAsset::from_named_accounts(&accounts(
            "So11111111111111111111111111111111111111112",
            "So11111111111111111111111111111111111111113",
        ))
        .unwrap();
        assert_eq!(asset.symbol(Denomination::Asset), "So11...1112");
        assert_eq!(asset.symbol(Denomination::Receipt), "So11...1113");
        assert_eq!(
            asset.amount(414_122_446, Denomination::Asset).phrase(),
            "414122446 raw units of So11...1112"
        );
    }

    #[test]
    fn test_known_mints_normalize_decimals() {
        let asset = LendAsset::from_named_accounts(&accounts(
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D",
        ))
        .unwrap();
        assert_eq!(asset.symbol(Denomination::Asset), "USDC");
        assert_eq!(asset.symbol(Denomination::Receipt), "jlUSDC");
        assert_eq!(
            asset.amount(414_122_446, Denomination::Asset).phrase(),
            "414.122446 USDC"
        );
        assert_eq!(
            asset.amount(100_000_000, Denomination::Receipt).phrase(),
            "100 jlUSDC"
        );
    }

    /// The receipt symbol is never derived from the asset symbol: the JupUSD
    /// market's receipt token is listed as JUICED, which no prefix rule yields.
    #[test]
    fn test_receipt_symbol_comes_from_the_receipt_mint() {
        let asset = LendAsset::from_named_accounts(&accounts(
            "JuprjznTrTSp2UFa3ZBUFgwdAmtZCq4MQCwysN55USD",
            "7GxATsNMnaC88vdwd2t3mwrFuQwwGvmYPrUQ4D6FotXk",
        ))
        .unwrap();
        assert_eq!(asset.symbol(Denomination::Asset), "JupUSD");
        assert_eq!(asset.symbol(Denomination::Receipt), "JUICED");
    }

    #[test]
    fn test_unresolved_lookup_table_mint_is_not_an_asset() {
        assert!(
            LendAsset::from_named_accounts(&accounts(
                "unresolved(12)",
                "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D",
            ))
            .is_none()
        );
        assert!(
            LendAsset::from_named_accounts(&accounts(
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "unresolved(7)",
            ))
            .is_none()
        );
    }
}
