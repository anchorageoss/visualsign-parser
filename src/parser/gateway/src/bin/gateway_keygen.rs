//! Dev helper: generate a fresh P256 keypair for `GATEWAY_SIGNING_KEY_FILE`
//! and print the public hex so it can be pasted into parser_app's
//! `GATEWAY_SIGNING_PUBKEY_HEX`.
//!
//! Usage:
//!   cargo run -p parser_gateway --bin gateway_keygen -- /path/to/key.json
//!
//! The file is written as `{"private": "<hex>", "public": "<hex>"}`. The
//! same hex parser_app pins is printed to stdout.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qos_p256::sign::P256SignPair;
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: gateway_keygen <output-path>")
        .into();

    let pair = P256SignPair::generate();
    let priv_hex = qos_hex::encode(&pair.to_bytes());
    let pub_hex = qos_hex::encode(&pair.public_key().to_bytes());

    let body = json!({ "private": priv_hex, "public": pub_hex });
    std::fs::write(&out, serde_json::to_vec_pretty(&body).unwrap()).expect("write key file");

    println!("Wrote {}", out.display());
    println!("GATEWAY_SIGNING_PUBKEY_HEX={pub_hex}");
}
