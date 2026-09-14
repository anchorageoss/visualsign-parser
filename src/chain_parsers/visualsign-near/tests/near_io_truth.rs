//! Ground truth for the Go mirror of the NEAR intermediate output.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use near_primitives::account::{AccessKey, AccessKeyPermission};
use near_primitives::action::{Action, AddKeyAction, FunctionCallAction, TransferAction};
use near_primitives::hash::CryptoHash;
use near_primitives::transaction::{Transaction, TransactionV0};
use near_primitives::types::Gas;
use near_sdk::NearToken;
use visualsign_near::intermediate::{NearIntermediateOutput, Nep413Io};
use visualsign_near::networks::NearNetwork;

const PK: &str = "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN";

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

#[test]
fn print_near_intermediate_outputs() {
    let tx = Transaction::V0(TransactionV0 {
        signer_id: "alice.near".parse().unwrap(),
        public_key: PK.parse().unwrap(),
        nonce: 7,
        receiver_id: "wrap.near".parse().unwrap(),
        block_hash: CryptoHash::default(),
        actions: vec![
            Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: "ft_transfer_call".to_string(),
                args: br#"{"b":2,"a":1}"#.to_vec(),
                gas: Gas::from_gas(30_000_000_000_000),
                deposit: NearToken::from_yoctonear(1),
            })),
            Action::Transfer(TransferAction { deposit: NearToken::from_yoctonear(42) }),
            Action::AddKey(Box::new(AddKeyAction {
                public_key: PK.parse().unwrap(),
                access_key: AccessKey { nonce: 0, permission: AccessKeyPermission::FullAccess },
            })),
        ],
    });
    let t = NearIntermediateOutput::for_transaction(NearNetwork::Mainnet, &tx);
    println!("TX {}", hex(&t.to_bytes().unwrap()));

    let nep = NearIntermediateOutput::for_nep413(NearNetwork::Testnet, Nep413Io {
        recipient: "intents.near".to_string(),
        nonce_base64: "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=".to_string(),
        callback_url: Some("https://example.com/cb".to_string()),
        message: "Sign in".to_string(),
    });
    println!("NEP {}", hex(&nep.to_bytes().unwrap()));

    let raw = NearIntermediateOutput::for_raw_message(NearNetwork::Mainnet, "payload");
    println!("RAW {}", hex(&raw.to_bytes().unwrap()));
}
