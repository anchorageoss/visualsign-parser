//! Report the fields an Anchorage wallet cannot render in a SignablePayload JSON.
//!
//! Usage:
//!   cargo run -p visualsign --features diagnostics \
//!     --example check_anchorage_render -- <payload.json>

use std::fs;

use visualsign::SignablePayload;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: check_anchorage_render <payload.json>")?;
    let json = fs::read_to_string(&path)?;
    let payload: SignablePayload = serde_json::from_str(&json).map_err(|e| {
        format!(
            "failed to parse {path} as a SignablePayload: {e}\n\
             hint: a \"diagnostic\" field requires rebuilding with `--features diagnostics`"
        )
    })?;

    let findings = payload.anchorage_render_findings();
    if findings.is_empty() {
        println!("{path}: CLEAN (0 unrenderable fields)");
        return Ok(());
    }

    println!("{path}: {} unrenderable field(s):", findings.len());
    for f in &findings {
        println!(
            "  - {} [{}] \"{}\" -> {:?}",
            f.path, f.field_type, f.label, f.reason
        );
    }
    Ok(())
}
