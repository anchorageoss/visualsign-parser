//! Differential tests: `visualsign_near::primitives` against `near-primitives`
//! 0.35.1 / `near-crypto` 0.35.1 (dev-dependencies only).
//!
//! For every input byte string, decoding with the local types must succeed
//! exactly when the upstream types succeed, fail with the same error message
//! when both fail, and re-encode to exactly the input bytes when both succeed.
//! Inputs are the render-snapshot corpus plus truncations, byte mutations and
//! random strings from a small inline PRNG (fixed seeds, so runs are
//! reproducible).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use borsh::{BorshDeserialize, BorshSerialize};
use near_crypto::{ED25519PublicKey, KeyType, Secp256K1PublicKey};
use visualsign_near::primitives as local;

const SNAPSHOT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/onchain_render_snapshot.json"
);

/// xorshift64*: deterministic, no dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Bytes that sit on decode boundaries: enum tags, key types, account-id
    /// separators and case, the nested-delegate tag.
    fn interesting_byte(&mut self) -> u8 {
        const INTERESTING: &[u8] = &[
            0, 1, 2, 3, 7, 8, 9, 11, 12, 13, 14, 15, 0x1f, 0x20, 0x40, 0x7f, 0x80, 0xe0, 0xff,
            b'A', b'Z', b'a', b'z', b'0', b'9', b'-', b'_', b'.', b'@', b' ',
        ];
        if self.below(3) == 0 {
            self.next() as u8
        } else {
            INTERESTING[self.below(INTERESTING.len())]
        }
    }
}

fn snapshot_inputs() -> Vec<Vec<u8>> {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(SNAPSHOT_PATH).unwrap()).unwrap();
    entries
        .iter()
        .map(|e| hex::decode(e["hex"].as_str().unwrap()).unwrap())
        .collect()
}

/// Inputs both sides accepted but neither re-encodes to the input bytes.
static NON_CANONICAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether the local decoder sees a DeterministicStateInit action anywhere in
/// `bytes` decoded as a transaction, signed transaction or action -- the only
/// place a `BTreeMap` (and so a non-canonical accepted encoding) can occur.
fn has_state_init(bytes: &[u8]) -> bool {
    let in_actions = |actions: &[local::Action]| {
        actions
            .iter()
            .any(|a| matches!(a, local::Action::DeterministicStateInit(_)))
    };
    borsh::from_slice::<local::Transaction>(bytes).is_ok_and(|t| in_actions(t.actions()))
        || borsh::from_slice::<local::SignedTransaction>(bytes)
            .is_ok_and(|t| in_actions(t.transaction.actions()))
        || borsh::from_slice::<local::Action>(bytes).is_ok_and(|a| in_actions(&[a]))
        || borsh::from_slice::<local::NonDelegateAction>(bytes)
            .is_ok_and(|a| in_actions(&[a.into()]))
}

/// Decode `bytes` as `U` (upstream) and `L` (local) and compare outcomes.
/// Returns whether both succeeded.
fn check<U, L>(what: &str, bytes: &[u8]) -> bool
where
    U: BorshDeserialize + BorshSerialize,
    L: BorshDeserialize + BorshSerialize,
{
    let upstream = borsh::from_slice::<U>(bytes);
    let ours = borsh::from_slice::<L>(bytes);
    match (&upstream, &ours) {
        (Ok(u), Ok(l)) => {
            // Both re-encode identically. That is the input itself, except
            // where upstream accepts a non-canonical encoding: borsh decodes a
            // `BTreeMap` (DeterministicStateInit data) with unsorted or
            // duplicate keys and re-encodes it sorted and deduplicated.
            let upstream_bytes = borsh::to_vec(u).unwrap();
            assert_eq!(
                borsh::to_vec(l).unwrap(),
                upstream_bytes,
                "{what}: local re-encoding differs from upstream for {}",
                hex::encode(bytes)
            );
            if upstream_bytes != bytes {
                NON_CANONICAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                assert!(
                    has_state_init(bytes),
                    "{what}: upstream does not round-trip {}",
                    hex::encode(bytes)
                );
            }
            true
        }
        (Err(u), Err(l)) => {
            assert_eq!(
                (u.kind(), u.to_string()),
                (l.kind(), l.to_string()),
                "{what}: error differs for {}",
                hex::encode(bytes)
            );
            false
        }
        _ => panic!(
            "{what}: upstream ok={} local ok={} for {} (upstream: {:?}, local: {:?})",
            upstream.is_ok(),
            ours.is_ok(),
            hex::encode(bytes),
            upstream.as_ref().err(),
            ours.as_ref().err(),
        ),
    }
}

/// Every top-level type the parser decodes, plus the leaf types whose custom
/// borsh impls are mirrored, so a mismatch is reported at the narrowest type.
fn check_all(bytes: &[u8], stats: &mut Stats) {
    use near_primitives::action::delegate::NonDelegateAction;
    stats.inputs += 1;
    if check::<near_primitives::transaction::Transaction, local::Transaction>("Transaction", bytes)
    {
        stats.tx_ok += 1;
    }
    if check::<near_primitives::transaction::SignedTransaction, local::SignedTransaction>(
        "SignedTransaction",
        bytes,
    ) {
        stats.signed_ok += 1;
    }
    if check::<near_primitives::action::Action, local::Action>("Action", bytes) {
        stats.action_ok += 1;
    }
    check::<NonDelegateAction, local::NonDelegateAction>("NonDelegateAction", bytes);
    check::<near_crypto::PublicKey, local::PublicKey>("PublicKey", bytes);
    check::<near_crypto::Signature, local::Signature>("Signature", bytes);
    check::<near_primitives::types::AccountId, local::AccountId>("AccountId", bytes);
}

#[derive(Default, Debug)]
struct Stats {
    inputs: usize,
    tx_ok: usize,
    signed_ok: usize,
    action_ok: usize,
}

#[test]
fn snapshot_corpus_and_truncations_match_upstream() {
    let mut stats = Stats::default();
    for input in snapshot_inputs() {
        for end in 0..=input.len() {
            check_all(&input[..end], &mut stats);
        }
        // Every action, as a standalone value, is a suffix of a single-action
        // transaction; walking suffixes reaches each one.
        for start in 1..input.len() {
            check_all(&input[start..], &mut stats);
        }
    }
    eprintln!("snapshot+truncations: {stats:?}");
    // Guard against a corpus that silently stops exercising success paths.
    assert!(stats.tx_ok >= 60, "{stats:?}");
    assert!(stats.signed_ok >= 4, "{stats:?}");
    assert!(stats.action_ok >= 25, "{stats:?}");
}

#[test]
fn mutated_corpus_matches_upstream() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut stats = Stats::default();
    for input in snapshot_inputs() {
        for _ in 0..300 {
            let mut bytes = input.clone();
            for _ in 0..=rng.below(3) {
                if bytes.is_empty() {
                    break;
                }
                let at = rng.below(bytes.len());
                match rng.below(4) {
                    0 | 1 => bytes[at] = rng.interesting_byte(),
                    2 => bytes.insert(at, rng.interesting_byte()),
                    _ => {
                        bytes.remove(at);
                    }
                }
            }
            check_all(&bytes, &mut stats);
        }
    }
    eprintln!(
        "mutated: {stats:?}, non-canonical accepted: {}",
        NON_CANONICAL.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert!(stats.tx_ok > 1000, "{stats:?}");
}

#[test]
fn random_bytes_match_upstream() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut stats = Stats::default();
    for _ in 0..20_000 {
        let len = rng.below(160);
        let bytes: Vec<u8> = (0..len).map(|_| rng.interesting_byte()).collect();
        check_all(&bytes, &mut stats);
    }
    // Random tag-prefixed blobs: steer toward each key and action decoder.
    for tag in 0u8..=15 {
        for len in [0usize, 1, 31, 32, 33, 63, 64, 65, 66, 80] {
            let mut bytes = vec![tag];
            bytes.extend((0..len).map(|_| rng.next() as u8));
            check_all(&bytes, &mut stats);
            // Signature edge: last ed25519 byte with high bits set or clear.
            if len == 64 {
                for last in [0x00u8, 0x1f, 0x20, 0x80, 0xe0] {
                    bytes[64] = last;
                    check_all(&bytes, &mut stats);
                }
            }
        }
    }
}

/// Account-id validation, directly: lengths 0..=70 over an alphabet that
/// includes every separator, upper case and non-ASCII.
#[test]
fn account_id_validation_matches_upstream() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let alphabet: Vec<&str> = vec![
        "a", "z", "0", "9", "-", "_", ".", "A", "@", " ", "\u{e9}", "\u{0}",
    ];
    let mut stats = Stats::default();
    for len in 0..=70usize {
        for _ in 0..60 {
            let s: String = (0..len)
                .map(|_| alphabet[rng.below(alphabet.len())])
                .collect();
            let bytes = borsh::to_vec(&s).unwrap();
            check::<near_primitives::types::AccountId, local::AccountId>("AccountId", &bytes);
            check_all(&bytes, &mut stats);
            // serde path (JSON args of ft_transfer & co.).
            let json = serde_json::to_string(&s).unwrap();
            let upstream = serde_json::from_str::<near_primitives::types::AccountId>(&json);
            let ours = serde_json::from_str::<local::AccountId>(&json);
            assert_eq!(
                upstream
                    .as_ref()
                    .map(|a| a.to_string())
                    .map_err(|e| e.to_string()),
                ours.as_ref()
                    .map(|a| a.to_string())
                    .map_err(|e| e.to_string()),
                "serde AccountId {s:?}"
            );
        }
    }
    // Invalid UTF-8 inside the length-prefixed string.
    check_all(&[2, 0, 0, 0, 0xc3, 0x28], &mut stats);
}

/// The rendered key strings come from `Display`, so it must match
/// `near-crypto`'s exactly for both curves.
#[test]
fn public_key_and_signature_display_match_upstream() {
    let mut rng = Rng(42);
    for _ in 0..500 {
        let mut ed = [0u8; 32];
        ed.iter_mut().for_each(|b| *b = rng.next() as u8);
        let mut secp = [0u8; 64];
        secp.iter_mut().for_each(|b| *b = rng.next() as u8);
        for (upstream, ours) in [
            (
                near_crypto::PublicKey::ED25519(ED25519PublicKey(ed)),
                local::PublicKey::ED25519(ed),
            ),
            (
                near_crypto::PublicKey::SECP256K1(Secp256K1PublicKey::from(secp)),
                local::PublicKey::SECP256K1(secp),
            ),
        ] {
            assert_eq!(upstream.to_string(), ours.to_string());
            assert_eq!(
                borsh::to_vec(&upstream).unwrap(),
                borsh::to_vec(&ours).unwrap()
            );
        }
        let mut sig = [0u8; 65];
        sig.iter_mut().for_each(|b| *b = rng.next() as u8);
        let upstream = near_crypto::Signature::from_parts(KeyType::SECP256K1, &sig).unwrap();
        let ours = local::Signature::SECP256K1(sig);
        assert_eq!(upstream.to_string(), ours.to_string());
    }
    for (key_type, local_type) in [
        (KeyType::ED25519, local::KeyType::ED25519),
        (KeyType::SECP256K1, local::KeyType::SECP256K1),
    ] {
        assert_eq!(
            near_crypto::PublicKey::empty(key_type).to_string(),
            local::PublicKey::empty(local_type).to_string()
        );
    }
    // near-crypto's `Signature::empty` is `unimplemented!()` for secp256k1.
    assert_eq!(
        borsh::to_vec(&near_crypto::Signature::empty(KeyType::ED25519)).unwrap(),
        borsh::to_vec(&local::Signature::empty(local::KeyType::ED25519)).unwrap()
    );
}
