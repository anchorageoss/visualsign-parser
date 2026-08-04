//! NEAR transaction envelope decoding.

use near_primitives::transaction::{SignedTransaction, Transaction};
use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::{DeveloperConfig, TransactionParseError};

/// VSP wrapper over `near-primitives` Transaction. Owns the borsh decoding logic.
#[derive(Debug, Clone)]
pub struct NearTransaction {
    inner: Transaction,
}

impl NearTransaction {
    #[must_use]
    pub fn new(inner: Transaction) -> Self {
        Self { inner }
    }

    #[must_use]
    pub fn inner(&self) -> &Transaction {
        &self.inner
    }

    /// Decode unsigned first. Only when `developer_config.allow_signed_transactions`
    /// is set does a `SignedTransaction` envelope get accepted, with its signature
    /// discarded to render the inner unsigned transaction -- production callers
    /// must pass `None`.
    pub fn from_string_with_options(
        s: &str,
        developer_config: Option<&DeveloperConfig>,
    ) -> Result<Self, TransactionParseError> {
        let bytes = decode_input(s.trim())?;
        let unsigned_err = match borsh::from_slice::<Transaction>(&bytes) {
            Ok(unsigned) => return Ok(Self::new(unsigned)),
            Err(e) => e,
        };
        let allow_signed = developer_config
            .map(|c| c.allow_signed_transactions)
            .unwrap_or(false);
        if allow_signed {
            if let Ok(signed) = borsh::from_slice::<SignedTransaction>(&bytes) {
                return Ok(Self::new(signed.transaction));
            }
        }
        Err(TransactionParseError::DecodeError(format!(
            "near borsh decode: {unsigned_err}"
        )))
    }
}

impl visualsign::vsptrait::Transaction for NearTransaction {
    fn from_string(s: &str) -> Result<Self, TransactionParseError> {
        Self::from_string_with_options(s, None)
    }

    fn transaction_type(&self) -> String {
        "NEAR".to_string()
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

    #[test]
    fn decode_rejects_garbage() {
        let result = NearTransaction::from_string("not-hex-not-base64");
        assert!(result.is_err());
    }

    #[test]
    fn decode_hex_transfer() {
        let tx = NearTransaction::from_string(TRANSFER_HEX).expect("decode hex");
        assert_eq!(tx.inner().signer_id().as_str(), "alice.near");
        assert_eq!(tx.inner().receiver_id().as_str(), "bob.near");
        assert_eq!(tx.inner().actions().len(), 1);
        let near_primitives::action::Action::Transfer(transfer) = &tx.inner().actions()[0] else {
            panic!("expected Transfer");
        };
        assert_eq!(
            transfer.deposit.as_yoctonear(),
            1_000_000_000_000_000_000_000_000
        );
    }

    #[test]
    fn decode_hex_with_0x_prefix() {
        let tx = NearTransaction::from_string(&format!("0x{TRANSFER_HEX}")).expect("decode 0x-hex");
        assert_eq!(tx.inner().signer_id().as_str(), "alice.near");
    }

    #[test]
    fn decode_base64_matches_hex() {
        use base64::Engine;
        let bytes = hex::decode(TRANSFER_HEX).expect("hex");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let tx = NearTransaction::from_string(&b64).expect("decode base64");
        assert_eq!(tx.inner().signer_id().as_str(), "alice.near");
        assert_eq!(tx.inner().receiver_id().as_str(), "bob.near");
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
        assert_eq!(tx.inner().signer_id().as_str(), "alice.near");
    }
}
