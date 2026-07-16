//! This is a script for generating rust types from proto definitions.
//! Generated types are written to the `generated` crate.
//!
//! This build script is not part of the workspace because it needs to be
//! able to run even if the workspace cannot compile

const PROTO_INCLUDE_PATH: &str = "../proto";
const GEN_DIR: &str = "./generated/src/generated";
const INCLUDE_FILE: &str = "_include.rs";
const DESCRIPTOR_PATH: &str = "./generated/src/generated/descriptor.bin";

const SERDE_DERIVE: &str = "#[cfg_attr(feature = \"serde_derive\", derive(::serde::Serialize, ::serde::Deserialize), serde(rename_all = \"camelCase\"))]";
const SERDE_ENUM_DERIVE: &str = "#[cfg_attr(feature = \"serde_derive\", serde(untagged))]";
const SERDE_DEFAULT: &str = "#[cfg_attr(feature = \"serde_derive\", serde(default))]";
// Serialize/deserialize the `abi_type` enum field as its protobuf string name
// (e.g. "ABI_TYPE_PROXY") rather than the raw i32 discriminant. `default` lets
// callers omit the field entirely. See `abi_type_serde` in the generated crate.
const SERDE_ABI_TYPE: &str =
    "#[cfg_attr(feature = \"serde_derive\", serde(with = \"crate::abi_type_serde\", default))]";
const BORSH_ENUM_DISC_ATTR: &str = "#[borsh(use_discriminant=true)]";
// Exclude a field from the derived Borsh encoding. Used for
// `ParsedTransactionPayload.intermediate_output` so the derived
// `borsh::to_vec` stays byte-identical to the legacy four-field encoding; the
// signing path appends the field's bytes to the digest only when non-empty.
const BORSH_SKIP: &str = "#[borsh(skip)]";
const TONIC_FEATURE_GATE: &str = "#[cfg(feature = \"tonic_types\")]";
const BORSH_DERIVE: &str = "#[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile protoc from source so we get consistent versions
    //std::env::set_var("PROTOC", protobuf_src::protoc());

    tonic_build::configure()
        .out_dir(GEN_DIR)
        // Force `BTreeMap` for every proto `map<.., ..>` field so iteration order
        // over these maps cannot affect rendered SignablePayload output. Aligns
        // with the project's determinism invariant ("BTreeMap for proto maps" --
        // CLAUDE.md / visualsign-solana clippy.toml). `.btree_map` is exposed on
        // tonic-build 0.10+.
        .btree_map(["."])
        // JSON - serde for types used in HTTP API requests/responses.
        // QOSParserRequest/QOSParserResponse are excluded: they embed health and
        // google.rpc types that don't implement serde.
        .type_attribute(".parser.ParseRequest", SERDE_DERIVE)
        .type_attribute(".parser.ParseResponse", SERDE_DERIVE)
        .type_attribute(".parser.ParsedTransaction", SERDE_DERIVE)
        .type_attribute(".parser.ParsedTransactionPayload", SERDE_DERIVE)
        .type_attribute(".parser.Signature", SERDE_DERIVE)
        .type_attribute(".parser.ChainMetadata", SERDE_DERIVE)
        .type_attribute(".parser.EthereumMetadata", SERDE_DERIVE)
        .type_attribute(".parser.SolanaMetadata", SERDE_DERIVE)
        .type_attribute(".parser.Abi", SERDE_DERIVE)
        .type_attribute(".parser.Idl", SERDE_DERIVE)
        .type_attribute(".parser.SignatureMetadata", SERDE_DERIVE)
        .type_attribute(".parser.Metadata", SERDE_DERIVE)
        // untagged for the ChainMetadata oneof so JSON doesn't include a variant tag
        // The path is message.oneof_field_name per prost-build enum_attribute docs
        .enum_attribute(".parser.ChainMetadata.metadata", SERDE_ENUM_DERIVE)
        // serde(default) on map fields so callers can omit them when empty
        .field_attribute(".parser.EthereumMetadata.abi_mappings", SERDE_DEFAULT)
        .field_attribute(".parser.SolanaMetadata.idl_mappings", SERDE_DEFAULT)
        // Represent the abi_type enum as its string name over JSON
        .field_attribute(".parser.Abi.abi_type", SERDE_ABI_TYPE)
        // BORSH - Used for QOS sha256 checks
        .type_attribute(".parser.ParsedTransactionPayload", BORSH_DERIVE)
        .enum_attribute(".parser.ParsedTransactionPayload", BORSH_ENUM_DISC_ATTR)
        .field_attribute(
            ".parser.ParsedTransactionPayload.intermediate_output",
            BORSH_SKIP,
        )
        .type_attribute(".parser.Metadata", BORSH_DERIVE)
        .enum_attribute(".parser.Metadata", BORSH_ENUM_DISC_ATTR)
        .type_attribute(".parser.SolanaMetadata", BORSH_DERIVE)
        .enum_attribute(".parser.SolanaMetadata", BORSH_ENUM_DISC_ATTR)
        .type_attribute(".parser.EthereumMetadata", BORSH_DERIVE)
        .enum_attribute(".parser.EthereumMetadata", BORSH_ENUM_DISC_ATTR)
        .type_attribute(".parser.Abi", BORSH_DERIVE)
        .enum_attribute(".parser.Abi", BORSH_ENUM_DISC_ATTR)
        .type_attribute(".parser.Idl", BORSH_DERIVE)
        .enum_attribute(".parser.Idl", BORSH_ENUM_DISC_ATTR)
        .type_attribute(".parser.SignatureMetadata", BORSH_DERIVE)
        .enum_attribute(".parser.SignatureMetadata", BORSH_ENUM_DISC_ATTR)
        .type_attribute(".parser.ChainMetadata", BORSH_DERIVE)
        .enum_attribute(".parser.ChainMetadata", BORSH_ENUM_DISC_ATTR)
        .client_mod_attribute(".", TONIC_FEATURE_GATE)
        .server_mod_attribute(".", TONIC_FEATURE_GATE)
        .file_descriptor_set_path(DESCRIPTOR_PATH)
        .include_file(INCLUDE_FILE)
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile(
            &[
                "../proto/parser/parser.proto",
                "../proto/health/rpc.proto",
                "../proto/grpc/health/v1/health.proto",
                "../proto/vendor/google/rpc/status.proto",
                "../proto/vendor/google/rpc/code.proto",
            ],
            &[PROTO_INCLUDE_PATH],
        )?;

    Ok(())
}
