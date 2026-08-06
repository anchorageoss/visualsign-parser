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
#[cfg(feature = "diagnostics")]
fn diagnostic(rule: &str, message: &str) -> Result<SignablePayloadField, VisualSignError> {
    Ok(visualsign::field_builders::create_diagnostic_field(
        rule,
        "near-intents",
        visualsign::lint::Severity::Warn,
        message,
        None,
    )
    .signable_payload_field)
}

/// See the `diagnostics`-enabled twin above.
#[cfg(not(feature = "diagnostics"))]
fn diagnostic(rule: &str, message: &str) -> Result<SignablePayloadField, VisualSignError> {
    Ok(create_text_field("Warning", &format!("{rule}: {message}"))?.signable_payload_field)
}

/// Report each caller-supplied token-metadata entry the parser refused.
///
/// Without this the signer sees only the consequence -- an amount rendered in
/// raw base units against an `unresolved` asset id, or resolved from a seed
/// instead of the supplied override -- with no indication that metadata was
/// supplied and thrown away. A rejection is a soft finding: the intents still
/// render, since the refusal protects them rather than invalidating them.
///
/// Both halves of the message are charset-filtered. `asset_id` is a
/// caller-controlled map key and `reason` can quote it back (a JSON parse
/// error, a length), so an embedded newline would otherwise render as extra
/// apparent fields on the signing screen.
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
            let message = crate::actions::charset_safe(&format!(
                "token metadata supplied for {} was rejected and not used: {}",
                r.asset_id, r.reason
            ));
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
fn token_amount_field(
    label: &str,
    asset_id: &str,
    raw: u128,
    registry: &Reg,
) -> Result<Fields, VisualSignError> {
    match tokens::resolve(asset_id, registry) {
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

/// Render the fields intrinsic to a single intent.
pub(crate) fn render_intent(intent: &Intent, registry: &Reg) -> Result<Fields, VisualSignError> {
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
                create_text_field("Message", &c.msg)?.signable_payload_field,
                near_amount_field("Attached Deposit", c.attached_deposit.as_yoctonear())?,
            ];
            if c.state_init.is_some() {
                fields.push(state_init_field()?);
            }
            Ok(fields)
        }
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
                "token_diff entry for {token_id} has a zero delta"
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
    if let Some(memo) = &td.memo {
        fields.push(create_text_field("Memo", memo)?.signable_payload_field);
    }
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
    if let Some(memo) = &t.memo {
        fields.push(create_text_field("Memo", memo)?.signable_payload_field);
    }
    // A `Transfer`'s optional notification changes what the transfer does
    // beyond moving the named tokens: `msg` calls `mt_on_transfer` on
    // `receiver_id` (the internal-transfer counterpart of the withdraws'
    // `_transfer_call` form), and `state_init` initializes the receiver's
    // contract in the same receipt.
    if let Some(notification) = &t.notification {
        fields.push(create_text_field("Message", &notification.msg)?.signable_payload_field);
        if notification.state_init.is_some() {
            fields.push(state_init_field()?);
        }
    }
    Ok(fields)
}

/// A withdraw's optional `msg` (switches the call into its `_transfer_call`
/// form, passing this to the receiver) and `storage_deposit` (a separate,
/// unconditional wNEAR debit for the receiver's storage on `token`, never
/// refunded on failure) -- both must render, since either changes what the
/// withdraw actually does beyond moving the named token/amount.
fn push_withdraw_call_details(
    fields: &mut Fields,
    msg: &Option<String>,
    storage_deposit: Option<near_sdk::NearToken>,
) -> Result<(), VisualSignError> {
    if let Some(msg) = msg.as_deref().filter(|m| !m.is_empty()) {
        fields.push(create_text_field("Message", msg)?.signable_payload_field);
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
    if let Some(memo) = &w.memo {
        fields.push(create_text_field("Memo", memo)?.signable_payload_field);
    }
    push_withdraw_call_details(&mut fields, &w.msg, w.storage_deposit)?;
    Ok(fields)
}

fn render_nft_withdraw(w: &NftWithdraw) -> Result<Fields, VisualSignError> {
    let mut fields = vec![
        create_text_field("Token", w.token.as_str())?.signable_payload_field,
        create_text_field("To", w.receiver_id.as_str())?.signable_payload_field,
        create_text_field("NFT Token Id", w.token_id.as_str())?.signable_payload_field,
    ];
    if let Some(memo) = &w.memo {
        fields.push(create_text_field("Memo", memo)?.signable_payload_field);
    }
    push_withdraw_call_details(&mut fields, &w.msg, w.storage_deposit)?;
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
        fields.push(
            create_text_field("MT Token", &format!("{} x{}", id, amount.0))?.signable_payload_field,
        );
    }
    push_withdraw_call_details(&mut fields, &w.msg, w.storage_deposit)?;
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
    for intent in &payload.intents {
        fields.extend(render_intent(intent, registry)?);
    }
    Ok(fields)
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

    #[test]
    fn auth_call_renders_state_init() {
        let intent = intent_from(&format!(
            r#"{{"intent":"auth_call","contract_id":"evil.near","msg":"{{}}","state_init":{STATE_INIT}}}"#
        ));
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(
            labels,
            ["Contract", "Message", "Attached Deposit", "State Init"]
        );
    }

    #[test]
    fn auth_call_omits_state_init_when_absent() {
        let intent =
            intent_from(r#"{"intent":"auth_call","contract_id":"callee.near","msg":"{}"}"#);
        let fields = render_intent(&intent, &empty_reg()).expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Contract", "Message", "Attached Deposit"]);
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
}
