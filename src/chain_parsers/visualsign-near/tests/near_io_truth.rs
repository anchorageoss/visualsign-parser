//! Ground truth for the Go mirror of the NEAR intermediate output.
//!
//! The constants below are duplicated in visualsign-turnkeyclient's
//! verify/near_intermediate_test.go. Both sides assert, so a schema change
//! here fails this test rather than waiting for the Go repository to notice --
//! a cross-language ground truth only works if the side that owns the schema
//! is the side that breaks. Run with --nocapture to print the bytes when
//! regenerating after a deliberate change.
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
    let tx_hex = hex(&t.to_bytes().unwrap());
    println!("TX {tx_hex}");
    assert_eq!(
        tx_hex,
        concat!(
            "01000c0000004e4541525f4d41494e4e4554000a000000616c6963652e6e65617234000000656432353531",
            "393a387256767448574672386861736451474744355769514254797234694832727545505056666a343931",
            "52504e07000000000000000009000000777261702e6e656172200000003131313131313131313131313131",
            "31313131313131313131313131313131313103000000001000000066745f7472616e736665725f63616c6c",
            "1a00000037623232363232323361333232633232363132323361333137640d0000007b2261223a312c2262",
            "223a327d00e057eb481b000001000000310102000000343202060000004164644b65795600000030353030",
            "37346166666137316162303330643430306664666131626564303333646661366664336165333466393264",
            "313763303436656265333638653830643533373531303030303030303030303030303030303031"
        )
    );

    let nep = NearIntermediateOutput::for_nep413(NearNetwork::Testnet, Nep413Io {
        recipient: "intents.near".to_string(),
        nonce_base64: "XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=".to_string(),
        callback_url: Some("https://example.com/cb".to_string()),
        message: "Sign in".to_string(),
    });
    let nep_hex = hex(&nep.to_bytes().unwrap());
    println!("NEP {nep_hex}");
    assert_eq!(
        nep_hex,
        concat!(
            "01000c0000004e4541525f544553544e4554010c000000696e74656e74732e6e6561722c00000058566f4b",
            "666d53636233472b587148396b652f66536c4a2f33784f3539734e6843786870473832314248383d011600",
            "000068747470733a2f2f6578616d706c652e636f6d2f6362070000005369676e20696e"
        )
    );

    let raw = NearIntermediateOutput::for_raw_message(NearNetwork::Mainnet, "payload");
    let raw_hex = hex(&raw.to_bytes().unwrap());
    println!("RAW {raw_hex}");
    assert_eq!(
        raw_hex,
        "01000c0000004e4541525f4d41494e4e455402070000007061796c6f6164"
    );
}
