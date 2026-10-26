#![cfg(feature = "hyperliquid")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::Command;

#[test]
fn complete_multisig_requests_render_through_cli() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../chain_parsers/visualsign-hyperliquid/tests/multisig");
    for role in ["member", "leader"] {
        let input = root.join(format!("order-{role}.json"));
        let output = Command::new(env!("CARGO_BIN_EXE_parser_cli"))
            .args(["decode", "--chain", "hyperliquid", "--output", "json", "-t"])
            .arg(format!("@{}", input.display()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let text = payload.to_string();
        assert!(text.contains("1100"), "{text}");
        assert!(text.contains("0.2"), "{text}");
        assert!(text.contains("Testnet"), "{text}");
        assert!(text.contains("Native Signing Digest"), "{text}");
        assert!(text.contains("Caller-supplied membership"), "{text}");
    }
}
