//! NEAR intermediate output for downstream policy engines.
//!
//! A Borsh-serialized structured decode of what a NEAR signer is being asked to
//! approve. The parser places these bytes verbatim into
//! `ParsedTransactionPayload.intermediate_output`, and they are appended to what
//! the HSM signs, so the encoding has to be byte-deterministic and consumers
//! that mirror these types decode them without re-parsing the input.
//!
//! [`NEAR_INTERMEDIATE_SCHEMA_VERSION`] is the first field so a shape change is
//! a single, reviewable signal that forces a mirrored decoder to update.
//!
//! The schema is NEAR's own. NEAR Intents content rides inside these envelopes
//! and is rendered for the signer, but it is not decoded here: the intents
//! decoder is chain-independent and giving it a place in a chain's wire schema
//! would make that schema the shared one.
//!
//! What is deliberately not modelled:
//! - Signatures. These envelopes are pre-signature by construction.
//! - Every action variant in full. [`NearActionIo::Other`] names the kind and
//!   carries the action's own Borsh bytes, so a policy sees that an action it
//!   does not model is present and can refuse, rather than not seeing it.

use borsh::{BorshDeserialize, BorshSerialize};
use near_primitives::action::Action;
use near_primitives::transaction::Transaction;
use serde_json::Value;

use crate::actions::action_kind;
use crate::networks::NearNetwork;

/// Version of the `NearIntermediateOutput` Borsh schema. Bump on ANY change to
/// the shapes below. Mirrored decoders assert this value, so a bump makes a
/// schema drift fail loudly instead of silently misparsing.
pub const NEAR_INTERMEDIATE_SCHEMA_VERSION: u16 = 1;

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct NearIntermediateOutput {
    /// Always [`NEAR_INTERMEDIATE_SCHEMA_VERSION`]. First field so decoders can
    /// gate on it before reading the rest.
    pub schema_version: u16,
    /// The canonical network identifier the payload was validated against --
    /// `NearNetwork::network_id`, not a caller-supplied spelling.
    pub network: String,
    pub envelope: NearEnvelopeIo,
}

/// Which of the three things a NEAR signature can cover.
///
/// The envelope decides what is signed: a transaction over
/// `sha256(borsh(tx))`, a NEP-413 message over `sha256(borsh(tag || payload))`,
/// and a raw message over its own bytes.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub enum NearEnvelopeIo {
    Transaction(NearTransactionIo),
    Nep413(Nep413Io),
    RawMessage(RawMessageIo),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct NearTransactionIo {
    pub signer_id: String,
    /// `ed25519:<base58>`, the form NEAR prints a public key in.
    pub public_key: String,
    pub nonce: u64,
    /// Set only for a gas key, whose nonce is indexed. Absent for an ordinary
    /// access key. Dropping it would make two different gas-key nonces look
    /// like the same nonce.
    pub nonce_index: Option<u16>,
    pub receiver_id: String,
    /// Base58, as NEAR prints a block hash.
    pub block_hash: String,
    pub actions: Vec<NearActionIo>,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub enum NearActionIo {
    FunctionCall(NearFunctionCallIo),
    Transfer(NearTransferIo),
    /// An action kind this schema does not model in full. `kind` is the
    /// `near_primitives` variant name and `borsh_hex` is the action as NEAR
    /// serializes it, so nothing about the transaction is hidden from a policy
    /// that allowlists the kinds it understands.
    Other(NearOtherActionIo),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct NearFunctionCallIo {
    pub method_name: String,
    /// Hex of the argument bytes exactly as signed. Always present: the JSON
    /// below is a re-encoding, and a policy that needs certainty reads these.
    pub args_hex: String,
    /// The arguments as canonical JSON, keys alphabetized at every nesting
    /// level. Empty when the arguments are not JSON, which is why `args_hex`
    /// is the field that is always populated.
    pub args_json: String,
    pub gas: u64,
    /// Yocto-NEAR as a decimal string: a NEAR deposit is a u128 and Borsh has
    /// no canonical primitive for that width.
    pub deposit: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct NearTransferIo {
    /// Yocto-NEAR as a decimal string.
    pub deposit: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct NearOtherActionIo {
    pub kind: String,
    pub borsh_hex: String,
}

/// A NEP-413 off-chain message envelope.
///
/// NEP-413 wraps an arbitrary `message`, so the envelope says who the message is
/// for without saying what it does -- which is why `recipient` and `nonce` are
/// the fields a policy binds a sign-in to.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct Nep413Io {
    pub recipient: String,
    /// The 32-byte nonce, base64 -- the encoding NEP-413 itself uses.
    pub nonce_base64: String,
    pub callback_url: Option<String>,
    pub message: String,
}

/// A message signed over its own bytes, with no envelope around it.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct RawMessageIo {
    pub message: String,
}

impl NearIntermediateOutput {
    #[must_use]
    pub fn for_transaction(network: NearNetwork, tx: &Transaction) -> Self {
        Self {
            schema_version: NEAR_INTERMEDIATE_SCHEMA_VERSION,
            network: network.network_id().to_string(),
            envelope: NearEnvelopeIo::Transaction(NearTransactionIo {
                signer_id: tx.signer_id().to_string(),
                public_key: tx.public_key().to_string(),
                nonce: tx.nonce().nonce(),
                nonce_index: tx.nonce().nonce_index(),
                receiver_id: tx.receiver_id().to_string(),
                block_hash: tx.block_hash().to_string(),
                actions: tx.actions().iter().map(NearActionIo::from).collect(),
            }),
        }
    }

    #[must_use]
    pub fn for_nep413(network: NearNetwork, payload: Nep413Io) -> Self {
        Self {
            schema_version: NEAR_INTERMEDIATE_SCHEMA_VERSION,
            network: network.network_id().to_string(),
            envelope: NearEnvelopeIo::Nep413(payload),
        }
    }

    #[must_use]
    pub fn for_raw_message(network: NearNetwork, message: &str) -> Self {
        Self {
            schema_version: NEAR_INTERMEDIATE_SCHEMA_VERSION,
            network: network.network_id().to_string(),
            envelope: NearEnvelopeIo::RawMessage(RawMessageIo {
                message: message.to_string(),
            }),
        }
    }

    /// The Borsh bytes, or `None` if they cannot be produced.
    ///
    /// Borsh serialization of these types does not fail, so `None` here means a
    /// bug rather than a rejected input; returning it keeps a rendering that
    /// succeeded from being turned into an error by the extra output.
    #[must_use]
    pub fn to_bytes(&self) -> Option<Vec<u8>> {
        match borsh::to_vec(self) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                tracing::warn!(%error, "NEAR intermediate output could not be serialized");
                None
            }
        }
    }
}

impl From<&Action> for NearActionIo {
    fn from(action: &Action) -> Self {
        match action {
            Action::FunctionCall(fc) => Self::FunctionCall(NearFunctionCallIo {
                method_name: fc.method_name.clone(),
                args_hex: hex::encode(&fc.args),
                args_json: canonical_args_json(&fc.args),
                gas: fc.gas.as_gas(),
                deposit: fc.deposit.as_yoctonear().to_string(),
            }),
            Action::Transfer(transfer) => Self::Transfer(NearTransferIo {
                deposit: transfer.deposit.as_yoctonear().to_string(),
            }),
            other => Self::Other(NearOtherActionIo {
                kind: action_kind(other).to_string(),
                // An action that does not serialize is not one that could have
                // been decoded from the transaction, so the empty string here is
                // unreachable rather than a silent omission.
                borsh_hex: borsh::to_vec(other).map(hex::encode).unwrap_or_default(),
            }),
        }
    }
}

/// Function-call arguments as canonical JSON, or an empty string when they are
/// not JSON at all.
fn canonical_args_json(args: &[u8]) -> String {
    serde_json::from_slice::<Value>(args)
        .map(|value| visualsign::json::canonical_json(&value))
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use near_primitives::account::{AccessKey, AccessKeyPermission};
    use near_primitives::action::{
        AddKeyAction, DeployContractAction, FunctionCallAction, TransferAction,
    };
    use near_primitives::hash::CryptoHash;
    use near_primitives::transaction::{Transaction, TransactionV0};
    use near_primitives::types::Gas;
    use near_sdk::NearToken;

    use super::*;

    const PUBLIC_KEY: &str = "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN";

    fn transaction(actions: Vec<Action>) -> Transaction {
        Transaction::V0(TransactionV0 {
            signer_id: "alice.near".parse().expect("signer"),
            public_key: PUBLIC_KEY.parse().expect("public key"),
            nonce: 7,
            receiver_id: "wrap.near".parse().expect("receiver"),
            block_hash: CryptoHash::default(),
            actions,
        })
    }

    fn function_call(method_name: &str, args: &[u8]) -> Action {
        Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: method_name.to_string(),
            args: args.to_vec(),
            gas: Gas::from_gas(30_000_000_000_000),
            deposit: NearToken::from_yoctonear(1_000_000_000_000_000_000_000_000),
        }))
    }

    fn only_transaction(output: &NearIntermediateOutput) -> &NearTransactionIo {
        match &output.envelope {
            NearEnvelopeIo::Transaction(tx) => tx,
            other => panic!("expected a transaction envelope, got {other:?}"),
        }
    }

    #[test]
    fn round_trips_through_borsh() {
        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![function_call("near_deposit", b"{}")]),
        );
        let bytes = output.to_bytes().expect("serializes");
        let recovered: NearIntermediateOutput = borsh::from_slice(&bytes).expect("deserializes");
        assert_eq!(output, recovered);
    }

    // A mirrored decoder gates on the version before reading anything else, so
    // it has to be the first two bytes.
    #[test]
    fn schema_version_is_the_first_field() {
        let output = NearIntermediateOutput::for_raw_message(NearNetwork::Mainnet, "hello");
        let bytes = output.to_bytes().expect("serializes");
        assert_eq!(
            u16::from_le_bytes([bytes[0], bytes[1]]),
            NEAR_INTERMEDIATE_SCHEMA_VERSION
        );
    }

    #[test]
    fn carries_the_canonical_network_identifier() {
        let output = NearIntermediateOutput::for_raw_message(NearNetwork::Testnet, "hello");
        assert_eq!(output.network, NearNetwork::Testnet.network_id());
    }

    #[test]
    fn transaction_carries_its_signing_context() {
        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![function_call("near_deposit", b"{}")]),
        );
        let tx = only_transaction(&output);
        assert_eq!(tx.signer_id, "alice.near");
        assert_eq!(tx.receiver_id, "wrap.near");
        assert_eq!(tx.public_key, PUBLIC_KEY);
        assert_eq!(tx.nonce, 7);
        assert_eq!(tx.nonce_index, None);
        assert_eq!(tx.block_hash, CryptoHash::default().to_string());
    }

    // The hex is the signed truth and the JSON is a re-encoding, so both are
    // carried and the JSON is alphabetized at every level.
    #[test]
    fn function_call_args_are_canonical_json_and_raw_hex() {
        let args = br#"{"b":{"d":1,"c":2},"a":3}"#;
        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![function_call("ft_transfer_call", args)]),
        );
        let NearActionIo::FunctionCall(call) = &only_transaction(&output).actions[0] else {
            panic!("expected a function call");
        };
        assert_eq!(call.method_name, "ft_transfer_call");
        assert_eq!(call.args_json, r#"{"a":3,"b":{"c":2,"d":1}}"#);
        assert_eq!(call.args_hex, hex::encode(args));
        assert_eq!(call.gas, 30_000_000_000_000);
        assert_eq!(call.deposit, "1000000000000000000000000");
    }

    // Arguments that are not JSON leave args_json empty, which is why args_hex
    // is the field that is always populated.
    #[test]
    fn non_json_function_call_args_still_carry_their_bytes() {
        let args = &[0xde, 0xad, 0xbe, 0xef];
        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![function_call("raw", args)]),
        );
        let NearActionIo::FunctionCall(call) = &only_transaction(&output).actions[0] else {
            panic!("expected a function call");
        };
        assert!(call.args_json.is_empty());
        assert_eq!(call.args_hex, "deadbeef");
    }

    #[test]
    fn transfer_carries_its_deposit_as_a_decimal_string() {
        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![Action::Transfer(TransferAction {
                deposit: NearToken::from_yoctonear(42),
            })]),
        );
        let NearActionIo::Transfer(transfer) = &only_transaction(&output).actions[0] else {
            panic!("expected a transfer");
        };
        assert_eq!(transfer.deposit, "42");
    }

    // An action this schema does not model is named and carried whole, so a
    // policy allowlisting the kinds it understands refuses it rather than not
    // seeing it.
    #[test]
    fn an_unmodelled_action_is_named_and_carried_whole() {
        let action = Action::AddKey(Box::new(AddKeyAction {
            public_key: PUBLIC_KEY.parse().expect("public key"),
            access_key: AccessKey {
                nonce: 0,
                permission: AccessKeyPermission::FullAccess,
            },
        }));
        let expected_hex = hex::encode(borsh::to_vec(&action).expect("borsh"));

        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![action]),
        );
        let NearActionIo::Other(other) = &only_transaction(&output).actions[0] else {
            panic!("expected an unmodelled action");
        };
        assert_eq!(other.kind, "AddKey");
        assert_eq!(other.borsh_hex, expected_hex);
    }

    #[test]
    fn every_action_is_represented_in_order() {
        let output = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![
                function_call("near_deposit", b"{}"),
                Action::Transfer(TransferAction {
                    deposit: NearToken::from_yoctonear(1),
                }),
                Action::DeployContract(DeployContractAction {
                    code: vec![1, 2, 3],
                }),
            ]),
        );
        let actions = &only_transaction(&output).actions;
        assert_eq!(actions.len(), 3);
        assert!(matches!(actions[0], NearActionIo::FunctionCall(_)));
        assert!(matches!(actions[1], NearActionIo::Transfer(_)));
        let NearActionIo::Other(other) = &actions[2] else {
            panic!("expected an unmodelled action");
        };
        assert_eq!(other.kind, "DeployContract");
    }

    #[test]
    fn nep413_envelope_carries_what_a_policy_binds_a_sign_in_to() {
        let output = NearIntermediateOutput::for_nep413(
            NearNetwork::Mainnet,
            Nep413Io {
                recipient: "app.example.com".to_string(),
                nonce_base64: "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=".to_string(),
                callback_url: None,
                message: "Sign in".to_string(),
            },
        );
        let NearEnvelopeIo::Nep413(envelope) = &output.envelope else {
            panic!("expected a NEP-413 envelope");
        };
        assert_eq!(envelope.recipient, "app.example.com");
        assert_eq!(envelope.message, "Sign in");
        assert_eq!(envelope.callback_url, None);
    }

    #[test]
    fn raw_message_envelope_carries_the_message() {
        let output = NearIntermediateOutput::for_raw_message(NearNetwork::Mainnet, "payload");
        let NearEnvelopeIo::RawMessage(envelope) = &output.envelope else {
            panic!("expected a raw message envelope");
        };
        assert_eq!(envelope.message, "payload");
    }

    // Two encodings of the same object must produce the same bytes: these are
    // appended to what the HSM signs.
    #[test]
    fn key_order_in_the_input_does_not_change_the_bytes() {
        let one = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![function_call("m", br#"{"z":1,"a":{"y":2,"b":3}}"#)]),
        );
        let other = NearIntermediateOutput::for_transaction(
            NearNetwork::Mainnet,
            &transaction(vec![function_call("m", br#"{"a":{"b":3,"y":2},"z":1}"#)]),
        );
        let NearActionIo::FunctionCall(one_call) = &only_transaction(&one).actions[0] else {
            panic!("expected a function call");
        };
        let NearActionIo::FunctionCall(other_call) = &only_transaction(&other).actions[0] else {
            panic!("expected a function call");
        };
        assert_eq!(one_call.args_json, other_call.args_json);
    }
}
