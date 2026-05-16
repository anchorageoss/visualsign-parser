//! Build-script-generated static descriptor table + lookup.

use crate::eip712::descriptor::Erc7730Descriptor;
use alloy_primitives::Address;
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/erc7730_embedded.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorKind {
    Eip712,
    Calldata,
}

#[derive(Debug)]
pub struct EmbeddedDescriptorEntry {
    pub entity: &'static str,
    pub source_path: &'static str,
    pub kind: DescriptorKind,
    /// `(chain_id, lowercased address hex with 0x)` pairs.
    pub deployments: &'static [(u64, &'static str)],
    pub primary_types: &'static [&'static str],
    pub json: &'static str,
}

static PARSED_CACHE: OnceLock<Vec<OnceLock<Erc7730Descriptor>>> = OnceLock::new();

fn cache() -> &'static Vec<OnceLock<Erc7730Descriptor>> {
    PARSED_CACHE.get_or_init(|| {
        (0..EMBEDDED_DESCRIPTORS.len())
            .map(|_| OnceLock::new())
            .collect()
    })
}

/// Find an EIP-712 descriptor for the given (chain_id, verifying_contract, primary_type).
pub fn find_eip712(
    chain_id: u64,
    verifying_contract: Address,
    primary_type: &str,
) -> Option<&'static Erc7730Descriptor> {
    let target = format!("{verifying_contract:?}").to_lowercase();
    // First pass: entries with explicit deployment match.
    for (i, entry) in EMBEDDED_DESCRIPTORS.iter().enumerate() {
        if entry.kind != DescriptorKind::Eip712 {
            continue;
        }
        if !entry
            .deployments
            .iter()
            .any(|(c, a)| *c == chain_id && *a == target)
        {
            continue;
        }
        if !entry.primary_types.contains(&primary_type) {
            continue;
        }
        return Some(load_cached(i, entry));
    }
    // Second pass: generic ERC descriptors (no deployments declared) that match by primaryType.
    // Type-shape verification (done in the registry layer) prevents false positives.
    for (i, entry) in EMBEDDED_DESCRIPTORS.iter().enumerate() {
        if entry.kind != DescriptorKind::Eip712 {
            continue;
        }
        if !entry.deployments.is_empty() {
            continue;
        }
        if !entry.primary_types.contains(&primary_type) {
            continue;
        }
        return Some(load_cached(i, entry));
    }
    None
}

fn load_cached(i: usize, entry: &'static EmbeddedDescriptorEntry) -> &'static Erc7730Descriptor {
    cache()[i].get_or_init(|| match serde_json::from_str(entry.json) {
        Ok(d) => d,
        Err(e) => {
            // Should not happen — build.rs validated all entries — but tolerate at runtime.
            log::error!(
                "embedded descriptor {} failed to deserialize at runtime: {e}",
                entry.source_path
            );
            Erc7730Descriptor::default()
        }
    })
}

/// Count of embedded descriptors. Useful for tests.
pub fn count() -> usize {
    EMBEDDED_DESCRIPTORS.len()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_descriptors() {
        // Submodule should be initialized; expect many descriptors.
        assert!(
            count() >= 1,
            "expected at least one embedded descriptor; is `static/eip7730/` initialized?"
        );
    }

    #[test]
    fn all_embedded_descriptors_parse() {
        for entry in EMBEDDED_DESCRIPTORS {
            let res: Result<Erc7730Descriptor, _> = serde_json::from_str(entry.json);
            assert!(
                res.is_ok(),
                "descriptor failed to parse: {} - {:?}",
                entry.source_path,
                res.err()
            );
        }
    }
}
