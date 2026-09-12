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
//! transaction and any other input is tried as an envelope. The two JSON
//! envelopes are disjoint by required field: a `DefusePayload` carries
//! `signer_id`, `verifying_contract` and `deadline`, none of which NEP-413
//! declares, and a NEP-413 payload carries `recipient`, which a `DefusePayload`
//! does not.
//!
//! A raw message is accepted only when its content is recognized -- today, a
//! `DefusePayload`. Unrecognized bytes are rejected rather than rendered
//! opaquely, so nothing the parser cannot read reaches a signer under an
//! attestation. NEP-413 differs because its envelope is itself structured: the
//! recipient and nonce are read even when the message it carries is free text.

use near_primitives::transaction::{SignedTransaction, Transaction};
use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::{DeveloperConfig, TransactionParseError};

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
                Ok(unsigned) => return Ok(Self::OnChain(unsigned)),
                Err(e) => e,
            };
            let allow_signed = developer_config
                .map(|c| c.allow_signed_transactions)
                .unwrap_or(false);
            if allow_signed {
                match borsh::from_slice::<SignedTransaction>(&bytes) {
                    Ok(signed) => {
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
        let intent_err = match serde_json::from_str::<
            defuse_core::payload::DefusePayload<defuse_core::intents::DefuseIntents>,
        >(trimmed)
        {
            Ok(_) => return Ok(Self::RawMessage(trimmed.to_string())),
            Err(e) => e,
        };

        // NEP-413 is tried second so input that decodes as a `DefusePayload`
        // keeps decoding as one. The envelopes are disjoint by required field,
        // so the order decides only which causes appear when input is neither.
        let nep413_err = match serde_json::from_str::<defuse_nep413::Nep413Payload>(trimmed) {
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
    use near_crypto::{KeyType, Signature};
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
        let near_primitives::action::Action::Transfer(transfer) = &inner.actions()[0] else {
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
