//! NEAR Intents protocol decoder.
//!
//! Public API uses only `visualsign` types + plain primitives. The defuse-*
//! and near-sdk transitive dependencies are confined to this module's
//! internals.

mod args;
mod render;
mod token_signature;
mod tokens;
mod verify;

pub use token_signature::{
    RejectedTokenMetadata, TokenMetadataExtraction, TokenMetadataSignerAllowlists,
    authorized_token_metadata_signers, sign_token_metadata_for_cli,
    try_extract_from_chain_metadata as try_extract_token_metadata_from_chain_metadata,
};

pub(crate) use render::rejected_metadata_diagnostics;

/// Dev/CLI signing helpers for constructing signed `TokenMetadataEntry` proto
/// values (e.g. for local test fixtures). Gated the same way as the
/// underlying implementation; see `token_signature` module docs.
#[cfg(any(test, feature = "dev-signing"))]
pub use token_signature::{
    DEV_ETHEREUM_SIGNING_KEY_SEED, DEV_NEAR_SIGNING_KEY_SEED, DEV_SOLANA_SIGNING_KEY_SEED,
    sign_token_metadata_ed25519, sign_token_metadata_secp256k1,
};

use thiserror::Error;

/// NEAR's native token symbol and decimal scale (yoctoNEAR = 10^-24 NEAR).
pub(crate) const NEAR_SYMBOL: &str = "NEAR";
pub(crate) const NEAR_DECIMALS: u8 = 24;

#[derive(Debug, Error)]
pub enum NearIntentsError {
    #[error("input was not valid JSON: {0}")]
    InputNotJson(String),
    #[error("Failed to render intents: {0}")]
    Render(String),
}

/// Plain NEP-141 token metadata. The lean baseline chain parser constructs
/// the registry; this module consumes it via `LayeredRegistry<NearTokenRegistry>`.
#[derive(Debug, Default)]
pub struct NearTokenRegistry {
    pub by_asset_id: std::collections::BTreeMap<String, TokenMeta>,
}

#[derive(Debug, Clone)]
pub struct TokenMeta {
    pub symbol: String,
    pub decimals: u8,
    /// Whether this metadata is trustworthy: resolved from the compiled-in
    /// `tokens::SEEDS` table, or from a signed, allowlisted request entry.
    /// `false` for an unsigned request entry (fills a gap for an asset
    /// `SEEDS` doesn't cover -- accepted, but unauthenticated). Rendering
    /// surfaces this so the signer sees the caveat, not just an operator log.
    pub verified: bool,
}

/// Decode `execute_intents` args and render them as `SignablePayloadField`s.
///
/// This is the sole public entry point for the signed-envelope batch.
pub fn try_decode_execute_intents(
    args: &[u8],
    token_registry: &visualsign::registry::LayeredRegistry<NearTokenRegistry>,
    _options: &visualsign::vsptrait::VisualSignOptions,
) -> Result<Vec<visualsign::SignablePayloadField>, NearIntentsError> {
    let payloads = args::decode_args(args)?;
    let total = payloads.len();
    let mut fields = Vec::new();
    for (i, mp) in payloads.iter().enumerate() {
        fields.extend(
            render::section(i + 1, total, mp, token_registry)
                .map_err(|e| NearIntentsError::Render(e.to_string()))?,
        );
    }
    Ok(fields)
}

/// A rendered pre-signature envelope: the fields, plus the title naming what
/// the envelope does.
pub struct RenderedEnvelope {
    /// Payload title. A lone intent names its type; a batch stays generic.
    pub title: String,
    /// Envelope + per-intent fields.
    pub fields: Vec<visualsign::SignablePayloadField>,
}

/// Render the single intent a user is about to sign, from the JSON of its
/// `DefusePayload` message (the inner message that gets signed, independent of
/// which signature standard later wraps it).
///
/// This is the user-signing view: it reuses the envelope + per-intent rendering
/// but skips signature verification (no signature exists at signing time).
pub fn try_render_single_intent(
    payload_json: &[u8],
    token_registry: &visualsign::registry::LayeredRegistry<NearTokenRegistry>,
    _options: &visualsign::vsptrait::VisualSignOptions,
) -> Result<RenderedEnvelope, NearIntentsError> {
    let payload: defuse_core::payload::DefusePayload<defuse_core::intents::DefuseIntents> =
        serde_json::from_slice(payload_json)
            .map_err(|e| NearIntentsError::InputNotJson(e.to_string()))?;
    let fields = render::render_single(&payload, token_registry)
        .map_err(|e| NearIntentsError::Render(e.to_string()))?;
    Ok(RenderedEnvelope {
        title: render::title_for_intents(&payload.intents),
        fields,
    })
}

/// Test-only matcher shared across this module's test submodules.
#[cfg(test)]
pub(crate) mod test_support {
    use visualsign::SignablePayloadField;

    /// Matches the soft-finding field for `rule` in whichever shape the build
    /// emits: the structured `Diagnostic` (diagnostics feature) or the
    /// `Warning`-labelled text fallback.
    pub(crate) fn is_warning_diagnostic(field: &SignablePayloadField, rule: &str) -> bool {
        #[cfg(feature = "diagnostics")]
        {
            matches!(field, SignablePayloadField::Diagnostic { diagnostic, .. }
                if diagnostic.rule == rule && diagnostic.level == "warn")
        }
        #[cfg(not(feature = "diagnostics"))]
        {
            matches!(field, SignablePayloadField::TextV2 { common, text_v2 }
                if common.label == "Warning" && text_v2.text.starts_with(rule))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use visualsign::SignablePayloadField;
    use visualsign::registry::LayeredRegistry;
    use visualsign::vsptrait::VisualSignOptions;

    fn label_of(f: &SignablePayloadField) -> Option<&str> {
        match f {
            SignablePayloadField::TextV2 { common, .. }
            | SignablePayloadField::AmountV2 { common, .. }
            | SignablePayloadField::AddressV2 { common, .. } => Some(common.label.as_str()),
            _ => None,
        }
    }

    #[test]
    fn pipeline_decodes_and_renders_intent_section() {
        // Current-format inner payload (ISO-8601 deadline). The key/signature are
        // well-formed but do not match, so the signature reads INVALID while the
        // intents still decode and render.
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000000000000000000000"}]}"#;
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});
        let bytes = serde_json::to_vec(&args).unwrap();
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));

        let fields =
            try_decode_execute_intents(&bytes, &reg, &VisualSignOptions::default()).unwrap();
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();

        for expected in [
            "Signed Intent",
            "Standard",
            "Signer",
            "Verifying Contract",
            "Deadline",
            "Nonce",
            "Token",
            "To",
            "Amount",
        ] {
            assert!(labels.contains(&expected), "missing field: {expected}");
        }
        // Signature did not match the payload -> a signature warning.
        let has_sig_warning = fields
            .iter()
            .any(|f| super::test_support::is_warning_diagnostic(f, "signature"));
        assert!(has_sig_warning, "expected a signature warning");
        // wrap.near resolved -> 1 wNEAR.
        let amount = fields
            .iter()
            .find(|f| label_of(f) == Some("Amount"))
            .unwrap();
        match amount {
            SignablePayloadField::AmountV2 { amount_v2, .. } => {
                assert_eq!(amount_v2.amount, "1");
                assert_eq!(amount_v2.abbreviation.as_deref(), Some("wNEAR"));
            }
            other => panic!("expected AmountV2, got {other:?}"),
        }
    }

    #[test]
    fn single_intent_renders_envelope_and_intent_without_signature() {
        // The pre-signature signing view: a bare DefusePayload (no MultiPayload
        // wrapper, no signature). Envelope + intent render; no signature section.
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000000000000000000000"}]}"#;
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));

        let rendered =
            try_render_single_intent(inner.as_bytes(), &reg, &VisualSignOptions::default())
                .unwrap();
        let fields = rendered.fields;
        let labels: Vec<&str> = fields.iter().filter_map(label_of).collect();

        // A lone intent names its type in both the title and the Intent field.
        assert_eq!(rendered.title, "NEAR Intent: FT Withdraw");
        assert!(labels.contains(&"Intent"), "missing the intent type field");

        for expected in [
            "Signer",
            "Verifying Contract",
            "Deadline",
            "Nonce",
            "Token",
            "To",
            "Amount",
        ] {
            assert!(labels.contains(&expected), "missing field: {expected}");
        }
        // Pre-signature view: no signature section header or standard field.
        assert!(
            !labels.contains(&"Signed Intent"),
            "unexpected signature section"
        );
        assert!(!labels.contains(&"Standard"), "unexpected signature field");
        // wrap.near resolves to 1 wNEAR.
        let amount = fields
            .iter()
            .find(|f| label_of(f) == Some("Amount"))
            .unwrap();
        match amount {
            SignablePayloadField::AmountV2 { amount_v2, .. } => {
                assert_eq!(amount_v2.amount, "1");
                assert_eq!(amount_v2.abbreviation.as_deref(), Some("wNEAR"));
            }
            other => panic!("expected AmountV2, got {other:?}"),
        }
    }

    #[test]
    fn single_intent_rejects_malformed_json() {
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        let err = try_render_single_intent(b"not json", &reg, &VisualSignOptions::default())
            .err()
            .expect("malformed JSON should error");
        assert!(matches!(err, NearIntentsError::InputNotJson(_)));
    }

    #[test]
    fn pipeline_flags_expired_deadline() {
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2020-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[]}"#;
        let args = serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]});
        let bytes = serde_json::to_vec(&args).unwrap();
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        let fields =
            try_decode_execute_intents(&bytes, &reg, &VisualSignOptions::default()).unwrap();
        let has_deadline_warning = fields
            .iter()
            .any(|f| super::test_support::is_warning_diagnostic(f, "deadline"));
        assert!(has_deadline_warning, "expected a deadline warning");
    }
}
