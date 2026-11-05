//! Shared rendering for programs where the signer supplies an asset, receives a receipt token
//! and later redeems it (Jupiter Lend Earn, Abra); a preset declares one [`ReceiptTokenProgram`].

use crate::core::{
    InstructionView, NamedAccounts, SolanaIntegrationConfigData, TransactionSummary,
    VisualizerContext, find_idl_instruction, format_arg_value, is_unresolved_placeholder,
    name_idl_accounts,
};
use crate::idl::builtin_programs::preset_idl_records;
use crate::presets::associated_token_account::interface::get_associated_token_address_with_program_id;
use crate::utils::{TokenInfo, format_token_amount, lookup_token, truncate_address};
use solana_parser::Idl;
use solana_parser::solana::idl_parser::parse_data_into_args;
use solana_pubkey::Pubkey;
use std::collections::BTreeMap;
use std::str::FromStr;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_raw_data_field, create_text_field,
};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod test_support;

const ESTIMATED: &str = "(estimated, rate at execution)";

/// One program rendered by a receipt-token preset.
pub(crate) struct ReceiptTokenProgram {
    pub program_id: &'static str,
    /// On-chain program name: the subtitle and the Program rows.
    pub display_name: &'static str,
    /// Product name substituted for `{program}` in titles.
    pub title_name: &'static str,
    /// IDL account names shared by every action. `asset_mint` is the default an
    /// [`Action`] may override.
    pub signer: &'static str,
    pub asset_mint: &'static str,
    pub receipt_mint: &'static str,
    pub token_program: &'static str,
    /// Name the leg settled at the program's rate ("Receive jlUSDC (estimated, ...)").
    /// Off for programs that must not show what the signer might receive.
    pub estimated_legs: bool,
    pub actions: &'static [Action],
}

/// A user-facing instruction. Instructions not listed keep the generic IDL view.
pub(crate) struct Action {
    pub instruction: &'static str,
    pub flow: Flow,
    /// Action row text, e.g. "Deposit".
    pub label: &'static str,
    /// Title template; `{asset}`, `{receipt}` and `{program}` are substituted. Names
    /// assets, never amounts: the Amount row carries the amount.
    pub title: &'static str,
    /// Title when `amount_arg` is `u64::MAX` and that sentinel is verified to mean the
    /// whole position. `None`: the literal is shown and flagged, and no meaning is claimed.
    pub full_position_title: Option<&'static str>,
    /// The `u64` argument carrying the exact amount, denominated per [`Flow::exact`].
    pub amount_arg: &'static str,
    /// Overrides the program's asset mint account name.
    pub asset_mint: Option<&'static str>,
    pub recipient: RecipientRule,
    /// The `u64` argument bounding the counter leg: a minimum for flows that receive it,
    /// a maximum for flows that pay or burn it.
    pub bound: Option<&'static str>,
    /// Extra condensed rows naming program accounts: (label, IDL account name).
    pub account_rows: &'static [(&'static str, &'static str)],
}

impl Action {
    pub(crate) const fn new(
        instruction: &'static str,
        flow: Flow,
        label: &'static str,
        title: &'static str,
        amount_arg: &'static str,
        recipient: RecipientRule,
    ) -> Self {
        Self {
            instruction,
            flow,
            label,
            title,
            full_position_title: None,
            amount_arg,
            asset_mint: None,
            recipient,
            bound: None,
            account_rows: &[],
        }
    }

    pub(crate) const fn with_bound(self, arg: &'static str) -> Self {
        Self {
            bound: Some(arg),
            ..self
        }
    }

    pub(crate) const fn with_full_position_title(self, title: &'static str) -> Self {
        // Verified on mainnet for a plain withdraw only; elsewhere "all {receipt} burned" is false.
        assert!(matches!(self.flow, Flow::Withdraw) && self.bound.is_none());
        Self {
            full_position_title: Some(title),
            ..self
        }
    }

    pub(crate) const fn with_asset_mint(self, name: &'static str) -> Self {
        Self {
            asset_mint: Some(name),
            ..self
        }
    }

    pub(crate) const fn with_account_rows(
        self,
        rows: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            account_rows: rows,
            ..self
        }
    }

    fn asset_mint_name(&self, program: &ReceiptTokenProgram) -> &'static str {
        self.asset_mint.unwrap_or(program.asset_mint)
    }
}

/// What the exact amount does and which leg the program settles at its own rate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Flow {
    /// Exact assets in, receipt out at the program's rate.
    Deposit,
    /// Exact receipt out, assets in at the program's rate.
    Mint,
    /// Exact assets out, receipt burned at the program's rate.
    Withdraw,
    /// Exact receipt burned, assets out at the program's rate.
    Redeem,
    /// Exact receipt set aside (escrowed or burned) now; assets claimable after a cooldown.
    RequestRedeem,
    /// Exact amount collected from the signer's escrow after the cooldown.
    Claim { exact: Denomination },
    /// Exact receipt returned to the signer from the escrow.
    CancelRequest,
}

impl Flow {
    /// Denomination of the action's `amount_arg`.
    fn exact(self) -> Denomination {
        match self {
            Flow::Deposit | Flow::Withdraw => Denomination::Asset,
            Flow::Mint | Flow::Redeem | Flow::RequestRedeem | Flow::CancelRequest => {
                Denomination::Receipt
            }
            Flow::Claim { exact } => exact,
        }
    }

    /// What lands in the recipient token account. A request only escrows, so the escrow
    /// rule says what it holds.
    fn received(self) -> Option<Denomination> {
        match self {
            Flow::Deposit | Flow::Mint | Flow::CancelRequest => Some(Denomination::Receipt),
            Flow::Withdraw | Flow::Redeem | Flow::Claim { .. } => Some(Denomination::Asset),
            Flow::RequestRedeem => None,
        }
    }

    fn counter_leg(self) -> Option<CounterLeg> {
        let leg = match self {
            Flow::Deposit => CounterLeg::minimum("Receive", "received", Denomination::Receipt),
            Flow::Mint => CounterLeg::maximum("Pay", "paid", Denomination::Asset),
            Flow::Withdraw => CounterLeg::maximum("Burn", "burned", Denomination::Receipt),
            Flow::Redeem => CounterLeg::minimum("Receive", "received", Denomination::Asset),
            Flow::RequestRedeem => {
                CounterLeg::minimum("Receive at claim", "received", Denomination::Asset)
            }
            Flow::Claim {
                exact: Denomination::Receipt,
            } => CounterLeg::minimum("Receive", "received", Denomination::Asset),
            Flow::Claim {
                exact: Denomination::Asset,
            }
            | Flow::CancelRequest => return None,
        };
        Some(leg)
    }
}

/// The leg the program settles at execution time, and how a user-set bound on it reads.
struct CounterLeg {
    verb: &'static str,
    past: &'static str,
    denomination: Denomination,
    bound: BoundKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoundKind {
    Minimum,
    Maximum,
}

impl CounterLeg {
    const fn minimum(verb: &'static str, past: &'static str, denomination: Denomination) -> Self {
        Self {
            verb,
            past,
            denomination,
            bound: BoundKind::Minimum,
        }
    }

    const fn maximum(verb: &'static str, past: &'static str, denomination: Denomination) -> Self {
        Self {
            verb,
            past,
            denomination,
            bound: BoundKind::Maximum,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Denomination {
    /// The underlying asset (USDC, USDaf, ...).
    Asset,
    /// The receipt token (jlUSDC, sUSDaf, ...).
    Receipt,
}

/// Which account receives the action's value and how to verify it is the signer's own.
pub(crate) enum RecipientRule {
    /// The named account must be the signer's associated token account for the received mint.
    SignerTokenAccount(&'static str),
    /// The named account must be the associated token account, for the mint it `holds`, of
    /// the per-signer `request` account, itself the program address of `seeds`.
    SignerEscrow {
        account: &'static str,
        request: &'static str,
        seeds: &'static [Seed],
        holds: Denomination,
    },
}

pub(crate) enum Seed {
    Literal(&'static [u8]),
    /// The key of the named IDL account.
    Account(&'static str),
}

pub(crate) struct ParsedInstruction {
    pub instruction_name: String,
    pub discriminator: String,
    pub args: serde_json::Map<String, serde_json::Value>,
    pub named_accounts: BTreeMap<String, String>,
    /// False when the instruction has fewer accounts than the IDL lists (omitted
    /// optionals), so positional names may be misaligned.
    pub accounts_complete: bool,
}

impl ReceiptTokenProgram {
    /// The IDL registered for `program_id` in `idl::builtin_programs`.
    pub(crate) fn idl(&self) -> Option<&'static Idl> {
        preset_idl_records()
            .get(self.program_id)?
            .custom_idl
            .as_ref()
    }

    pub(crate) fn parse(
        &self,
        data: &[u8],
        accounts: &[String],
    ) -> Result<ParsedInstruction, Box<dyn std::error::Error>> {
        if data.len() < 8 {
            return Err("Invalid instruction data length".into());
        }
        let idl = self
            .idl()
            .ok_or_else(|| format!("{} IDL not available", self.display_name))?;
        let instruction = find_idl_instruction(idl, data).ok_or_else(|| {
            format!(
                "no matching instruction discriminator found for instruction data: {}",
                hex::encode(data)
            )
        })?;
        let args = parse_data_into_args(data, instruction, idl).map_err(|e| {
            format!(
                "failed to parse instruction data for instruction '{}': {e}",
                instruction.name
            )
        })?;
        let NamedAccounts {
            accounts: named_accounts,
            complete: accounts_complete,
        } = name_idl_accounts(instruction, accounts);
        Ok(ParsedInstruction {
            instruction_name: instruction.name.clone(),
            discriminator: hex::encode(instruction.discriminator.as_deref().unwrap_or_default()),
            args,
            named_accounts,
            accounts_complete,
        })
    }

    fn action(&self, instruction_name: &str) -> Option<&'static Action> {
        self.actions
            .iter()
            .find(|action| action.instruction == instruction_name)
    }

    fn program_field(&self, program_id: &str) -> Result<AnnotatedPayloadField, VisualSignError> {
        create_address_field(
            "Program",
            program_id,
            Some(self.display_name),
            None,
            None,
            None,
        )
    }
}

pub(crate) fn find_program<'a>(
    programs: &'a [ReceiptTokenProgram],
    program_id: &str,
) -> Option<&'a ReceiptTokenProgram> {
    programs
        .iter()
        .find(|program| program.program_id == program_id)
}

/// Registers every instruction of every program with the dispatcher.
pub(crate) fn config_data(programs: &[ReceiptTokenProgram]) -> SolanaIntegrationConfigData {
    let programs = programs
        .iter()
        .map(|program| (program.program_id, BTreeMap::from([("*", vec!["*"])])))
        .collect();
    SolanaIntegrationConfigData { programs }
}

pub(crate) fn visualize(
    programs: &[ReceiptTokenProgram],
    context: &VisualizerContext,
) -> Result<AnnotatedPayloadField, VisualSignError> {
    visualize_with_summary(programs, context).map(|(field, _)| field)
}

pub(crate) fn transaction_summary(
    programs: &[ReceiptTokenProgram],
    context: &VisualizerContext,
) -> Option<TransactionSummary> {
    visualize_with_summary(programs, context).ok()?.1
}

/// The instruction's field and, for a single recognized action, its transaction summary.
pub(crate) fn visualize_with_summary(
    programs: &[ReceiptTokenProgram],
    context: &VisualizerContext,
) -> Result<(AnnotatedPayloadField, Option<TransactionSummary>), VisualSignError> {
    let view = InstructionView::from_context(context);
    let data = context.data();
    let program = find_program(programs, &view.program_id).ok_or_else(|| {
        VisualSignError::DecodeError(format!(
            "program {} is not handled by this preset",
            view.program_id
        ))
    })?;

    let instruction_data_hex = hex::encode(data);
    let fallback_text = format!(
        "Program ID: {}\nData: {instruction_data_hex}",
        view.program_id
    );

    let (parts, summary) = match program.parse(data, &view.accounts) {
        Ok(parsed) => {
            let action = UserAction::from_parsed(program, &parsed);
            let summary = action
                .as_ref()
                .and_then(|action| action.summary(&parsed, &view.program_id, context));
            (
                build_parsed_fields(program, &parsed, action.as_ref(), &view.program_id)?,
                summary,
            )
        }
        Err(e) => {
            let index = context.instruction_index();
            tracing::warn!(
                "Failed to parse {} instruction {index} with IDL: {e}",
                program.display_name
            );
            (build_fallback_fields(program, &view.program_id)?, None)
        }
    };
    let (title, condensed_fields, mut expanded_fields) = parts;

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
    let field = AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                label: format!("Instruction {index}"),
                fallback_text,
            },
            preview_layout,
        },
    };
    Ok((field, summary))
}

/// Both mints of a position. `None` info = unknown mint: raw units and a truncated address.
pub(crate) struct Position {
    asset_mint: String,
    asset_info: Option<TokenInfo>,
    receipt_mint: String,
    receipt_info: Option<TokenInfo>,
}

impl Position {
    /// `None` on a missing or unresolved (lookup-table placeholder) mint: generic view.
    pub(crate) fn from_named_accounts(
        named_accounts: &BTreeMap<String, String>,
        asset_mint: &str,
        receipt_mint: &str,
    ) -> Option<Self> {
        let asset = named_accounts.get(asset_mint)?;
        let receipt = named_accounts.get(receipt_mint)?;
        if is_unresolved_placeholder(asset) || is_unresolved_placeholder(receipt) {
            return None;
        }
        Some(Self {
            asset_mint: asset.clone(),
            asset_info: lookup_token(asset),
            receipt_mint: receipt.clone(),
            receipt_info: lookup_token(receipt),
        })
    }

    pub(crate) fn mint(&self, denomination: Denomination) -> &str {
        match denomination {
            Denomination::Asset => &self.asset_mint,
            Denomination::Receipt => &self.receipt_mint,
        }
    }

    fn info(&self, denomination: Denomination) -> Option<&TokenInfo> {
        match denomination {
            Denomination::Asset => self.asset_info.as_ref(),
            Denomination::Receipt => self.receipt_info.as_ref(),
        }
    }

    pub(crate) fn symbol(&self, denomination: Denomination) -> String {
        match self.info(denomination) {
            Some(info) => info.symbol.to_string(),
            None => truncate_address(self.mint(denomination)),
        }
    }

    /// Title phrase and row derive from one classification so they cannot disagree.
    pub(crate) fn amount(&self, raw: u64, denomination: Denomination) -> Amount<'_> {
        Amount {
            position: self,
            raw,
            denomination,
        }
    }

    /// Caps only: `u64::MAX` means no cap. A `u64::MAX` minimum is unsatisfiable, so
    /// minimums render the literal via `amount`.
    fn max_bound_phrase(&self, raw: u64, denomination: Denomination) -> String {
        if raw == u64::MAX {
            "no limit".to_string()
        } else {
            self.amount(raw, denomination).phrase()
        }
    }
}

/// `u64::MAX` is a verified sentinel only where an action says so (handled before this);
/// everywhere else the literal is shown and flagged, and no meaning is claimed.
enum AmountKind {
    Decimal(String),
    RawUnits,
    UnverifiedMax,
}

pub(crate) struct Amount<'a> {
    position: &'a Position,
    raw: u64,
    denomination: Denomination,
}

impl Amount<'_> {
    fn kind(&self) -> AmountKind {
        if self.raw == u64::MAX {
            return AmountKind::UnverifiedMax;
        }
        match self.position.info(self.denomination) {
            Some(info) => AmountKind::Decimal(format_token_amount(self.raw, info.decimals)),
            None => AmountKind::RawUnits,
        }
    }

    /// Lower-case phrase: "12.5 USDC", "12500000 raw units of So11...1112", "... (u64::MAX)".
    pub(crate) fn phrase(&self) -> String {
        let symbol = self.position.symbol(self.denomination);
        match self.kind() {
            AmountKind::Decimal(amount) => format!("{amount} {symbol}"),
            AmountKind::RawUnits => format!("{} raw units of {symbol}", self.raw),
            AmountKind::UnverifiedMax => format!("{} raw units of {symbol} (u64::MAX)", self.raw),
        }
    }

    fn field(&self, label: &str) -> Result<AnnotatedPayloadField, VisualSignError> {
        let symbol = self.position.symbol(self.denomination);
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

/// Static classification of where the value lands. `Other` also catches signer-owned
/// auxiliary accounts; that false positive is accepted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RecipientOwnership {
    Signer,
    /// Not the signer's account (third party or auxiliary account). No summary is proposed.
    Other,
    /// Signer, token program or a seed unresolved or unknown: badged UNVERIFIED, no summary.
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RecipientKind {
    TokenAccount,
    Escrow,
}

/// `token_program` is author-controlled; ATA derivation only means anything under the two
/// real token programs, so any other key must not classify the recipient.
pub(crate) fn is_known_token_program(token_program: &Pubkey) -> bool {
    *token_program == spl_token_interface::id() || *token_program == token_2022_program_id()
}

pub(crate) fn token_2022_program_id() -> Pubkey {
    spl_token_2022_interface::id()
}

/// The account the IDL leaves to the client, so it decides who ends up with the value.
pub(crate) struct Recipient {
    pub account: String,
    pub ownership: RecipientOwnership,
    kind: RecipientKind,
}

impl Recipient {
    /// `None` when the account is missing or unresolved: generic view.
    fn resolve(
        program: &ReceiptTokenProgram,
        action: &Action,
        named_accounts: &BTreeMap<String, String>,
        position: &Position,
    ) -> Option<Self> {
        let signer = named_accounts.get(program.signer);
        let token_program = named_accounts.get(program.token_program);
        match &action.recipient {
            RecipientRule::SignerTokenAccount(name) => {
                let account = named_accounts.get(*name)?.clone();
                if is_unresolved_placeholder(&account) {
                    return None;
                }
                let ownership = match (signer, token_program, action.flow.received()) {
                    (Some(signer), Some(token_program), Some(received)) => classify_token_account(
                        &account,
                        signer,
                        position.mint(received),
                        token_program,
                    ),
                    _ => RecipientOwnership::Unknown,
                };
                Some(Self {
                    account,
                    ownership,
                    kind: RecipientKind::TokenAccount,
                })
            }
            RecipientRule::SignerEscrow {
                account: name,
                request,
                seeds,
                holds,
            } => {
                let account = named_accounts.get(*name)?.clone();
                if is_unresolved_placeholder(&account) {
                    return None;
                }
                let keyed_by_signer = seeds.iter().any(
                    |seed| matches!(seed, Seed::Account(seed_name) if *seed_name == program.signer),
                );
                let ownership = match (named_accounts.get(*request), token_program) {
                    (Some(request_account), Some(token_program)) if keyed_by_signer => {
                        classify_escrow(
                            program.program_id,
                            &account,
                            request_account,
                            seeds,
                            named_accounts,
                            position.mint(*holds),
                            token_program,
                        )
                    }
                    _ => RecipientOwnership::Unknown,
                };
                Some(Self {
                    account,
                    ownership,
                    kind: RecipientKind::Escrow,
                })
            }
        }
    }

    fn field(&self) -> Result<AnnotatedPayloadField, VisualSignError> {
        let (label, own, other) = match self.kind {
            RecipientKind::TokenAccount => (
                "Recipient",
                "Signer's associated token account",
                "Not the signer's associated token account",
            ),
            RecipientKind::Escrow => (
                "Escrow",
                "Signer's escrow account",
                "Not the signer's escrow account",
            ),
        };
        let (name, badge) = match self.ownership {
            RecipientOwnership::Signer => (own, None),
            RecipientOwnership::Other => (other, Some("THIRD PARTY")),
            RecipientOwnership::Unknown => ("Ownership not verified", Some("UNVERIFIED")),
        };
        create_address_field(label, &self.account, Some(name), None, None, badge)
    }
}

/// The signer's ATA is a pure function of signer, mint and token program.
fn classify_token_account(
    account: &str,
    signer: &str,
    mint: &str,
    token_program: &str,
) -> RecipientOwnership {
    match (
        Pubkey::from_str(signer),
        Pubkey::from_str(mint),
        Pubkey::from_str(token_program),
        Pubkey::from_str(account),
    ) {
        (Ok(signer), Ok(mint), Ok(token_program), Ok(account))
            if is_known_token_program(&token_program) =>
        {
            let signer_ata =
                get_associated_token_address_with_program_id(&signer, &mint, &token_program);
            if account == signer_ata {
                RecipientOwnership::Signer
            } else {
                RecipientOwnership::Other
            }
        }
        _ => RecipientOwnership::Unknown,
    }
}

/// The request account must be the program address of `seeds` and the escrow its ATA for
/// `mint`: both derivations must hold for the escrow to count as the signer's.
fn classify_escrow(
    program_id: &str,
    account: &str,
    request_account: &str,
    seeds: &[Seed],
    named_accounts: &BTreeMap<String, String>,
    mint: &str,
    token_program: &str,
) -> RecipientOwnership {
    let keys = (
        Pubkey::from_str(program_id),
        Pubkey::from_str(account),
        Pubkey::from_str(request_account),
        Pubkey::from_str(mint),
        Pubkey::from_str(token_program),
    );
    let (Ok(program_id), Ok(account), Ok(request_account), Ok(mint), Ok(token_program)) = keys
    else {
        return RecipientOwnership::Unknown;
    };
    if !is_known_token_program(&token_program) {
        return RecipientOwnership::Unknown;
    }
    let mut seed_bytes: Vec<Vec<u8>> = Vec::with_capacity(seeds.len());
    for seed in seeds {
        match seed {
            Seed::Literal(bytes) => seed_bytes.push(bytes.to_vec()),
            Seed::Account(name) => {
                let Some(Ok(key)) = named_accounts.get(*name).map(|key| Pubkey::from_str(key))
                else {
                    return RecipientOwnership::Unknown;
                };
                seed_bytes.push(key.to_bytes().to_vec());
            }
        }
    }
    let seed_refs: Vec<&[u8]> = seed_bytes.iter().map(Vec::as_slice).collect();
    let Some((derived_request, _bump)) = Pubkey::try_find_program_address(&seed_refs, &program_id)
    else {
        return RecipientOwnership::Unknown;
    };
    if derived_request != request_account {
        return RecipientOwnership::Other;
    }
    let escrow =
        get_associated_token_address_with_program_id(&request_account, &mint, &token_program);
    if account == escrow {
        RecipientOwnership::Signer
    } else {
        RecipientOwnership::Other
    }
}

/// A program account the action names as a row; a lookup-table placeholder is shown badged.
struct AccountRow {
    label: &'static str,
    account: String,
}

impl AccountRow {
    fn resolved(&self) -> bool {
        !is_unresolved_placeholder(&self.account)
    }

    fn field(&self) -> Result<AnnotatedPayloadField, VisualSignError> {
        if self.resolved() {
            create_address_field(self.label, &self.account, None, None, None, None)
        } else {
            create_address_field(
                self.label,
                &self.account,
                Some("Not resolved (address lookup table)"),
                None,
                None,
                Some("UNVERIFIED"),
            )
        }
    }
}

pub(crate) struct UserAction<'a> {
    program: &'a ReceiptTokenProgram,
    action: &'static Action,
    position: Position,
    pub recipient: Recipient,
    rows: Vec<AccountRow>,
    amount: u64,
    bound: Option<u64>,
}

impl<'a> UserAction<'a> {
    /// `None` for admin instructions and anything not fully decoded (short account list,
    /// missing amount or bound, unresolved mint or recipient): those keep the generic view.
    pub(crate) fn from_parsed(
        program: &'a ReceiptTokenProgram,
        instruction: &ParsedInstruction,
    ) -> Option<Self> {
        if !instruction.accounts_complete {
            return None;
        }
        let action = program.action(&instruction.instruction_name)?;
        let args = &instruction.args;
        let amount = u64_arg(args, action.amount_arg)?;
        let bound = match action.bound {
            Some(arg) => Some(u64_arg(args, arg)?),
            None => None,
        };
        let position = Position::from_named_accounts(
            &instruction.named_accounts,
            action.asset_mint_name(program),
            program.receipt_mint,
        )?;
        let recipient =
            Recipient::resolve(program, action, &instruction.named_accounts, &position)?;
        let mut rows = Vec::with_capacity(action.account_rows.len());
        for &(label, name) in action.account_rows {
            rows.push(AccountRow {
                label,
                account: instruction.named_accounts.get(name)?.clone(),
            });
        }
        Some(Self {
            program,
            action,
            position,
            recipient,
            rows,
            amount,
            bound,
        })
    }

    /// Hoisted rows are reserved for verified content: value staying with the signer and
    /// resolved destinations. Badged rows stay in the instruction view.
    fn hoistable(&self) -> bool {
        self.recipient.ownership == RecipientOwnership::Signer
            && self.rows.iter().all(AccountRow::resolved)
    }

    /// Only when the signer is the fee payer, since the hoisted From row names the fee payer.
    fn summary(
        &self,
        instruction: &ParsedInstruction,
        program_id: &str,
        context: &VisualizerContext,
    ) -> Option<TransactionSummary> {
        let signer = instruction.named_accounts.get(self.program.signer)?;
        if signer != &context.sender().account_key || !self.hoistable() {
            return None;
        }
        let fields = self.summary_fields(instruction, program_id).ok()?;
        Some(TransactionSummary {
            title: self.title(),
            subtitle: Some(self.program.display_name.to_string()),
            fields,
        })
    }

    fn withdraws_all(&self) -> bool {
        self.action.full_position_title.is_some() && self.amount == u64::MAX
    }

    pub(crate) fn title(&self) -> String {
        let template = match self.action.full_position_title {
            Some(title) if self.withdraws_all() => title,
            _ => self.action.title,
        };
        template
            .replace("{asset}", &self.position.symbol(Denomination::Asset))
            .replace("{receipt}", &self.position.symbol(Denomination::Receipt))
            .replace("{program}", self.program.title_name)
    }

    fn amount_field(&self) -> Result<AnnotatedPayloadField, VisualSignError> {
        if self.withdraws_all() {
            return create_text_field(
                "Amount",
                &format!(
                    "Full {} position (all {} burned)",
                    self.position.symbol(Denomination::Asset),
                    self.position.symbol(Denomination::Receipt)
                ),
            );
        }
        self.position
            .amount(self.amount, self.action.flow.exact())
            .field("Amount")
    }

    /// The other leg, settled at the program's execution-time rate, plus any user-set bound.
    fn leg_fields(&self) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let mut fields = Vec::new();
        let Some(leg) = self.action.flow.counter_leg() else {
            return Ok(fields);
        };
        if self.program.estimated_legs && !self.withdraws_all() {
            fields.push(create_text_field(
                leg.verb,
                &format!("{} {ESTIMATED}", self.position.symbol(leg.denomination)),
            )?);
        }
        if let Some(bound) = self.bound {
            let (label, phrase) = match leg.bound {
                BoundKind::Minimum => (
                    format!("Minimum {}", leg.past),
                    self.position.amount(bound, leg.denomination).phrase(),
                ),
                BoundKind::Maximum => (
                    format!("Maximum {}", leg.past),
                    self.position.max_bound_phrase(bound, leg.denomination),
                ),
            };
            fields.push(create_text_field(&label, &phrase)?);
        }
        Ok(fields)
    }

    fn row_fields(&self) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        self.rows.iter().map(AccountRow::field).collect()
    }

    /// Top-level rows for a single-action transaction. The signer is omitted because the
    /// From row already names it.
    fn summary_fields(
        &self,
        instruction: &ParsedInstruction,
        program_id: &str,
    ) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
        let mut fields = vec![
            self.program.program_field(program_id)?,
            self.amount_field()?,
            self.recipient.field()?,
        ];
        fields.extend(self.row_fields()?);
        fields.push(create_text_field(
            "Instruction",
            &instruction.instruction_name,
        )?);
        fields.extend(self.leg_fields()?);
        Ok(fields)
    }

    fn mint_field(
        &self,
        label: &str,
        denomination: Denomination,
    ) -> Result<AnnotatedPayloadField, VisualSignError> {
        let info = self.position.info(denomination);
        create_address_field(
            label,
            self.position.mint(denomination),
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
    program: &ReceiptTokenProgram,
    instruction: &ParsedInstruction,
    action: Option<&UserAction>,
    program_id: &str,
) -> Result<PreviewParts, VisualSignError> {
    let (title, condensed_fields) = match action {
        Some(action) => (
            action.title(),
            build_user_action_condensed(action, instruction, program_id)?,
        ),
        None => (
            format!("{}: {}", program.display_name, instruction.instruction_name),
            build_generic_condensed(program, instruction)?,
        ),
    };

    let mut expanded_fields = vec![
        create_text_field("Program ID", program_id)?,
        create_text_field("Instruction", &instruction.instruction_name)?,
        create_text_field("Discriminator", &instruction.discriminator)?,
    ];

    for (account_name, account_address) in &instruction.named_accounts {
        expanded_fields.push(create_text_field(account_name, account_address)?);
    }

    for (key, value) in &instruction.args {
        expanded_fields.push(create_text_field(key, &format_arg_value(value))?);
    }

    Ok((title, condensed_fields, expanded_fields))
}

/// Ordered by what an approver confirms first.
fn build_user_action_condensed(
    action: &UserAction,
    instruction: &ParsedInstruction,
    program_id: &str,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        action.program.program_field(program_id)?,
        create_text_field("Action", action.action.label)?,
    ];

    if let Some(signer) = instruction.named_accounts.get(action.program.signer) {
        fields.push(create_address_field(
            "Signer", signer, None, None, None, None,
        )?);
    }
    fields.push(action.recipient.field()?);
    fields.extend(action.row_fields()?);

    fields.push(action.mint_field("Asset", Denomination::Asset)?);
    fields.push(action.amount_field()?);
    fields.extend(action.leg_fields()?);
    fields.push(action.mint_field("Receipt token", Denomination::Receipt)?);

    Ok(fields)
}

fn build_generic_condensed(
    program: &ReceiptTokenProgram,
    instruction: &ParsedInstruction,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let mut fields = vec![
        create_text_field("Program", program.display_name)?,
        create_text_field("Instruction", &instruction.instruction_name)?,
    ];
    for (key, value) in &instruction.args {
        fields.push(create_text_field(key, &format_arg_value(value))?);
    }
    Ok(fields)
}

fn build_fallback_fields(
    program: &ReceiptTokenProgram,
    program_id: &str,
) -> Result<PreviewParts, VisualSignError> {
    let title = format!("{}: Unknown Instruction", program.display_name);

    let condensed_fields = vec![
        create_text_field("Program", program.display_name)?,
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

    fn accounts(asset: &str, receipt: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("mint".to_string(), asset.to_string()),
            ("f_token_mint".to_string(), receipt.to_string()),
        ])
    }

    fn position(asset: &str, receipt: &str) -> Option<Position> {
        Position::from_named_accounts(&accounts(asset, receipt), "mint", "f_token_mint")
    }

    #[test]
    fn test_unknown_mints_render_raw_units_without_guessing() {
        let position = position(
            "So11111111111111111111111111111111111111112",
            "So11111111111111111111111111111111111111113",
        )
        .unwrap();
        assert_eq!(position.symbol(Denomination::Asset), "So11...1112");
        assert_eq!(position.symbol(Denomination::Receipt), "So11...1113");
        assert_eq!(
            position.amount(414_122_446, Denomination::Asset).phrase(),
            "414122446 raw units of So11...1112"
        );
    }

    #[test]
    fn test_known_mints_normalize_decimals() {
        let position = position(
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D",
        )
        .unwrap();
        assert_eq!(position.symbol(Denomination::Asset), "USDC");
        assert_eq!(position.symbol(Denomination::Receipt), "jlUSDC");
        assert_eq!(
            position.amount(414_122_446, Denomination::Asset).phrase(),
            "414.122446 USDC"
        );
        assert_eq!(
            position.amount(100_000_000, Denomination::Receipt).phrase(),
            "100 jlUSDC"
        );
    }

    /// The receipt symbol is never derived from the asset symbol: the JupUSD
    /// market's receipt token is listed as JUICED, which no prefix rule yields.
    #[test]
    fn test_receipt_symbol_comes_from_the_receipt_mint() {
        let position = position(
            "JuprjznTrTSp2UFa3ZBUFgwdAmtZCq4MQCwysN55USD",
            "7GxATsNMnaC88vdwd2t3mwrFuQwwGvmYPrUQ4D6FotXk",
        )
        .unwrap();
        assert_eq!(position.symbol(Denomination::Asset), "JupUSD");
        assert_eq!(position.symbol(Denomination::Receipt), "JUICED");
    }

    #[test]
    fn test_unresolved_lookup_table_mint_is_not_a_position() {
        assert!(
            position(
                "unresolved(12)",
                "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D"
            )
            .is_none()
        );
        assert!(
            position(
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "unresolved(7)"
            )
            .is_none()
        );
    }

    #[test]
    fn test_u64_max_is_no_limit_only_for_caps() {
        let position = position(
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D",
        )
        .unwrap();
        assert_eq!(
            position.max_bound_phrase(u64::MAX, Denomination::Asset),
            "no limit"
        );
        assert_eq!(
            position.amount(u64::MAX, Denomination::Asset).phrase(),
            "18446744073709551615 raw units of USDC (u64::MAX)"
        );
    }

    /// Every flow denominates its exact amount and its counter leg in opposite tokens, so
    /// a bound can never be read in the amount's own denomination.
    #[test]
    fn test_counter_leg_is_the_other_denomination() {
        for flow in [
            Flow::Deposit,
            Flow::Mint,
            Flow::Withdraw,
            Flow::Redeem,
            Flow::RequestRedeem,
            Flow::Claim {
                exact: Denomination::Receipt,
            },
        ] {
            let leg = flow.counter_leg().unwrap();
            assert_ne!(leg.denomination, flow.exact(), "{flow:?}");
        }
        assert!(
            Flow::Claim {
                exact: Denomination::Asset
            }
            .counter_leg()
            .is_none()
        );
        assert!(Flow::CancelRequest.counter_leg().is_none());
    }

    const TEST_PROGRAM_ID: &str = "H9Cyw2RRHStwtKTAgGPm3nU48xwQ2Scy8u2gjg82dyQQ";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const JL_USDC: &str = "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D";
    const KEYED_BY_SIGNER: &[Seed] = &[Seed::Literal(b"req"), Seed::Account("user")];
    const KEYED_BY_PROGRAM: &[Seed] = &[Seed::Literal(b"global_queue")];

    fn test_program(actions: &'static [Action]) -> ReceiptTokenProgram {
        ReceiptTokenProgram {
            program_id: TEST_PROGRAM_ID,
            display_name: "Test Program",
            title_name: "Test",
            signer: "user",
            asset_mint: "mint",
            receipt_mint: "f_token_mint",
            token_program: "token_program",
            estimated_legs: false,
            actions,
        }
    }

    /// Both derivations can hold for a request keyed by something other than the signer; such
    /// an escrow is still not the signer's.
    #[test]
    fn test_escrow_not_keyed_by_the_signer_is_unverified() {
        let program = test_program(&[]);
        let program_id = Pubkey::from_str(TEST_PROGRAM_ID).unwrap();
        let user = Pubkey::new_unique();
        let token_program = spl_token_interface::id();
        let position = position(USDC, JL_USDC).unwrap();
        for (seeds, expected) in [
            (KEYED_BY_SIGNER, RecipientOwnership::Signer),
            (KEYED_BY_PROGRAM, RecipientOwnership::Unknown),
        ] {
            let seed_bytes: Vec<Vec<u8>> = seeds
                .iter()
                .map(|seed| match seed {
                    Seed::Literal(bytes) => bytes.to_vec(),
                    Seed::Account(_) => user.to_bytes().to_vec(),
                })
                .collect();
            let seed_refs: Vec<&[u8]> = seed_bytes.iter().map(Vec::as_slice).collect();
            let (request, _) = Pubkey::find_program_address(&seed_refs, &program_id);
            let escrow = get_associated_token_address_with_program_id(
                &request,
                &Pubkey::from_str(JL_USDC).unwrap(),
                &token_program,
            );
            let named_accounts = BTreeMap::from([
                ("user".to_string(), user.to_string()),
                ("request".to_string(), request.to_string()),
                ("escrow".to_string(), escrow.to_string()),
                ("token_program".to_string(), token_program.to_string()),
            ]);
            let action = Action::new(
                "request",
                Flow::RequestRedeem,
                "Request",
                "Request {receipt}",
                "amount",
                RecipientRule::SignerEscrow {
                    account: "escrow",
                    request: "request",
                    seeds,
                    holds: Denomination::Receipt,
                },
            );
            let recipient =
                Recipient::resolve(&program, &action, &named_accounts, &position).unwrap();
            assert_eq!(recipient.ownership, expected, "{} seeds", seeds.len());
        }
    }

    /// A destination in a lookup table is shown badged, never dropped, and blocks the summary.
    #[test]
    fn test_unresolved_account_row_is_badged_and_not_hoisted() {
        const ACTIONS: &[Action] = &[Action::new(
            "deposit",
            Flow::Deposit,
            "Deposit",
            "Supply {asset}",
            "amount",
            RecipientRule::SignerTokenAccount("recipient"),
        )
        .with_account_rows(&[("Paid to", "treasury")])];
        let program = test_program(ACTIONS);
        let user = Pubkey::new_unique();
        let token_program = spl_token_interface::id();
        let recipient = get_associated_token_address_with_program_id(
            &user,
            &Pubkey::from_str(JL_USDC).unwrap(),
            &token_program,
        );
        let mut args = serde_json::Map::new();
        args.insert("amount".to_string(), serde_json::json!(1_000_000));
        let instruction = ParsedInstruction {
            instruction_name: "deposit".to_string(),
            discriminator: String::new(),
            args,
            named_accounts: BTreeMap::from([
                ("user".to_string(), user.to_string()),
                ("mint".to_string(), USDC.to_string()),
                ("f_token_mint".to_string(), JL_USDC.to_string()),
                ("token_program".to_string(), token_program.to_string()),
                ("recipient".to_string(), recipient.to_string()),
                ("treasury".to_string(), "unresolved(9)".to_string()),
            ]),
            accounts_complete: true,
        };

        let action = UserAction::from_parsed(&program, &instruction).unwrap();

        assert_eq!(action.recipient.ownership, RecipientOwnership::Signer);
        assert!(!action.hoistable());
        let rows = action.row_fields().unwrap();
        let SignablePayloadField::AddressV2 { address_v2, .. } = &rows[0].signable_payload_field
        else {
            panic!("Paid to must be an address_v2");
        };
        assert_eq!(address_v2.address, "unresolved(9)");
        assert_eq!(address_v2.badge_text.as_deref(), Some("UNVERIFIED"));
    }
}
