use std::collections::BTreeMap;

use clap::Args as ClapArgs;
use generated::parser::{
    ChainMetadata, Idl, SolanaIdlType, SolanaMetadata, chain_metadata::Metadata,
};
use visualsign::registry::{Chain, TransactionConverterRegistry};

use crate::mapping_parser;

/// CLI arguments specific to Solana.
#[derive(ClapArgs, Debug, Default, Clone)]
pub struct SolanaArgs {
    /// Map custom IDL JSON file to a Solana program.
    /// Format: `IdlName:/path/to/idl.json:base58_program_id`. Can be used multiple times.
    #[arg(
        long = "idl-json-mappings",
        value_name = "IDL_NAME:FILE_PATH:PROGRAM_ID"
    )]
    pub idl_json_mappings: Vec<String>,
}

/// [`crate::ChainPlugin`] implementation for Solana.
pub struct SolanaPlugin {
    args: SolanaArgs,
}

impl SolanaPlugin {
    /// Creates a new `SolanaPlugin` with the given CLI args.
    #[must_use]
    pub fn new(args: SolanaArgs) -> Self {
        Self { args }
    }
}

impl crate::ChainPlugin for SolanaPlugin {
    fn chain(&self) -> Chain {
        Chain::Solana
    }

    fn register(&self, registry: &mut TransactionConverterRegistry) {
        registry.register::<visualsign_solana::SolanaTransactionWrapper, _>(
            Chain::Solana,
            visualsign_solana::SolanaVisualSignConverter,
        );
    }

    fn create_metadata(&self, _network: Option<String>) -> Result<Option<ChainMetadata>, String> {
        Ok(create_chain_metadata(&self.args.idl_json_mappings))
    }
}

fn build_idl_mappings_from_files(idl_json_mappings: &[String]) -> (BTreeMap<String, Idl>, usize) {
    mapping_parser::load_mappings(
        idl_json_mappings,
        "IDL",
        "JupiterSwap:/home/user/jupiter.json:JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
        "ProgramId",
        |components, json| Idl {
            value: json,
            idl_type: Some(SolanaIdlType::Anchor as i32),
            idl_version: None,
            signature: None,
            program_name: Some(components.name.clone()),
        },
    )
}

/// Creates Solana chain metadata from IDL mappings.
/// Returns `None` if no IDL mappings are provided.
#[must_use]
pub fn create_chain_metadata(idl_json_mappings: &[String]) -> Option<ChainMetadata> {
    if idl_json_mappings.is_empty() {
        return None;
    }

    eprintln!("Loading custom IDLs:");
    let (idl_mappings, valid_count) = build_idl_mappings_from_files(idl_json_mappings);
    eprintln!(
        "Successfully loaded {}/{} IDL mappings\n",
        valid_count,
        idl_json_mappings.len()
    );

    Some(ChainMetadata {
        metadata: Some(Metadata::Solana(SolanaMetadata {
            network_id: None,
            idl: None,
            idl_mappings,
        })),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn write_temp_json(name: &str, content: &str) -> std::path::PathBuf {
        crate::test_utils::write_temp_json("vsp_sol_tests", name, content)
    }

    #[test]
    fn test_create_chain_metadata_empty_returns_none() {
        assert!(create_chain_metadata(&[]).is_none());
    }

    #[test]
    fn test_create_chain_metadata_with_idl_mapping() {
        let path = write_temp_json("jupiter.json", r#"{"version":"0.1.0","name":"jupiter"}"#);
        let mappings = vec![format!(
            "Jupiter:{}:JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
            path.display()
        )];

        let meta = create_chain_metadata(&mappings).expect("should return Some");
        let Metadata::Solana(sol) = meta.metadata.unwrap() else {
            panic!("expected Solana metadata");
        };
        assert_eq!(sol.idl_mappings.len(), 1);

        let idl = sol
            .idl_mappings
            .get("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")
            .expect("mapping present");
        assert!(idl.value.contains("jupiter"));
        assert_eq!(idl.idl_type, Some(SolanaIdlType::Anchor as i32));
        assert_eq!(idl.program_name.as_deref(), Some("Jupiter"));
        assert!(idl.signature.is_none());
        assert!(idl.idl_version.is_none());
    }

    #[test]
    fn test_create_chain_metadata_invalid_file_skipped() {
        let mappings = vec!["Bad:/nonexistent/idl.json:ProgramId123".to_string()];
        let meta = create_chain_metadata(&mappings).expect("should return Some");
        let Metadata::Solana(sol) = meta.metadata.unwrap() else {
            panic!("expected Solana metadata");
        };
        assert!(sol.idl_mappings.is_empty());
    }

    #[test]
    fn test_create_chain_metadata_multiple_idl_mappings() {
        let path1 = write_temp_json("idl_a.json", r#"{"name":"a"}"#);
        let path2 = write_temp_json("idl_b.json", r#"{"name":"b"}"#);
        let mappings = vec![
            format!(
                "A:{}:Prog1111111111111111111111111111111111111111",
                path1.display()
            ),
            format!(
                "B:{}:Prog2222222222222222222222222222222222222222",
                path2.display()
            ),
        ];

        let meta = create_chain_metadata(&mappings).expect("should return Some");
        let Metadata::Solana(sol) = meta.metadata.unwrap() else {
            panic!("expected Solana metadata");
        };
        assert_eq!(sol.idl_mappings.len(), 2);
        assert!(
            sol.idl_mappings
                .contains_key("Prog1111111111111111111111111111111111111111")
        );
        assert!(
            sol.idl_mappings
                .contains_key("Prog2222222222222222222222222222222222222222")
        );
    }

    #[test]
    fn test_create_chain_metadata_sets_no_network_or_legacy_idl() {
        let path = write_temp_json("meta_check.json", r#"{"ok": true}"#);
        let mappings = vec![format!("Test:{}:ProgXYZ", path.display())];

        let meta = create_chain_metadata(&mappings).expect("should return Some");
        let Metadata::Solana(sol) = meta.metadata.unwrap() else {
            panic!("expected Solana metadata");
        };
        assert!(sol.network_id.is_none());
        assert!(sol.idl.is_none());
    }

    #[test]
    fn test_create_chain_metadata_mixed_valid_and_invalid() {
        let path = write_temp_json("sol_good.json", r#"{"works": true}"#);
        let mappings = vec![
            "bad-format".to_string(),
            format!("Good:{}:GoodProg", path.display()),
            "Also:/missing/file.json:BadProg".to_string(),
        ];

        let meta = create_chain_metadata(&mappings).expect("should return Some");
        let Metadata::Solana(sol) = meta.metadata.unwrap() else {
            panic!("expected Solana metadata");
        };
        assert_eq!(sol.idl_mappings.len(), 1);
        assert!(sol.idl_mappings.contains_key("GoodProg"));
    }
}
