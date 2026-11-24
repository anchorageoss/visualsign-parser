//! The preset list, over [`visualsign_solana::preset_idl_sources`]: the
//! single source of truth for what the nightly workflow re-fetches and
//! where a refreshed IDL has to land.

use anyhow::{Result, bail};
use solana_idl_fetch::json::{self, Json};

/// One preset IDL, in the shape the workflow matrix consumes.
#[derive(Clone, Debug)]
pub struct PresetEntry {
    /// Base58 program id the IDL decodes.
    pub program_id: String,
    /// Path of the committed IDL, relative to the repository root.
    pub idl_path: String,
}

impl PresetEntry {
    /// Path of the provenance record next to the committed IDL, relative
    /// to the repository root: `<idl>.json` -> `<idl>.provenance.json`.
    pub fn record_path(&self) -> String {
        match self.idl_path.strip_suffix(".json") {
            Some(stem) => format!("{stem}.provenance.json"),
            None => format!("{}.provenance.json", self.idl_path),
        }
    }
}

/// Every preset IDL, sorted by program id. Fails when empty: a preset list
/// with nothing in it means the join in `preset_idl_sources` broke, and the
/// workflow must not continue with an empty matrix.
pub fn preset_entries() -> Result<Vec<PresetEntry>> {
    let sources = visualsign_solana::preset_idl_sources();
    if sources.is_empty() {
        bail!("no preset IDLs found; nothing to sync");
    }
    Ok(sources
        .into_iter()
        .map(|source| {
            let idl_path = source.idl_path();
            PresetEntry {
                program_id: source.program_id,
                idl_path,
            }
        })
        .collect())
}

/// The `list-presets` output: a single-line JSON array of
/// `{"program_id": ..., "idl_path": ...}` objects, sorted by program id,
/// for the workflow's matrix (single-line so it survives one
/// `$GITHUB_OUTPUT` entry).
pub fn list_presets_json() -> Result<String> {
    let entries = preset_entries()?;
    let array = Json::Array(
        entries
            .iter()
            .map(|entry| {
                Json::object(vec![
                    ("program_id", Json::str(&entry.program_id)),
                    ("idl_path", Json::str(&entry.idl_path)),
                ])
            })
            .collect(),
    );
    Ok(json::canonical(&array))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Base58 alphabet minus `0`, `O`, `I`, `l`, the same character set the
    /// fetch workflow validates program ids against.
    fn is_base58(text: &str) -> bool {
        !text.is_empty()
            && text
                .bytes()
                .all(|b| matches!(b, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
    }

    #[test]
    fn listing_is_a_single_line_matrix_of_every_preset() {
        let text = list_presets_json().unwrap();
        assert!(
            !text.contains('\n'),
            "the matrix must be single-line: {text}"
        );
        let parsed = json::parse(&text).unwrap();
        let entries = parsed.as_array().unwrap();
        assert!(
            entries.len() >= 20,
            "expected every preset IDL, got {}",
            entries.len()
        );

        let mut ids: Vec<&str> = Vec::new();
        for entry in entries {
            let program_id = entry.get("program_id").and_then(Json::as_str).unwrap();
            let idl_path = entry.get("idl_path").and_then(Json::as_str).unwrap();
            assert!(is_base58(program_id), "{program_id} is not base58");
            assert!(
                idl_path.starts_with("src/chain_parsers/visualsign-solana/src/presets/"),
                "{idl_path} is not a preset IDL path"
            );
            assert!(idl_path.ends_with(".json"));
            assert!(!idl_path.contains(".provenance.json"));
            ids.push(program_id);
        }
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "the matrix must be sorted by program id");
        let mut unique = sorted.clone();
        unique.dedup();
        assert_eq!(sorted.len(), unique.len(), "program ids must be unique");
    }

    #[test]
    fn jupiter_and_marginfi_are_listed() {
        // One ENTRIES-registered preset (its IDL file is named after the
        // program, not the preset) and the one preset that registers its
        // IDL outside ENTRIES.
        let entries = preset_entries().unwrap();
        let jupiter = entries
            .iter()
            .find(|entry| entry.program_id == "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")
            .unwrap();
        assert_eq!(
            jupiter.idl_path,
            "src/chain_parsers/visualsign-solana/src/presets/jupiter_swap/jupiter_agg_v6.json"
        );
        assert_eq!(
            jupiter.record_path(),
            "src/chain_parsers/visualsign-solana/src/presets/jupiter_swap/jupiter_agg_v6.provenance.json"
        );
        let marginfi = entries
            .iter()
            .find(|entry| entry.program_id == "MFv2hWf31Z9kbCa1snEPdcgp7X3GK4Y9EtyMkRxBUGA")
            .unwrap();
        assert_eq!(
            marginfi.idl_path,
            "src/chain_parsers/visualsign-solana/src/presets/marginfi/marginfi.json"
        );
    }
}
