//! Render snapshot for on-chain NEAR transactions.
//!
//! Inputs are built with `near-primitives` (a dev-dependency) so they are
//! exactly the bytes a real NEAR client would produce. Each is rendered through
//! the public string entry point and compared with
//! feature-specific snapshots in `tests/fixtures/`, captured from the parser
//! while it still decoded with `near-primitives`. The rendered payload
//! (field labels, order and values) and every error string must stay
//! byte-identical.
//!
//! Both fixtures were generated with near-primitives decoding at pre-port commit
//! `97e4269d9f2d8ff918a75518c4022e2a40a0c6bd`, using its original generator:
//!
//! ```text
//! NEAR_SNAPSHOT_REGEN=1 cargo test -p visualsign-near --locked --test onchain_render_snapshot
//! NEAR_SNAPSHOT_REGEN=1 cargo test -p visualsign-near --locked --features diagnostics --test onchain_render_snapshot
//! ```
//!
//! Run these only at that commit, copying the generated fixture aside between
//! runs. Store the first as `onchain_render_snapshot.json` (diagnostics off)
//! and the second as `onchain_render_snapshot_diagnostics.json` (diagnostics
//! on). The intents signature warning intentionally renders differently for
//! each feature setting, so both independent outputs are pinned.
//!
//! There is deliberately no regeneration path here: regenerating from this
//! build would capture the local decoder's output and compare it against
//! itself. If a render change is ever intended, regenerate only from a build
//! that still decodes with near-primitives, and review the diff.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use generated::parser::{ChainMetadata, NearMetadata, chain_metadata};
use near_crypto::{ED25519PublicKey, KeyType, PublicKey, Secp256K1PublicKey, Signature};
use near_primitives::account::{
    AccessKey, AccessKeyPermission, FunctionCallPermission, GasKeyInfo,
};
use near_primitives::action::delegate::{DelegateAction, NonDelegateAction, SignedDelegateAction};
use near_primitives::action::{
    Action, AddKeyAction, CreateAccountAction, DeleteAccountAction, DeleteKeyAction,
    DeployContractAction, DeployGlobalContractAction, DeterministicStateInitAction,
    FunctionCallAction, GlobalContractDeployMode, GlobalContractIdentifier, StakeAction,
    TransferAction, TransferToGasKeyAction, UseGlobalContractAction, WithdrawFromGasKeyAction,
};
use near_primitives::deterministic_account_id::{
    DeterministicAccountStateInit, DeterministicAccountStateInitV1,
};
use near_primitives::hash::CryptoHash;
use near_primitives::transaction::{
    SignedTransaction, Transaction, TransactionNonce, TransactionV0, TransactionV1,
};
use near_primitives::types::{Balance, Gas};
use serde::{Deserialize, Serialize};
use visualsign::vsptrait::{DeveloperConfig, VisualSignConverterFromString, VisualSignOptions};
use visualsign_near::NearVisualSignConverter;

#[cfg(not(feature = "diagnostics"))]
const SNAPSHOT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/onchain_render_snapshot.json"
);

#[cfg(feature = "diagnostics")]
const SNAPSHOT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/onchain_render_snapshot_diagnostics.json"
);

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Entry {
    name: String,
    hex: String,
    allow_signed: bool,
    network_id: Option<String>,
    /// `Ok` carries the payload JSON, `Err` the error's Debug form.
    result: Result<serde_json::Value, String>,
}

fn ed_key(seed: u8) -> PublicKey {
    PublicKey::ED25519(ED25519PublicKey([seed; 32]))
}

fn secp_key(seed: u8) -> PublicKey {
    PublicKey::SECP256K1(Secp256K1PublicKey::from([seed; 64]))
}

fn ed_sig() -> Signature {
    let mut bytes = [0x11u8; 64];
    bytes[63] = 0x1f;
    Signature::from_parts(KeyType::ED25519, &bytes).unwrap()
}

fn secp_sig() -> Signature {
    Signature::from_parts(KeyType::SECP256K1, &[0x22u8; 65]).unwrap()
}

fn hash(seed: u8) -> CryptoHash {
    CryptoHash([seed; 32])
}

fn near(n: u128) -> Balance {
    Balance::from_yoctonear(n * 1_000_000_000_000_000_000_000_000)
}

fn fc(method: &str, args: &[u8], gas: u64, deposit: u128) -> Action {
    Action::FunctionCall(Box::new(FunctionCallAction {
        method_name: method.to_string(),
        args: args.to_vec(),
        gas: Gas::from_gas(gas),
        deposit: Balance::from_yoctonear(deposit),
    }))
}

fn fc_perm(allowance: Option<u128>, methods: &[&str]) -> FunctionCallPermission {
    FunctionCallPermission {
        allowance: allowance.map(Balance::from_yoctonear),
        receiver_id: "contract.near".to_string(),
        method_names: methods.iter().map(|m| (*m).to_string()).collect(),
    }
}

fn add_key(public_key: PublicKey, permission: AccessKeyPermission) -> Action {
    Action::AddKey(Box::new(AddKeyAction {
        public_key,
        access_key: AccessKey {
            nonce: 7,
            permission,
        },
    }))
}

fn state_init(code: GlobalContractIdentifier) -> Action {
    let mut data = BTreeMap::new();
    data.insert(b"k1".to_vec(), b"v1".to_vec());
    data.insert(b"k2".to_vec(), Vec::new());
    Action::DeterministicStateInit(Box::new(DeterministicStateInitAction {
        state_init: DeterministicAccountStateInit::V1(DeterministicAccountStateInitV1 {
            code,
            data,
        }),
        deposit: near(2),
    }))
}

/// One of every non-delegate action variant, in discriminant order, with the
/// sub-variants that change the render.
fn every_action() -> Vec<(&'static str, Action)> {
    vec![
        ("create_account", Action::CreateAccount(CreateAccountAction {})),
        (
            "deploy_contract",
            Action::DeployContract(DeployContractAction {
                code: vec![0, 97, 115, 109, 1, 0, 0, 0],
            }),
        ),
        (
            "function_call_ft_transfer_call",
            fc(
                "ft_transfer_call",
                br#"{"receiver_id":"intents.near","amount":"1000000","msg":"{\"x\":1}","memo":"hi"}"#,
                100_000_000_000_000,
                1,
            ),
        ),
        (
            "function_call_ft_transfer",
            fc(
                "ft_transfer",
                br#"{"receiver_id":"bob.near","amount":"5"}"#,
                30_000_000_000_000,
                1,
            ),
        ),
        (
            "function_call_ft_withdraw",
            fc(
                "ft_withdraw",
                br#"{"token":"wrap.near","receiver_id":"bob.near","amount":"7","msg":"0xdead"}"#,
                100_000_000_000_000,
                1,
            ),
        ),
        (
            "function_call_invalid_receiver",
            fc(
                "ft_transfer",
                br#"{"receiver_id":"Bob.near","amount":"5"}"#,
                30_000_000_000_000,
                1,
            ),
        ),
        (
            "function_call_unknown_method",
            fc("do_something", br#"{"a":1}"#, 1_234_567_890_123, 0),
        ),
        ("function_call_empty_args", fc("noop", b"", 0, 0)),
        (
            "transfer",
            Action::Transfer(TransferAction {
                deposit: Balance::from_yoctonear(1_500_000_000_000_000_000_000_001),
            }),
        ),
        (
            "stake_ed25519",
            Action::Stake(Box::new(StakeAction {
                stake: near(100),
                public_key: ed_key(3),
            })),
        ),
        (
            "stake_secp256k1",
            Action::Stake(Box::new(StakeAction {
                stake: Balance::from_yoctonear(0),
                public_key: secp_key(4),
            })),
        ),
        (
            "add_key_full_access",
            add_key(ed_key(5), AccessKeyPermission::FullAccess),
        ),
        (
            "add_key_function_call_unbounded",
            add_key(secp_key(6), AccessKeyPermission::FunctionCall(fc_perm(None, &[]))),
        ),
        (
            "add_key_function_call_bounded",
            add_key(
                ed_key(7),
                AccessKeyPermission::FunctionCall(fc_perm(
                    Some(250_000_000_000_000_000_000_000),
                    &["ft_transfer", "storage_deposit"],
                )),
            ),
        ),
        (
            "add_key_gas_key_function_call",
            add_key(
                ed_key(8),
                AccessKeyPermission::GasKeyFunctionCall(
                    GasKeyInfo {
                        balance: near(2),
                        num_nonces: 4,
                    },
                    fc_perm(Some(1), &["a"]),
                ),
            ),
        ),
        (
            "add_key_gas_key_full_access",
            add_key(
                secp_key(9),
                AccessKeyPermission::GasKeyFullAccess(GasKeyInfo {
                    balance: Balance::from_yoctonear(500_000_000_000_000_000_000_000),
                    num_nonces: 1024,
                }),
            ),
        ),
        (
            "delete_key_ed25519",
            Action::DeleteKey(Box::new(DeleteKeyAction {
                public_key: ed_key(10),
            })),
        ),
        (
            "delete_key_secp256k1",
            Action::DeleteKey(Box::new(DeleteKeyAction {
                public_key: secp_key(11),
            })),
        ),
        (
            "delete_account",
            Action::DeleteAccount(DeleteAccountAction {
                beneficiary_id: "beneficiary.near".parse().unwrap(),
            }),
        ),
        (
            "deploy_global_contract_code_hash",
            Action::DeployGlobalContract(DeployGlobalContractAction {
                code: vec![1u8, 2, 3].into(),
                deploy_mode: GlobalContractDeployMode::CodeHash,
            }),
        ),
        (
            "deploy_global_contract_account_id",
            Action::DeployGlobalContract(DeployGlobalContractAction {
                code: vec![4u8].into(),
                deploy_mode: GlobalContractDeployMode::AccountId,
            }),
        ),
        (
            "use_global_contract_code_hash",
            Action::UseGlobalContract(Box::new(UseGlobalContractAction {
                contract_identifier: GlobalContractIdentifier::CodeHash(hash(12)),
            })),
        ),
        (
            "use_global_contract_account_id",
            Action::UseGlobalContract(Box::new(UseGlobalContractAction {
                contract_identifier: GlobalContractIdentifier::AccountId(
                    "global.near".parse().unwrap(),
                ),
            })),
        ),
        (
            "deterministic_state_init_code_hash",
            state_init(GlobalContractIdentifier::CodeHash(hash(13))),
        ),
        (
            "deterministic_state_init_account_id",
            state_init(GlobalContractIdentifier::AccountId("code.near".parse().unwrap())),
        ),
        (
            "transfer_to_gas_key",
            Action::TransferToGasKey(Box::new(TransferToGasKeyAction {
                public_key: ed_key(14),
                deposit: near(3),
            })),
        ),
        (
            "withdraw_from_gas_key",
            Action::WithdrawFromGasKey(Box::new(WithdrawFromGasKeyAction {
                public_key: secp_key(15),
                amount: Balance::from_yoctonear(42),
            })),
        ),
    ]
}

fn delegate(inner: Vec<Action>) -> Action {
    Action::Delegate(Box::new(SignedDelegateAction {
        delegate_action: DelegateAction {
            sender_id: "alice.near".parse().unwrap(),
            receiver_id: "contract.near".parse().unwrap(),
            actions: inner
                .into_iter()
                .map(|a| NonDelegateAction::try_from(a).unwrap())
                .collect(),
            nonce: 1,
            max_block_height: 2,
            public_key: ed_key(16),
        },
        signature: ed_sig(),
    }))
}

fn v0(signer: &str, receiver: &str, public_key: PublicKey, actions: Vec<Action>) -> Transaction {
    Transaction::V0(TransactionV0 {
        signer_id: signer.parse().unwrap(),
        public_key,
        nonce: 99,
        receiver_id: receiver.parse().unwrap(),
        block_hash: hash(1),
        actions,
    })
}

fn v1(nonce: TransactionNonce, actions: Vec<Action>) -> Transaction {
    Transaction::V1(TransactionV1 {
        signer_id: "alice.near".parse().unwrap(),
        public_key: secp_key(2),
        nonce,
        receiver_id: "bob.near".parse().unwrap(),
        block_hash: hash(3),
        actions,
    })
}

struct Case {
    name: String,
    bytes: Vec<u8>,
    allow_signed: bool,
    network_id: Option<&'static str>,
}

fn case(name: impl Into<String>, bytes: Vec<u8>) -> Case {
    Case {
        name: name.into(),
        bytes,
        allow_signed: false,
        network_id: None,
    }
}

fn enc(tx: &Transaction) -> Vec<u8> {
    borsh::to_vec(tx).unwrap()
}

fn corpus() -> Vec<Case> {
    let mut cases = Vec::new();
    let actions = every_action();

    // Every variant alone, as V0 and V1, so single-action titles and the
    // un-prefixed field set are covered.
    for (name, action) in &actions {
        cases.push(case(
            format!("v0_single_{name}"),
            enc(&v0(
                "alice.near",
                "bob.near",
                ed_key(1),
                vec![action.clone()],
            )),
        ));
        cases.push(case(
            format!("v1_single_{name}"),
            enc(&v1(TransactionNonce::from_nonce(5), vec![action.clone()])),
        ));
    }

    // Every variant in one transaction: the boundary-field path.
    let all: Vec<Action> = actions.iter().map(|(_, a)| a.clone()).collect();
    cases.push(case(
        "v0_all_actions",
        enc(&v0("alice.near", "bob.near", ed_key(1), all.clone())),
    ));
    cases.push(case(
        "v1_gas_key_nonce_all_actions",
        enc(&v1(TransactionNonce::from_nonce_and_index(77, 3), all)),
    ));

    // Delegate is refused, alone and mixed.
    let delegate_one = delegate(vec![Action::CreateAccount(CreateAccountAction {})]);
    cases.push(case(
        "v0_delegate",
        enc(&v0(
            "alice.near",
            "bob.near",
            ed_key(1),
            vec![delegate_one.clone()],
        )),
    ));
    cases.push(case(
        "v0_transfer_then_delegate",
        enc(&v0(
            "alice.near",
            "bob.near",
            ed_key(1),
            vec![
                Action::Transfer(TransferAction { deposit: near(1) }),
                delegate_one,
            ],
        )),
    ));

    // No actions.
    cases.push(case(
        "v0_no_actions",
        enc(&v0("alice.near", "bob.near", ed_key(1), vec![])),
    ));

    // Accounts: implicit (64-hex), testnet suffix, min length.
    let implicit = "a".repeat(64);
    cases.push(case(
        "v0_implicit_signer",
        enc(&v0(
            &implicit,
            "bob.near",
            ed_key(1),
            vec![Action::Transfer(TransferAction { deposit: near(1) })],
        )),
    ));
    cases.push(case(
        "v0_testnet_accounts_on_mainnet",
        enc(&v0(
            "alice.testnet",
            "bob.testnet",
            ed_key(1),
            vec![Action::Transfer(TransferAction { deposit: near(1) })],
        )),
    ));
    let mut testnet = case(
        "v0_testnet_accounts_on_testnet",
        enc(&v0(
            "alice.testnet",
            "bob.testnet",
            ed_key(1),
            vec![Action::Transfer(TransferAction { deposit: near(1) })],
        )),
    );
    testnet.network_id = Some("NEAR_TESTNET");
    cases.push(testnet);
    cases.push(case(
        "v0_two_char_accounts",
        enc(&v0(
            "ab",
            "cd",
            secp_key(1),
            vec![Action::Transfer(TransferAction { deposit: near(1) })],
        )),
    ));

    // execute_intents to intents.near: the intents preset path.
    let inner = r#"{"signer_id":"alice.near","verifying_contract":"intents.near","deadline":"2999-01-01T00:00:00Z","nonce":"XVoKfmScb3G+XqH9ke/fSlJ/3xO59sNhCxhpG821BH8=","intents":[{"intent":"ft_withdraw","token":"wrap.near","receiver_id":"bob.near","amount":"1000000"}]}"#;
    let args = serde_json::json!({"signed":[{
        "standard": "raw_ed25519",
        "payload": inner,
        "public_key": "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
        "signature": "ed25519:3vtbNQJHZfuV1s5DykzyjkbNLc583hnkrhTz57eDhd966iqzkor6Twgr4Loh2C195SCSEsiGfrd6KcxpjNq9ZbVj"
    }]});
    cases.push(case(
        "v0_execute_intents",
        enc(&v0(
            "alice.near",
            "intents.near",
            ed_key(1),
            vec![fc(
                "execute_intents",
                &serde_json::to_vec(&args).unwrap(),
                30_000_000_000_000,
                0,
            )],
        )),
    ));

    // Signed envelopes, with and without the developer opt-in.
    let transfer = vec![Action::Transfer(TransferAction { deposit: near(1) })];
    for (sig_name, sig) in [("ed25519", ed_sig()), ("secp256k1", secp_sig())] {
        for (tx_name, tx) in [
            (
                "v0",
                v0("alice.near", "bob.near", ed_key(1), transfer.clone()),
            ),
            ("v1", v1(TransactionNonce::from_nonce(1), transfer.clone())),
        ] {
            let bytes = borsh::to_vec(&SignedTransaction::new(sig.clone(), tx)).unwrap();
            for allow_signed in [false, true] {
                cases.push(Case {
                    name: format!("signed_{tx_name}_{sig_name}_allow_{allow_signed}"),
                    bytes: bytes.clone(),
                    allow_signed,
                    network_id: None,
                });
            }
        }
    }

    // Malformed byte strings: trailing byte, truncation, bad tags, bad
    // account ids. Built by editing a valid encoding so the error comes from
    // the targeted field.
    let base = enc(&v0("alice.near", "bob.near", ed_key(1), transfer.clone()));
    let mut trailing = base.clone();
    trailing.push(0);
    cases.push(case("err_trailing_byte", trailing));
    cases.push(case("err_truncated", base[..base.len() - 1].to_vec()));
    cases.push(case("err_empty", Vec::new()));
    cases.push(case("err_one_byte", vec![1]));
    let mut bad_version = base.clone();
    bad_version[0] = 2;
    bad_version[1] = 1;
    cases.push(case("err_bad_version_tag", bad_version));
    let mut upper = base.clone();
    upper[4] = b'A';
    cases.push(case("err_uppercase_signer", upper));
    let mut bad_key_tag = base.clone();
    bad_key_tag[4 + 10] = 7;
    cases.push(case("err_bad_key_tag", bad_key_tag));
    let mut bad_action_tag = base.clone();
    let action_tag_at = base.len() - 17;
    bad_action_tag[action_tag_at] = 14;
    cases.push(case("err_unknown_action_tag", bad_action_tag));

    cases.push(case("err_nested_delegate", nested_delegate_tx()));

    cases
}

/// A transaction whose delegate action carries another delegate action: not
/// constructible through `NonDelegateAction`, so the inner `CreateAccount` tag
/// is replaced by a whole encoded delegate.
fn nested_delegate_tx() -> Vec<u8> {
    let outer = delegate(vec![Action::CreateAccount(CreateAccountAction {})]);
    let outer_len = borsh::to_vec(&outer).unwrap().len();
    let mut bytes = enc(&v0("alice.near", "bob.near", ed_key(1), vec![outer]));
    // tag, sender "alice.near", receiver "contract.near", action count.
    let tag_at = bytes.len() - outer_len + 1 + (4 + 10) + (4 + 13) + 4;
    assert_eq!(bytes[tag_at], 0, "expected the CreateAccount tag");
    bytes.splice(tag_at..=tag_at, borsh::to_vec(&delegate(vec![])).unwrap());
    bytes
}

fn render(case: &Case) -> Result<serde_json::Value, String> {
    let options = VisualSignOptions {
        developer_config: case.allow_signed.then_some(DeveloperConfig {
            allow_signed_transactions: true,
        }),
        metadata: case.network_id.map(|network_id| ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Near(NearMetadata {
                network_id: Some(network_id.to_string()),
                token_mappings: Default::default(),
            })),
        }),
        ..Default::default()
    };
    NearVisualSignConverter::new()
        .to_visual_sign_payload_from_string(&hex::encode(&case.bytes), options)
        .map(|result| serde_json::to_value(&result.payload).unwrap())
        .map_err(|e| format!("{e:?}"))
}

#[test]
fn onchain_render_matches_snapshot() {
    let entries: Vec<Entry> = corpus()
        .iter()
        .map(|case| Entry {
            name: case.name.clone(),
            hex: hex::encode(&case.bytes),
            allow_signed: case.allow_signed,
            network_id: case.network_id.map(str::to_string),
            result: render(case),
        })
        .collect();

    let expected: Vec<Entry> =
        serde_json::from_str(&std::fs::read_to_string(SNAPSHOT_PATH).unwrap()).unwrap();
    assert_eq!(entries.len(), expected.len(), "corpus size changed");
    for (actual, expected) in entries.iter().zip(&expected) {
        assert_eq!(actual, expected, "case {}", expected.name);
    }
}
