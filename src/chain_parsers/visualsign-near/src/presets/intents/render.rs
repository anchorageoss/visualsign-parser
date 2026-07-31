//! Render decoded intents into `visualsign` fields. Only this module reads
//! defuse types and emits `visualsign` types.

use defuse_core::intents::token_diff::TokenDiff;
use defuse_core::intents::tokens::{FtWithdraw, MtWithdraw, NativeWithdraw, NftWithdraw, Transfer};
use defuse_core::intents::{DefuseIntents, Intent};
use defuse_core::payload::DefusePayload;
use defuse_core::payload::multi::MultiPayload;
use visualsign::SignablePayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{create_amount_field, create_text_field};
use visualsign::registry::LayeredRegistry;

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

/// Render one token amount, resolving symbol/decimals when the asset is known;
/// otherwise show the raw base-unit amount tagged with the unresolved asset id.
fn token_amount_field(
    label: &str,
    asset_id: &str,
    raw: u128,
    registry: &Reg,
) -> Result<SignablePayloadField, VisualSignError> {
    match tokens::resolve(asset_id, registry) {
        Some(meta) => Ok(create_amount_field(
            label,
            &tokens::format_units(raw, meta.decimals),
            &meta.symbol,
        )?
        .signable_payload_field),
        None => Ok(
            create_text_field(label, &format!("{raw} (unresolved {asset_id})"))?
                .signable_payload_field,
        ),
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
) -> Result<Fields, VisualSignError> {
    let (check, extracted) = super::verify::verify_and_extract(mp);
    let mut fields = vec![
        create_text_field("Signed Intent", &format!("{index} of {total}"))?.signable_payload_field,
    ];
    fields.extend(render_signature(standard_name(mp), &check)?);
    match &extracted {
        Ok(payload) => fields.extend(render_single(payload, registry)?),
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
        Intent::AuthCall(c) => Ok(vec![
            create_text_field("Contract", c.contract_id.as_str())?.signable_payload_field,
            create_text_field("Message", &c.msg)?.signable_payload_field,
            near_amount_field("Attached Deposit", c.attached_deposit.as_yoctonear())?,
        ]),
    }
}

fn render_token_diff(td: &TokenDiff, registry: &Reg) -> Result<Fields, VisualSignError> {
    let mut fields = Fields::new();
    for (token_id, delta) in td.diff.iter() {
        let label = if *delta < 0 { "Send" } else { "Receive" };
        fields.push(token_amount_field(
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
        fields.push(token_amount_field(
            "Amount",
            &token_id.to_string(),
            *amount,
            registry,
        )?);
    }
    if let Some(memo) = &t.memo {
        fields.push(create_text_field("Memo", memo)?.signable_payload_field);
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
        token_amount_field(
            "Amount",
            &format!("nep141:{}", w.token),
            w.amount.0,
            registry,
        )?,
    ];
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

/// Render the standard + signature-status fields for one signed payload.
pub(crate) fn render_signature(
    standard: &str,
    check: &SignatureCheck,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    let mut fields = vec![create_text_field("Standard", standard)?.signable_payload_field];
    match check {
        SignatureCheck::Valid { recovered_key } => fields.push(
            create_text_field("Signature", &format!("valid (recovered {recovered_key})"))?
                .signable_payload_field,
        ),
        SignatureCheck::Invalid => {
            fields.push(diagnostic("signature", "signature verification failed")?);
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
) -> Result<Fields, VisualSignError> {
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

    fn intent_from(json: &str) -> Intent {
        serde_json::from_str(json).expect("intent json")
    }

    #[test]
    fn signature_fields_present_for_valid() {
        let fields = render_signature(
            "nep413",
            &SignatureCheck::Valid {
                recovered_key: "ed25519:abc".to_string(),
            },
        )
        .expect("render");
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();
        assert_eq!(labels, ["Standard", "Signature"]);
    }

    #[test]
    fn invalid_signature_renders_warning() {
        let fields = render_signature("erc191", &SignatureCheck::Invalid).expect("render");
        assert!(
            super::super::test_support::is_warning_diagnostic(&fields[1], "signature"),
            "expected a signature warning, got {:?}",
            fields[1]
        );
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

        let fields = section(1, 1, &mp, &empty_reg()).expect("render");

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
