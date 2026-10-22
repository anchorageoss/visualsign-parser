use base64::Engine;

use visualsign::encodings::SupportedEncodings;
use visualsign::vsptrait::TransactionParseError;

use serde::Deserialize;
use sui_sdk_types::{Transaction, UserSignature};

use crate::types::{SuiTransactionBlockData, SuiTransactionBlockKind};

/// BCS layout of Sui's `SenderSignedData`: exactly one transaction, wrapped in
/// the `(TransactionData, V0, Sui)` intent and followed by its signatures.
/// Signatures are parsed (not verified) so a malformed envelope is rejected as
/// it was by `sui_types`.
#[derive(Deserialize)]
struct SenderSignedTransaction {
    intent: (u8, u8, u8),
    transaction: Transaction,
    tx_signatures: Vec<UserSignature>,
}

fn decode_sender_signed_data(bytes: &[u8]) -> Option<Transaction> {
    let signed: Vec<SenderSignedTransaction> = bcs::from_bytes(bytes).ok()?;
    match <[SenderSignedTransaction; 1]>::try_from(signed) {
        Ok([only])
            if only.intent == (0, 0, 0)
                && only
                    .tx_signatures
                    .iter()
                    .all(signature_matches_legacy_decode) =>
        {
            Some(only.transaction)
        }
        _ => None,
    }
}

/// Preserve validations performed by `GenericSignature::from_bytes` in
/// sui-types mainnet-v1.68.1, which SDK signature deserialization omits.
/// This checks encodings and structure only; it never verifies a signature.
fn signature_matches_legacy_decode(signature: &UserSignature) -> bool {
    match signature {
        UserSignature::Simple(_) | UserSignature::ZkLogin(_) => true,
        UserSignature::Multisig(multisig) => {
            let committee = multisig.committee();
            committee.is_valid()
                && !multisig.signatures().is_empty()
                && multisig.signatures().len() <= committee.members().len()
                && multisig.bitmap() <= 1023
                // Modern multisig keys are opaque during old BCS decoding.
                // Legacy base64 keys pass through PublicKey::decode_base64,
                // which also parses their curve points.
                && (multisig.legacy_bitmap().is_none()
                    || committee.members().iter().all(|member| {
                        match member.public_key() {
                            sui_sdk_types::MultisigMemberPublicKey::Ed25519(key) => {
                                ed25519_consensus::VerificationKey::try_from(key.as_bytes()).is_ok()
                            }
                            sui_sdk_types::MultisigMemberPublicKey::Secp256k1(key) => {
                                k256::PublicKey::from_sec1_bytes(key.as_bytes()).is_ok()
                            }
                            sui_sdk_types::MultisigMemberPublicKey::Secp256r1(key) => {
                                p256::PublicKey::from_sec1_bytes(key.as_bytes()).is_ok()
                            }
                            _ => false,
                        }
                    }))
        }
        UserSignature::Passkey(passkey) => {
            // The old WebAuthn type also parses the optional crossOrigin
            // boolean; the SDK ignores it while parsing required fields.
            #[derive(Deserialize)]
            struct ClientData {
                #[serde(rename = "crossOrigin")]
                _cross_origin: Option<bool>,
            }
            let sui_sdk_types::SimpleSignature::Secp256r1 {
                signature,
                public_key,
            } = passkey.signature()
            else {
                return false;
            };
            passkey.challenge().len() == 32
                && serde_json::from_str::<ClientData>(passkey.client_data_json()).is_ok()
                && p256::PublicKey::from_sec1_bytes(public_key.as_bytes()).is_ok()
                && p256::ecdsa::Signature::from_slice(signature.as_bytes()).is_ok()
        }
        _ => false,
    }
}

/// Decode a transaction from string format
pub fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
) -> Result<Transaction, TransactionParseError> {
    if raw_transaction.is_empty() {
        return Err(TransactionParseError::DecodeError(
            "Transaction is empty".into(),
        ));
    }

    let bytes = match encodings {
        SupportedEncodings::Base64 => base64::engine::general_purpose::STANDARD
            .decode(raw_transaction)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?,
        SupportedEncodings::Hex => visualsign::encodings::decode_hex(raw_transaction)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?,
    };

    if let Some(transaction) = decode_sender_signed_data(&bytes) {
        return Ok(transaction);
    }

    if let Ok(transaction_data) = bcs::from_bytes::<Transaction>(&bytes) {
        return Ok(transaction_data);
    }

    Err(TransactionParseError::DecodeError(
        "Unable to decode transaction data as either `SenderSignedData` or `TransactionData`"
            .into(),
    ))
}

/// Determine transaction title based on type
pub fn determine_transaction_type_string(tx_data: &SuiTransactionBlockData) -> &'static str {
    match &tx_data.transaction() {
        SuiTransactionBlockKind::ProgrammableTransaction(_) => "Programmable Transaction",
        SuiTransactionBlockKind::ChangeEpoch => "Change Epoch",
        SuiTransactionBlockKind::Genesis => "Genesis",
        SuiTransactionBlockKind::ConsensusCommitPrologue => "Consensus Commit",
        SuiTransactionBlockKind::AuthenticatorStateUpdate => "Authenticator State Update",
        SuiTransactionBlockKind::RandomnessStateUpdate => "Randomness State Update",
        SuiTransactionBlockKind::EndOfEpochTransaction => "End of Epoch Transaction",
        SuiTransactionBlockKind::ConsensusCommitPrologueV2 => "Consensus Commit Prologue V2",
        SuiTransactionBlockKind::ConsensusCommitPrologueV3 => "Consensus Commit Prologue V3",
        SuiTransactionBlockKind::ConsensusCommitPrologueV4 => "Consensus Commit Prologue V4",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_transaction_empty() {
        let result = decode_transaction("", SupportedEncodings::Base64);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Decode error: Transaction is empty"
        );
    }

    #[test]
    fn test_decode_transaction_native_transfer() {
        let result = decode_transaction(
            "AAACACCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJCrze8SNFZ4kAAIAMqaOwAAAAACAgABAQEAAQECAAABAADW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwgHLSF74CguvkHmmIcQsiwy2XOmYbhyB/RbuiAOPAEpa7Rua1BcAAAAAIGOAX4LpV/FYmnpiNGs3y1rsDwwf9O10x5SdK7vXP+9Q1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysLoAwAAAAAAAEBLTAAAAAAAAA==",
            SupportedEncodings::Base64,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_decode_transaction_basic_info() {
        let test_data = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";
        let result = decode_transaction(test_data, SupportedEncodings::Base64);

        assert!(result.is_ok());
    }

    #[derive(Deserialize)]
    struct SignatureFixture {
        name: String,
        bcs_base64: String,
        accepted: bool,
    }

    fn signature_fixtures() -> Vec<SignatureFixture> {
        #[derive(Deserialize)]
        struct Fixtures {
            fixtures: Vec<SignatureFixture>,
        }
        serde_json::from_str::<Fixtures>(include_str!(
            "../../../tests/fixtures/signatures/sdk-0.4.0.json"
        ))
        .unwrap()
        .fixtures
    }

    fn fixture_signature(name: &str) -> UserSignature {
        let fixture = signature_fixtures()
            .into_iter()
            .find(|fixture| fixture.name == name)
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(fixture.bcs_base64)
            .unwrap();
        bcs::from_bytes(&bytes).unwrap()
    }

    fn envelope_with_signatures(signatures: Vec<Vec<u8>>) -> Vec<u8> {
        let transaction = decode_transaction(
            "AAACACCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJCrze8SNFZ4kAAIAMqaOwAAAAACAgABAQEAAQECAAABAADW6S4ALibDr7IIgAHBtYILZPK8NRv9paI0Ksv59cHKwgHLSF74CguvkHmmIcQsiwy2XOmYbhyB/RbuiAOPAEpa7Rua1BcAAAAAIGOAX4LpV/FYmnpiNGs3y1rsDwwf9O10x5SdK7vXP+9Q1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysLoAwAAAAAAAEBLTAAAAAAAAA==",
            SupportedEncodings::Base64,
        )
        .unwrap();
        bcs::to_bytes(&vec![((0_u8, 0_u8, 0_u8), transaction, signatures)]).unwrap()
    }

    fn accepts_envelope(bytes: &[u8]) -> bool {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let accepts = decode_transaction(&encoded, SupportedEncodings::Base64).is_ok();
        let original_accepts =
            bcs::from_bytes::<sui_types::transaction::SenderSignedData>(bytes).is_ok();
        assert_eq!(
            accepts, original_accepts,
            "decode boundary diverged from sui-types mainnet-v1.68.1"
        );
        accepts
    }

    #[test]
    fn signed_envelopes_preserve_old_sdk_signature_acceptance() {
        for fixture in signature_fixtures() {
            // These are checked-in, real SDK fixture bytes, not signatures
            // constructed with the decoder's own SDK representation.
            let signature_bcs = base64::engine::general_purpose::STANDARD
                .decode(&fixture.bcs_base64)
                .unwrap();
            let signature: Vec<u8> = bcs::from_bytes(&signature_bcs).unwrap();
            let envelope = envelope_with_signatures(vec![signature]);
            assert_eq!(
                accepts_envelope(&envelope),
                fixture.accepted,
                "{}",
                fixture.name
            );
        }
    }

    #[test]
    fn signed_envelopes_reject_wrong_intent_or_entry_count() {
        let signature = fixture_signature("ed25519").to_bytes();
        let envelope = envelope_with_signatures(vec![signature]);
        for intent_byte in 1..=3 {
            let mut malformed = envelope.clone();
            malformed[intent_byte] = 1;
            assert!(!accepts_envelope(&malformed));
        }
        assert!(!accepts_envelope(&[0]));
        let mut two = vec![2];
        two.extend_from_slice(&envelope[1..]);
        two.extend_from_slice(&envelope[1..]);
        assert!(!accepts_envelope(&two));
    }

    #[test]
    fn signed_envelopes_reject_bad_signature_flags_and_lengths() {
        for flag in [3, 4, 5, 6, 7, 255] {
            assert!(!accepts_envelope(&envelope_with_signatures(vec![vec![
                flag
            ]])));
        }
        assert!(!accepts_envelope(&envelope_with_signatures(vec![vec![]])));
        for name in ["ed25519", "secp256k1", "secp256r1"] {
            let signature = fixture_signature(name).to_bytes();
            assert!(!accepts_envelope(&envelope_with_signatures(vec![
                signature[..signature.len() - 1].to_vec()
            ])));
        }
    }

    fn passkey_bytes(
        challenge: &[u8],
        signature: sui_sdk_types::SimpleSignature,
        ty: &str,
    ) -> Vec<u8> {
        let json = serde_json::json!({
            "type": ty,
            "challenge": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge),
            "origin": "https://www.sui.io",
        })
        .to_string();
        let mut bytes = vec![6];
        bytes.extend(bcs::to_bytes(&(vec![0_u8; 37], json, signature)).unwrap());
        bytes
    }

    fn passkey_inner_signature() -> sui_sdk_types::SimpleSignature {
        match fixture_signature("passkey_35_byte_challenge") {
            UserSignature::Passkey(passkey) => passkey.signature(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn passkey_envelopes_require_32_byte_challenge() {
        for size in [0, 31, 32, 33, 35] {
            let signature =
                passkey_bytes(&vec![0; size], passkey_inner_signature(), "webauthn.get");
            assert_eq!(
                accepts_envelope(&envelope_with_signatures(vec![signature])),
                size == 32,
                "challenge size {size}"
            );
        }
    }

    #[test]
    fn passkey_envelopes_validate_signature_encoding_without_verifying() {
        let valid_signature = passkey_inner_signature();
        assert!(accepts_envelope(&envelope_with_signatures(vec![
            passkey_bytes(&[0; 32], valid_signature.clone(), "webauthn.get")
        ])));
        assert!(!accepts_envelope(&envelope_with_signatures(vec![
            passkey_bytes(&[0; 32], valid_signature.clone(), "webauthn.create")
        ])));
        let sui_sdk_types::SimpleSignature::Secp256r1 {
            signature,
            public_key,
        } = valid_signature
        else {
            unreachable!()
        };
        let bad_public_key = sui_sdk_types::SimpleSignature::Secp256r1 {
            signature,
            public_key: sui_sdk_types::Secp256r1PublicKey::new([0; 33]),
        };
        assert!(!accepts_envelope(&envelope_with_signatures(vec![
            passkey_bytes(&[0; 32], bad_public_key, "webauthn.get")
        ])));
        let bad_signature = sui_sdk_types::SimpleSignature::Secp256r1 {
            signature: sui_sdk_types::Secp256r1Signature::new([0; 64]),
            public_key,
        };
        assert!(!accepts_envelope(&envelope_with_signatures(vec![
            passkey_bytes(&[0; 32], bad_signature, "webauthn.get")
        ])));
    }

    #[test]
    fn multisig_envelopes_preserve_decode_validation() {
        let UserSignature::Multisig(valid) = fixture_signature("multisig") else {
            unreachable!()
        };
        let members = valid.committee().members().to_vec();
        let signatures = valid.signatures().to_vec();
        let zero_weight = sui_sdk_types::MultisigMember::new(members[0].public_key().clone(), 0);
        let too_many_members = (0..11)
            .map(|i| {
                sui_sdk_types::MultisigMember::new(
                    sui_sdk_types::MultisigMemberPublicKey::Ed25519(
                        sui_sdk_types::Ed25519PublicKey::new([i; 32]),
                    ),
                    1,
                )
            })
            .collect();
        for (committee, sigs, bitmap) in [
            (
                sui_sdk_types::MultisigCommittee::new(vec![], 1),
                signatures.clone(),
                valid.bitmap(),
            ),
            (
                sui_sdk_types::MultisigCommittee::new(vec![zero_weight], 1),
                signatures[..1].to_vec(),
                1,
            ),
            (
                sui_sdk_types::MultisigCommittee::new(members.clone(), 4),
                signatures.clone(),
                valid.bitmap(),
            ),
            (
                sui_sdk_types::MultisigCommittee::new(too_many_members, 1),
                signatures.clone(),
                valid.bitmap(),
            ),
            (
                sui_sdk_types::MultisigCommittee::new(members[..2].to_vec(), 2),
                signatures.clone(),
                3,
            ),
            (
                sui_sdk_types::MultisigCommittee::new(members.clone(), 0),
                signatures.clone(),
                valid.bitmap(),
            ),
            (
                sui_sdk_types::MultisigCommittee::new(members.clone(), 2),
                vec![],
                valid.bitmap(),
            ),
            (
                sui_sdk_types::MultisigCommittee::new(members.clone(), 2),
                signatures.clone(),
                1024,
            ),
            (
                sui_sdk_types::MultisigCommittee::new(vec![members[0].clone(); 2], 1),
                signatures[..1].to_vec(),
                1,
            ),
        ] {
            let signature = UserSignature::Multisig(
                sui_sdk_types::MultisigAggregatedSignature::new(committee, sigs, bitmap),
            );
            assert!(!accepts_envelope(&envelope_with_signatures(vec![
                signature.to_bytes()
            ])));
        }
    }

    #[test]
    fn passkey_envelopes_reject_malformed_cross_origin() {
        let signature = passkey_inner_signature();
        let client_json = serde_json::json!({
            "type": "webauthn.get",
            "challenge": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0; 32]),
            "origin": "https://www.sui.io",
            "crossOrigin": "false",
        })
        .to_string();
        let mut bytes = vec![6];
        bytes.extend(bcs::to_bytes(&(vec![0_u8; 37], client_json, signature)).unwrap());
        assert!(!accepts_envelope(&envelope_with_signatures(vec![bytes])));
    }

    #[test]
    fn legacy_multisig_envelopes_validate_public_key_encoding() {
        let UserSignature::Multisig(valid) = fixture_signature("legacy_multisig") else {
            unreachable!()
        };
        for public_key in [
            sui_sdk_types::MultisigMemberPublicKey::Ed25519(sui_sdk_types::Ed25519PublicKey::new(
                [2; 32],
            )),
            sui_sdk_types::MultisigMemberPublicKey::Secp256k1(
                sui_sdk_types::Secp256k1PublicKey::new([0; 33]),
            ),
            sui_sdk_types::MultisigMemberPublicKey::Secp256r1(
                sui_sdk_types::Secp256r1PublicKey::new([0; 33]),
            ),
        ] {
            let committee = sui_sdk_types::MultisigCommittee::new(
                vec![sui_sdk_types::MultisigMember::new(public_key, 1)],
                1,
            );
            let mut signature = sui_sdk_types::MultisigAggregatedSignature::new(
                committee,
                valid.signatures()[..1].to_vec(),
                1,
            );
            signature.with_legacy_bitmap(valid.legacy_bitmap().unwrap().clone());
            assert!(!accepts_envelope(&envelope_with_signatures(vec![
                UserSignature::Multisig(signature).to_bytes()
            ])));
        }
    }
}
