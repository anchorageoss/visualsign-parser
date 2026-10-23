//! Builds the committed PCZT fixtures with the librustzcash roles a
//! coordinator runs: zcash_primitives `Builder::build_for_pczt` -> pczt
//! Creator -> IO Finalizer -> Updater. No proofs: the sighash does not commit
//! to them. Randomness comes from the OS, so the bytes differ on every run;
//! that is why the fixtures are generated once and committed.
//!
//! Notes being spent are synthesized with valid commitments and a Merkle path
//! of empty-subtree siblings at position 0; the anchor is that path's root.
//! Adapted from Sigil's decoder tests (iqlusioninc).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use getrandom::SysRng;
use incrementalmerkletree::{Hashable, Level};
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use orchard::note::{NoteVersion, RandomSeed, Rho};
use orchard::tree::{MerkleHashOrchard, MerklePath as OrchardPath};
use pczt::roles::{creator::Creator, io_finalizer::IoFinalizer, updater::Updater};
use rand_core::UnwrapErr;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{BlockHeight, Network};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zcash_transparent::address::TransparentAddress;
use zcash_transparent::pczt::Bip32Derivation;

pub const MAIN: Network = Network::MainNetwork;
/// NU6.2 active, NU6.3 not yet (v5 transactions) on mainnet.
pub const PRE_NU6_3: u32 = 3_400_000;
/// NU6.3 active (v6 transactions, Ironwood pool) on mainnet.
pub const POST_NU6_3: u32 = 3_500_000;

fn p2pkh_script(t: &TransparentAddress) -> Vec<u8> {
    let TransparentAddress::PublicKeyHash(h) = t else {
        panic!("not P2PKH")
    };
    [&[0x76, 0xa9, 0x14][..], &h[..], &[0x88, 0xac][..]].concat()
}

fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

/// The sender's Orchard key.
pub fn sender_orchard() -> FullViewingKey {
    FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap())
}

/// Someone else's Orchard key.
pub fn foreign_orchard() -> FullViewingKey {
    FullViewingKey::from(&SpendingKey::from_bytes([7; 32]).unwrap())
}

pub fn foreign_orchard_address() -> orchard::Address {
    foreign_orchard().address_at(0u32, Scope::External)
}

pub fn sender_sapling() -> sapling_crypto::zip32::DiversifiableFullViewingKey {
    sapling_crypto::zip32::ExtendedSpendingKey::master(&[3; 32])
        .unwrap()
        .to_diversifiable_full_viewing_key()
}

pub fn foreign_sapling_address() -> sapling_crypto::PaymentAddress {
    sapling_crypto::zip32::ExtendedSpendingKey::master(&[9; 32])
        .unwrap()
        .to_diversifiable_full_viewing_key()
        .default_address()
        .1
}

pub fn sender_tkey() -> secp256k1::SecretKey {
    secp256k1::SecretKey::from_secret_bytes([0x11; 32]).unwrap()
}

pub fn foreign_taddr() -> TransparentAddress {
    TransparentAddress::PublicKeyHash([0x42; 20])
}

fn orchard_note(
    recipient: orchard::Address,
    value: u64,
    version: NoteVersion,
    tag: u8,
) -> orchard::Note {
    let rho = Rho::from_bytes(&[tag; 32].map(|b| b & 0x3f)).unwrap();
    for k in 0u8..=255 {
        let rseed = RandomSeed::from_bytes([k ^ tag; 32], &rho);
        if let Some(rseed) = Option::<RandomSeed>::from(rseed) {
            let note = orchard::Note::from_parts(
                recipient,
                orchard::value::NoteValue::from_raw(value),
                rho,
                rseed,
                version,
            );
            if let Some(n) = Option::<orchard::Note>::from(note) {
                return n;
            }
        }
    }
    panic!("no valid rseed");
}

fn orchard_path(note: &orchard::Note) -> (orchard::Anchor, OrchardPath) {
    let auth = core::array::from_fn(|i| MerkleHashOrchard::empty_root(Level::from(i as u8)));
    let path = OrchardPath::from_parts(0, auth);
    let cmx: orchard::note::ExtractedNoteCommitment = note.commitment().into();
    (path.root(cmx), path)
}

fn sapling_note(
    recipient: sapling_crypto::PaymentAddress,
    value: u64,
    tag: u8,
) -> sapling_crypto::Note {
    sapling_crypto::Note::from_parts(
        recipient,
        sapling_crypto::value::NoteValue::from_raw(value),
        sapling_crypto::Rseed::AfterZip212([tag; 32]),
    )
}

fn sapling_path(
    note: &sapling_crypto::Note,
) -> (sapling_crypto::Anchor, sapling_crypto::MerklePath) {
    let elems = (0u8..32)
        .map(|l| sapling_crypto::Node::empty_root(Level::from(l)))
        .collect();
    let path = sapling_crypto::MerklePath::from_parts(elems, 0u64.into()).unwrap();
    let anchor = path.root(sapling_crypto::Node::from_cmu(&note.cmu()));
    (anchor.into(), path)
}

fn memo(text: &str) -> MemoBytes {
    if text.is_empty() {
        return MemoBytes::empty();
    }
    MemoBytes::from_bytes(text.as_bytes()).unwrap()
}

fn zat(v: u64) -> Zatoshis {
    Zatoshis::const_from_u64(v)
}

/// What a fixture transaction contains. Spends are the sender's.
#[derive(Clone, Default)]
pub struct Spec {
    pub height: u32,
    pub orchard_in: Option<u64>,
    pub sapling_in: Option<u64>,
    /// One transparent P2PKH input from the sender's key.
    pub transparent_in: Option<u64>,
    /// Orchard sends to the foreign address (value, memo).
    pub orchard_out: Vec<(u64, &'static str)>,
    pub ironwood_out: Vec<(u64, &'static str)>,
    pub sapling_out: Vec<(u64, &'static str)>,
    /// Transparent sends to a foreign P2PKH address.
    pub transparent_out: Vec<u64>,
    /// Pool for the change output (value computed to balance with ZIP-317).
    pub change: Option<ChangePool>,
    /// Recipient of the Orchard and Ironwood sends, instead of the foreign key.
    pub orchard_to: Option<orchard::Address>,
    /// Recipient of the Sapling sends, instead of the foreign key.
    pub sapling_to: Option<sapling_crypto::PaymentAddress>,
    /// Pad the Orchard bundle to at least this many actions.
    pub orchard_pad_to: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangePool {
    Orchard,
    Ironwood,
    Sapling,
    /// Back to the sender's transparent address, with a BIP 44 derivation
    /// claim set by the Updater.
    Transparent,
}

fn builder_for(spec: &Spec, change_value: u64) -> Builder<Network, ()> {
    let fvk = sender_orchard();
    let dfvk = sender_sapling();
    let ours_ext = fvk.address_at(0u32, Scope::External);

    let o_note = spec
        .orchard_in
        .map(|v| orchard_note(ours_ext, v, NoteVersion::V2, 1));
    let s_note = spec
        .sapling_in
        .map(|v| sapling_note(dfvk.default_address().1, v, 3));
    let o_path = o_note.as_ref().map(orchard_path);
    let s_path = s_note.as_ref().map(sapling_path);
    let uses_orchard = o_note.is_some()
        || !spec.orchard_out.is_empty()
        || spec.change == Some(ChangePool::Orchard);
    let uses_ironwood = !spec.ironwood_out.is_empty() || spec.change == Some(ChangePool::Ironwood);
    let uses_sapling = s_note.is_some()
        || !spec.sapling_out.is_empty()
        || spec.change == Some(ChangePool::Sapling);
    let o_anchor = uses_orchard.then(|| {
        o_path
            .as_ref()
            .map_or(orchard::Anchor::empty_tree(), |p| p.0)
    });
    let i_anchor = uses_ironwood.then(orchard::Anchor::empty_tree);
    let s_anchor = uses_sapling.then(|| {
        s_path
            .as_ref()
            .map_or(sapling_crypto::Anchor::empty_tree(), |p| p.0)
    });
    let mut b = Builder::new(
        MAIN,
        BlockHeight::from_u32(spec.height),
        BuildConfig::Standard {
            sapling_anchor: s_anchor,
            orchard_anchor: o_anchor,
            ironwood_anchor: i_anchor,
            orchard_padding: BundlePadding {
                bundle_required: false,
                pad_to_minimum: spec.orchard_pad_to,
            },
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    if let (Some(n), Some((_, p))) = (o_note, o_path) {
        b.add_orchard_spend::<zip317::FeeRule>(fvk.clone(), n, p)
            .unwrap();
    }
    if let (Some(n), Some((_, p))) = (s_note, s_path) {
        b.add_sapling_spend::<zip317::FeeRule>(dfvk.fvk().clone(), n, p)
            .unwrap();
    }
    let tpk = sender_tkey().public_key();
    let taddr = TransparentAddress::from_pubkey(&tpk);
    if let Some(v) = spec.transparent_in {
        let utxo = zcash_transparent::bundle::OutPoint::new([1; 32], 0);
        let coin = zcash_transparent::bundle::TxOut::new(zat(v), taddr.script().into());
        b.add_transparent_p2pkh_input(tpk, utxo, coin).unwrap();
    }
    let to = spec.orchard_to.unwrap_or_else(foreign_orchard_address);
    let ovk = Some(fvk.to_ovk(Scope::External));
    for (v, m) in &spec.orchard_out {
        b.add_orchard_output::<zip317::FeeRule>(ovk.clone(), to, zat(*v), memo(m))
            .unwrap();
    }
    for (v, m) in &spec.ironwood_out {
        b.add_ironwood_output::<zip317::FeeRule>(ovk.clone(), to, zat(*v), memo(m))
            .unwrap();
    }
    for (v, m) in &spec.sapling_out {
        b.add_sapling_output::<zip317::FeeRule>(
            Some(dfvk.to_ovk(Scope::External)),
            spec.sapling_to.unwrap_or_else(foreign_sapling_address),
            zat(*v),
            memo(m),
        )
        .unwrap();
    }
    for v in &spec.transparent_out {
        b.add_transparent_output(&foreign_taddr(), zat(*v)).unwrap();
    }
    match spec.change {
        Some(ChangePool::Orchard) => b
            .add_orchard_output::<zip317::FeeRule>(
                Some(fvk.to_ovk(Scope::Internal)),
                fvk.address_at(0u32, Scope::Internal),
                zat(change_value),
                MemoBytes::empty(),
            )
            .unwrap(),
        Some(ChangePool::Ironwood) => b
            .add_ironwood_output::<zip317::FeeRule>(
                Some(fvk.to_ovk(Scope::Internal)),
                fvk.address_at(0u32, Scope::Internal),
                zat(change_value),
                MemoBytes::empty(),
            )
            .unwrap(),
        Some(ChangePool::Sapling) => b
            .add_sapling_output::<zip317::FeeRule>(
                Some(dfvk.to_ovk(Scope::Internal)),
                dfvk.change_address().1,
                zat(change_value),
                MemoBytes::empty(),
            )
            .unwrap(),
        Some(ChangePool::Transparent) => {
            b.add_transparent_output(&taddr, zat(change_value)).unwrap()
        }
        None => {}
    }
    b
}

/// The ZIP-317 fee the builder requires for `spec`.
pub fn fee_for(spec: &Spec) -> u64 {
    u64::from(
        builder_for(spec, 1)
            .get_fee(&zip317::FeeRule::standard())
            .unwrap(),
    )
}

/// Change value that balances `spec` at the ZIP-317 fee.
pub fn change_for(spec: &Spec) -> u64 {
    let ins = spec.orchard_in.unwrap_or(0)
        + spec.sapling_in.unwrap_or(0)
        + spec.transparent_in.unwrap_or(0);
    let outs: u64 = spec.orchard_out.iter().map(|o| o.0).sum::<u64>()
        + spec.ironwood_out.iter().map(|o| o.0).sum::<u64>()
        + spec.sapling_out.iter().map(|o| o.0).sum::<u64>()
        + spec.transparent_out.iter().sum::<u64>();
    ins - outs - fee_for(spec)
}

/// Build, finalize IO, and add transparent derivation metadata (the input's
/// key, and the change output's claim if any). Returns the serialized PCZT.
pub fn build(spec: &Spec) -> Vec<u8> {
    let change = if spec.change.is_some() {
        change_for(spec)
    } else {
        0
    };
    let b = builder_for(spec, change);
    let PcztResult { pczt_parts, .. } = b
        .build_for_pczt(rng(), &zip317::FeeRule::standard())
        .unwrap();
    let pczt = Creator::build_from_parts(pczt_parts).unwrap();
    let pczt = IoFinalizer::new(pczt).finalize_io(rng()).unwrap();
    let tpk = sender_tkey().public_key().serialize();
    let taddr = TransparentAddress::from_pubkey(&sender_tkey().public_key());
    let change_out = pczt
        .transparent()
        .outputs()
        .iter()
        .position(|o| *o.script_pubkey() == p2pkh_script(&taddr));
    let fp = [0xab; 32];
    let path = |change: u32| vec![44 | 1 << 31, 133 | 1 << 31, 1 << 31, change, 0];
    let pczt = Updater::new(pczt)
        .update_transparent_with(|mut u| {
            if spec.transparent_in.is_some() {
                u.update_input_with(0, |mut iu| {
                    iu.set_bip32_derivation(tpk, Bip32Derivation::parse(fp, path(0)).unwrap());
                    Ok(())
                })?;
            }
            if let Some(i) = change_out.filter(|_| spec.change == Some(ChangePool::Transparent)) {
                u.update_output_with(i, |mut ou| {
                    ou.set_bip32_derivation(tpk, Bip32Derivation::parse(fp, path(1)).unwrap());
                    Ok(())
                })?;
            }
            Ok(())
        })
        .unwrap()
        .finish();
    pczt.serialize().unwrap()
}

/// The committed fixtures: (file stem, spec).
pub fn fixtures() -> Vec<(&'static str, Spec)> {
    vec![
        (
            "orchard_send_v5",
            Spec {
                height: PRE_NU6_3,
                orchard_in: Some(100_000_000),
                orchard_out: vec![(100_000, "thanks"), (200_000, "caf\u{e9}")],
                change: Some(ChangePool::Orchard),
                ..Spec::default()
            },
        ),
        (
            "ironwood_send_v6",
            Spec {
                height: POST_NU6_3,
                orchard_in: Some(10_000_000),
                ironwood_out: vec![(1_000_000, "iw")],
                change: Some(ChangePool::Ironwood),
                ..Spec::default()
            },
        ),
        (
            "sapling_send_v5",
            Spec {
                height: PRE_NU6_3,
                sapling_in: Some(50_000_000),
                sapling_out: vec![(1_000_000, "rent")],
                change: Some(ChangePool::Sapling),
                ..Spec::default()
            },
        ),
        (
            "transparent_mix_v5",
            Spec {
                height: PRE_NU6_3,
                transparent_in: Some(2_000_000),
                orchard_out: vec![(500_000, "")],
                transparent_out: vec![300_000],
                change: Some(ChangePool::Transparent),
                ..Spec::default()
            },
        ),
    ]
}
