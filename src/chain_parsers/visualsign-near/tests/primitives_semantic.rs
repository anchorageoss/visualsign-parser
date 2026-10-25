//! Semantic differential test: every variant built with `near-primitives`
//! types must decode locally to the *same* variant with the same field values.
//!
//! `primitives_differential.rs` checks accept/reject and re-encoding, which
//! cannot tell apart two variants with the same shape: swapping the tags of
//! `TransferToGasKey`/`WithdrawFromGasKey` (both `(PublicKey, Balance)`) or of
//! `GlobalContractDeployMode::{CodeHash, AccountId}` still round-trips. This
//! test compares a description of each decoded value -- variant names, enum
//! sub-variants, amounts, gas, nonces, key types -- against upstream's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use near_crypto::{ED25519PublicKey, KeyType, Secp256K1PublicKey};
use near_primitives::account as ua;
use near_primitives::action as uact;
use near_primitives::action::delegate as udel;
use near_primitives::deterministic_account_id as udet;
use near_primitives::transaction as utx;
use near_primitives::types::{Balance, Gas};
use visualsign_near::primitives as l;

fn ed(seed: u8) -> near_crypto::PublicKey {
    near_crypto::PublicKey::ED25519(ED25519PublicKey([seed; 32]))
}

fn secp(seed: u8) -> near_crypto::PublicKey {
    near_crypto::PublicKey::SECP256K1(Secp256K1PublicKey::from([seed; 64]))
}

fn bal(n: u128) -> Balance {
    Balance::from_yoctonear(n)
}

// ---------------------------------------------------------------------------
// Descriptions. Each pair of functions renders the same facts from the
// upstream and the local value; a tag swap changes the local description.
// ---------------------------------------------------------------------------

fn u_key(k: &near_crypto::PublicKey) -> String {
    k.to_string()
}

fn l_key(k: &l::PublicKey) -> String {
    k.to_string()
}

fn u_perm(p: &ua::AccessKeyPermission) -> String {
    let fcp = |f: &ua::FunctionCallPermission| {
        format!(
            "allowance={:?} receiver={} methods={:?}",
            f.allowance.map(|a| a.as_yoctonear()),
            f.receiver_id,
            f.method_names
        )
    };
    let gki = |g: &ua::GasKeyInfo| {
        format!(
            "balance={} nonces={}",
            g.balance.as_yoctonear(),
            g.num_nonces
        )
    };
    match p {
        ua::AccessKeyPermission::FunctionCall(f) => format!("FunctionCall({})", fcp(f)),
        ua::AccessKeyPermission::FullAccess => "FullAccess".to_string(),
        ua::AccessKeyPermission::GasKeyFunctionCall(g, f) => {
            format!("GasKeyFunctionCall({}; {})", gki(g), fcp(f))
        }
        ua::AccessKeyPermission::GasKeyFullAccess(g) => format!("GasKeyFullAccess({})", gki(g)),
    }
}

fn l_perm(p: &l::AccessKeyPermission) -> String {
    let fcp = |f: &l::FunctionCallPermission| {
        format!(
            "allowance={:?} receiver={} methods={:?}",
            f.allowance.map(|a| a.as_yoctonear()),
            f.receiver_id,
            f.method_names
        )
    };
    let gki = |g: &l::GasKeyInfo| {
        format!(
            "balance={} nonces={}",
            g.balance.as_yoctonear(),
            g.num_nonces
        )
    };
    match p {
        l::AccessKeyPermission::FunctionCall(f) => format!("FunctionCall({})", fcp(f)),
        l::AccessKeyPermission::FullAccess => "FullAccess".to_string(),
        l::AccessKeyPermission::GasKeyFunctionCall(g, f) => {
            format!("GasKeyFunctionCall({}; {})", gki(g), fcp(f))
        }
        l::AccessKeyPermission::GasKeyFullAccess(g) => format!("GasKeyFullAccess({})", gki(g)),
    }
}

fn u_gci(g: &uact::GlobalContractIdentifier) -> String {
    match g {
        uact::GlobalContractIdentifier::CodeHash(h) => format!("CodeHash({:?})", h.0),
        uact::GlobalContractIdentifier::AccountId(a) => format!("AccountId({a})"),
    }
}

fn l_gci(g: &l::GlobalContractIdentifier) -> String {
    match g {
        l::GlobalContractIdentifier::CodeHash(h) => format!("CodeHash({:?})", h.0),
        l::GlobalContractIdentifier::AccountId(a) => format!("AccountId({a})"),
    }
}

fn u_sig_type(s: &near_crypto::Signature) -> &'static str {
    match s.key_type() {
        KeyType::ED25519 => "ed25519",
        KeyType::SECP256K1 => "secp256k1",
    }
}

fn l_sig_type(s: &l::Signature) -> &'static str {
    match s.key_type() {
        l::KeyType::ED25519 => "ed25519",
        l::KeyType::SECP256K1 => "secp256k1",
    }
}

/// Upstream's own variant name (`strum::AsRefStr`) plus the variant's facts.
fn u_action(a: &uact::Action) -> String {
    let body = match a {
        uact::Action::CreateAccount(_) => String::new(),
        uact::Action::DeployContract(d) => format!("code={:?}", d.code),
        uact::Action::FunctionCall(f) => format!(
            "method={} args={:?} gas={} deposit={}",
            f.method_name,
            f.args,
            f.gas.as_gas(),
            f.deposit.as_yoctonear()
        ),
        uact::Action::Transfer(t) => format!("deposit={}", t.deposit.as_yoctonear()),
        uact::Action::Stake(s) => format!(
            "stake={} key={}",
            s.stake.as_yoctonear(),
            u_key(&s.public_key)
        ),
        uact::Action::AddKey(k) => format!(
            "key={} nonce={} perm={}",
            u_key(&k.public_key),
            k.access_key.nonce,
            u_perm(&k.access_key.permission)
        ),
        uact::Action::DeleteKey(k) => format!("key={}", u_key(&k.public_key)),
        uact::Action::DeleteAccount(d) => format!("beneficiary={}", d.beneficiary_id),
        uact::Action::Delegate(s) => {
            let d = &s.delegate_action;
            format!(
                "sender={} receiver={} nonce={} max_height={} key={} sig={} actions=[{}]",
                d.sender_id,
                d.receiver_id,
                d.nonce,
                d.max_block_height,
                u_key(&d.public_key),
                u_sig_type(&s.signature),
                d.get_actions()
                    .iter()
                    .map(u_action)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        uact::Action::DeployGlobalContract(g) => {
            format!("code={:?} mode={:?}", &g.code[..], g.deploy_mode)
        }
        uact::Action::UseGlobalContract(u) => format!("id={}", u_gci(&u.contract_identifier)),
        uact::Action::DeterministicStateInit(d) => {
            let udet::DeterministicAccountStateInit::V1(v1) = &d.state_init;
            format!(
                "V1 code={} data={:?} deposit={}",
                u_gci(&v1.code),
                v1.data,
                d.deposit.as_yoctonear()
            )
        }
        uact::Action::TransferToGasKey(t) => {
            format!(
                "key={} deposit={}",
                u_key(&t.public_key),
                t.deposit.as_yoctonear()
            )
        }
        uact::Action::WithdrawFromGasKey(w) => {
            format!(
                "key={} amount={}",
                u_key(&w.public_key),
                w.amount.as_yoctonear()
            )
        }
    };
    format!("{}({body})", a.as_ref())
}

/// The local variant name comes from an exhaustive match, not from `Debug`.
fn l_action(a: &l::Action) -> String {
    let (name, body) = match a {
        l::Action::CreateAccount(_) => ("CreateAccount", String::new()),
        l::Action::DeployContract(d) => ("DeployContract", format!("code={:?}", d.code)),
        l::Action::FunctionCall(f) => (
            "FunctionCall",
            format!(
                "method={} args={:?} gas={} deposit={}",
                f.method_name,
                f.args,
                f.gas.as_gas(),
                f.deposit.as_yoctonear()
            ),
        ),
        l::Action::Transfer(t) => ("Transfer", format!("deposit={}", t.deposit.as_yoctonear())),
        l::Action::Stake(s) => (
            "Stake",
            format!(
                "stake={} key={}",
                s.stake.as_yoctonear(),
                l_key(&s.public_key)
            ),
        ),
        l::Action::AddKey(k) => (
            "AddKey",
            format!(
                "key={} nonce={} perm={}",
                l_key(&k.public_key),
                k.access_key.nonce,
                l_perm(&k.access_key.permission)
            ),
        ),
        l::Action::DeleteKey(k) => ("DeleteKey", format!("key={}", l_key(&k.public_key))),
        l::Action::DeleteAccount(d) => {
            ("DeleteAccount", format!("beneficiary={}", d.beneficiary_id))
        }
        l::Action::Delegate(s) => {
            let d = &s.delegate_action;
            (
                "Delegate",
                format!(
                    "sender={} receiver={} nonce={} max_height={} key={} sig={} actions=[{}]",
                    d.sender_id,
                    d.receiver_id,
                    d.nonce,
                    d.max_block_height,
                    l_key(&d.public_key),
                    l_sig_type(&s.signature),
                    d.actions
                        .iter()
                        .map(|a| l_action(a.action()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        }
        l::Action::DeployGlobalContract(g) => {
            let mode = match g.deploy_mode {
                l::GlobalContractDeployMode::CodeHash => "CodeHash",
                l::GlobalContractDeployMode::AccountId => "AccountId",
            };
            (
                "DeployGlobalContract",
                format!("code={:?} mode={mode}", &g.code[..]),
            )
        }
        l::Action::UseGlobalContract(u) => (
            "UseGlobalContract",
            format!("id={}", l_gci(&u.contract_identifier)),
        ),
        l::Action::DeterministicStateInit(d) => {
            let l::DeterministicAccountStateInit::V1(v1) = &d.state_init;
            (
                "DeterministicStateInit",
                format!(
                    "V1 code={} data={:?} deposit={}",
                    l_gci(&v1.code),
                    v1.data,
                    d.deposit.as_yoctonear()
                ),
            )
        }
        l::Action::TransferToGasKey(t) => (
            "TransferToGasKey",
            format!(
                "key={} deposit={}",
                l_key(&t.public_key),
                t.deposit.as_yoctonear()
            ),
        ),
        l::Action::WithdrawFromGasKey(w) => (
            "WithdrawFromGasKey",
            format!(
                "key={} amount={}",
                l_key(&w.public_key),
                w.amount.as_yoctonear()
            ),
        ),
    };
    format!("{name}({body})")
}

fn u_nonce(n: &utx::TransactionNonce) -> String {
    match n {
        utx::TransactionNonce::Nonce { nonce } => format!("Nonce({nonce})"),
        utx::TransactionNonce::GasKeyNonce { nonce, nonce_index } => {
            format!("GasKeyNonce({nonce}, {nonce_index})")
        }
    }
}

fn l_nonce(n: &l::TransactionNonce) -> String {
    match n {
        l::TransactionNonce::Nonce { nonce } => format!("Nonce({nonce})"),
        l::TransactionNonce::GasKeyNonce { nonce, nonce_index } => {
            format!("GasKeyNonce({nonce}, {nonce_index})")
        }
    }
}

fn u_tx(t: &utx::Transaction) -> String {
    let (version, nonce) = match t {
        utx::Transaction::V0(v0) => ("V0", format!("Nonce({})", v0.nonce)),
        utx::Transaction::V1(v1) => ("V1", u_nonce(&v1.nonce)),
    };
    format!(
        "{version} signer={} key={} nonce={nonce} receiver={} hash={:?} actions=[{}]",
        t.signer_id(),
        u_key(t.public_key()),
        t.receiver_id(),
        t.block_hash().0,
        t.actions()
            .iter()
            .map(u_action)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn l_tx(t: &l::Transaction) -> String {
    let (version, nonce) = match t {
        l::Transaction::V0(v0) => ("V0", format!("Nonce({})", v0.nonce)),
        l::Transaction::V1(v1) => ("V1", l_nonce(&v1.nonce)),
    };
    format!(
        "{version} signer={} key={} nonce={nonce} receiver={} hash={:?} actions=[{}]",
        t.signer_id(),
        l_key(t.public_key()),
        t.receiver_id(),
        t.block_hash().0,
        t.actions()
            .iter()
            .map(l_action)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ---------------------------------------------------------------------------
// Corpus: every variant, with distinct values so a misrouted field shows up.
// ---------------------------------------------------------------------------

fn fcp(n: u128) -> ua::FunctionCallPermission {
    ua::FunctionCallPermission {
        allowance: Some(bal(n)),
        receiver_id: "r.near".into(),
        method_names: vec!["m1".into(), "m2".into()],
    }
}

fn gki(n: u128, nonces: u16) -> ua::GasKeyInfo {
    ua::GasKeyInfo {
        balance: bal(n),
        num_nonces: nonces,
    }
}

fn add_key(
    public_key: near_crypto::PublicKey,
    permission: ua::AccessKeyPermission,
) -> uact::Action {
    uact::Action::AddKey(Box::new(uact::AddKeyAction {
        public_key,
        access_key: ua::AccessKey {
            nonce: 21,
            permission,
        },
    }))
}

/// Every non-delegate variant (tags 0-7 and 9-13), every permission, both
/// deploy modes, both global-contract identifiers, both key types.
fn non_delegate_actions() -> Vec<uact::Action> {
    let mut data = BTreeMap::new();
    data.insert(vec![1u8], vec![2u8, 3]);
    vec![
        uact::Action::CreateAccount(uact::CreateAccountAction {}),
        uact::Action::DeployContract(uact::DeployContractAction { code: vec![1, 2] }),
        uact::Action::FunctionCall(Box::new(uact::FunctionCallAction {
            method_name: "f".into(),
            args: vec![3],
            gas: Gas::from_gas(7),
            deposit: bal(8),
        })),
        uact::Action::Transfer(uact::TransferAction { deposit: bal(11) }),
        uact::Action::Stake(Box::new(uact::StakeAction {
            stake: bal(12),
            public_key: ed(1),
        })),
        uact::Action::Stake(Box::new(uact::StakeAction {
            stake: bal(13),
            public_key: secp(2),
        })),
        add_key(ed(3), ua::AccessKeyPermission::FunctionCall(fcp(31))),
        add_key(secp(4), ua::AccessKeyPermission::FullAccess),
        add_key(
            ed(5),
            ua::AccessKeyPermission::GasKeyFunctionCall(gki(51, 3), fcp(52)),
        ),
        add_key(
            secp(6),
            ua::AccessKeyPermission::GasKeyFullAccess(gki(61, 1024)),
        ),
        uact::Action::DeleteKey(Box::new(uact::DeleteKeyAction { public_key: ed(7) })),
        uact::Action::DeleteKey(Box::new(uact::DeleteKeyAction {
            public_key: secp(8),
        })),
        uact::Action::DeleteAccount(uact::DeleteAccountAction {
            beneficiary_id: "b.near".parse().unwrap(),
        }),
        uact::Action::DeployGlobalContract(uact::DeployGlobalContractAction {
            code: vec![4u8].into(),
            deploy_mode: uact::GlobalContractDeployMode::CodeHash,
        }),
        uact::Action::DeployGlobalContract(uact::DeployGlobalContractAction {
            code: vec![5u8].into(),
            deploy_mode: uact::GlobalContractDeployMode::AccountId,
        }),
        uact::Action::UseGlobalContract(Box::new(uact::UseGlobalContractAction {
            contract_identifier: uact::GlobalContractIdentifier::CodeHash(
                near_primitives::hash::CryptoHash([9; 32]),
            ),
        })),
        uact::Action::UseGlobalContract(Box::new(uact::UseGlobalContractAction {
            contract_identifier: uact::GlobalContractIdentifier::AccountId(
                "g.near".parse().unwrap(),
            ),
        })),
        uact::Action::DeterministicStateInit(Box::new(uact::DeterministicStateInitAction {
            state_init: udet::DeterministicAccountStateInit::V1(
                udet::DeterministicAccountStateInitV1 {
                    code: uact::GlobalContractIdentifier::AccountId("c.near".parse().unwrap()),
                    data,
                },
            ),
            deposit: bal(14),
        })),
        uact::Action::TransferToGasKey(Box::new(uact::TransferToGasKeyAction {
            public_key: ed(9),
            deposit: bal(15),
        })),
        uact::Action::WithdrawFromGasKey(Box::new(uact::WithdrawFromGasKeyAction {
            public_key: secp(10),
            amount: bal(16),
        })),
    ]
}

fn delegate(sig: near_crypto::Signature) -> uact::Action {
    uact::Action::Delegate(Box::new(udel::SignedDelegateAction {
        delegate_action: udel::DelegateAction {
            sender_id: "s.near".parse().unwrap(),
            receiver_id: "r.near".parse().unwrap(),
            actions: non_delegate_actions()
                .into_iter()
                .map(|a| udel::NonDelegateAction::try_from(a).unwrap())
                .collect(),
            nonce: 31,
            max_block_height: 32,
            public_key: secp(11),
        },
        signature: sig,
    }))
}

fn all_actions() -> Vec<uact::Action> {
    let mut actions = non_delegate_actions();
    actions.push(delegate(
        near_crypto::Signature::from_parts(KeyType::ED25519, &[0x0f; 64]).unwrap(),
    ));
    actions.push(delegate(
        near_crypto::Signature::from_parts(KeyType::SECP256K1, &[0x33; 65]).unwrap(),
    ));
    actions
}

#[test]
fn corpus_covers_every_variant() {
    let actions = all_actions();
    let mut names: Vec<&str> = actions.iter().map(|a| a.as_ref()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 14, "{names:?}");
}

#[test]
fn every_action_decodes_to_the_same_variant_and_values() {
    for action in all_actions() {
        let bytes = borsh::to_vec(&action).unwrap();
        let ours: l::Action = borsh::from_slice(&bytes).unwrap();
        assert_eq!(
            l_action(&ours),
            u_action(&action),
            "bytes {}",
            hex::encode(&bytes)
        );
        assert_eq!(borsh::to_vec(&ours).unwrap(), bytes);
    }
}

#[test]
fn every_transaction_shape_decodes_to_the_same_values() {
    let actions = all_actions();
    let mut txs = vec![utx::Transaction::V0(utx::TransactionV0 {
        signer_id: "a.near".parse().unwrap(),
        public_key: ed(12),
        nonce: 41,
        receiver_id: "b.near".parse().unwrap(),
        block_hash: near_primitives::hash::CryptoHash([1; 32]),
        actions: actions.clone(),
    })];
    for (nonce, key) in [
        (utx::TransactionNonce::from_nonce(42), ed(13)),
        (utx::TransactionNonce::from_nonce_and_index(43, 7), secp(14)),
    ] {
        txs.push(utx::Transaction::V1(utx::TransactionV1 {
            signer_id: "a.near".parse().unwrap(),
            public_key: key,
            nonce,
            receiver_id: "b.near".parse().unwrap(),
            block_hash: near_primitives::hash::CryptoHash([2; 32]),
            actions: actions.clone(),
        }));
    }
    for tx in txs {
        let bytes = borsh::to_vec(&tx).unwrap();
        let ours: l::Transaction = borsh::from_slice(&bytes).unwrap();
        assert_eq!(l_tx(&ours), u_tx(&tx));
        assert_eq!(borsh::to_vec(&ours).unwrap(), bytes);

        for sig in [
            near_crypto::Signature::from_parts(KeyType::ED25519, &[0x01; 64]).unwrap(),
            near_crypto::Signature::from_parts(KeyType::SECP256K1, &[0x02; 65]).unwrap(),
        ] {
            let signed = utx::SignedTransaction::new(sig.clone(), tx.clone());
            let bytes = borsh::to_vec(&signed).unwrap();
            let ours: l::SignedTransaction = borsh::from_slice(&bytes).unwrap();
            assert_eq!(l_tx(&ours.transaction), u_tx(&signed.transaction));
            assert_eq!(l_sig_type(&ours.signature), u_sig_type(&sig));
            assert_eq!(ours.signature.to_string(), sig.to_string());
            assert_eq!(borsh::to_vec(&ours).unwrap(), bytes);
        }
    }
}

/// `AccountId` and `Balance` are upstream's own types, not look-alikes.
#[test]
fn account_id_and_balance_are_upstream_types() {
    let account: near_primitives::types::AccountId = "x.near".parse().unwrap();
    let _: l::AccountId = account;
    let _: l::Balance = bal(1);
}
