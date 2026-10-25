//! Recognition of the multisig script templates a PSBT can carry:
//! `wsh(multi/sortedmulti)` witness scripts and `tr(_, multi_a)` tap leaves.
//!
//! A script is only reported as a multisig when it is bound to the output it
//! belongs to (witness script hash, or taproot commitment through the control
//! block or the tap tree). Anything else is reported as unrecognized, with the
//! raw script, so a PSBT cannot make a spend look like a different policy.

use bitcoin::bip32::{DerivationPath, Fingerprint};
use bitcoin::opcodes::all::{OP_CHECKMULTISIG, OP_CHECKSIG, OP_CHECKSIGADD, OP_NUMEQUAL};
use bitcoin::psbt::{Input, Output};
use bitcoin::script::Instruction;
use bitcoin::secp256k1::{PublicKey, Secp256k1, XOnlyPublicKey};
use bitcoin::taproot::{TapLeafHash, TaprootSpendInfo};
use bitcoin::{Script, ScriptBuf};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

/// BIP-341 NUMS point: an internal key nobody can sign for.
const NUMS_HEX: &str = "50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";
const MAX_WSH_KEYS: usize = 20;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Cosigner {
    pub key_hex: String,
    /// `fingerprint m/path` when the PSBT carries an origin for the key.
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Multisig {
    pub kind: &'static str,
    pub threshold: usize,
    pub cosigners: Vec<Cosigner>,
    /// Taproot only: the internal key and whether it is the NUMS point.
    pub internal_key: Option<(String, bool)>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ScriptDetail {
    Multisig(Multisig),
    Unrecognized {
        script_hex: String,
        reason: &'static str,
    },
}

type Origins = BTreeMap<Vec<u8>, String>;

fn origin_text(fp: &Fingerprint, path: &DerivationPath) -> String {
    if path.is_empty() {
        format!("{fp} m")
    } else {
        format!("{fp} m/{path}")
    }
}

fn is_nums(key: &XOnlyPublicKey) -> bool {
    XOnlyPublicKey::from_str(NUMS_HEX).is_ok_and(|n| n == *key)
}

fn small_int(i: &Instruction<'_>) -> Option<usize> {
    match i {
        Instruction::Op(op) => {
            let b = op.to_u8();
            (0x51..=0x60).contains(&b).then(|| usize::from(b - 0x50))
        }
        Instruction::PushBytes(b) if b.len() == 1 => {
            let v = b.as_bytes()[0];
            (17..=127).contains(&v).then(|| usize::from(v))
        }
        Instruction::PushBytes(_) => None,
    }
}

fn instructions(script: &Script) -> Option<Vec<Instruction<'_>>> {
    script.instructions().collect::<Result<_, _>>().ok()
}

/// `<k> <pubkey>... <n> OP_CHECKMULTISIG`
fn parse_multi(script: &Script) -> Option<(usize, Vec<PublicKey>)> {
    let ins = instructions(script)?;
    let (last, rest) = ins.split_last()?;
    if *last != Instruction::Op(OP_CHECKMULTISIG) {
        return None;
    }
    let (n_ins, rest) = rest.split_last()?;
    let (k_ins, key_ins) = rest.split_first()?;
    let (k, n) = (small_int(k_ins)?, small_int(n_ins)?);
    let keys = key_ins
        .iter()
        .map(|i| match i {
            Instruction::PushBytes(b) if b.len() == 33 => PublicKey::from_slice(b.as_bytes()).ok(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let unique = keys.iter().collect::<BTreeSet<_>>().len() == keys.len();
    (unique && keys.len() == n && (1..=n).contains(&k) && n <= MAX_WSH_KEYS).then_some((k, keys))
}

/// `<x1> OP_CHECKSIG (<xi> OP_CHECKSIGADD)* <k> OP_NUMEQUAL`
fn parse_multi_a(script: &Script) -> Option<(usize, Vec<XOnlyPublicKey>)> {
    let ins = instructions(script)?;
    let (last, rest) = ins.split_last()?;
    if *last != Instruction::Op(OP_NUMEQUAL) {
        return None;
    }
    let (k_ins, key_ins) = rest.split_last()?;
    let k = small_int(k_ins)?;
    if key_ins.is_empty() || key_ins.len() % 2 != 0 {
        return None;
    }
    let mut keys = Vec::new();
    for (idx, pair) in key_ins.chunks(2).enumerate() {
        let want = if idx == 0 {
            OP_CHECKSIG
        } else {
            OP_CHECKSIGADD
        };
        match pair {
            [Instruction::PushBytes(b), Instruction::Op(op)] if *op == want && b.len() == 32 => {
                keys.push(XOnlyPublicKey::from_slice(b.as_bytes()).ok()?);
            }
            _ => return None,
        }
    }
    let unique = keys.iter().collect::<BTreeSet<_>>().len() == keys.len();
    (unique && (1..=keys.len()).contains(&k)).then_some((k, keys))
}

fn unrecognized(script: &Script, reason: &'static str) -> ScriptDetail {
    ScriptDetail::Unrecognized {
        script_hex: script.to_hex_string(),
        reason,
    }
}

/// `redeem_script` is `Some` for the P2SH-wrapped form.
fn wsh_detail(
    witness_script: &Script,
    prevout_script: Option<&Script>,
    redeem_script: Option<&Script>,
    origins: &Origins,
) -> ScriptDetail {
    let wsh = ScriptBuf::new_p2wsh(&witness_script.wscript_hash());
    let (bound, kind) = match (prevout_script, redeem_script) {
        (Some(p), Some(r)) => (
            *r == wsh && *p == ScriptBuf::new_p2sh(&r.script_hash()),
            "P2SH-P2WSH",
        ),
        (Some(p), None) => (*p == wsh, "P2WSH"),
        (None, _) => (false, "P2WSH"),
    };
    if !bound {
        return unrecognized(witness_script, "witness script does not match the output");
    }
    match parse_multi(witness_script) {
        Some((threshold, keys)) => ScriptDetail::Multisig(Multisig {
            kind,
            threshold,
            cosigners: keys
                .iter()
                .map(|k| Cosigner {
                    key_hex: k.to_string(),
                    origin: origins.get(&k.serialize().to_vec()).cloned(),
                })
                .collect(),
            internal_key: None,
        }),
        None => unrecognized(witness_script, "unrecognized script"),
    }
}

fn tap_detail(
    leaf: &Script,
    internal: XOnlyPublicKey,
    bound: bool,
    origins: &Origins,
) -> ScriptDetail {
    if !bound {
        return unrecognized(leaf, "tap script is not committed to by the output");
    }
    match parse_multi_a(leaf) {
        Some((threshold, keys)) => ScriptDetail::Multisig(Multisig {
            kind: "P2TR script path",
            threshold,
            cosigners: keys
                .iter()
                .map(|k| Cosigner {
                    key_hex: k.to_string(),
                    origin: origins.get(&k.serialize().to_vec()).cloned(),
                })
                .collect(),
            internal_key: Some((internal.to_string(), is_nums(&internal))),
        }),
        None => unrecognized(leaf, "unrecognized script"),
    }
}

fn tap_origins(
    map: &BTreeMap<XOnlyPublicKey, (Vec<TapLeafHash>, (Fingerprint, DerivationPath))>,
) -> Origins {
    map.iter()
        .map(|(k, (_, (fp, path)))| (k.serialize().to_vec(), origin_text(fp, path)))
        .collect()
}

fn ecdsa_origins(map: &BTreeMap<PublicKey, (Fingerprint, DerivationPath)>) -> Origins {
    map.iter()
        .map(|(k, (fp, path))| (k.serialize().to_vec(), origin_text(fp, path)))
        .collect()
}

fn p2tr_output_key(script: &Script) -> Option<XOnlyPublicKey> {
    script
        .is_p2tr()
        .then(|| XOnlyPublicKey::from_slice(&script.as_bytes()[2..34]).ok())
        .flatten()
}

/// Multisig details for the scripts an input carries. Empty when the input
/// carries no witness script or tap leaf script.
pub fn input_scripts(input: &Input, prevout_script: Option<&Script>) -> Vec<ScriptDetail> {
    let mut out = Vec::new();
    if let Some(ws) = &input.witness_script {
        out.push(wsh_detail(
            ws,
            prevout_script,
            input.redeem_script.as_deref(),
            &ecdsa_origins(&input.bip32_derivation),
        ));
    }
    let origins = tap_origins(&input.tap_key_origins);
    let secp = Secp256k1::verification_only();
    for (control, (leaf, _version)) in &input.tap_scripts {
        let bound = prevout_script
            .and_then(p2tr_output_key)
            .is_some_and(|key| control.verify_taproot_commitment(&secp, key, leaf));
        out.push(tap_detail(leaf, control.internal_key, bound, &origins));
    }
    out
}

/// Multisig details for the scripts an output carries.
pub fn output_scripts(output: &Output, script_pubkey: &Script) -> Vec<ScriptDetail> {
    let mut out = Vec::new();
    if let Some(ws) = &output.witness_script {
        out.push(wsh_detail(
            ws,
            Some(script_pubkey),
            output.redeem_script.as_deref(),
            &ecdsa_origins(&output.bip32_derivation),
        ));
    }
    if let (Some(tree), Some(internal)) = (&output.tap_tree, output.tap_internal_key) {
        let secp = Secp256k1::verification_only();
        let info = TaprootSpendInfo::from_node_info(&secp, internal, tree.clone().into_node_info());
        let committed = ScriptBuf::new_p2tr_tweaked(info.output_key());
        let bound = committed.as_script() == script_pubkey;
        let origins = tap_origins(&output.tap_key_origins);
        for leaf in tree.script_leaves() {
            out.push(tap_detail(leaf.script(), internal, bound, &origins));
        }
    }
    out
}
