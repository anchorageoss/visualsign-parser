#![no_main]

use libfuzzer_sys::fuzz_target;
use visualsign_solana::versioned_transaction_to_visual_sign;
use visualsign::vsptrait::VisualSignOptions;
use solana_sdk::transaction::VersionedTransaction;

// Try to deserialize arbitrary bytes as a VersionedTransaction then pass it
// through the full visualsign-solana stack. Exercises the versioned transaction
// path including address table lookup handling and IDL dispatch.
fuzz_target!(|data: &[u8]| {
    if let Ok(tx) = bincode::deserialize::<VersionedTransaction>(data) {
        let _ = versioned_transaction_to_visual_sign(tx, VisualSignOptions::default());
    }
});
