//! NEAR Intents protocol decoder.
//!
//! Content, not an envelope: the same `DefusePayload` arrives through a NEAR
//! transaction, a NEP-413 message, a Solana raw ed25519 message or an ERC-191
//! message, so this crate knows how to read one and nothing about how it
//! travelled. It has no chain-parser dependency: only the NEAR SDK and the
//! defuse types. Those are not free of C, though: `near-sdk` pulls
//! `near-crypto` -> `secp256k1` -> `secp256k1-sys`, so a crate depending on this
//! one is not a no-C build (see `APPLE_NO_C_CRATES` in `src/Makefile`).
//!
//! Public API uses only `visualsign` types + plain primitives. The defuse-*
//! and near-sdk transitive dependencies are confined to this module's
//! internals.

mod args;
pub mod network;

/// The account the intents protocol settles through. A multi-token asset id
/// names this contract when the balance it wraps is one the protocol holds, so
/// resolving such an id is a property of intents rather than of any chain.
pub const INTENTS_RECEIVER: &str = "intents.near";
mod render;
pub mod strict;
mod token_signature;
mod tokens;
mod verify;

pub use token_signature::{
    RejectedTokenMetadata, TokenMetadataDomain, TokenMetadataExtraction,
    authorized_token_metadata_signers, insert_token_metadata_signer, sign_token_metadata_for_cli,
    try_extract_from_chain_metadata as try_extract_token_metadata_from_chain_metadata,
};

pub use render::rejected_metadata_diagnostics;
pub use tokens::format_tgas;

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
    /// The input is not a NEAR Intents payload: not JSON, or JSON that is not
    /// [intent-shaped](is_intent_shaped). A caller that renders arbitrary
    /// messages shows this input as text.
    #[error("input was not valid JSON: {0}")]
    InputNotJson(String),
    #[error("Failed to render intents: {0}")]
    Render(String),
    /// An envelope account contradicts the resolved network. Kept distinct from
    /// [`Self::Render`] so the caller can raise it as the same validation error
    /// the on-chain transaction accounts raise, rather than as a conversion
    /// failure: it is the same condition, and a wallet branching on the error
    /// class must not see two different ones depending on which path carried the
    /// request.
    #[error("{0}")]
    NetworkMismatch(String),
    /// The input decodes as a `DefusePayload` but carries a top-level key the
    /// type does not declare. Kept distinct from [`Self::InputNotJson`]: a
    /// caller that falls back to rendering a non-intents message as text must
    /// not do so here, because this message is intents with something extra
    /// that would be signed without being shown.
    #[error("{0}")]
    UndeclaredFields(String),
    /// The input is [intent-shaped](is_intent_shaped), so it is meant as a
    /// `DefusePayload`, but it does not decode as one this build understands:
    /// an intent kind it does not know (`{"intent":"future_unknown"}`), or an
    /// `intents` value of the wrong shape (`null`). Kept distinct from
    /// [`Self::InputNotJson`] so a caller that shows non-intents text as text
    /// refuses this instead: rendering it as a plain message would show a
    /// signer raw JSON for an intent the parser could not interpret.
    #[error("recognized NEAR Intents payload is malformed or unsupported: {0}")]
    MalformedIntentEnvelope(String),
}

/// The keys every `DefusePayload` must carry. `intents` is not among them:
/// defuse defaults a missing list to an empty one, and a payload with none is
/// valid (it spends the nonce, which is how a pending request is cancelled).
pub const REQUIRED_ENVELOPE_KEYS: [&str; 4] =
    ["signer_id", "verifying_contract", "deadline", "nonce"];

/// Whether `json` is meant as a NEAR Intents payload: a JSON object carrying
/// every [required envelope key](REQUIRED_ENVELOPE_KEYS), or an `intents` key,
/// whatever that key holds.
///
/// Classified by the required keys rather than by `intents` alone, because
/// `intents` is optional: `{"signer_id", "verifying_contract", "deadline",
/// "nonce"}` with no `intents` is a payload the verifier executes (a cancel).
/// An `intents` key also counts, so a payload that names intents but lacks an
/// envelope key is refused as malformed rather than shown as text. JSON with
/// only some envelope keys cannot decode as a payload, so the verifier would
/// not accept a signature over it either.
///
/// A chain that renders arbitrary messages uses this to fail closed: an
/// intent-shaped message that does not decode must be refused, not shown as
/// text.
#[must_use]
pub fn is_intent_shaped(json: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(json).is_ok_and(|value| {
        value.as_object().is_some_and(|o| {
            o.contains_key("intents") || REQUIRED_ENVELOPE_KEYS.iter().all(|k| o.contains_key(*k))
        })
    })
}

/// The keys every intents message inside a NEP-413 envelope must carry. The
/// envelope supplies the rest: `extract_defuse_payload` takes the verifying
/// contract from `recipient` and the nonce from the envelope's `nonce`. As in
/// [`REQUIRED_ENVELOPE_KEYS`], `intents` is optional.
pub const REQUIRED_NEP413_MESSAGE_KEYS: [&str; 2] = ["signer_id", "deadline"];

/// [`is_intent_shaped`] for the `message` of a NEP-413 envelope: a JSON object
/// with an `intents` key, or every [required NEP-413 message
/// key](REQUIRED_NEP413_MESSAGE_KEYS). `{"signer_id", "deadline"}` with no
/// `intents` extracts as a payload that only spends the nonce, a cancel, so it
/// is intents, not free text.
#[must_use]
pub fn is_nep413_message_intent_shaped(json: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(json).is_ok_and(|value| {
        value.as_object().is_some_and(|o| {
            o.contains_key("intents")
                || REQUIRED_NEP413_MESSAGE_KEYS
                    .iter()
                    .all(|k| o.contains_key(*k))
        })
    })
}

/// Translate a render-layer failure into this module's error.
///
/// `render_single` reports an envelope whose accounts contradict the resolved
/// network as [`visualsign::errors::VisualSignError::ValidationError`], which no
/// field builder produces, so that variant identifies the mismatch
/// unambiguously.
fn render_error(e: visualsign::errors::VisualSignError) -> NearIntentsError {
    match e {
        visualsign::errors::VisualSignError::ValidationError(mismatch) => {
            NearIntentsError::NetworkMismatch(mismatch)
        }
        other => NearIntentsError::Render(other.to_string()),
    }
}

/// Plain NEP-141 token metadata. The lean baseline chain parser constructs
/// the registry; this module consumes it via `LayeredRegistry<NearTokenRegistry>`.
#[derive(Debug, Default)]
pub struct NearTokenRegistry {
    pub by_asset_id: std::collections::BTreeMap<String, TokenMeta>,
}

/// Where a resolved [`TokenMeta`] came from.
///
/// Distinguishes the two ways a request entry can be unattributed: one carrying
/// no signature, and one whose signature verifies under a key this deployment
/// has not enrolled. Both are worth the same in trust terms -- neither may
/// override a curated seed (see `token_signature::SignerIdentity`) -- and they
/// differ only in what the signer is told, which is why the render needs to tell
/// them apart rather than reading a single bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenProvenance {
    /// The compiled-in `tokens::SEEDS` table.
    Seed,
    /// A request entry signed by a key enrolled for its origin chain.
    RecognizedSigner,
    /// A request entry whose signature verifies under an unenrolled key.
    UnrecognizedSigner,
    /// A request entry carrying no signature.
    Unsigned,
}

impl TokenProvenance {
    /// Whether metadata from this source is trustworthy: it may override a
    /// curated seed, and its amounts render without a provenance caveat.
    #[must_use]
    pub fn verified(self) -> bool {
        matches!(self, Self::Seed | Self::RecognizedSigner)
    }

    /// Names an unattributed source for the signer-facing diagnostic. `Seed` and
    /// `RecognizedSigner` carry no caveat, so they have no phrasing here.
    #[must_use]
    pub(crate) fn unverified_cause(self) -> Option<&'static str> {
        match self {
            Self::Seed | Self::RecognizedSigner => None,
            Self::UnrecognizedSigner => {
                Some("unrecognized signer (signature verified, key not enrolled)")
            }
            Self::Unsigned => Some("unsigned request entry"),
        }
    }

    /// Names an unattributed source in a refusal, where the phrasing is the
    /// subject of "... would override a curated seed".
    ///
    /// Matched exhaustively rather than with a wildcard: only the two
    /// unattributed variants can reach a refusal (the caller guards on
    /// [`Self::verified`]), but a fifth variant should have to name its own
    /// phrasing here rather than silently inherit "unsigned entry" and describe
    /// itself falsely.
    #[must_use]
    pub(crate) fn override_subject(self) -> &'static str {
        match self {
            Self::UnrecognizedSigner => "unrecognized signer",
            Self::Unsigned => "unsigned entry",
            Self::Seed => "compiled-in seed",
            Self::RecognizedSigner => "recognized signer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TokenMeta {
    pub symbol: String,
    pub decimals: u8,
    /// Where this metadata came from. Rendering surfaces an unattributed source
    /// so the signer sees the caveat, not just an operator log.
    pub provenance: TokenProvenance,
}

/// Decode `execute_intents` args and render them as `SignablePayloadField`s.
///
/// This is the sole public entry point for the signed-envelope batch.
pub fn try_decode_execute_intents(
    args: &[u8],
    token_registry: &visualsign::registry::LayeredRegistry<NearTokenRegistry>,
    _options: &visualsign::vsptrait::VisualSignOptions,
    network: crate::network::SettlementNetwork,
) -> Result<Vec<visualsign::SignablePayloadField>, NearIntentsError> {
    let payloads = args::decode_args(args)?;
    let total = payloads.len();
    let mut fields = Vec::new();
    for (i, mp) in payloads.iter().enumerate() {
        fields.extend(
            render::section(i + 1, total, mp, token_registry, network).map_err(render_error)?,
        );
    }
    Ok(fields)
}

/// Whether the pre-signature single-intent envelope's decoded intents include
/// a kind that consults the token registry. The equivalent, for this path, of
/// the on-chain converter's action-level gate: the envelope can name any of
/// eleven intent kinds and only three read the registry, so a caller must
/// check this before building one -- otherwise an envelope that never
/// consults token metadata (e.g. a plain `native_withdraw`) gets a rejection
/// diagnostic for metadata nothing here reads.
///
/// Malformed JSON reads as inapplicable rather than as an error here: the
/// subsequent [`try_render_single_intent`] call decodes the same bytes and
/// reports the parse failure properly.
pub fn single_intent_consumes_token_registry(payload_json: &[u8]) -> bool {
    serde_json::from_slice::<
        defuse_core::payload::DefusePayload<defuse_core::intents::DefuseIntents>,
    >(payload_json)
    .is_ok_and(|payload| {
        payload
            .intents
            .iter()
            .any(render::intent_consumes_token_registry)
    })
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
///
/// Input that is not [intent-shaped](is_intent_shaped) is
/// [`NearIntentsError::InputNotJson`]; input that is, but does not decode as a
/// `DefusePayload`, is [`NearIntentsError::MalformedIntentEnvelope`]. One that
/// decodes but carries an undeclared key, on the envelope or inside an intent, is
/// [`NearIntentsError::UndeclaredFields`] (see [`strict`]).
pub fn try_render_single_intent(
    payload_json: &[u8],
    token_registry: &visualsign::registry::LayeredRegistry<NearTokenRegistry>,
    _options: &visualsign::vsptrait::VisualSignOptions,
    network: crate::network::SettlementNetwork,
) -> Result<RenderedEnvelope, NearIntentsError> {
    // Classified before the typed decode, so JSON that is not a payload (a
    // sign-in challenge naming a `signer_id` and a `deadline`, say) reaches a
    // caller's text fallback instead of being refused as malformed. A payload
    // with every required key and no `intents` is not such JSON: it decodes to
    // an empty list and renders as a cancel.
    if !is_intent_shaped(payload_json) {
        return Err(NearIntentsError::InputNotJson(
            "not a NEAR Intents payload: no top-level `intents` key and not every envelope key"
                .to_string(),
        ));
    }
    let payload: defuse_core::payload::DefusePayload<defuse_core::intents::DefuseIntents> =
        serde_json::from_slice(payload_json)
            .map_err(|e| NearIntentsError::MalformedIntentEnvelope(e.to_string()))?;
    // The typed decode ignores keys it does not declare; a signer would approve
    // them without seeing them.
    strict::require_declared_payload_keys(payload_json, strict::DEFUSE_PAYLOAD_KEYS)
        .map_err(NearIntentsError::UndeclaredFields)?;
    let fields = render::render_single(&payload, token_registry, network).map_err(render_error)?;
    Ok(RenderedEnvelope {
        title: render::title_for_intents(&payload.intents),
        fields,
    })
}

/// Test-only matcher, shared with the chain crates that render these fields.
///
/// Behind a feature rather than `#[cfg(test)]` because a chain crate's tests are
/// a separate compilation: `visualsign-near` asserts on the same diagnostics
/// this crate produces, and a second copy of the matcher could drift from the
/// shape actually emitted.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use visualsign::SignablePayloadField;

    /// Matches the soft-finding field for `rule` in whichever shape the build
    /// emits: the structured `Diagnostic` (diagnostics feature) or the
    /// `Warning`-labelled text fallback.
    pub fn is_warning_diagnostic(field: &SignablePayloadField, rule: &str) -> bool {
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

    /// Render the execute_intents args and return the labels plus the fields'
    /// JSON, so an assertion holds whether diagnostics render as `Diagnostic` or
    /// as the `Warning` text fallback.
    fn render_execute_intents(args: &serde_json::Value) -> (Vec<String>, String) {
        let bytes = serde_json::to_vec(args).unwrap();
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        let fields = try_decode_execute_intents(
            &bytes,
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        )
        .unwrap();
        let labels = fields
            .iter()
            .filter_map(label_of)
            .map(str::to_string)
            .collect();
        (labels, serde_json::to_string(&fields).unwrap())
    }

    /// A signed raw_ed25519 payload whose `DefusePayload` carries an undeclared
    /// key does not render its intents: the key is part of what was signed but
    /// would never show. The extraction diagnostic names it instead.
    #[test]
    fn execute_intents_withholds_a_payload_with_an_undeclared_key() {
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000000000000000000000"}],"memo":"hidden"}"#;
        let (labels, json) = render_execute_intents(&serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]}));
        assert!(
            !labels.iter().any(|l| l == "Intent"),
            "intents rendered: {labels:?}"
        );
        assert!(
            json.contains("does not declare") && json.contains("memo"),
            "{json}"
        );
    }

    /// The same holds for a key inside one intent that its kind does not
    /// declare: `native_withdraw` has no `memo`, so a signed one would never
    /// render.
    #[test]
    fn execute_intents_withholds_an_intent_with_an_undeclared_key() {
        let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"native_withdraw","receiver_id":"bob.near","amount":"1000000000000000000000000","memo":"hidden"}]}"#;
        let (labels, json) = render_execute_intents(&serde_json::json!({"signed":[{
            "standard": "raw_ed25519",
            "payload": inner,
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]}));
        assert!(
            !labels.iter().any(|l| l == "Intent"),
            "intents rendered: {labels:?}"
        );
        assert!(
            json.contains("does not declare") && json.contains("memo"),
            "{json}"
        );
    }

    /// The same holds for a NEP-413-signed payload, whose intents ride inside
    /// the envelope's `message` and have their own key set.
    #[test]
    fn execute_intents_withholds_nep413_intents_with_an_undeclared_key() {
        let message = r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","intents":[],"memo":"hidden"}"#;
        let (labels, json) = render_execute_intents(&serde_json::json!({"signed":[{
            "standard": "nep413",
            "payload": {
                "message": message,
                "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
                "recipient": "intents.near"
            },
            "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
            "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
        }]}));
        assert!(
            !labels.iter().any(|l| l == "Signer"),
            "envelope rendered: {labels:?}"
        );
        assert!(
            json.contains("does not declare") && json.contains("memo"),
            "{json}"
        );
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

        let fields = try_decode_execute_intents(
            &bytes,
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        )
        .unwrap();
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

        let rendered = try_render_single_intent(
            inner.as_bytes(),
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        )
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
        let err = try_render_single_intent(
            b"not json",
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        )
        .err()
        .expect("malformed JSON should error");
        assert!(matches!(err, NearIntentsError::InputNotJson(_)));
    }

    /// A payload that names `intents` but does not decode (an intent kind this
    /// build does not know, or `intents` of the wrong shape) is refused as a
    /// malformed intents payload, not reported as non-JSON: a caller that falls
    /// back to rendering text on `InputNotJson` must not show it as a message.
    /// Plain text and JSON without `intents` keep the `InputNotJson` fallback.
    #[test]
    fn single_intent_classifies_a_malformed_intent_envelope() {
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        let render = |json: &str| {
            try_render_single_intent(
                json.as_bytes(),
                &reg,
                &VisualSignOptions::default(),
                crate::network::SettlementNetwork::Mainnet,
            )
            .err()
            .expect("must not render")
        };
        for malformed in [
            r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"future_unknown"}]}"#,
            r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","intents":null}"#,
            r#"{"intents":[{"intent":"future_unknown"}]}"#,
        ] {
            let err = render(malformed);
            assert!(
                matches!(err, NearIntentsError::MalformedIntentEnvelope(_)),
                "{malformed}: {err:?}"
            );
        }
        for not_intents in [
            "hello world",
            r#"{"greeting":"hi"}"#,
            "[1,2]",
            r#""intents""#,
        ] {
            let err = render(not_intents);
            assert!(
                matches!(err, NearIntentsError::InputNotJson(_)),
                "{not_intents}: {err:?}"
            );
        }
    }

    /// `intents` is optional in a `DefusePayload` (defuse defaults it to an
    /// empty list), and a payload with none is valid: it spends the nonce,
    /// which is how a user cancels a pending signed quote or order. Both the
    /// absent and the empty form must render as a signable cancel showing the
    /// signer, verifying contract, deadline and nonce.
    #[test]
    fn a_payload_without_intents_renders_as_a_cancel() {
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        for message in [
            r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8="}"#,
            r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[]}"#,
        ] {
            assert!(is_intent_shaped(message.as_bytes()), "{message}");
            let rendered = try_render_single_intent(
                message.as_bytes(),
                &reg,
                &VisualSignOptions::default(),
                crate::network::SettlementNetwork::Mainnet,
            )
            .unwrap_or_else(|e| panic!("{message}: {e}"));
            assert_eq!(rendered.title, "Cancel: invalidate nonce", "{message}");
            let labels: Vec<&str> = rendered.fields.iter().filter_map(label_of).collect();
            for expected in [
                "Signer",
                "Verifying Contract",
                "Deadline",
                "Nonce",
                "Intent",
            ] {
                assert!(labels.contains(&expected), "{message}: missing {expected}");
            }
            assert!(
                rendered
                    .fields
                    .iter()
                    .any(|f| super::test_support::is_warning_diagnostic(f, "empty-intents")),
                "{message}: expected the nonce-only warning"
            );
        }
    }

    /// JSON with only some of the envelope keys cannot decode as a payload, so
    /// no verifier accepts a signature over it: it stays on the text fallback.
    #[test]
    fn json_with_only_some_envelope_keys_stays_text() {
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        for message in [
            r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8="}"#,
            r#"{"signer_id":"alice.near","action":"login"}"#,
        ] {
            let result = try_render_single_intent(
                message.as_bytes(),
                &reg,
                &VisualSignOptions::default(),
                crate::network::SettlementNetwork::Mainnet,
            );
            assert!(
                matches!(result, Err(NearIntentsError::InputNotJson(_))),
                "{message}: {:?}",
                result.map(|rendered| rendered.title)
            );
        }
    }

    /// A `DefusePayload` with an undeclared top-level key is refused with its
    /// own error, so a caller that renders non-intents JSON as text cannot fall
    /// back to that here and show the intents partly.
    #[test]
    fn single_intent_with_an_undeclared_key_is_refused() {
        let payload = br#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[],"memo":"hidden"}"#;
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        let Err(NearIntentsError::UndeclaredFields(message)) = try_render_single_intent(
            payload,
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        ) else {
            panic!("expected UndeclaredFields");
        };
        assert!(message.contains("\"memo\""), "{message}");
    }

    /// The single-intent path holds each intent to its kind's keys too.
    #[test]
    fn single_intent_with_an_undeclared_key_inside_an_intent_is_refused() {
        let payload = br#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"native_withdraw","receiver_id":"bob.near","amount":"1","memo":"hidden"}]}"#;
        let reg = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
        let Err(NearIntentsError::UndeclaredFields(message)) = try_render_single_intent(
            payload,
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        ) else {
            panic!("expected UndeclaredFields");
        };
        assert!(message.contains("\"memo\""), "{message}");
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
        let fields = try_decode_execute_intents(
            &bytes,
            &reg,
            &VisualSignOptions::default(),
            crate::network::SettlementNetwork::Mainnet,
        )
        .unwrap();
        let has_deadline_warning = fields
            .iter()
            .any(|f| super::test_support::is_warning_diagnostic(f, "deadline"));
        assert!(has_deadline_warning, "expected a deadline warning");
    }
}
