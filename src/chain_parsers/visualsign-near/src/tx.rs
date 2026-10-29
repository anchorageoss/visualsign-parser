//! NEAR input decoding. The three accepted inputs are envelopes -- what a
//! signature covers -- rather than applications of them:
//!
//! - a borsh transaction, signed over `sha256(borsh(tx))`;
//! - a NEP-413 off-chain message, signed over `sha256(borsh(tag || payload))`;
//! - a raw message, signed over its own bytes.
//!
//! NEAR Intents is content rather than a fourth envelope. A `DefusePayload`
//! rides in a raw message under the `RawEd25519` standard and inside `message`
//! under NEP-413, and rendering recognizes it in either, so the envelope decides
//! what is signed and the content decides what is shown. NEP-413 is NEAR's own
//! message-signing standard; the verifier documents `RawEd25519` as Phantom's
//! convention for Solana wallets. Both reach the same verifier, so both decode
//! here, and a NEAR wallet signing for a NEAR account produces NEP-413.
//!
//! Borsh bytes are never valid JSON, so a successful borsh decode is a
//! transaction and any other input is tried as an envelope. Each JSON envelope
//! is decoded strictly: every top-level key must be one its type declares. The
//! typed decoders alone ignore unknown keys, so without this an object holding
//! both envelopes' fields would decode as a `DefusePayload` while a NEP-413
//! wallet signs the borsh envelope instead, and an extra key (or a misspelled
//! one, such as `callback_url` for `callbackUrl`) would be signed without being
//! shown. Strict key sets also make the two envelopes disjoint for every
//! input, not only for well-formed ones: a `DefusePayload` declares
//! `signer_id`, `verifying_contract` and `deadline`, which NEP-413 does not, and
//! NEP-413 declares `message` and `recipient`, which a `DefusePayload` does not.
//! The intents a NEP-413 `message` carries are held to the same rule.
//!
//! A raw message is accepted only when its content is recognized -- today, a
//! `DefusePayload`. Unrecognized bytes are rejected rather than rendered
//! opaquely, so nothing the parser cannot read reaches a signer under an
//! attestation. NEP-413 differs because its envelope is itself structured: the
//! recipient and nonce are read even when the message it carries is free text.

use crate::primitives::{SignedTransaction, Transaction};
use defuse_core::intents::DefuseIntents;
use defuse_core::payload::DefusePayload;
use defuse_core::payload::ExtractDefusePayload;
use defuse_nep413::Nep413Payload;
use serde::de::DeserializeOwned;
use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::{DeveloperConfig, TransactionParseError};
use visualsign_intents::strict;

/// A NEAR input: an on-chain transaction, or a pre-signature intents envelope.
#[derive(Debug, Clone)]
pub enum NearTransaction {
    /// A borsh-decoded NEAR transaction (`near::sign_transaction`).
    OnChain(Transaction),
    /// A raw message, signed over its own bytes -- the `RawEd25519` standard,
    /// which the verifier documents as Phantom's convention for Solana wallets.
    /// NEP-413 is NEAR's own message-signing standard, so a NEAR wallet signing
    /// for a NEAR account produces [`Self::Nep413`] instead. The verifier
    /// accepts either from an ed25519 key, so both are decoded.
    ///
    /// Kept as the validated raw text: the signature covers the string itself
    /// rather than a digest of a re-serialization, so the bytes rendering reads
    /// have to be the bytes that get signed.
    RawMessage(String),
    /// A NEP-413 off-chain message envelope, kept as the validated raw text --
    /// rendering re-parses it. NEP-413 wraps an arbitrary `message`, so this
    /// envelope carries a signing request for any purpose, of which NEAR Intents
    /// is one.
    Nep413(String),
}

impl NearTransaction {
    /// Decode unsigned first. Only when `developer_config.allow_signed_transactions`
    /// is set does a `SignedTransaction` envelope get accepted, with its signature
    /// discarded to render the inner unsigned transaction -- production callers
    /// must pass `None`.
    pub fn from_string_with_options(
        s: &str,
        developer_config: Option<&DeveloperConfig>,
    ) -> Result<Self, TransactionParseError> {
        let trimmed = s.trim();
        let mut borsh_failure: Option<String> = None;
        if let Ok(bytes) = decode_input(trimmed) {
            let unsigned_err = match borsh::from_slice::<Transaction>(&bytes) {
                Ok(unsigned) => {
                    require_canonical(&unsigned, &bytes)?;
                    return Ok(Self::OnChain(unsigned));
                }
                Err(e) => e,
            };
            let allow_signed = developer_config
                .map(|c| c.allow_signed_transactions)
                .unwrap_or(false);
            if allow_signed {
                match borsh::from_slice::<SignedTransaction>(&bytes) {
                    Ok(signed) => {
                        require_canonical(&signed, &bytes)?;
                        // Developer-only posture: production callers pass `None`, so
                        // reaching here in production means a misconfiguration and
                        // must leave a trail.
                        tracing::warn!(
                            "accepted a signed NEAR transaction and discarded its signature; \
                             allow_signed_transactions is a developer-only setting"
                        );
                        return Ok(Self::OnChain(signed.transaction));
                    }
                    Err(signed_err) => {
                        borsh_failure =
                            Some(format!("unsigned={unsigned_err}, signed={signed_err}"));
                    }
                }
            } else {
                borsh_failure = Some(unsigned_err.to_string());
            }
        }
        // Validate eagerly so malformed input is rejected at parse time
        // rather than at render time.
        // The typed decode needs every required envelope key and defaults a
        // missing `intents` to an empty list: such a payload only spends the
        // nonce (a cancel) and is a raw intents message like any other. A failed
        // decode keeps serde's own cause. Each intent is held to its own kind's
        // keys as well as the envelope.
        let intent_err = match serde_json::from_str::<DefusePayload<DefuseIntents>>(trimmed)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                strict::require_declared_payload_keys(
                    trimmed.as_bytes(),
                    strict::DEFUSE_PAYLOAD_KEYS,
                )
            }) {
            Ok(()) if visualsign_intents::is_intent_shaped(trimmed.as_bytes()) => {
                return Ok(Self::RawMessage(trimmed.to_string()));
            }
            Ok(()) => "not a DefusePayload: not intent-shaped".to_string(),
            Err(e) => e,
        };

        // NEP-413 is tried second. With strict key sets the envelopes are
        // disjoint for every input, so the order decides only which causes
        // appear when input is neither.
        let nep413_err = match decode_nep413(trimmed) {
            Ok(_) => return Ok(Self::Nep413(trimmed.to_string())),
            Err(e) => e,
        };

        // The causes are appended rather than interpolated into the summary, so
        // the sentence naming the accepted formats stays contiguous for callers
        // that match on it.
        let borsh_cause = borsh_failure
            .as_deref()
            .unwrap_or("input is not hex or base64");
        Err(TransactionParseError::DecodeError(format!(
            "input is neither a NEAR borsh transaction, a DefusePayload JSON envelope, nor a \
             NEP-413 message envelope: {intent_err}; nep413 decode: {nep413_err}; near borsh \
             decode: {borsh_cause}"
        )))
    }
}

/// Decode `json` as `T`, then refuse any top-level key outside `allowed`.
///
/// The typed decode runs first, so a malformed envelope reports serde's own
/// cause (including a repeated struct field, which serde refuses; a repeated
/// key inside a map-valued field such as `token_diff.diff` or
/// `transfer.tokens` keeps its last value, as the contract does). The key
/// check, shared
/// with every chain that renders intents, then catches what serde would
/// silently drop.
fn strict_decode<T: DeserializeOwned>(json: &str, allowed: &[&str]) -> Result<T, String> {
    let value = serde_json::from_str::<T>(json).map_err(|e| e.to_string())?;
    strict::require_declared_keys(json.as_bytes(), allowed)?;
    Ok(value)
}

/// Decode a NEP-413 envelope under every rule [`NearTransaction::Nep413`]
/// promises: only the keys NEP-413 declares, and a `message` that is either an
/// intents request held to its own key set or free text. Returns the intents
/// the message carries, or `None` when it is free text.
///
/// Shared by string decoding and the converter, because the enum is public: a
/// caller can build `NearTransaction::Nep413` without going through
/// [`NearTransaction::from_string_with_options`], and the rules have to hold for
/// that envelope too.
pub(crate) fn decode_nep413(
    json: &str,
) -> Result<(Nep413Payload, Option<DefusePayload<DefuseIntents>>), String> {
    let payload = strict_decode::<Nep413Payload>(json, strict::NEP413_KEYS)?;
    let intents = nep413_intents(&payload)?;
    Ok((payload, intents))
}

/// The intents a NEP-413 `message` carries, held to the same rule as a raw
/// `DefusePayload`: every key must be one the decoder reads. A message that is
/// not [intent-shaped](visualsign_intents::is_nep413_message_intent_shaped) is
/// free text and renders as such.
///
/// The message is classified before extraction by its keys: an `intents` key,
/// or both `signer_id` and `deadline`. The second form with no `intents`
/// extracts as a payload that only spends the nonce, which is how a pending
/// signed request is cancelled, so it renders as a cancel rather than as text;
/// with a key outside the declared set it is refused, like any intents
/// message. A message that is intent-shaped but does not extract (an intent kind
/// this build does not know, `intents: null`) is refused rather than treated as
/// free text: showing it as a plain message would put raw JSON in front of a
/// signer for an intent the parser could not interpret.
fn nep413_intents(payload: &Nep413Payload) -> Result<Option<DefusePayload<DefuseIntents>>, String> {
    if !visualsign_intents::is_nep413_message_intent_shaped(payload.message.as_bytes()) {
        return Ok(None);
    }
    let intents = ExtractDefusePayload::<DefuseIntents>::extract_defuse_payload(payload.clone())
        .map_err(|e| {
            format!(
                "this NEP-413 message is a NEAR Intents payload that is malformed or \
                 unsupported: {e}"
            )
        })?;
    strict::require_declared_payload_keys(
        payload.message.as_bytes(),
        strict::NEP413_INTENTS_MESSAGE_KEYS,
    )
    .map_err(|e| format!("the intents in this NEP-413 message carry {e}"))?;
    Ok(Some(intents))
}

impl visualsign::vsptrait::Transaction for NearTransaction {
    fn from_string(s: &str) -> Result<Self, TransactionParseError> {
        Self::from_string_with_options(s, None)
    }

    fn transaction_type(&self) -> String {
        match self {
            Self::OnChain(_) => "NEAR".to_string(),
            Self::RawMessage(_) => "NEAR Intent".to_string(),
            Self::Nep413(_) => "NEAR Message".to_string(),
        }
    }
}

/// Refuses input whose bytes are not the canonical borsh encoding of what was
/// decoded. Borsh accepts map entries in any order and with duplicate keys,
/// then re-encodes them sorted and deduplicated. NEAR nodes hash the
/// re-encoded form, so a signature over the input bytes would not verify.
fn require_canonical<T: borsh::BorshSerialize>(
    decoded: &T,
    input: &[u8],
) -> Result<(), TransactionParseError> {
    let encoded = borsh::to_vec(decoded)
        .map_err(|e| TransactionParseError::DecodeError(format!("borsh re-encode: {e}")))?;
    if encoded != input {
        let offset = encoded
            .iter()
            .zip(input)
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| encoded.len().min(input.len()));
        let lengths = if encoded.len() == input.len() {
            format!("both are {} bytes", input.len())
        } else {
            format!(
                "input is {} bytes, canonical encoding is {} bytes",
                input.len(),
                encoded.len()
            )
        };
        return Err(TransactionParseError::DecodeError(format!(
            "non-canonical NEAR transaction: input differs from its canonical encoding \
             at byte offset {offset} ({lengths}; unsorted or duplicate map keys re-encode differently)"
        )));
    }
    Ok(())
}

fn decode_input(s: &str) -> Result<Vec<u8>, TransactionParseError> {
    match SupportedEncodings::detect(s) {
        SupportedEncodings::Hex => visualsign::encodings::decode_hex(s)
            .map_err(|e| TransactionParseError::DecodeError(format!("hex: {e}"))),
        SupportedEncodings::Base64 => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(|e| TransactionParseError::DecodeError(format!("base64: {e}")))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::primitives::{KeyType, Signature};
    use visualsign::vsptrait::Transaction as _;

    /// Borsh-encoded unsigned NEAR Transfer: alice.near -> bob.near, 1 NEAR.
    const TRANSFER_HEX: &str = "0a000000616c6963652e6e656172000000000000000000000000000000000000000000000000000000000000000000010000000000000008000000626f622e6e65617200000000000000000000000000000000000000000000000000000000000000000100000003000000a1edccce1bc2d3000000000000";

    const SWAP_INTENT: &str = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2100-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000000000000000000000"}]}"#;

    fn onchain(tx: &NearTransaction) -> &Transaction {
        match tx {
            NearTransaction::OnChain(inner) => inner,
            other => panic!("expected OnChain, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_garbage() {
        let result = NearTransaction::from_string("not-hex-not-base64-not-json");
        assert!(result.is_err());
    }

    #[test]
    fn decode_hex_transfer() {
        let tx = NearTransaction::from_string(TRANSFER_HEX).expect("decode hex");
        let inner = onchain(&tx);
        assert_eq!(inner.signer_id().as_str(), "alice.near");
        assert_eq!(inner.receiver_id().as_str(), "bob.near");
        assert_eq!(inner.actions().len(), 1);
        let crate::primitives::Action::Transfer(transfer) = &inner.actions()[0] else {
            panic!("expected Transfer");
        };
        assert_eq!(
            transfer.deposit.as_yoctonear(),
            1_000_000_000_000_000_000_000_000
        );
    }

    /// `borsh::from_slice` fails unless the whole buffer is consumed, so bytes
    /// appended after a valid transaction cannot be silently dropped from the
    /// render while remaining in what gets signed.
    #[test]
    fn decode_rejects_trailing_bytes_after_a_valid_transaction() {
        let result = NearTransaction::from_string(&format!("{TRANSFER_HEX}00"));
        let Err(TransactionParseError::DecodeError(message)) = result else {
            panic!("expected a DecodeError for trailing bytes");
        };
        assert!(message.contains("Not all bytes read"), "message: {message}");
    }

    #[test]
    fn decode_hex_with_0x_prefix() {
        let tx = NearTransaction::from_string(&format!("0x{TRANSFER_HEX}")).expect("decode 0x-hex");
        assert_eq!(onchain(&tx).signer_id().as_str(), "alice.near");
    }

    #[test]
    fn decode_base64_matches_hex() {
        use base64::Engine;
        let bytes = hex::decode(TRANSFER_HEX).expect("hex");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let tx = NearTransaction::from_string(&b64).expect("decode base64");
        let inner = onchain(&tx);
        assert_eq!(inner.signer_id().as_str(), "alice.near");
        assert_eq!(inner.receiver_id().as_str(), "bob.near");
    }

    fn signed_transfer_bytes() -> Vec<u8> {
        let unsigned: Transaction =
            borsh::from_slice(&visualsign::encodings::decode_hex(TRANSFER_HEX).expect("hex"))
                .expect("decode unsigned");
        let signed = SignedTransaction::new(Signature::empty(KeyType::ED25519), unsigned);
        borsh::to_vec(&signed).expect("borsh encode")
    }

    #[test]
    fn signed_transaction_rejected_by_default() {
        let bytes = signed_transfer_bytes();
        let hex = hex::encode(bytes);
        let result = NearTransaction::from_string_with_options(&hex, None);
        assert!(result.is_err());
    }

    #[test]
    fn signed_transaction_accepted_when_developer_config_allows_it() {
        let bytes = signed_transfer_bytes();
        let hex = hex::encode(bytes);
        let developer_config = DeveloperConfig {
            allow_signed_transactions: true,
        };
        let tx = NearTransaction::from_string_with_options(&hex, Some(&developer_config))
            .expect("decode signed");
        assert_eq!(onchain(&tx).signer_id().as_str(), "alice.near");
    }

    /// V0 transaction alice.near -> bob.near with one `DeterministicStateInit`
    /// whose data map holds the given entries, encoded in the given order.
    fn state_init_hex(entries: &[(&str, &str)]) -> String {
        let mut s = String::from(
            "0a000000616c6963652e6e656172002222222222222222222222222222222222222222222222222222222222222222\
             010000000000000008000000626f622e6e6561723333333333333333333333333333333333333333333333333333333333333333\
             010000000b00001111111111111111111111111111111111111111111111111111111111111111",
        );
        s.push_str(&hex::encode((entries.len() as u32).to_le_bytes()));
        for (k, v) in entries {
            s.push_str(&format!(
                "01000000{}01000000{}",
                hex::encode(k),
                hex::encode(v)
            ));
        }
        s.push_str("00000000000000000000000000000000");
        s
    }

    #[test]
    fn state_init_canonical_map_is_accepted() {
        let hex = state_init_hex(&[("a", "1"), ("b", "2")]);
        NearTransaction::from_string(&hex).expect("canonical map decodes");
    }

    #[test]
    fn state_init_unsorted_map_is_rejected() {
        let hex = state_init_hex(&[("b", "2"), ("a", "1")]);
        let Err(TransactionParseError::DecodeError(message)) = NearTransaction::from_string(&hex)
        else {
            panic!("expected a DecodeError for an unsorted map");
        };
        assert!(message.contains("non-canonical"), "message: {message}");
        let input = hex::decode(&hex).expect("hex");
        let canonical = hex::decode(state_init_hex(&[("a", "1"), ("b", "2")])).expect("hex");
        let offset = input
            .iter()
            .zip(&canonical)
            .position(|(a, b)| a != b)
            .expect("payloads differ");
        assert!(
            message.contains(&format!(
                "byte offset {offset} (both are {} bytes",
                input.len()
            )),
            "message: {message}"
        );
    }

    #[test]
    fn state_init_duplicate_key_is_rejected() {
        let hex = state_init_hex(&[("a", "1"), ("a", "9")]);
        let Err(TransactionParseError::DecodeError(message)) = NearTransaction::from_string(&hex)
        else {
            panic!("expected a DecodeError for a duplicate key");
        };
        assert!(message.contains("non-canonical"), "message: {message}");
        // Borsh keeps the last value of a repeated key, so the canonical form has
        // one entry and the first difference is the entry count.
        let input = hex::decode(&hex).expect("hex");
        let canonical = hex::decode(state_init_hex(&[("a", "9")])).expect("hex");
        let offset = input
            .iter()
            .zip(&canonical)
            .position(|(a, b)| a != b)
            .expect("payloads differ");
        assert_eq!(offset, 138);
        assert_eq!((input.len(), canonical.len()), (178, 168));
        assert!(
            message.contains(&format!(
                "byte offset {offset} (input is {} bytes, canonical encoding is {} bytes",
                input.len(),
                canonical.len()
            )),
            "message: {message}"
        );
    }

    #[test]
    fn decode_json_envelope_is_intent() {
        let tx = NearTransaction::from_string(SWAP_INTENT).expect("decode intent");
        match tx {
            NearTransaction::RawMessage(json) => assert_eq!(json, SWAP_INTENT),
            other => panic!("expected Intent, got {other:?}"),
        }
        assert_eq!(
            NearTransaction::from_string(SWAP_INTENT)
                .expect("decode intent")
                .transaction_type(),
            "NEAR Intent"
        );
    }

    #[test]
    fn decode_rejects_malformed_json() {
        // Valid JSON, but not a DefusePayload shape.
        let result = NearTransaction::from_string(r#"{"foo":"bar"}"#);
        assert!(result.is_err());
    }

    /// A NEP-413 envelope carrying a plain message. `recipient` is what the
    /// signature binds the message to; it is not an account id in the general
    /// case, since NEP-413 is used for web sign-in where it is a domain.
    const PLAIN_NEP413: &str = r#"{"message":"Sign in to app.example.com","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","recipient":"app.example.com"}"#;

    /// A sign-in message whose keys overlap an intents request's (`signer_id`,
    /// `deadline`) but that names no `intents`.
    const OVERLAPPING_SIGN_IN: &str =
        r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","action":"login"}"#;

    /// The same intents content as `SWAP_INTENT` in a different envelope:
    /// `signer_id` and `deadline` sit inside `message`, and
    /// `verifying_contract` comes from the envelope's `recipient`.
    const NEP413_FRAMED_INTENT: &str = r#"{"message":"{\"signer_id\":\"alice.near\",\"deadline\":\"2999-01-01T00:00:00Z\",\"intents\":[{\"intent\":\"ft_withdraw\",\"token\":\"wrap.near\",\"receiver_id\":\"bob.near\",\"amount\":\"1000000\"}]}","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","recipient":"intents.near"}"#;

    #[test]
    fn decode_nep413_envelope_is_a_message() {
        let tx = NearTransaction::from_string(PLAIN_NEP413).expect("decode nep413");
        match &tx {
            NearTransaction::Nep413(json) => assert_eq!(json, PLAIN_NEP413),
            other => panic!("expected Nep413, got {other:?}"),
        }
        assert_eq!(tx.transaction_type(), "NEAR Message");
    }

    /// The decoder tries `DefusePayload` before NEP-413, so the order is
    /// immaterial only if neither format parses as the other. Asserted at the
    /// serde layer rather than through the decoder, because it is serde's
    /// required-field sets that make the ordering safe.
    #[test]
    fn the_two_json_envelopes_are_disjoint() {
        assert!(
            serde_json::from_str::<defuse_nep413::Nep413Payload>(SWAP_INTENT).is_err(),
            "a DefusePayload must not parse as a NEP-413 payload"
        );
        assert!(
            serde_json::from_str::<
                defuse_core::payload::DefusePayload<defuse_core::intents::DefuseIntents>,
            >(PLAIN_NEP413)
            .is_err(),
            "a NEP-413 payload must not parse as a DefusePayload"
        );
    }

    /// A NEP-413 envelope carrying intents stays a NEP-413 envelope. The
    /// envelope decides what the signature covers -- `sha256(borsh(tag ||
    /// payload))` here, the message's own bytes for a raw message -- so
    /// decoding this as a raw message would attest a digest the signer never
    /// produces.
    #[test]
    fn a_nep413_envelope_carrying_intents_stays_an_envelope() {
        let tx = NearTransaction::from_string(NEP413_FRAMED_INTENT).expect("decode");
        match &tx {
            NearTransaction::Nep413(json) => assert_eq!(json, NEP413_FRAMED_INTENT),
            other => panic!("expected Nep413, got {other:?}"),
        }
        assert_eq!(tx.transaction_type(), "NEAR Message");
    }

    #[test]
    fn decode_error_names_all_three_accepted_formats() {
        let Err(TransactionParseError::DecodeError(message)) =
            NearTransaction::from_string(r#"{"foo":"bar"}"#)
        else {
            panic!("expected a DecodeError");
        };
        for expected in ["borsh transaction", "DefusePayload", "NEP-413"] {
            assert!(
                message.contains(expected),
                "the refusal must name {expected}: {message}"
            );
        }
    }

    /// Merge `extra` into the JSON object `base`, as a caller could.
    fn with_fields(base: &str, extra: serde_json::Value) -> String {
        let mut value: serde_json::Value = serde_json::from_str(base).expect("base json");
        let object = value.as_object_mut().expect("base object");
        for (k, v) in extra.as_object().expect("extra object") {
            object.insert(k.clone(), v.clone());
        }
        value.to_string()
    }

    fn decode_error(input: &str) -> String {
        match NearTransaction::from_string(input) {
            Err(TransactionParseError::DecodeError(message)) => message,
            other => panic!("expected a DecodeError, got {other:?}"),
        }
    }

    /// An object holding both envelopes' fields decodes as neither. Before
    /// strict key sets it decoded as a raw `DefusePayload`, while a NEP-413
    /// wallet would sign the borsh envelope instead: the render and the
    /// signature would describe different bytes.
    #[test]
    fn an_object_with_both_envelopes_fields_is_refused() {
        let both = with_fields(
            SWAP_INTENT,
            serde_json::json!({ "message": "hello", "recipient": "intents.near" }),
        );
        let message = decode_error(&both);
        assert!(message.contains("does not declare"), "{message}");
        for expected in ["borsh transaction", "DefusePayload", "NEP-413"] {
            assert!(
                message.contains(expected),
                "the refusal must name {expected}: {message}"
            );
        }
    }

    /// A key the decoder does not read is signed without being shown, so it is
    /// refused on either envelope and named in the refusal.
    #[test]
    fn an_undeclared_key_is_refused_on_either_envelope() {
        for base in [SWAP_INTENT, PLAIN_NEP413] {
            let message = decode_error(&with_fields(base, serde_json::json!({ "extra": 1 })));
            assert!(
                message.contains("\"extra\""),
                "the refusal must name the key: {message}"
            );
        }
    }

    /// NEP-413's JSON form is camelCase. A snake_case `callback_url` would be
    /// dropped by the typed decoder, so the signer would never see the URL
    /// their wallet binds; it is refused by name instead.
    #[test]
    fn a_snake_case_callback_url_is_refused() {
        let message = decode_error(&with_fields(
            PLAIN_NEP413,
            serde_json::json!({ "callback_url": "https://evil.example" }),
        ));
        assert!(message.contains("\"callback_url\""), "{message}");
        let accepted = with_fields(
            PLAIN_NEP413,
            serde_json::json!({ "callbackUrl": "https://app.example.com" }),
        );
        assert!(matches!(
            NearTransaction::from_string(&accepted),
            Ok(NearTransaction::Nep413(_))
        ));
    }

    /// The intents a NEP-413 message carries are held to the same rule as a
    /// raw `DefusePayload`: an undeclared field inside them is refused.
    #[test]
    fn nep413_intents_with_an_undeclared_field_are_refused() {
        let inner = r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","intents":[],"extra":1}"#;
        let envelope = serde_json::json!({
            "message": inner,
            "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
            "recipient": "intents.near",
        })
        .to_string();
        let message = decode_error(&envelope);
        assert!(message.contains("\"extra\""), "{message}");
    }

    /// A NEP-413 message that is intent-shaped (a JSON object naming `intents`)
    /// but does not decode is refused rather than accepted as free text: an
    /// unknown intent kind and `intents: null` both.
    #[test]
    fn nep413_intent_shaped_message_that_does_not_decode_is_refused() {
        for inner in [
            r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","intents":[{"intent":"future_unknown"}]}"#,
            r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","intents":null}"#,
        ] {
            let envelope = serde_json::json!({
                "message": inner,
                "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
                "recipient": "intents.near",
            })
            .to_string();
            let message = decode_error(&envelope);
            assert!(message.contains("malformed or unsupported"), "{message}");
        }
    }

    /// A NEP-413 message that is JSON but not an intents request is free text:
    /// other applications sign structured messages too, and those keep
    /// rendering as the text they are.
    #[test]
    fn a_nep413_message_that_is_json_but_not_intents_still_decodes() {
        let envelope = serde_json::json!({
            "message": r#"{"app":"example","action":"login","extra":true}"#,
            "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
            "recipient": "app.example.com",
        })
        .to_string();
        assert!(matches!(
            NearTransaction::from_string(&envelope),
            Ok(NearTransaction::Nep413(_))
        ));
    }

    /// A key inside an intent that its kind does not declare is refused on
    /// both envelopes: `native_withdraw` has no `memo`, so the typed decode
    /// would drop it and the signer would never see it.
    #[test]
    fn an_undeclared_key_inside_an_intent_is_refused_on_either_envelope() {
        let intent =
            r#"{"intent":"native_withdraw","receiver_id":"bob.near","amount":"1","memo":"hidden"}"#;
        let raw = format!(
            r#"{{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{intent}]}}"#
        );
        let nep413 = serde_json::json!({
            "message": format!(
                r#"{{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z","intents":[{intent}]}}"#
            ),
            "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
            "recipient": "intents.near",
        })
        .to_string();
        for input in [raw, nep413] {
            let message = decode_error(&input);
            assert!(
                message.contains("\"memo\"") && message.contains("native_withdraw"),
                "{message}"
            );
        }
    }

    /// A message with an intents request's `signer_id` and `deadline` extracts
    /// as a payload even with no `intents`: one that only spends the nonce (a
    /// cancel), so it is intents rather than free text. With a key the intents
    /// message does not declare (`action`), it would be signed without that key
    /// being shown, so it is refused rather than shown as text.
    #[test]
    fn a_nep413_message_with_the_intents_keys_is_intents_without_an_intents_list() {
        let envelope = |message: &str| {
            serde_json::json!({
                "message": message,
                "nonce": "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=",
                "recipient": "intents.near",
            })
            .to_string()
        };
        let cancel = envelope(r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z"}"#);
        let (_, intents) = decode_nep413(&cancel).expect("a cancel decodes");
        assert!(intents.is_some_and(|p| p.intents.is_empty()));

        let message = decode_error(&envelope(OVERLAPPING_SIGN_IN));
        assert!(message.contains("\"action\""), "{message}");

        // Without both keys it is free text.
        let free = envelope(r#"{"signer_id":"alice.near","action":"login"}"#);
        assert!(matches!(decode_nep413(&free), Ok((_, None))));
    }

    /// `intents` is optional in a `DefusePayload`: one with every envelope key
    /// and no `intents` only spends its nonce, which is how a pending signed
    /// request is cancelled, so it decodes as a raw intents message. JSON with
    /// only some of the envelope keys is neither envelope.
    #[test]
    fn a_raw_defuse_payload_without_intents_is_a_raw_message() {
        let without_intents = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8="}"#;
        assert!(matches!(
            NearTransaction::from_string(without_intents),
            Ok(NearTransaction::RawMessage(_))
        ));
        let partial = r#"{"signer_id":"alice.near","deadline":"2999-01-01T00:00:00Z"}"#;
        let message = decode_error(partial);
        assert!(message.contains("DefusePayload"), "{message}");
    }

    /// A key that appears twice has no single value to render, so either
    /// envelope refuses it rather than keeping whichever value the parser saw
    /// last.
    #[test]
    fn a_repeated_key_is_refused_on_either_envelope() {
        let defuse = r#"{"signer_id":"alice.near","signer_id":"mallory.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[]}"#;
        let nep413 = r#"{"message":"hi","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","recipient":"app.example.com","recipient":"evil.example"}"#;
        for input in [defuse, nep413] {
            assert!(
                NearTransaction::from_string(input).is_err(),
                "accepted: {input}"
            );
        }
    }

    /// `recipient` is what a NEP-413 signature binds the message to, so an
    /// envelope without one is rejected rather than rendered with the field
    /// blank.
    #[test]
    fn decode_rejects_a_nep413_envelope_without_a_recipient() {
        let result = NearTransaction::from_string(
            r#"{"message":"hello","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8="}"#,
        );
        assert!(result.is_err());
    }
}
