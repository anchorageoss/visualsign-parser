//! Render decoded intents into `visualsign` fields. Only this module reads
//! defuse types and emits `visualsign` types.

use defuse_core::intents::token_diff::TokenDiff;
use defuse_core::intents::tokens::{FtWithdraw, MtWithdraw, NativeWithdraw, NftWithdraw, Transfer};
use defuse_core::intents::{DefuseIntents, Intent};
use defuse_core::payload::DefusePayload;
use defuse_core::payload::multi::MultiPayload;
use visualsign::SignablePayloadField;
use visualsign::encodings::split_hex_prefix;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_amount_field, create_text_field};
use visualsign::registry::LayeredRegistry;

use crate::fmt::charset_safe;
use crate::networks::NearNetwork;

use super::tokens;
use super::verify::SignatureCheck;
use super::{NEAR_DECIMALS, NEAR_SYMBOL, NearTokenRegistry};

type Reg = LayeredRegistry<NearTokenRegistry>;
type Fields = Vec<SignablePayloadField>;

/// Surfaces a soft finding: the intents still render, and the problem is
/// reported alongside them. With the `diagnostics` feature this is the
/// structured [`SignablePayloadField::Diagnostic`]; the default build carries
/// the same information as a `Warning`-labelled text field, keeping the
/// production payload shape unchanged.
///
/// `message` is charset-filtered here rather than at each call site. Every
/// rule on this path quotes untrusted input -- an asset id, a decode error
/// that echoes the offending value -- so filtering at this choke point is what
/// makes a newly added rule safe by construction. `rule` is a literal at every
/// call site and needs no filtering.
#[cfg(feature = "diagnostics")]
fn diagnostic(rule: &str, message: &str) -> Result<SignablePayloadField, VisualSignError> {
    Ok(visualsign::field_builders::create_diagnostic_field(
        rule,
        "near-intents",
        visualsign::lint::Severity::Warn,
        &charset_safe(message),
        None,
    )
    .signable_payload_field)
}

/// See the `diagnostics`-enabled twin above.
#[cfg(not(feature = "diagnostics"))]
fn diagnostic(rule: &str, message: &str) -> Result<SignablePayloadField, VisualSignError> {
    Ok(
        create_text_field("Warning", &format!("{rule}: {}", charset_safe(message)))?
            .signable_payload_field,
    )
}

/// Report each caller-supplied token-metadata entry the parser refused.
///
/// Without this the signer sees only the consequence -- an amount rendered in
/// raw base units against an `unresolved` asset id, or resolved from a seed
/// instead of the supplied override -- with no indication that metadata was
/// supplied and thrown away. A rejection is a soft finding: the intents still
/// render, since the refusal protects them rather than invalidating them.
///
/// `asset_id` is a caller-controlled map key and `reason` can quote it back (a
/// JSON parse error, a length); [`diagnostic`] charset-filters the assembled
/// message, so neither half can render as extra apparent fields on the signing
/// screen.
pub(crate) fn rejected_metadata_diagnostics(
    rejected: &[super::token_signature::RejectedTokenMetadata],
) -> Fields {
    // Infallible by construction: reporting a refusal must never be able to
    // withhold the transaction. A field that fails to build (an empty message
    // after charset filtering, say) is logged and dropped, leaving the signer
    // a payload minus one caveat rather than no payload at all.
    rejected
        .iter()
        .filter_map(|r| {
            let message = format!(
                "token metadata supplied for {} was rejected and not used: {}",
                r.asset_id, r.reason
            );
            match diagnostic("rejected-token-metadata", &message) {
                Ok(field) => Some(field),
                Err(e) => {
                    tracing::warn!(
                        "could not render the rejection diagnostic for '{}': {e}",
                        r.asset_id
                    );
                    None
                }
            }
        })
        .collect()
}

/// Render one token amount, resolving symbol/decimals when the asset is known;
/// otherwise show the raw base-unit amount tagged with the unresolved asset id.
/// Metadata resolved from an unattributed request entry (a gap-fill for an asset
/// `SEEDS` doesn't cover, either unsigned or signed by a key this deployment has
/// not enrolled) carries an extra diagnostic alongside the amount naming which,
/// so the signer sees the caveat rather than just an operator log.
///
/// A `TokenId` is only half account-typed: its `FromStr` parses the contract
/// half as an `AccountId` and takes the remainder verbatim into a plain
/// `String` (`Nep245TokenId::mt_token_id`, `Nep171TokenId::nft_token_id`), and
/// `Display` round-trips it. So an asset id echoed into field text carries
/// whatever bytes the sender chose and must be filtered.
///
/// The filtered form is for display only. Resolution runs against the raw id,
/// because stripping first would let a crafted id collapse onto a seeded one
/// (`nep141:wrap\u{7f}.near` -> `nep141:wrap.near`) and borrow that token's
/// symbol and decimals.
fn token_amount_field(
    label: &str,
    asset_id: &str,
    raw: u128,
    registry: &Reg,
) -> Result<Fields, VisualSignError> {
    let resolved = tokens::resolve(asset_id, registry);
    let asset_id = charset_safe(asset_id);
    match resolved {
        Some(meta) => {
            let mut fields = vec![
                create_amount_field(
                    label,
                    &tokens::format_units(raw, meta.decimals),
                    &meta.symbol,
                )?
                .signable_payload_field,
            ];
            if let Some(cause) = meta.provenance.unverified_cause() {
                fields.push(diagnostic(
                    "unverified-token-metadata",
                    &format!("symbol/decimals for {asset_id}: {cause}"),
                )?);
            }
            Ok(fields)
        }
        None => Ok(vec![
            create_text_field(label, &format!("{raw} (unresolved {asset_id})"))?
                .signable_payload_field,
        ]),
    }
}

/// Stands in for a present value that renders as nothing, where the value
/// being present is itself what the signer needs to see.
///
/// Ends in a literal backslash so no attacker-controlled string can ever
/// equal it: [`charset_safe`] elides every backslash a caller's text
/// contains, so this marker can be produced only here, never by a real `msg`
/// that happens to spell out the same words.
const EMPTY_VALUE: &str = "(empty)\\";

/// Charset-filter an optional field string, dropping it when the value is
/// empty.
///
/// Only for fields whose presence carries no meaning of its own, so that a
/// value rendering as nothing and a value never supplied are genuinely the
/// same thing to the signer. A `memo` is such a field: it annotates the
/// transfer and changes nothing the transfer does. Where presence does change
/// what executes, use [`present_value`] instead -- dropping the field there
/// hides the difference between the two on-chain behaviours.
///
/// `charset_safe` marks what it cannot render rather than deleting it, so an
/// all-non-ASCII memo renders as markers and reaches the signer; only a
/// genuinely empty string is dropped, which would otherwise render as a
/// labelled blank line reading as a deliberately empty memo.
fn nonempty_filtered(text: Option<&str>) -> Option<String> {
    text.map(charset_safe).filter(|t| !t.is_empty())
}

/// Charset-filter a value whose presence changes what the transaction does,
/// substituting [`EMPTY_VALUE`] when nothing renders.
///
/// An empty `msg` still selects a contract-calling form on-chain: a
/// `NotifyOnTransfer` invokes `mt_on_transfer` on the receiver whatever its
/// `msg` holds, and a withdraw's `Some("")` takes the `_transfer_call` branch
/// rather than the plain one. Dropping the field for want of text would render
/// those byte-identically to the transfer that calls nothing, so the signer
/// would approve a receiver callback with nothing on screen distinguishing it.
fn present_value(text: &str) -> String {
    let filtered = charset_safe(text);
    if filtered.is_empty() {
        EMPTY_VALUE.to_string()
    } else {
        filtered
    }
}

/// Format a yoctoNEAR balance as a `<amount> NEAR` text field.
fn near_amount_field(label: &str, yocto: u128) -> Result<SignablePayloadField, VisualSignError> {
    Ok(create_amount_field(
        label,
        &tokens::format_units(yocto, NEAR_DECIMALS),
        NEAR_SYMBOL,
    )?
    .signable_payload_field)
}

/// Wire-standard name for a signed payload variant.
pub(crate) fn standard_name(mp: &MultiPayload) -> &'static str {
    match mp {
        MultiPayload::Nep413(_) => "nep413",
        MultiPayload::Erc191(_) => "erc191",
        MultiPayload::Tip191(_) => "tip191",
        MultiPayload::RawEd25519(_) => "raw_ed25519",
        MultiPayload::WebAuthn(_) => "webauthn",
        MultiPayload::TonConnect(_) => "ton_connect",
        MultiPayload::Sep53(_) => "sep53",
    }
}

/// Render one signed payload as a section: header + signature + envelope +
/// per-intent fields. Verification and structural decode are independent, so a
/// bad signature still renders the decoded intents (flagged at the signature
/// line).
pub(crate) fn section(
    index: usize,
    total: usize,
    mp: &MultiPayload,
    registry: &Reg,
    network: NearNetwork,
) -> Result<Fields, VisualSignError> {
    let (check, extracted) = super::verify::verify_and_extract(mp);
    let signer_id = extracted.as_ref().ok().map(|p| p.signer_id.as_str());
    let mut fields = vec![
        create_text_field("Signed Intent", &format!("{index} of {total}"))?.signable_payload_field,
    ];
    fields.extend(render_signature(standard_name(mp), &check, signer_id)?);
    match &extracted {
        Ok(payload) => fields.extend(render_single(payload, registry, network)?),
        Err(e) => fields.push(diagnostic(
            "extraction",
            &format!("could not extract the envelope/intents: {e}"),
        )?),
    }
    Ok(fields)
}

/// Human-readable name for an intent variant, mirroring `actions::action_label`
/// on the transaction path.
pub(crate) fn intent_label(intent: &Intent) -> &'static str {
    match intent {
        Intent::TokenDiff(_) => "Token Diff",
        Intent::Transfer(_) => "Transfer",
        Intent::FtWithdraw(_) => "FT Withdraw",
        Intent::NftWithdraw(_) => "NFT Withdraw",
        Intent::MtWithdraw(_) => "MT Withdraw",
        Intent::NativeWithdraw(_) => "Native Withdraw",
        Intent::AddPublicKey(_) => "Add Public Key",
        Intent::RemovePublicKey(_) => "Remove Public Key",
        Intent::SetAuthByPredecessorId(_) => "Set Auth By Predecessor Id",
        Intent::StorageDeposit(_) => "Storage Deposit",
        Intent::AuthCall(_) => "Auth Call",
    }
}

/// Title for a pre-signature envelope, mirroring `convert::title_for` on the
/// transaction path: a lone intent names its type, a batch stays generic
/// because no single name describes it.
pub(crate) fn title_for_intents(intents: &[Intent]) -> String {
    match intents {
        [single] => format!("NEAR Intent: {}", intent_label(single)),
        _ => "NEAR Intent".to_string(),
    }
}

/// The `"Intent"` field naming what the following fields belong to.
///
/// Unlike `actions::action_boundary_field`, which a single-action transaction
/// omits because the title already names it, this renders for a lone intent
/// too: a signed batch nests intents inside per-payload sections, so a section
/// carrying one intent has nothing else to name its type. The index is added
/// only when there is more than one, where it is what separates them.
///
/// The type is otherwise absent from the render entirely -- serde consumes the
/// `intent` tag to select the variant, so it is known and dropped. Without it
/// `transfer` and `ft_withdraw` differ only by the presence of one `Token`
/// field.
fn intent_boundary_field(
    intent: &Intent,
    index: usize,
    total: usize,
) -> Result<SignablePayloadField, VisualSignError> {
    let label = intent_label(intent);
    let text = if total > 1 {
        format!("{} of {total}: {label}", index + 1)
    } else {
        label.to_string()
    };
    Ok(create_text_field("Intent", &text)?.signable_payload_field)
}

/// Warns that an intent hands over authority rather than moving a named
/// amount. These render as one unremarkable line each, and appended to a
/// legitimate swap they read as part of it -- but an added key holds
/// permanent authority over the account's entire intents balance, and
/// `auth_call` invokes a contract with the signer's own authority. The
/// transaction path gives the equivalent `AddKey` action the same treatment,
/// breaking its permission out field by field because "their absent forms
/// widen the grant".
fn account_control_warning(
    intent: &Intent,
) -> Result<Option<SignablePayloadField>, VisualSignError> {
    let consequence = match intent {
        Intent::AddPublicKey(_) => {
            "this key gains permanent authority over the account's entire intents balance, until it is explicitly removed"
        }
        Intent::RemovePublicKey(_) => {
            "removing a key revokes its authority over the account; removing the only remaining key can lock the account out"
        }
        Intent::SetAuthByPredecessorId(_) => {
            "this changes which callers the account authorizes, independently of its keys"
        }
        Intent::AuthCall(_) => {
            "this calls the named contract with the signer's own authority, and the attached deposit is not refunded on failure"
        }
        _ => return Ok(None),
    };
    Ok(Some(diagnostic("account-control", consequence)?))
}

/// Render the fields intrinsic to a single intent.
pub(crate) fn render_intent(intent: &Intent, registry: &Reg) -> Result<Fields, VisualSignError> {
    let mut fields = render_intent_body(intent, registry)?;
    if let Some(warning) = account_control_warning(intent)? {
        fields.push(warning);
    }
    Ok(fields)
}

/// The intent's own fields, without the boundary or any warning.
fn render_intent_body(intent: &Intent, registry: &Reg) -> Result<Fields, VisualSignError> {
    match intent {
        Intent::TokenDiff(td) => render_token_diff(td, registry),
        Intent::Transfer(t) => render_transfer(t, registry),
        Intent::FtWithdraw(w) => render_ft_withdraw(w, registry),
        Intent::NftWithdraw(w) => render_nft_withdraw(w),
        Intent::MtWithdraw(w) => render_mt_withdraw(w),
        Intent::NativeWithdraw(w) => render_native_withdraw(w),
        Intent::AddPublicKey(a) => Ok(vec![
            create_text_field("Add Public Key", &a.public_key.to_string())?.signable_payload_field,
        ]),
        Intent::RemovePublicKey(a) => Ok(vec![
            create_text_field("Remove Public Key", &a.public_key.to_string())?
                .signable_payload_field,
        ]),
        Intent::SetAuthByPredecessorId(s) => Ok(vec![
            create_text_field(
                "Auth By Predecessor",
                if s.enabled { "enabled" } else { "disabled" },
            )?
            .signable_payload_field,
        ]),
        Intent::StorageDeposit(s) => Ok(vec![
            create_text_field("Contract", s.contract_id.as_str())?.signable_payload_field,
            create_text_field("For Account", s.deposit_for_account_id.as_str())?
                .signable_payload_field,
            near_amount_field("Amount", s.amount.as_yoctonear())?,
        ]),
        Intent::AuthCall(c) => {
            let mut fields = vec![
                create_text_field("Contract", c.contract_id.as_str())?.signable_payload_field,
                create_text_field("Message", &charset_safe(&c.msg))?.signable_payload_field,
                near_amount_field("Attached Deposit", c.attached_deposit.as_yoctonear())?,
            ];
            if c.state_init.is_some() {
                fields.push(state_init_field()?);
            }
            Ok(fields)
        }
    }
}

/// Whether [`render_intent`] passes this intent's kind through to a token
/// registry lookup. Matched exhaustively against the same arms so a kind that
/// starts (or stops) reading `registry` there forces this to be revisited
/// rather than silently drifting out of step.
pub(crate) fn intent_consumes_token_registry(intent: &Intent) -> bool {
    match intent {
        Intent::TokenDiff(_) | Intent::Transfer(_) | Intent::FtWithdraw(_) => true,
        Intent::NftWithdraw(_)
        | Intent::MtWithdraw(_)
        | Intent::NativeWithdraw(_)
        | Intent::AddPublicKey(_)
        | Intent::RemovePublicKey(_)
        | Intent::SetAuthByPredecessorId(_)
        | Intent::StorageDeposit(_)
        | Intent::AuthCall(_) => false,
    }
}

/// Flags an attached NEP-616 `state_init`: a global-contract id plus initial
/// state, which initializes the callee's contract in the same receipt. The
/// code/data have no cheap field-level render, so surface that the attachment
/// exists rather than let the other fields imply the whole picture -- the same
/// treatment `actions.rs` gives NEAR's own `DeterministicStateInit` action.
fn state_init_field() -> Result<SignablePayloadField, VisualSignError> {
    Ok(create_text_field("State Init", "(not fully decoded)")?.signable_payload_field)
}

/// Renders a token diff as one signed `Send`/`Receive` line per entry.
///
/// An empty diff, or an entry whose delta is zero, is refused rather than
/// rendered: the contract refuses both itself (`DefuseError::InvalidIntent`,
/// `defuse_core::intents::token_diff`), so such an intent cannot execute, and
/// a zero delta would otherwise render as `Receive 0 <token>` -- a line
/// claiming a movement that does not happen.
fn render_token_diff(td: &TokenDiff, registry: &Reg) -> Result<Fields, VisualSignError> {
    if td.diff.is_empty() {
        return Err(VisualSignError::ValidationError(
            "token_diff carries no entries".to_string(),
        ));
    }
    let mut fields = Fields::new();
    for (token_id, delta) in td.diff.iter() {
        if *delta == 0 {
            return Err(VisualSignError::ValidationError(format!(
                "token_diff entry for {} has a zero delta",
                charset_safe(&token_id.to_string())
            )));
        }
        let label = if *delta < 0 { "Send" } else { "Receive" };
        fields.extend(token_amount_field(
            label,
            &token_id.to_string(),
            (*delta).unsigned_abs(),
            registry,
        )?);
    }
    if let Some(memo) = nonempty_filtered(td.memo.as_deref()) {
        fields.push(create_text_field("Memo", &memo)?.signable_payload_field);
    }
    // `referral` is an `AccountId`: its own charset rules already exclude
    // everything `charset_safe` would strip, so filtering it would be dead code.
    if let Some(referral) = &td.referral {
        fields.push(create_text_field("Referral", referral.as_str())?.signable_payload_field);
    }
    Ok(fields)
}

fn render_transfer(t: &Transfer, registry: &Reg) -> Result<Fields, VisualSignError> {
    let mut fields = vec![create_text_field("To", t.receiver_id.as_str())?.signable_payload_field];
    for (token_id, amount) in t.tokens.iter() {
        fields.extend(token_amount_field(
            "Amount",
            &token_id.to_string(),
            *amount,
            registry,
        )?);
    }
    if let Some(memo) = nonempty_filtered(t.memo.as_deref()) {
        fields.push(create_text_field("Memo", &memo)?.signable_payload_field);
    }
    // A `Transfer`'s optional notification changes what the transfer does
    // beyond moving the named tokens: `msg` calls `mt_on_transfer` on
    // `receiver_id` (the internal-transfer counterpart of the withdraws'
    // `_transfer_call` form), and `state_init` initializes the receiver's
    // contract in the same receipt.
    if let Some(notification) = &t.notification {
        // Rendered on presence, not on content: `msg` is a required `String`
        // here, and `notify_on_transfer` builds the `mt_on_transfer` promise
        // whenever the notification exists, whatever it holds.
        fields.push(
            create_text_field("Message", &present_value(notification.msg.as_str()))?
                .signable_payload_field,
        );
        if notification.state_init.is_some() {
            fields.push(state_init_field()?);
        }
    }
    Ok(fields)
}

/// A withdraw's trailing fields, shared by all three token standards so none
/// can silently drop one: the optional `memo`, the optional `msg` (switches
/// the call into its `_transfer_call` form, passing this to the receiver), and
/// `storage_deposit` (a separate, unconditional wNEAR debit for the receiver's
/// storage on `token`, never refunded on failure). Each changes what the
/// withdraw does beyond moving the named token/amount, so each must render.
fn push_withdraw_call_details(
    fields: &mut Fields,
    memo: &Option<String>,
    msg: &Option<String>,
    storage_deposit: Option<near_sdk::NearToken>,
) -> Result<(), VisualSignError> {
    if let Some(memo) = nonempty_filtered(memo.as_deref()) {
        fields.push(create_text_field("Memo", &memo)?.signable_payload_field);
    }
    // `Some("")` is not the same withdraw as `None`: it selects the
    // `_transfer_call` form, which invokes a callback on the receiver. The
    // field renders on presence so the two cannot look alike.
    if let Some(msg) = msg.as_deref() {
        fields.push(create_text_field("Message", &present_value(msg))?.signable_payload_field);
    }
    if let Some(deposit) = storage_deposit {
        fields.push(near_amount_field(
            "Storage Deposit",
            deposit.as_yoctonear(),
        )?);
    }
    Ok(())
}

fn render_ft_withdraw(w: &FtWithdraw, registry: &Reg) -> Result<Fields, VisualSignError> {
    let mut fields = vec![
        create_text_field("Token", w.token.as_str())?.signable_payload_field,
        create_text_field("To", w.receiver_id.as_str())?.signable_payload_field,
    ];
    fields.extend(token_amount_field(
        "Amount",
        &format!("nep141:{}", w.token),
        w.amount.0,
        registry,
    )?);
    push_withdraw_call_details(&mut fields, &w.memo, &w.msg, w.storage_deposit)?;
    Ok(fields)
}

fn render_nft_withdraw(w: &NftWithdraw) -> Result<Fields, VisualSignError> {
    let mut fields = vec![
        create_text_field("Token", w.token.as_str())?.signable_payload_field,
        create_text_field("To", w.receiver_id.as_str())?.signable_payload_field,
        // `token_id` is a plain `String` (`non_fungible_token::TokenId`), not
        // an `AccountId`, so it carries whatever bytes the sender chose.
        create_text_field("NFT Token Id", &charset_safe(w.token_id.as_str()))?
            .signable_payload_field,
    ];
    push_withdraw_call_details(&mut fields, &w.memo, &w.msg, w.storage_deposit)?;
    Ok(fields)
}

fn render_mt_withdraw(w: &MtWithdraw) -> Result<Fields, VisualSignError> {
    if w.token_ids.len() != w.amounts.len() {
        return Err(VisualSignError::ValidationError(format!(
            "mt_withdraw token_ids/amounts length mismatch: {} token_ids vs {} amounts",
            w.token_ids.len(),
            w.amounts.len()
        )));
    }
    let mut fields = vec![
        create_text_field("Token", w.token.as_str())?.signable_payload_field,
        create_text_field("To", w.receiver_id.as_str())?.signable_payload_field,
    ];
    for (id, amount) in w.token_ids.iter().zip(w.amounts.iter()) {
        // As with `nft_withdraw`'s `token_id`, an MT token id is a plain
        // `String` (`defuse_nep245::TokenId`); filter it before it joins the
        // composite, so the amount half cannot be pushed onto its own line.
        fields.push(
            create_text_field("MT Token", &format!("{} x{}", charset_safe(id), amount.0))?
                .signable_payload_field,
        );
    }
    push_withdraw_call_details(&mut fields, &w.memo, &w.msg, w.storage_deposit)?;
    Ok(fields)
}

fn render_native_withdraw(w: &NativeWithdraw) -> Result<Fields, VisualSignError> {
    Ok(vec![
        create_text_field("To", w.receiver_id.as_str())?.signable_payload_field,
        near_amount_field("Amount", w.amount.as_yoctonear())?,
    ])
}

/// Whether `signer_id` has the shape of a key-derived implicit account --
/// NEAR's 64-hex-char convention, or defuse's own `0x`/`0X` + 40-hex
/// convention for EVM-style signers -- rather than a human-chosen named
/// account (e.g.
/// `alice.near`). Only for these shapes is key-to-account binding checkable
/// offline: a named account's access keys are registered on-chain, decoupled
/// from any implicit derivation, so comparing against one would produce
/// false-positive mismatches on entirely legitimate transactions.
fn looks_like_implicit_account(signer_id: &str) -> bool {
    let is_hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    (signer_id.len() == 64 && is_hex(signer_id))
        || split_hex_prefix(signer_id).is_some_and(|rest| rest.len() == 40 && is_hex(rest))
}

/// Render the standard + signature-status fields for one signed payload.
///
/// `signer_id` is the payload's claimed signer, when extraction succeeded.
/// When it has an implicit-account shape, the recovered key's own implied
/// account id is compared against it and the outcome stated literally: the key
/// either derives `signer_id` or it doesn't. Neither outcome settles
/// authorization, because the contract accepts a key that either derives the
/// account id *or* sits in the account's on-chain key set
/// (`Account::has_public_key`, `defuse/v0.4.2`): a derivation match can still
/// be rejected on-chain (an account may remove its implicit key), and a
/// non-match is expected for any account that added keys via `AddPublicKey`.
/// For any other shape (a named account) there is nothing to compare against
/// at all, so no claim is made.
pub(crate) fn render_signature(
    standard: &str,
    check: &SignatureCheck,
    signer_id: Option<&str>,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    let mut fields = vec![create_text_field("Standard", standard)?.signable_payload_field];
    match check {
        SignatureCheck::Valid {
            recovered_key,
            implied_account_id,
        } => match signer_id.filter(|id| looks_like_implicit_account(id)) {
            // Hex digit case is not part of the identity: both shapes this arm
            // accepts are hex, and `to_implicit_account_id` emits lowercase, so
            // a differently-cased spelling of the same account is a match, not
            // a failed derivation.
            Some(id) if id.eq_ignore_ascii_case(implied_account_id) => {
                fields.push(
                    create_text_field(
                        "Signature",
                        &format!("valid for key {recovered_key} (derives signer_id {id})"),
                    )?
                    .signable_payload_field,
                );
            }
            Some(id) => {
                fields.push(
                    create_text_field("Signature", &format!("valid for key {recovered_key}"))?
                        .signable_payload_field,
                );
                fields.push(diagnostic(
                        "account-binding",
                        &format!(
                            "the signing key does not derive signer_id {id} (it derives {implied_account_id}); the account may still have authorized this key on-chain, which this parser cannot check"
                        ),
                    )?);
            }
            None => {
                fields.push(
                    create_text_field(
                        "Signature",
                        &format!(
                            "valid for key {recovered_key} (key-to-account binding not verified)"
                        ),
                    )?
                    .signable_payload_field,
                );
            }
        },
        SignatureCheck::Invalid => {
            fields.push(diagnostic("signature", "signature verification failed")?);
        }
        SignatureCheck::MalformedEncoding(reason) => {
            fields.push(diagnostic("signature", reason)?);
        }
    }
    Ok(fields)
}

/// Render the envelope fields shared by every signed payload: who signed, the
/// contract being authorized, the deadline, and the nonce.
pub(crate) fn render_envelope(
    payload: &DefusePayload<DefuseIntents>,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    Ok(vec![
        create_text_field("Signer", payload.signer_id.as_str())?.signable_payload_field,
        create_text_field("Verifying Contract", payload.verifying_contract.as_str())?
            .signable_payload_field,
        create_text_field("Deadline", &payload.deadline.into_timestamp().to_rfc3339())?
            .signable_payload_field,
        create_text_field("Nonce", &format!("0x{}", hex::encode(payload.nonce)))?
            .signable_payload_field,
    ])
}

/// Render the envelope + per-intent fields for a single payload the user is
/// about to sign -- the pre-signature signing view. No signature exists at
/// signing time, so verification is skipped (unlike [`section`], which renders
/// a signed `MultiPayload`).
pub(crate) fn render_single(
    payload: &DefusePayload<DefuseIntents>,
    registry: &Reg,
    network: NearNetwork,
) -> Result<Fields, VisualSignError> {
    // Every envelope reaches the signer through here -- the standalone
    // pre-signature view and each section of an on-chain signed batch alike --
    // so the network agreement check lives here rather than at either entry
    // point. `network` is caller-influenceable (a request's `network_id`
    // overrides the converter's default) and it selects the token-metadata
    // signature scope, so an envelope naming mainnet accounts must not render
    // as testnet on either path.
    //
    // `ValidationError` is the signal both callers translate back into their own
    // network-mismatch error; no field builder produces that variant, so it
    // cannot be confused with a rendering failure.
    for (role, account_id) in [
        ("signer", payload.signer_id.as_str()),
        ("verifying contract", payload.verifying_contract.as_str()),
    ] {
        if let Some(mismatch) = crate::networks::network_mismatch(role, account_id, network) {
            return Err(VisualSignError::ValidationError(mismatch));
        }
    }
    let mut fields = render_envelope(payload)?;
    if payload.deadline.has_expired() {
        fields.push(diagnostic(
            "deadline",
            "deadline has passed; the intents would be rejected",
        )?);
    }
    // An empty list is valid and does nothing, but still consumes the nonce
    // for this signer -- so it renders as an envelope with no body, and a
    // signer has nothing on screen telling them that is all it does.
    if payload.intents.is_empty() {
        fields.push(diagnostic(
            "empty-intents",
            "this envelope carries no intents; signing it moves nothing but spends the nonce",
        )?);
    }
    let total = payload.intents.len();
    for (index, intent) in payload.intents.iter().enumerate() {
        fields.push(intent_boundary_field(intent, index, total)?);
        fields.extend(render_intent(intent, registry)?);
        if let Some(warning) = self_transfer_warning(payload, intent)? {
            fields.push(warning);
        }
    }
    Ok(fields)
}

/// Warns that a transfer to the signer's own account cannot execute:
/// `Transfer::execute_intent` returns `InvalidIntent` on
/// `sender_id == receiver_id`. Renders clean otherwise, the same way an
/// expired deadline would without the check beside this one.
fn self_transfer_warning(
    payload: &DefusePayload<DefuseIntents>,
    intent: &Intent,
) -> Result<Option<SignablePayloadField>, VisualSignError> {
    let Intent::Transfer(t) = intent else {
        return Ok(None);
    };
    if t.receiver_id != payload.signer_id {
        return Ok(None);
    }
    Ok(Some(diagnostic(
        "self-transfer",
        "the recipient is the signer's own account; the contract rejects a transfer to self",
    )?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    use std::sync::Arc;

    fn label_of(f: &SignablePayloadField) -> Option<&str> {
        match f {
            SignablePayloadField::TextV2 { common, .. }
            | SignablePayloadField::AmountV2 { common, .. }
            | SignablePayloadField::AddressV2 { common, .. } => Some(common.label.as_str()),
            _ => None,
        }
    }

    /// Field labels excluding soft findings. The non-`diagnostics` build
    /// surfaces those as a `Warning`-labelled text field, so a test about
    /// which value fields render must not count them.
    fn value_labels(fields: &Fields) -> Vec<&str> {
        fields
            .iter()
            .filter_map(label_of)
            .filter(|l| *l != "Warning")
            .collect()
    }

    fn empty_reg() -> Reg {
        LayeredRegistry::new(Arc::new(NearTokenRegistry::default()))
    }

    /// A registry whose request-scoped layer has one entry for `asset_id`,
    /// carrying the given provenance.
    fn reg_with_entry(asset_id: &str, provenance: super::super::TokenProvenance) -> Reg {
        let mut request = NearTokenRegistry::default();
        request.by_asset_id.insert(
            asset_id.to_string(),
            super::super::TokenMeta {
                symbol: "TEST".to_string(),
                decimals: 6,
                provenance,
            },
        );
        LayeredRegistry::with_request(Arc::new(NearTokenRegistry::default()), request)
    }

    fn intent_from(json: &str) -> Intent {
        serde_json::from_str(json).expect("intent json")
    }

    #[test]
    fn looks_like_implicit_account_accepts_both_recognized_shapes() {
        assert!(looks_like_implicit_account(
            "74affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d53751"
        ));
        assert!(looks_like_implicit_account(
            "0x17c5185167401ed00cf5f5b2fc97d9bbfdb7d025"
        ));
    }

    #[test]
    fn looks_like_implicit_account_accepts_an_uppercase_hex_prefix() {
        assert!(looks_like_implicit_account(
            "0X17c5185167401ed00cf5f5b2fc97d9bbfdb7d025"
        ));
    }

    #[test]
    fn a_differently_cased_signer_id_still_counts_as_derived() {
        // `to_implicit_account_id` emits lowercase; an equal account id spelled
        // with any other hex case must not read as a failed derivation.
        let fields = render_signature(
            "erc191",
            &valid_check("0x17c5185167401ed00cf5f5b2fc97d9bbfdb7d025"),
            Some("0X17C5185167401ED00CF5F5B2FC97D9BBFDB7D025"),
        )
        .expect("render");
        let text = fields
            .iter()
            .filter_map(|f| match f {
                SignablePayloadField::TextV2 { text_v2, common } if common.label == "Signature" => {
                    Some(text_v2.text.as_str())
                }
                _ => None,
            })
            .next()
            .expect("signature field");
        assert!(text.contains("derives signer_id"), "{text}");
        // Holds whether or not the `diagnostics` feature is on: the finding
        // carries this wording as a structured message or as `Warning` text.
        let rendered = format!("{fields:?}");
        assert!(
            !rendered.contains("does not derive"),
            "a case difference must not raise an account-binding finding: {rendered}"
        );
    }

    #[test]
    fn looks_like_implicit_account_rejects_named_and_malformed_ids() {
        assert!(!looks_like_implicit_account("alice.near"));
        assert!(!looks_like_implicit_account(""));
        // 63 hex chars: one short of the implicit-account length.
        assert!(!looks_like_implicit_account(
            "4affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d53751"
        ));
        // 64 chars but not all hex.
        assert!(!looks_like_implicit_account(
            "74affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d5375g"
        ));
        // 0x + 39 hex chars: one short of the EVM-address length.
        assert!(!looks_like_implicit_account(
            "0x7c5185167401ed00cf5f5b2fc97d9bbfdb7d025"
        ));
        // 0x + 40 chars but not all hex.
        assert!(!looks_like_implicit_account(
            "0x17c5185167401ed00cf5f5b2fc97d9bbfdb7d0zz"
        ));
        // No 0x prefix at all, despite being 40 hex chars.
        assert!(!looks_like_implicit_account(
            "17c5185167401ed00cf5f5b2fc97d9bbfdb7d025"
        ));
    }

    fn valid_check(implied_account_id: &str) -> SignatureCheck {
        SignatureCheck::Valid {
            recovered_key: "ed25519:abc".to_string(),
            implied_account_id: implied_account_id.to_string(),
        }
    }

    #[test]
    fn signature_fields_present_for_valid() {
        // signer_id is a named account: binding genuinely isn't checkable,
        // so this stays the two-field (Standard, Signature) hedge shape.
        let fields = render_signature("nep413", &valid_check("64hex..."), Some("alice.near"))
            .expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Standard", "Signature"]);
    }

    #[test]
    fn invalid_signature_renders_warning() {
        let fields = render_signature("erc191", &SignatureCheck::Invalid, None).expect("render");
        assert!(
            super::super::test_support::is_warning_diagnostic(&fields[1], "signature"),
            "expected a signature warning, got {:?}",
            fields[1]
        );
    }

    /// A malformed recovery-id encoding has to reach the screen as its own
    /// finding. Without this, the arm could be dropped or mislabelled and a
    /// cryptographically-sound wallet signature would render no signature
    /// field at all -- `verify`'s reason string alone proves nothing about
    /// what a signer sees.
    #[test]
    fn malformed_encoding_renders_its_reason_as_a_warning() {
        let check = SignatureCheck::MalformedEncoding(
            "malformed signature encoding: recovery id 28, expected 0-3 (Ethereum v=27/28 must be normalized)"
                .to_string(),
        );
        let fields = render_signature("erc191", &check, None).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(
            !labels.contains(&"Signature"),
            "a malformed encoding must not claim a Signature field: {labels:?}"
        );
        assert!(
            super::super::test_support::is_warning_diagnostic(&fields[1], "signature"),
            "expected a signature warning, got {:?}",
            fields[1]
        );
    }

    #[test]
    fn signature_binding_confirmed_when_implicit_signer_matches() {
        let id = "74affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d53751";
        let fields = render_signature("raw_ed25519", &valid_check(id), Some(id)).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Standard", "Signature"]);
        match &fields[1] {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("derives signer_id"),
                    "{}",
                    text_v2.text
                );
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn signature_binding_confirmed_when_evm_style_signer_matches() {
        // Real signer_id from the pinned _vector_erc191.input fixture.
        let id = "0x17c5185167401ed00cf5f5b2fc97d9bbfdb7d025";
        let fields = render_signature("erc191", &valid_check(id), Some(id)).expect("render");
        match &fields[1] {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(
                    text_v2.text.contains("derives signer_id"),
                    "{}",
                    text_v2.text
                );
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn signature_binding_mismatch_is_a_diagnostic_not_a_hedge() {
        // The key recovers to the pinned fixture's real address, but the
        // payload claims an unrelated one -- e.g. attacker.near substituting
        // their own signer_id onto someone else's signature.
        let fields = render_signature(
            "erc191",
            &valid_check("0x17c5185167401ed00cf5f5b2fc97d9bbfdb7d025"),
            Some("0x00000000000000000000000000000000000000ff"),
        )
        .expect("render");
        assert!(
            super::super::test_support::is_warning_diagnostic(&fields[2], "account-binding"),
            "expected an account-binding warning, got {fields:?}"
        );
    }

    #[test]
    fn signature_binding_mismatch_detected_for_implicit_ed25519_signer() {
        let fields = render_signature(
            "raw_ed25519",
            &valid_check("74affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d53751"),
            Some("000000000000000000000000000000000000000000000000000000000000dead"),
        )
        .expect("render");
        assert!(
            super::super::test_support::is_warning_diagnostic(&fields[2], "account-binding"),
            "expected an account-binding warning, got {fields:?}"
        );
    }

    #[test]
    fn signature_binding_not_checked_for_named_account_even_with_valid_key() {
        // Named accounts' keys are registered on-chain; an implicit-derived
        // id has nothing to compare against, so this must stay a hedge, not
        // a false mismatch.
        let fields = render_signature(
            "nep413",
            &valid_check("74affa71ab030d400fdfa1bed033dfa6fd3ae34f92d17c046ebe368e80d53751"),
            Some("alice.near"),
        )
        .expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Standard", "Signature"]);
        match &fields[1] {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert!(text_v2.text.contains("not verified"), "{}", text_v2.text);
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    /// When the inner `payload` string doesn't parse as a `DefusePayload`,
    /// `section()` must surface why nothing rendered rather than silently
    /// omitting the envelope/intents.
    #[test]
    fn section_surfaces_extraction_failure_instead_of_silently_omitting_envelope() {
        let mp: MultiPayload = serde_json::from_str(
            r#"{"standard":"raw_ed25519","payload":"not a valid defuse payload","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN","signature":"ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"}"#,
        )
        .expect("multi payload json");

        let fields = section(1, 1, &mp, &empty_reg(), NearNetwork::Mainnet).expect("render");

        let has_extraction_warning = fields
            .iter()
            .any(|f| super::super::test_support::is_warning_diagnostic(f, "extraction"));
        assert!(
            has_extraction_warning,
            "expected an extraction warning, got {fields:?}"
        );
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(
            !labels.contains(&"Signer"),
            "envelope must not render when extraction failed: {labels:?}"
        );
    }

    fn sample_payload() -> DefusePayload<DefuseIntents> {
        DefusePayload {
            signer_id: "alice.near".parse().expect("account"),
            verifying_contract: "intents.near".parse().expect("account"),
            deadline: defuse_deadline::Deadline::new(
                chrono::DateTime::from_timestamp(1_900_000_000, 0).expect("timestamp"),
            ),
            nonce: [0u8; 32],
            message: DefuseIntents { intents: vec![] },
        }
    }

    #[test]
    fn envelope_emits_signer_verifying_deadline_nonce() {
        let fields = render_envelope(&sample_payload()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(
            labels,
            ["Signer", "Verifying Contract", "Deadline", "Nonce"]
        );
        match &fields[0] {
            SignablePayloadField::TextV2 { text_v2, .. } => assert_eq!(text_v2.text, "alice.near"),
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn token_diff_renders_send_and_receive_resolving_known_token() {
        let intent = intent_from(
            r#"{"intent":"token_diff","diff":{"nep141:wrap.near":"-1000000000000000000000000","nep141:usdc.near":"5000000"}}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Send"));
        assert!(labels.contains(&"Receive"));
        // wrap.near is seeded -> rendered as an AmountV2 of "1" wNEAR.
        let wnear = fields.iter().find_map(|f| match f {
            SignablePayloadField::AmountV2 { amount_v2, .. } => Some(amount_v2),
            _ => None,
        });
        let wnear = wnear.expect("a resolved AmountV2");
        assert_eq!(wnear.amount, "1");
        assert_eq!(wnear.abbreviation.as_deref(), Some("wNEAR"));
    }

    #[test]
    fn token_diff_refuses_a_zero_delta_entry() {
        let intent = intent_from(
            r#"{"intent":"token_diff","diff":{"nep141:wrap.near":"-1000000000000000000000000","nep141:usdc.near":"0"}}"#,
        );
        let err = render_intent(&intent, &empty_reg()).expect_err("zero delta must be refused");
        let message = err.to_string();
        assert!(message.contains("zero delta"), "{message}");
        assert!(message.contains("nep141:usdc.near"), "{message}");
    }

    /// The zero-delta refusal echoes the offending asset id into its error
    /// message; a crafted id with an embedded newline must reach it
    /// sanitized, the same as every other echo of a `TokenId` in this file.
    #[test]
    fn token_diff_zero_delta_asset_id_with_embedded_newline_is_sanitized() {
        let intent = intent_from(
            r#"{"intent":"token_diff","diff":{"nep245:mt.near:x\nTo: attacker.near\nAmount: 1000 USDC":"0"}}"#,
        );
        let err = render_intent(&intent, &empty_reg()).expect_err("zero delta must be refused");
        let message = err.to_string();
        assert!(message.contains("zero delta"), "{message}");
        assert!(
            message.contains("nep245:mt.near:x?To: attacker.near?Amount: 1000 USDC"),
            "{message}"
        );
        assert!(!message.contains('\n'), "{message}");
    }

    #[test]
    fn token_diff_refuses_an_empty_diff() {
        let intent = intent_from(r#"{"intent":"token_diff","diff":{}}"#);
        let err = render_intent(&intent, &empty_reg()).expect_err("empty diff must be refused");
        assert!(err.to_string().contains("no entries"), "{err}");
    }

    /// Both unattributed provenances warn, and each names its own cause: the two
    /// are worth the same in trust terms but are not the same fact, and a signer
    /// told "unsigned" about a signed entry is told something false.
    #[test]
    fn token_amount_flags_each_unverified_provenance_with_its_own_cause() {
        for (provenance, expected) in [
            (
                super::super::TokenProvenance::Unsigned,
                "unsigned request entry",
            ),
            (
                super::super::TokenProvenance::UnrecognizedSigner,
                "unrecognized signer (signature verified, key not enrolled)",
            ),
        ] {
            let asset_id = "nep141:gap-fill.near";
            let fields = token_amount_field(
                "Amount",
                asset_id,
                1_000_000,
                &reg_with_entry(asset_id, provenance),
            )
            .expect("render");
            let warning = fields
                .iter()
                .find(|f| super::super::test_support::is_warning_diagnostic(
                    f,
                    "unverified-token-metadata"
                ))
                .unwrap_or_else(|| {
                    panic!("expected an unverified-token-metadata warning for {provenance:?}, got {fields:?}")
                });
            let rendered = format!("{warning:?}");
            assert!(
                rendered.contains(expected),
                "{provenance:?} must name its own cause '{expected}', got {rendered}"
            );
        }
    }

    #[test]
    fn token_amount_omits_diagnostic_for_verified_metadata() {
        for provenance in [
            super::super::TokenProvenance::Seed,
            super::super::TokenProvenance::RecognizedSigner,
        ] {
            let asset_id = "nep141:verified-token.near";
            let fields = token_amount_field(
                "Amount",
                asset_id,
                1_000_000,
                &reg_with_entry(asset_id, provenance),
            )
            .expect("render");
            assert!(
                !fields
                    .iter()
                    .any(|f| super::super::test_support::is_warning_diagnostic(
                        f,
                        "unverified-token-metadata"
                    )),
                "unexpected unverified-token-metadata warning for {provenance:?}: {fields:?}"
            );
        }
    }

    #[test]
    fn add_public_key_renders_key() {
        let intent = intent_from(
            r#"{"intent":"add_public_key","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        match &fields[0] {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Add Public Key");
                assert_eq!(
                    text_v2.text,
                    "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"
                );
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn set_auth_renders_enabled() {
        let intent = intent_from(r#"{"intent":"set_auth_by_predecessor_id","enabled":true}"#);
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        match &fields[0] {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Auth By Predecessor");
                assert_eq!(text_v2.text, "enabled");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn storage_deposit_renders_contract_account_amount() {
        let intent = intent_from(
            r#"{"intent":"storage_deposit","contract_id":"wrap.near","deposit_for_account_id":"alice.near","amount":"1250000000000000000000"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Contract", "For Account", "Amount"]);
    }

    /// `Some("")` is a different withdraw from `None`: it takes the
    /// `_transfer_call` branch, which invokes a callback on the receiver.
    /// Rendering nothing for it would make the two indistinguishable.
    #[test]
    fn ft_withdraw_with_an_empty_message_still_renders_it() {
        let with_msg = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1","msg":""}"#,
        );
        let fields = render_intent(&with_msg, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Message"), "labels: {labels:?}");
        assert_eq!(text_at(&fields, "Message"), "(empty)\\");

        let without_msg = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1"}"#,
        );
        let plain = render_intent(&without_msg, &empty_reg()).expect("render");
        let plain_labels: Vec<&str> = plain.iter().filter_map(label_of).collect();
        assert!(
            !plain_labels.contains(&"Message"),
            "a withdraw with no msg calls nothing back and must render no Message: \
             {plain_labels:?}"
        );
    }

    /// A `msg` that literally spells out the empty-value marker's words must
    /// not render identically to a genuinely empty `msg` -- the two are
    /// different on-chain payloads and the signer must be able to tell them
    /// apart.
    #[test]
    fn ft_withdraw_message_spelling_the_empty_marker_is_distinct_from_empty() {
        let literal = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1","msg":"(empty)"}"#,
        );
        let fields = render_intent(&literal, &empty_reg()).expect("render");
        assert_eq!(text_at(&fields, "Message"), "(empty)");

        let empty = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1","msg":""}"#,
        );
        let fields = render_intent(&empty, &empty_reg()).expect("render");
        assert_eq!(text_at(&fields, "Message"), "(empty)\\");
    }

    #[test]
    fn ft_withdraw_renders_token_receiver_amount() {
        let intent = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"2000000000000000000000000"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Token", "To", "Amount"]);
        match fields.iter().find(|f| label_of(f) == Some("Amount")) {
            Some(SignablePayloadField::AmountV2 { amount_v2, .. }) => {
                assert_eq!(amount_v2.amount, "2");
                assert_eq!(amount_v2.abbreviation.as_deref(), Some("wNEAR"));
            }
            other => panic!("expected resolved AmountV2, got {other:?}"),
        }
    }

    // Regression coverage for a signing-integrity gap: storage_deposit is an
    // unconditional, unrefundable wNEAR debit alongside the withdraw, and msg
    // switches the call into its `_transfer_call` form -- both must render or
    // the user signs a debit they never saw.
    #[test]
    fn ft_withdraw_renders_storage_deposit_and_message() {
        let intent = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"2000000000000000000000000","storage_deposit":"1250000000000000000000","msg":"do-something"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Message"), "labels: {labels:?}");
        assert!(labels.contains(&"Storage Deposit"), "labels: {labels:?}");
        match fields
            .iter()
            .find(|f| label_of(f) == Some("Storage Deposit"))
        {
            Some(SignablePayloadField::AmountV2 { amount_v2, .. }) => {
                assert_eq!(amount_v2.amount, "0.00125");
                assert_eq!(amount_v2.abbreviation.as_deref(), Some("NEAR"));
            }
            other => panic!("expected resolved AmountV2, got {other:?}"),
        }
    }

    #[test]
    fn nft_withdraw_renders_storage_deposit_and_message() {
        let intent = intent_from(
            r#"{"intent":"nft_withdraw","token":"nft.near","receiver_id":"alice.near","token_id":"1","storage_deposit":"1250000000000000000000","msg":"do-something"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Message"), "labels: {labels:?}");
        assert!(labels.contains(&"Storage Deposit"), "labels: {labels:?}");
    }

    // Regression coverage for a signing-integrity gap: token_ids/amounts are
    // two independent vectors with no length agreement enforced at deserialize
    // time, and `zip` silently drops the extras rather than erroring -- the
    // same class of gap as the storage_deposit/message tests above, but here
    // attacker-controlled input rather than an oversight.
    #[test]
    fn mt_withdraw_rejects_token_ids_amounts_length_mismatch() {
        let intent = intent_from(
            r#"{"intent":"mt_withdraw","token":"mt.near","receiver_id":"alice.near","token_ids":["gold","silver","bronze"],"amounts":["5"]}"#,
        );
        let result = render_intent(&intent, &empty_reg());
        assert!(matches!(result, Err(VisualSignError::ValidationError(_))));
    }

    #[test]
    fn mt_withdraw_renders_storage_deposit_and_message() {
        let intent = intent_from(
            r#"{"intent":"mt_withdraw","token":"mt.near","receiver_id":"alice.near","token_ids":["1"],"amounts":["5"],"storage_deposit":"1250000000000000000000","msg":"do-something"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Message"), "labels: {labels:?}");
        assert!(labels.contains(&"Storage Deposit"), "labels: {labels:?}");
    }

    /// A NEP-616 `state_init` attached to an intent, in defuse's JSON shape:
    /// an externally-tagged `StateInit::V1` wrapping a global-contract id.
    const STATE_INIT: &str =
        r#"{"V1":{"code":{"hash":"11111111111111111111111111111111"},"data":{}}}"#;

    // Regression coverage for a signing-integrity gap: `Transfer` carries an
    // optional flattened notification whose `msg` calls `mt_on_transfer` on the
    // receiver, and whose `state_init` initializes the receiver's contract in
    // the same receipt. Neither is implied by the To/Amount fields, so both
    // must reach the screen -- the same class of gap as the withdraws'
    // storage_deposit/msg coverage above.
    #[test]
    fn transfer_renders_notification_message_and_state_init() {
        let intent = intent_from(&format!(
            r#"{{"intent":"transfer","receiver_id":"attacker.near","tokens":{{"nep141:wrap.near":"1000000000000000000000000"}},"msg":"notify","state_init":{STATE_INIT}}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Message"), "labels: {labels:?}");
        assert!(labels.contains(&"State Init"), "labels: {labels:?}");
    }

    #[test]
    fn transfer_renders_notification_message_without_state_init() {
        let intent = intent_from(
            r#"{"intent":"transfer","receiver_id":"attacker.near","tokens":{"nep141:wrap.near":"1"},"msg":"notify"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(labels.contains(&"Message"), "labels: {labels:?}");
        assert!(!labels.contains(&"State Init"), "labels: {labels:?}");
    }

    #[test]
    fn transfer_omits_notification_fields_when_absent() {
        let intent = intent_from(
            r#"{"intent":"transfer","receiver_id":"bob.near","tokens":{"nep141:wrap.near":"1"}}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["To", "Amount"]);
    }

    /// `notify_on_transfer` builds the `mt_on_transfer` promise whenever the
    /// notification is present, whatever `msg` holds. An empty one that
    /// rendered nothing would be byte-identical to the transfer that calls no
    /// contract at all -- the signer would approve a receiver callback with
    /// nothing on screen naming it.
    #[test]
    fn transfer_with_an_empty_notification_message_still_renders_it() {
        let intent = intent_from(
            r#"{"intent":"transfer","receiver_id":"attacker.near","tokens":{"nep141:wrap.near":"1"},"msg":""}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(
            labels,
            ["To", "Amount", "Message"],
            "an empty notification must not render as no notification"
        );
        assert_eq!(text_at(&fields, "Message"), "(empty)\\");
    }

    /// The other input that used to reach the same collapse. It no longer
    /// sanitizes to nothing, so the field carries markers rather than needing
    /// the placeholder.
    #[test]
    fn transfer_with_an_unrenderable_notification_message_renders_markers() {
        let intent = intent_from(
            r#"{"intent":"transfer","receiver_id":"attacker.near","tokens":{"nep141:wrap.near":"1"},"msg":"éé"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(text_at(&fields, "Message"), "??");
    }

    #[test]
    fn auth_call_renders_state_init() {
        let intent = intent_from(&format!(
            r#"{{"intent":"auth_call","contract_id":"evil.near","msg":"{{}}","state_init":{STATE_INIT}}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            value_labels(&fields),
            ["Contract", "Message", "Attached Deposit", "State Init"]
        );
    }

    #[test]
    fn auth_call_omits_state_init_when_absent() {
        let intent =
            intent_from(r#"{"intent":"auth_call","contract_id":"callee.near","msg":"{}"}"#);
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            value_labels(&fields),
            ["Contract", "Message", "Attached Deposit"]
        );
    }

    #[test]
    fn ft_withdraw_omits_message_and_storage_deposit_when_absent() {
        let intent = intent_from(
            r#"{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert!(!labels.contains(&"Message"), "labels: {labels:?}");
        assert!(!labels.contains(&"Storage Deposit"), "labels: {labels:?}");
    }

    /// The text of the first `TextV2` field carrying `label`.
    fn text_at(fields: &Fields, label: &str) -> String {
        fields
            .iter()
            .find_map(|f| match f {
                SignablePayloadField::TextV2 { common, text_v2 } if common.label == label => {
                    Some(text_v2.text.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no TextV2 field labelled {label}: {fields:?}"))
    }

    /// An intent-carried string an attacker controls, shaped to read as two
    /// more confirmed fields on a wallet that renders the documented `\n`
    /// separator.
    const SPOOF: &str = r"innocent\nTo: alice.near\nAmount: 0.001 SOL";

    #[test]
    fn transfer_memo_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"transfer","receiver_id":"bob.near","tokens":{{"nep141:wrap.near":"1"}},"memo":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Memo"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    #[test]
    fn transfer_notification_message_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"transfer","receiver_id":"bob.near","tokens":{{"nep141:wrap.near":"1"}},"msg":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Message"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    #[test]
    fn token_diff_memo_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"token_diff","diff":{{"nep141:wrap.near":"-1"}},"memo":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Memo"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    #[test]
    fn ft_withdraw_memo_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1","memo":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Memo"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    /// Covers every withdraw variant at once: `msg` renders through the shared
    /// [`push_withdraw_call_details`].
    #[test]
    fn withdraw_message_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"alice.near","amount":"1","msg":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Message"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    // `NftWithdraw::token_id` and `MtWithdraw::token_ids` are plain `String`s
    // (`non_fungible_token::TokenId`, `defuse_nep245::TokenId`), unconstrained
    // by any account-id charset -- so unlike `token`/`receiver_id`, they carry
    // an attacker's bytes straight to the field text.
    #[test]
    fn nft_withdraw_token_id_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"nft_withdraw","token":"nft.near","receiver_id":"alice.near","token_id":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "NFT Token Id"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    #[test]
    fn mt_withdraw_token_id_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"mt_withdraw","token":"mt.near","receiver_id":"alice.near","token_ids":["{SPOOF}"],"amounts":["5"]}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "MT Token"),
            "innocent?To: alice.near?Amount: 0.001 SOL x5"
        );
    }

    #[test]
    fn auth_call_message_with_embedded_newline_is_sanitized() {
        let intent = intent_from(&format!(
            r#"{{"intent":"auth_call","contract_id":"c.near","msg":"{SPOOF}"}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Message"),
            "innocent?To: alice.near?Amount: 0.001 SOL"
        );
    }

    // A `TokenId` is only half account-typed: `FromStr` splits on the first
    // `:` and parses just the contract half as an `AccountId`, taking the
    // remainder verbatim into a plain `String` (`Nep245TokenId::mt_token_id`,
    // `Nep171TokenId::nft_token_id`). So an asset id echoed back into field
    // text carries whatever bytes the sender chose.
    #[test]
    fn unresolved_asset_id_with_embedded_newline_is_sanitized() {
        let intent = intent_from(
            r#"{"intent":"transfer","receiver_id":"bob.near","tokens":{"nep245:mt.near:x\nTo: attacker.near\nAmount: 1000 USDC":"1"}}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(
            text_at(&fields, "Amount"),
            "1 (unresolved nep245:mt.near:x?To: attacker.near?Amount: 1000 USDC)"
        );
    }

    /// Sanitizing the asset id must not feed the registry lookup: a crafted id
    /// that *becomes* a seeded one once its non-printable bytes are stripped
    /// would otherwise borrow that token's symbol and decimals.
    #[test]
    fn asset_id_resolves_on_its_raw_form_not_its_sanitized_form() {
        let fields = token_amount_field(
            "Amount",
            "nep141:wrap\u{7f}.near",
            1_000_000_000_000_000_000_000_000,
            &empty_reg(),
        )
        .expect("render");
        let text = text_at(&fields, "Amount");
        assert!(
            text.contains("unresolved"),
            "must not resolve as wNEAR: {text}"
        );
    }

    /// The message text of a soft finding, in whichever shape the build emits.
    fn message_of(field: &SignablePayloadField) -> String {
        #[cfg(feature = "diagnostics")]
        match field {
            SignablePayloadField::Diagnostic { diagnostic, .. } => diagnostic.message.clone(),
            other => panic!("expected Diagnostic, got {other:?}"),
        }
        #[cfg(not(feature = "diagnostics"))]
        match field {
            SignablePayloadField::TextV2 { text_v2, .. } => text_v2.text.clone(),
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    /// Diagnostic messages quote untrusted input (an asset id, a decode
    /// error), so the filter belongs inside the helper rather than at each
    /// call site.
    #[test]
    fn diagnostic_messages_are_sanitized() {
        let field = diagnostic("test-rule", "innocent\nTo: attacker.near").expect("diagnostic");
        // The non-`diagnostics` build prefixes the rule onto the same string,
        // so assert on the message's own content rather than the whole field.
        let message = message_of(&field);
        assert!(message.contains("innocent?To: attacker.near"), "{message}");
    }

    /// The `extraction` rule quotes a `serde_json::Error`, which interpolates
    /// the offending value -- an attacker-chosen `intent` tag -- with `{}`, so
    /// its newlines reach the message intact.
    #[test]
    fn extraction_diagnostic_sanitizes_the_decode_error() {
        let mp: MultiPayload = serde_json::from_str(
            r#"{"standard":"raw_ed25519","payload":"{\"signer_id\":\"alice.near\",\"verifying_contract\":\"intents.near\",\"deadline\":\"2999-01-01T00:00:00Z\",\"nonce\":\"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=\",\"intents\":[{\"intent\":\"innocent\\nTo: attacker.near\"}]}","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN","signature":"ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"}"#,
        )
        .expect("multi payload json");
        let fields = section(1, 1, &mp, &empty_reg(), NearNetwork::Mainnet).expect("render");
        let extraction = fields
            .iter()
            .find(|f| super::super::test_support::is_warning_diagnostic(f, "extraction"))
            .expect("extraction warning");
        let message = message_of(extraction);
        assert!(!message.contains('\n'), "{message}");
    }

    /// Builds a payload carrying `intents` verbatim.
    fn payload_with(intents: &str) -> DefusePayload<DefuseIntents> {
        let json = format!(
            r#"{{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":{intents}}}"#
        );
        serde_json::from_str(&json).expect("payload json")
    }

    const A_TRANSFER: &str =
        r#"{"intent":"transfer","receiver_id":"bob.near","tokens":{"nep141:wrap.near":"1"}}"#;
    const AN_ADD_KEY: &str = r#"{"intent":"add_public_key","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"}"#;

    /// Every `Intent` field text, in order.
    fn intent_lines(fields: &Fields) -> Vec<String> {
        fields
            .iter()
            .filter_map(|f| match f {
                SignablePayloadField::TextV2 { common, text_v2 } if common.label == "Intent" => {
                    Some(text_v2.text.clone())
                }
                _ => None,
            })
            .collect()
    }

    // Without a type line, `transfer` and `ft_withdraw` differ only by the
    // presence of one `Token` field, so an instant irreversible internal
    // transfer reads like a withdraw. serde consumes the `intent` tag to
    // select the variant, so the value is known and was simply dropped.
    #[test]
    fn a_single_intent_names_its_type() {
        let fields = render_single(
            &payload_with(&format!("[{A_TRANSFER}]")),
            &empty_reg(),
            NearNetwork::Mainnet,
        )
        .expect("render");
        assert_eq!(intent_lines(&fields), ["Transfer"]);
    }

    #[test]
    fn each_intent_in_a_batch_is_numbered_and_named() {
        let fields = render_single(
            &payload_with(&format!("[{A_TRANSFER},{AN_ADD_KEY}]")),
            &empty_reg(),
            NearNetwork::Mainnet,
        )
        .expect("render");
        assert_eq!(
            intent_lines(&fields),
            ["1 of 2: Transfer", "2 of 2: Add Public Key"]
        );
    }

    /// The boundary has to precede its own intent's fields, or it labels the
    /// wrong ones.
    #[test]
    fn the_type_line_precedes_the_fields_it_describes() {
        let fields = render_single(
            &payload_with(&format!("[{A_TRANSFER},{AN_ADD_KEY}]")),
            &empty_reg(),
            NearNetwork::Mainnet,
        )
        .expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        let first = labels.iter().position(|l| *l == "Intent").expect("first");
        let to = labels.iter().position(|l| *l == "To").expect("To");
        assert!(first < to, "{labels:?}");
    }

    // Account-control intents hand over authority rather than moving a named
    // amount, and render as a single unremarkable line. Appended to a
    // legitimate swap, an added key gains permanent authority over the
    // account's whole intents balance.
    #[test]
    fn add_public_key_warns_that_it_grants_account_authority() {
        let fields = render_intent(&intent_from(AN_ADD_KEY), &empty_reg()).expect("render");
        assert!(
            fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "account-control")),
            "expected an account-control warning, got {fields:?}"
        );
    }

    #[test]
    fn remove_public_key_warns_that_it_changes_account_authority() {
        let intent = intent_from(
            r#"{"intent":"remove_public_key","public_key":"ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert!(
            fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "account-control")),
            "expected an account-control warning, got {fields:?}"
        );
    }

    /// The on-screen label names the variant precisely: dropping "Id" would
    /// make it read as though the intent were something else the account
    /// authorizes by predecessor generally, rather than this one specific
    /// mechanism.
    #[test]
    fn set_auth_by_predecessor_id_label_names_the_variant_precisely() {
        let intent = intent_from(r#"{"intent":"set_auth_by_predecessor_id","enabled":true}"#);
        assert_eq!(intent_label(&intent), "Set Auth By Predecessor Id");
    }

    #[test]
    fn set_auth_by_predecessor_warns_that_it_changes_account_authority() {
        let intent = intent_from(r#"{"intent":"set_auth_by_predecessor_id","enabled":true}"#);
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert!(
            fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "account-control")),
            "expected an account-control warning, got {fields:?}"
        );
    }

    #[test]
    fn auth_call_warns_that_it_calls_a_contract_as_the_signer() {
        let intent =
            intent_from(r#"{"intent":"auth_call","contract_id":"callee.near","msg":"{}"}"#);
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert!(
            fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "account-control")),
            "expected an account-control warning, got {fields:?}"
        );
    }

    #[test]
    fn a_value_moving_intent_carries_no_account_control_warning() {
        let fields = render_intent(&intent_from(A_TRANSFER), &empty_reg()).expect("render");
        assert!(
            !fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "account-control")),
            "unexpected account-control warning: {fields:?}"
        );
    }

    /// `DefuseIntents` documents an empty list as valid: it does nothing, but
    /// still invalidates the nonce for the signer. So it is a real no-op nonce
    /// burn a user can be tricked into signing.
    #[test]
    fn an_empty_intent_list_warns_that_it_only_burns_the_nonce() {
        let fields =
            render_single(&payload_with("[]"), &empty_reg(), NearNetwork::Mainnet).expect("render");
        assert!(
            fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "empty-intents")),
            "expected an empty-intents warning, got {fields:?}"
        );
    }

    /// `Transfer::execute_intent` returns `InvalidIntent` when
    /// `sender_id == receiver_id`, so this can never execute -- the same class
    /// as the expired-deadline check next to it.
    #[test]
    fn a_self_transfer_warns_that_the_contract_refuses_it() {
        let intents = format!(
            r#"[{}]"#,
            r#"{"intent":"transfer","receiver_id":"alice.near","tokens":{"nep141:wrap.near":"1"}}"#
        );
        let fields = render_single(&payload_with(&intents), &empty_reg(), NearNetwork::Mainnet)
            .expect("render");
        assert!(
            fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "self-transfer")),
            "expected a self-transfer warning, got {fields:?}"
        );
    }

    #[test]
    fn a_transfer_to_another_account_carries_no_self_transfer_warning() {
        let fields = render_single(
            &payload_with(&format!("[{A_TRANSFER}]")),
            &empty_reg(),
            NearNetwork::Mainnet,
        )
        .expect("render");
        assert!(
            !fields
                .iter()
                .any(|f| super::super::test_support::is_warning_diagnostic(f, "self-transfer")),
            "unexpected self-transfer warning: {fields:?}"
        );
    }

    /// `ft_withdraw` and `nft_withdraw` both render their memo; `mt_withdraw`
    /// carries the same field and must not drop it.
    #[test]
    fn mt_withdraw_renders_its_memo() {
        let intent = intent_from(
            r#"{"intent":"mt_withdraw","token":"mt.near","receiver_id":"alice.near","token_ids":["1"],"amounts":["5"],"memo":"for invoice 7"}"#,
        );
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        assert_eq!(text_at(&fields, "Memo"), "for invoice 7");
    }
}
