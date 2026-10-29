//! Regenerates the committed PCZT fixtures. The builder draws OS randomness,
//! so output differs on every run; the golden tests read the committed bytes.
//!
//! ZCASH_REGEN_FIXTURES=1 cargo test -p visualsign-zcash --test regenerate_fixtures -- --ignored
//!
//! Then refresh the goldens:
//! REGEN_FIXTURES=1 cargo test -p visualsign-zcash --test fixtures golden
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/builder.rs"]
mod builder;

use std::fs;
use std::path::PathBuf;

#[test]
#[ignore = "writes fixtures; set ZCASH_REGEN_FIXTURES=1"]
fn regenerate_fixtures() {
    if std::env::var("ZCASH_REGEN_FIXTURES").is_err() {
        println!("ZCASH_REGEN_FIXTURES not set; skipping");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    for (name, spec) in builder::fixtures() {
        let bytes = builder::build(&spec);
        let fee = builder::fee_for(&spec);
        let change = builder::change_for(&spec);
        fs::write(dir.join(format!("{name}.input")), hex::encode(bytes) + "\n").unwrap();
        println!("{name}: fee {fee} change {change}");
    }
}
