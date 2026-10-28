#![no_main]

use libfuzzer_sys::fuzz_target;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::message::{Message, MessageHeader};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::Transaction;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_solana::transaction_to_visual_sign;

/// Token-2022 program id (TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb).
const TOKEN_2022: [u8; 32] = [
    6, 221, 246, 225, 238, 117, 143, 222, 24, 66, 93, 188, 228, 108, 205, 218, 182, 26, 252, 77,
    131, 185, 13, 39, 254, 189, 249, 40, 216, 161, 139, 252,
];

/// Account keys in the message: fee payer, Token-2022, then these.
const EXTRA_KEYS: u8 = 10;

// One Token-2022 instruction through the full legacy-transaction pipeline.
// Input: [n, n account indices, instruction data...]. Index i names message
// account i + 2 (wrapping), so 0..=9 are the ten plain accounts, 10..=253 are
// out of range, 254 is the fee payer and 255 is Token-2022 itself. Seeds in
// `seeds/fuzz_token_2022_instruction/` come from the token_2022 fixtures and
// solana-program/token's p-token conformance corpus.
//
// Asserts no panic, and that the payload stays under a loose size ceiling
// (run with -malloc_limit_mb / -rss_limit_mb to bound allocation too).
fuzz_target!(|input: &[u8]| {
    let Some((&n, rest)) = input.split_first() else {
        return;
    };
    let n = usize::from(n).min(rest.len());
    let (indices, data) = rest.split_at(n);

    let mut account_keys = vec![
        Pubkey::new_from_array([1; 32]),
        Pubkey::new_from_array(TOKEN_2022),
    ];
    account_keys.extend((0..EXTRA_KEYS).map(|i| Pubkey::new_from_array([0x40 + i; 32])));
    let message = Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        account_keys,
        recent_blockhash: Hash::default(),
        instructions: vec![CompiledInstruction {
            program_id_index: 1,
            accounts: indices.iter().map(|i| i.wrapping_add(2)).collect(),
            data: data.to_vec(),
        }],
    };
    let tx = Transaction {
        signatures: vec![Signature::default()],
        message,
    };
    if let Ok(payload) = transaction_to_visual_sign(tx, VisualSignOptions::default()) {
        let json = payload.to_json().expect("payload serializes");
        // A loose ceiling against blow-up (output growing much faster than
        // the input), not a proportionality check: typical payloads are a
        // few KB, well under the 64 KiB base.
        let bound = 64 * 1024 + 8 * data.len() + 256 * indices.len();
        assert!(
            json.len() <= bound,
            "payload {} bytes for {} input bytes",
            json.len(),
            input.len()
        );
    }
});
