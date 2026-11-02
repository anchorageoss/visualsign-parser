//! Independent candidate Ironwood v6 vectors from zcash/zcash-test-vectors PR #130.
//! Pinned to 5eb4b7eba44485b18bfffd6bf6928343b485540f, not a merged release.
//! Generator: https://github.com/zcash/zcash-test-vectors/blob/5eb4b7eba44485b18bfffd6bf6928343b485540f/zcash_test_vectors/ironwood_v6_tx_hash.py
//! These raw transaction vectors exercise the hash implementation used by PCZT;
//! they do not independently validate PCZT extraction or rendered field mapping.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use core::ops::Deref;
use zcash_primitives::transaction::{
    Authorization, Transaction, TransactionData, sighash::SignableInput,
    sighash_v6::v6_signature_hash, txid::TxIdDigester,
};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::value::Zatoshis;
use zcash_transparent::address::Script;
use zcash_transparent::bundle::{self as tb, TxIn};

const VECTORS: &str = include_str!("fixtures/ironwood_v6_tx_hash_5eb4b7eb.json");

fn hex(s: &str) -> Vec<u8> {
    visualsign::encodings::decode_hex(s).unwrap()
}

/// Transparent authorization carrying the prevout amounts and scripts the
/// vectors supply (ZIP 244 commits to them), as librustzcash's own
/// `zip_0244` test does.
#[derive(Debug)]
struct PrevoutAuth {
    amounts: Vec<Zatoshis>,
    scripts: Vec<Script>,
}

impl tb::Authorization for PrevoutAuth {
    type ScriptSig = <tb::Authorized as tb::Authorization>::ScriptSig;
}

impl zcash_transparent::sighash::TransparentAuthorizingContext for PrevoutAuth {
    fn input_amounts(&self) -> Vec<Zatoshis> {
        self.amounts.clone()
    }
    fn input_scriptpubkeys(&self) -> Vec<Script> {
        self.scripts.clone()
    }
}

struct VectorAuth;

impl Authorization for VectorAuth {
    type TransparentAuth = PrevoutAuth;
    type SaplingAuth = sapling_crypto::bundle::Authorized;
    type OrchardAuth = orchard::bundle::Authorized;
}

#[test]
fn ironwood_v6_txid_auth_digest_and_shielded_sighash_match_independent_vectors() {
    let rows: Vec<serde_json::Value> = serde_json::from_str(VECTORS).unwrap();
    let header: Vec<String> = rows[1][0]
        .as_str()
        .unwrap()
        .split(',')
        .map(|h| h.trim().to_string())
        .collect();
    let col = |name: &str| header.iter().position(|h| h == name).unwrap();

    let mut checked = 0;
    for row in &rows[2..] {
        let field = |name: &str| hex(row[col(name)].as_str().unwrap());
        let raw = field("tx");
        let tx = Transaction::read(&raw[..], BranchId::Nu6_3).unwrap();
        assert_eq!(tx.txid().as_ref().to_vec(), field("txid"), "txid");
        assert_eq!(tx.auth_commitment().as_ref().to_vec(), field("auth_digest"));

        let amounts = row[col("amounts")]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| Zatoshis::from_nonnegative_i64(a.as_i64().unwrap()).unwrap())
            .collect();
        let scripts = row[col("script_pubkeys")]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Script(zcash_script::script::Code(hex(s.as_str().unwrap()))))
            .collect();
        let auth = PrevoutAuth { amounts, scripts };
        let data: &TransactionData<_> = tx.deref();
        let data: TransactionData<VectorAuth> = TransactionData::from_parts_v6(
            data.consensus_branch_id(),
            data.lock_time(),
            data.expiry_height(),
            data.transparent_bundle().map(|b| tb::Bundle {
                vin: b
                    .vin
                    .iter()
                    .map(|i| {
                        TxIn::from_parts(i.prevout().clone(), i.script_sig().clone(), i.sequence())
                    })
                    .collect(),
                vout: b.vout.clone(),
                authorization: auth,
            }),
            data.sapling_bundle().cloned(),
            data.orchard_bundle().cloned(),
            data.ironwood_bundle().cloned(),
        );
        let parts = data.digest(TxIdDigester);
        let sighash = v6_signature_hash(&data, &SignableInput::Shielded, &parts);
        assert_eq!(
            sighash.as_ref().to_vec(),
            field("sighash_shielded"),
            "sighash_shielded"
        );
        checked += 1;
    }
    assert_eq!(checked, 18);
}
