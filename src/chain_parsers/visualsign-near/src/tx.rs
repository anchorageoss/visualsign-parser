//! NEAR transaction envelope decoding.

use near_primitives::transaction::{SignedTransaction, Transaction};
use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::TransactionParseError;

/// VSP wrapper over `near-primitives` Transaction. Owns the borsh decoding logic.
#[derive(Debug, Clone)]
pub struct NearTransaction {
    pub inner: Transaction,
}

impl visualsign::vsptrait::Transaction for NearTransaction {
    fn from_string(s: &str) -> Result<Self, TransactionParseError> {
        let bytes = decode_input(s.trim())?;
        // Try signed first (it's a superset); fall back to unsigned.
        if let Ok(signed) = borsh::from_slice::<SignedTransaction>(&bytes) {
            return Ok(Self {
                inner: signed.transaction,
            });
        }
        let unsigned: Transaction = borsh::from_slice(&bytes)
            .map_err(|e| TransactionParseError::DecodeError(format!("near borsh decode: {e}")))?;
        Ok(Self { inner: unsigned })
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
        assert_eq!(tx.inner.signer_id().as_str(), "alice.near");
        assert_eq!(tx.inner.receiver_id().as_str(), "bob.near");
        assert_eq!(tx.inner.actions().len(), 1);
        assert!(matches!(
            tx.inner.actions()[0],
            near_primitives::action::Action::Transfer(_)
        ));
    }

    #[test]
    fn decode_hex_with_0x_prefix() {
        let tx = NearTransaction::from_string(&format!("0x{TRANSFER_HEX}")).expect("decode 0x-hex");
        assert_eq!(tx.inner.signer_id().as_str(), "alice.near");
    }

    #[test]
    fn decode_base64_matches_hex() {
        use base64::Engine;
        let bytes = hex::decode(TRANSFER_HEX).expect("hex");
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let tx = NearTransaction::from_string(&b64).expect("decode base64");
        assert_eq!(tx.inner.signer_id().as_str(), "alice.near");
        assert_eq!(tx.inner.receiver_id().as_str(), "bob.near");
    }
}
