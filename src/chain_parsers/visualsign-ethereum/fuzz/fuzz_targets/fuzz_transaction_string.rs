#![no_main]

use libfuzzer_sys::fuzz_target;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::transaction_string_to_visual_sign;

// Feed arbitrary bytes as a transaction string into the full visualsign-ethereum
// stack. Exercises hex/RLP and JSON transaction parsing, dispatch by `to`
// address, calldata decoding and SignablePayload construction.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = transaction_string_to_visual_sign(s, VisualSignOptions::default());
    }
});
