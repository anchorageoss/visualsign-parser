//! NEAR input decoding: a borsh transaction, or a pre-signature NEAR Intents
//! envelope (`DefusePayload` JSON, the `near::sign_intent` payload).
//!
//! Borsh bytes are never valid JSON, so the two formats are distinguished
//! unambiguously: a successful borsh decode is a transaction, otherwise the
//! input is validated as an envelope. Input that is neither is rejected.

use near_primitives::transaction::{SignedTransaction, Transaction};
use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::{DeveloperConfig, TransactionParseError};

/// A NEAR input: an on-chain transaction, or a pre-signature intents envelope.
#[derive(Debug, Clone)]
pub enum NearTransaction {
    /// A borsh-decoded NEAR transaction (`near::sign_transaction`).
    OnChain(Transaction),
    /// A pre-signature `DefusePayload` JSON envelope (`near::sign_intent`),
    /// kept as the validated raw text -- rendering re-parses it.
    Intent(String),
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
        serde_json::from_str::<
            defuse_core::payload::DefusePayload<defuse_core::intents::DefuseIntents>,
        >(trimmed)
        .map_err(|e| {
            // Both causes are appended rather than interpolated into the
            // summary, so the sentence naming the two accepted formats stays
            // contiguous for callers that match on it.
            let borsh_cause = borsh_failure
                .as_deref()
                .unwrap_or("input is not hex or base64");
            TransactionParseError::DecodeError(format!(
                "input is neither a NEAR borsh transaction nor a DefusePayload JSON envelope: \
                 {e}; near borsh decode: {borsh_cause}"
            ))
        })?;
        Ok(Self::Intent(trimmed.to_string()))
    }
}

impl visualsign::vsptrait::Transaction for NearTransaction {
    fn from_string(s: &str) -> Result<Self, TransactionParseError> {
        Self::from_string_with_options(s, None)
    }

    fn transaction_type(&self) -> String {
        match self {
            Self::OnChain(_) => "NEAR".to_string(),
            Self::Intent(_) => "NEAR Intent".to_string(),
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
            NearTransaction::Intent(_) => panic!("expected OnChain"),
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
            NearTransaction::Intent(json) => assert_eq!(json, SWAP_INTENT),
            NearTransaction::OnChain(_) => panic!("expected Intent"),
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
}
