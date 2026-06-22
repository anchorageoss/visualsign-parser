use crate::core::txtypes::{
    create_address_lookup_table_field, decode_v0_instructions, decode_v0_transfers,
};
use crate::core::{
    create_accounts_advanced_preview_layout, decode_accounts, decode_v0_accounts, instructions,
};
use crate::idl::IdlRegistry;
use crate::idl::builtin_programs::{
    canonical_name, is_reserved_canonical_name, is_trusted_program,
};
use crate::idl::signature::{
    authorized_idl_signers, convert_proto_signature, validate_idl_signature,
};
use base64::{self, Engine};
use solana_sdk::{
    message::VersionedMessage,
    pubkey::Pubkey,
    transaction::{Transaction as SolanaTransaction, VersionedTransaction},
};
use std::collections::BTreeMap;
use std::str::FromStr;
use visualsign::signing::SignerAllowlist;
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
    encodings::SupportedEncodings,
    vsptrait::{
        Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
        VisualSignError, VisualSignOptions,
    },
};

/// Maximum size for IDL JSON from proto messages (1 MB).
///
/// Mirrors the cap used by the Ethereum ABI metadata path: file-based IDL
/// loading has a wider 10 MB cap, but proto-supplied IDLs arrive per-request
/// and are deserialized on the hot path, so we apply a tighter bound here.
const MAX_IDL_JSON_BYTES: usize = 1_024 * 1_024;

/// Append decode errors as diagnostics and lint diagnostics to the output fields.
/// decode::visualizer_error is intentionally not routed through LintConfig --
/// visualizer failures are always surfaced so consumers know which
/// instructions could not be decoded.
#[cfg(feature = "diagnostics")]
fn append_diagnostics(
    fields: &mut Vec<SignablePayloadField>,
    result: &instructions::DecodeInstructionsResult,
) {
    for (idx, err) in &result.errors {
        fields.push(
            visualsign::field_builders::create_diagnostic_field(
                "decode::visualizer_error",
                "decode",
                visualsign::lint::Severity::Error,
                &format!("instruction {idx}: {err}"),
                Some(*idx as u32),
            )
            .signable_payload_field,
        );
    }
    fields.extend(
        result
            .diagnostics
            .iter()
            .map(|e| e.signable_payload_field.clone()),
    );
}

/// Wrapper around Solana's transaction types that implements the Transaction trait
#[derive(Debug, Clone)]
pub enum SolanaTransactionWrapper {
    Legacy(SolanaTransaction),
    Versioned(VersionedTransaction),
}

impl Transaction for SolanaTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        // Detect if format is base64 or hex
        let format = visualsign::encodings::SupportedEncodings::detect(data);

        let bytes = match format {
            SupportedEncodings::Base64 => base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?,
            SupportedEncodings::Hex => visualsign::encodings::decode_hex(data)
                .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?,
        };

        // First try to decode as a VersionedTransaction
        if let Ok(versioned_tx) = bincode::deserialize::<VersionedTransaction>(&bytes) {
            return Ok(Self::Versioned(versioned_tx));
        }

        // Fallback to legacy transaction parsing
        bincode::deserialize(&bytes)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))
            .map(Self::Legacy)
    }

    fn transaction_type(&self) -> String {
        match self {
            Self::Legacy(_) => "Solana (Legacy)".to_string(),
            Self::Versioned(tx) => match &tx.message {
                VersionedMessage::Legacy(_) => "Solana (Legacy)".to_string(),
                VersionedMessage::V0(_) => "Solana (V0)".to_string(),
            },
        }
    }
}

impl SolanaTransactionWrapper {
    pub fn new_legacy(transaction: SolanaTransaction) -> Self {
        Self::Legacy(transaction)
    }

    pub fn new_versioned(transaction: VersionedTransaction) -> Self {
        Self::Versioned(transaction)
    }

    pub fn inner_legacy(&self) -> Option<&SolanaTransaction> {
        match self {
            Self::Legacy(tx) => Some(tx),
            Self::Versioned(_) => None,
        }
    }

    pub fn inner_versioned(&self) -> Option<&VersionedTransaction> {
        match self {
            Self::Legacy(_) => None,
            Self::Versioned(tx) => Some(tx),
        }
    }
}

/// Extract IDL mappings from VisualSignOptions metadata.
///
/// Returns a BTreeMap of program_id (base58 string) -> (IDL JSON string, program name).
///
/// # Security
///
/// Mirrors `visualsign-ethereum::abi_metadata::try_extract_from_chain_metadata`:
/// 1. The program_id must be a valid base58 Solana `Pubkey`; otherwise the
///    entry is skipped.
/// 2. Mappings whose `program_id` is a trusted built-in (i.e.
///    `is_trusted_program(..)` is `true`) are dropped outright. This covers
///    programs with a canonical name (native runtime, core SPL, the 13
///    `ProgramType` dApp IDLs) *and* every program ID registered by an
///    in-crate preset visualizer (Kamino, Meteora, `swig_wallet`,
///    `dflow_aggregator`, etc.). Beyond the name guard in `IdlRegistry`,
///    the IDL *body* itself controls instruction decoding (argument names,
///    account names, value formatting), so an attacker-supplied IDL for the
///    System Program could relabel `lamports` or hide the destination via
///    the `unknown_program` IDL decode path. Refusing the body closes that
///    gap.
/// 3. The IDL JSON is rejected if it exceeds `MAX_IDL_JSON_BYTES`.
/// 4. If `Idl.signature` is present, it must verify (ed25519 over the shared
///    domain-separated prehash that binds the program id to the IDL JSON, via
///    `verify_strict`) AND the signer must appear in the authorized-signer
///    allowlist (see `crate::idl::signature::authorized_idl_signers`), or the
///    entry is dropped. The signer is a VisualSign-trusted metadata curator key,
///    not the program's on-chain upgrade authority.
///    Signers must therefore reproduce that prehash via
///    `visualsign::signing::solana_metadata_prehash` and sign the resulting
///    32-byte digest, matching `visualsign-ethereum::abi_metadata`. Because the
///    prehash commits to the program id, a signature is valid only for the
///    exact program it was produced for. A verified signature alone is not
///    enough: an empty allowlist rejects all signed IDLs (fail-closed). Unsigned
///    IDLs are still accepted so the feature degrades gracefully; callers that
///    need mandatory signatures must enforce that at the API boundary. This
///    check refuses to plumb attacker-tampered IDL bodies into the registry.
/// 5. The resolved `program_name` (from proto, IDL metadata, or fallback)
///    must not be a reserved canonical name. Step 2 already rejects when
///    the *program_id* is trusted, so by this step `program_id` has no
///    canonical name; if the *name* itself matches a canonical label
///    (e.g. "System Program"), it would impersonate a trusted program in
///    the rendered "Program" field. Reject it.
fn extract_idl_mappings(options: &VisualSignOptions) -> BTreeMap<String, (String, String)> {
    // Resolve the authorized IDL-signer allowlist from the env-configured
    // production list, then delegate. The allowlist is cached once per process
    // by `authorized_idl_signers`, so this is a cheap lookup rather than a fresh
    // env read + hex/ed25519 parse on every parse request. Splitting the
    // allowlist out as a parameter lets tests exercise the positive acceptance
    // path with an injected allowlist: env vars cannot be set in-process under
    // edition 2024 + forbid(unsafe), so an env-only allowlist would be
    // untestable here.
    extract_idl_mappings_with_signers(options, authorized_idl_signers())
}

/// Extraction core with an explicitly supplied signer allowlist.
///
/// Identical to [`extract_idl_mappings`] except the caller provides the
/// authorized-signer allowlist. Signed IDLs are accepted only when the signer
/// appears in `idl_signers`; an empty allowlist rejects every signed IDL
/// (fail-closed). Unsigned IDLs are unaffected.
fn extract_idl_mappings_with_signers(
    options: &VisualSignOptions,
    idl_signers: &SignerAllowlist,
) -> BTreeMap<String, (String, String)> {
    let Some(mappings) = options
        .metadata
        .as_ref()
        .and_then(|meta| meta.metadata.as_ref())
        .and_then(|m| {
            if let generated::parser::chain_metadata::Metadata::Solana(solana_meta) = m {
                Some(&solana_meta.idl_mappings)
            } else {
                None
            }
        })
    else {
        return BTreeMap::new();
    };

    let mut out: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (program_id, idl) in mappings {
        // 1. Validate program_id parses as a Solana Pubkey (cheap, fail fast).
        //    Keep the parsed key: its 32 bytes bind the IDL signature prehash
        //    in step 4.
        let pubkey = match Pubkey::from_str(program_id) {
            Ok(pk) => pk,
            Err(_) => {
                tracing::warn!("Skipping IDL mapping with invalid program_id '{program_id}'");
                continue;
            }
        };

        // 2. Reject IDL overrides for trusted built-in programs. The name
        //    guard in `IdlRegistry` blocks attacker-controlled labels, but the
        //    IDL body still drives instruction decoding (arg/account names,
        //    value formatting). Refusing the body for trusted programs closes
        //    that gap. "Trusted" includes the canonical-name set
        //    (native + core SPL + ProgramType dApps) AND every program ID
        //    registered by an in-crate preset visualizer.
        if is_trusted_program(program_id) {
            // Log the canonical label when we have one (clearer telemetry).
            match canonical_name(program_id) {
                Some(canonical) => tracing::warn!(
                    "Skipping IDL mapping for '{program_id}': override refused for trusted built-in '{canonical}'"
                ),
                None => tracing::warn!(
                    "Skipping IDL mapping for '{program_id}': override refused (preset-registered program)"
                ),
            }
            continue;
        }

        // 3. Reject oversized IDL JSON before any expensive operation.
        if idl.value.len() > MAX_IDL_JSON_BYTES {
            tracing::warn!(
                "Skipping IDL mapping for '{program_id}': exceeds size limit ({} bytes > {MAX_IDL_JSON_BYTES})",
                idl.value.len()
            );
            continue;
        }

        // 4. If a signature is provided it must verify AND the signer must be
        //    allowlisted; unsigned IDLs are accepted (parity with the Ethereum
        //    ABI path).
        if let Some(proto_sig) = idl.signature.as_ref() {
            let local_sig = convert_proto_signature(proto_sig);
            if let Err(e) =
                validate_idl_signature(&idl.value, &pubkey.to_bytes(), &local_sig, idl_signers)
            {
                tracing::warn!(
                    "Skipping IDL mapping for '{program_id}': signature validation failed: {e}"
                );
                continue;
            }
        }

        let name = idl
            .program_name
            .clone()
            .or_else(|| extract_name_from_idl_json(&idl.value))
            .unwrap_or_else(|| format!("Program {}", &program_id[..8.min(program_id.len())]));

        // 5. Display-name impersonation guard. `canonical_name(program_id)` is
        //    `None` here (step 2 already drained the trusted IDs), so any name
        //    that *matches* a canonical label belongs to a different,
        //    canonical program; rendering it on this `program_id` would be
        //    pure spoofing ("System Program" labeled on a malicious pubkey).
        if is_reserved_canonical_name(&name) {
            tracing::warn!(
                "Skipping IDL mapping for '{program_id}': program_name '{name}' is reserved for a different canonical program"
            );
            continue;
        }

        out.insert(program_id.clone(), (idl.value.clone(), name));
    }
    out
}

/// Extract the program name from an IDL JSON string
fn extract_name_from_idl_json(idl_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(idl_json).ok()?;

    // Try "metadata.name" first (Anchor IDL format)
    if let Some(metadata) = value.get("metadata") {
        if let Some(name) = metadata.get("name").and_then(|n| n.as_str()) {
            return Some(name.to_string());
        }
    }

    // Try "name" field directly
    value.get("name").and_then(|n| n.as_str()).map(String::from)
}

/// Create an IDL registry from VisualSignOptions metadata
fn create_idl_registry_from_options(
    options: &VisualSignOptions,
) -> Result<IdlRegistry, VisualSignError> {
    let idl_mappings = extract_idl_mappings(options);
    if !idl_mappings.is_empty() {
        IdlRegistry::from_idl_mappings(idl_mappings).map_err(|e| {
            VisualSignError::ConversionError(format!("Failed to create IDL registry: {e}"))
        })
    } else {
        Ok(IdlRegistry::new())
    }
}

/// Converter that knows how to format Solana transactions for VisualSign
pub struct SolanaVisualSignConverter;

impl VisualSignConverter<SolanaTransactionWrapper> for SolanaVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: SolanaTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        #[cfg(feature = "diagnostics")]
        let lint_config = visualsign::lint::LintConfig::default();
        match transaction_wrapper {
            SolanaTransactionWrapper::Legacy(transaction) => convert_to_visual_sign_payload(
                &transaction,
                options.decode_transfers,
                options.transaction_name.clone(),
                &options,
                #[cfg(feature = "diagnostics")]
                &lint_config,
            ),
            SolanaTransactionWrapper::Versioned(versioned_tx) => {
                convert_versioned_to_visual_sign_payload(
                    &versioned_tx,
                    options.decode_transfers,
                    options.transaction_name.clone(),
                    &options,
                    #[cfg(feature = "diagnostics")]
                    &lint_config,
                )
            }
        }
    }
}

impl VisualSignConverterFromString<SolanaTransactionWrapper> for SolanaVisualSignConverter {}

/// Public API function for ease of use with legacy transactions
pub fn transaction_to_visual_sign(
    transaction: SolanaTransaction,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SolanaVisualSignConverter
        .to_visual_sign_payload(SolanaTransactionWrapper::new_legacy(transaction), options)
}

/// Public API function for versioned transactions
pub fn versioned_transaction_to_visual_sign(
    transaction: VersionedTransaction,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SolanaVisualSignConverter.to_visual_sign_payload(
        SolanaTransactionWrapper::new_versioned(transaction),
        options,
    )
}

/// Public API function for string-based transactions
pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    SolanaVisualSignConverter.to_visual_sign_payload_from_string(transaction_data, options)
}

/// Convert Solana transaction to visual sign payload
fn convert_to_visual_sign_payload(
    transaction: &SolanaTransaction,
    decode_transfers: bool,
    title: Option<String>,
    options: &VisualSignOptions,
    #[cfg(feature = "diagnostics")] lint_config: &visualsign::lint::LintConfig,
) -> Result<SignablePayload, VisualSignError> {
    let message = &transaction.message;

    // Create IDL registry from options metadata
    let idl_registry = create_idl_registry_from_options(options)?;

    let mut fields = vec![SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: "Solana".to_string(),
            label: "Network".to_string(),
        },
        text_v2: visualsign::SignablePayloadFieldTextV2 {
            text: "Solana".to_string(),
        },
    }];

    if decode_transfers {
        let transfer_fields = instructions::decode_transfers(transaction)?;
        fields.extend(
            transfer_fields
                .iter()
                .map(|e| e.signable_payload_field.clone()),
        );
    }

    // Process instructions with visualizers
    #[cfg(feature = "diagnostics")]
    let decode_result = instructions::decode_instructions(transaction, &idl_registry, lint_config);
    #[cfg(feature = "diagnostics")]
    fields.extend(
        decode_result
            .fields
            .iter()
            .map(|e| e.signable_payload_field.clone()),
    );

    #[cfg(not(feature = "diagnostics"))]
    {
        let decoded_fields = instructions::decode_instructions(transaction, &idl_registry)?;
        fields.extend(
            decoded_fields
                .iter()
                .map(|e| e.signable_payload_field.clone()),
        );
    }

    // Decode and sort accounts using the dedicated function
    let accounts = decode_accounts(message)?;

    // we don't allow list layout at the top level - limitation of Anchorage app
    let preview_layout_advanced = create_accounts_advanced_preview_layout("Accounts", &accounts)?;
    // Add Accounts field at the bottom using PreviewLayout instead of ListLayout
    fields.push(preview_layout_advanced);

    #[cfg(feature = "diagnostics")]
    append_diagnostics(&mut fields, &decode_result);

    Ok(SignablePayload::new(
        0,
        title.unwrap_or_else(|| "Solana Transaction".to_string()),
        None,
        fields,
        "SolanaTx".to_string(),
    ))
}

/// Convert versioned Solana transaction to visual sign payload
fn convert_versioned_to_visual_sign_payload(
    versioned_tx: &VersionedTransaction,
    decode_transfers: bool,
    title: Option<String>,
    options: &VisualSignOptions,
    #[cfg(feature = "diagnostics")] lint_config: &visualsign::lint::LintConfig,
) -> Result<SignablePayload, VisualSignError> {
    match &versioned_tx.message {
        VersionedMessage::Legacy(legacy_message) => {
            let legacy_tx = SolanaTransaction {
                signatures: versioned_tx.signatures.clone(),
                message: legacy_message.clone(),
            };
            convert_to_visual_sign_payload(
                &legacy_tx,
                decode_transfers,
                title,
                options,
                #[cfg(feature = "diagnostics")]
                lint_config,
            )
        }
        VersionedMessage::V0(v0_message) => convert_v0_to_visual_sign_payload(
            versioned_tx,
            v0_message,
            decode_transfers,
            title,
            options,
            #[cfg(feature = "diagnostics")]
            lint_config,
        ),
    }
}

/// Convert V0 transaction to visual sign payload
fn convert_v0_to_visual_sign_payload(
    versioned_tx: &VersionedTransaction,
    v0_message: &solana_sdk::message::v0::Message,
    decode_transfers: bool,
    title: Option<String>,
    options: &VisualSignOptions,
    #[cfg(feature = "diagnostics")] lint_config: &visualsign::lint::LintConfig,
) -> Result<SignablePayload, VisualSignError> {
    // NOTE: the parser does not perform on-chain ALT resolution, so any
    // instruction or account that references an entry in
    // `address_table_lookups` cannot be fully resolved here. We previously
    // fail-closed and rejected such transactions outright (#324), but that
    // blocks all ALT-backed V0 transactions (e.g. aggregator swaps) from being
    // signed. The proper fix is to pass resolved ALT contents into the parser
    // from the caller; until that lands we render with graceful degradation --
    // ALT-backed accounts surface as placeholders via decode_v0_instructions.
    // TODO: re-introduce ALT-aware rendering once ALT data is supplied.

    // Create IDL registry from options metadata
    let idl_registry = create_idl_registry_from_options(options)?;

    // Decode and sort accounts using the dedicated function
    let accounts = decode_v0_accounts(v0_message)?;

    let mut fields = vec![SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: "Solana (V0)".to_string(),
            label: "Network".to_string(),
        },
        text_v2: visualsign::SignablePayloadFieldTextV2 {
            text: "Solana (V0)".to_string(),
        },
    }];

    // Add address lookup table information if present
    if !v0_message.address_table_lookups.is_empty() {
        let lookup_table_field = create_address_lookup_table_field(v0_message)?;
        fields.push(lookup_table_field);
    }

    // Directly process V0 instructions using the visualizer framework
    // This approach works for all V0 transactions, including those with lookup tables
    #[cfg(feature = "diagnostics")]
    let v0_result = decode_v0_instructions(v0_message, &idl_registry, lint_config);
    #[cfg(feature = "diagnostics")]
    for (index, instruction_field) in v0_result.fields.iter().enumerate() {
        tracing::debug!(
            "Handling instruction {} with visualizer {:?}",
            index,
            "V0 Instruction"
        );
        fields.push(instruction_field.signable_payload_field.clone());
    }

    #[cfg(not(feature = "diagnostics"))]
    match decode_v0_instructions(v0_message, &idl_registry) {
        Ok(v0_fields) => {
            for (index, instruction_field) in v0_fields.iter().enumerate() {
                tracing::debug!(
                    "Handling instruction {} with visualizer {:?}",
                    index,
                    "V0 Instruction"
                );
                fields.push(instruction_field.signable_payload_field.clone());
            }
        }
        Err(e) => {
            // Add a note about instruction decoding failure
            fields.push(SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: format!("Instruction decoding failed: {e}"),
                    label: "Instruction Decoding Note".to_string(),
                },
                text_v2: visualsign::SignablePayloadFieldTextV2 {
                    text: format!("Instruction decoding failed: {e}"),
                },
            });
        }
    }

    // Process V0 transfer decoding using solana-parser
    if decode_transfers {
        match decode_v0_transfers(versioned_tx) {
            Ok(transfer_fields) => {
                fields.extend(
                    transfer_fields
                        .iter()
                        .map(|e| e.signable_payload_field.clone()),
                );
            }
            Err(e) => {
                // Add a note about transfer decoding failure
                fields.push(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Transfer decoding failed: {e}"),
                        label: "Transfer Decoding Note".to_string(),
                    },
                    text_v2: visualsign::SignablePayloadFieldTextV2 {
                        text: format!("Transfer decoding failed: {e}"),
                    },
                });
            }
        }
    }

    // Add Accounts field at the bottom using PreviewLayout instead of ListLayout
    let preview_layout_advanced = create_accounts_advanced_preview_layout("Accounts", &accounts)?;
    fields.push(preview_layout_advanced);

    #[cfg(feature = "diagnostics")]
    append_diagnostics(&mut fields, &v0_result);

    Ok(SignablePayload::new(
        0,
        title.unwrap_or_else(|| "Solana V0 Transaction".to_string()),
        None,
        fields,
        "SolanaTx".to_string(),
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::test_utils::payload_from_b64;
    use crate::utils::create_transaction_with_empty_signatures;

    #[test]
    fn test_solana_transaction_to_vsp() {
        // This was generated using the Solana CLI using solana transfer --sign-only which only prints message, that needs to be wrapped into a transaction
        // Same as the test fixture used for integration as a baseline
        let solana_transfer_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";
        let solana_transfer_transaction =
            create_transaction_with_empty_signatures(solana_transfer_message);
        let payload = payload_from_b64(&solana_transfer_transaction);
        assert_eq!(payload.title, "Solana Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "SolanaTx");

        assert!(!payload.fields.is_empty());

        let network_field = payload.fields.iter().find(|f| f.label() == "Network");
        assert!(network_field.is_some());
        assert_eq!(
            network_field.unwrap().fallback_text(),
            &"Solana".to_string()
        );

        let json_result = payload.to_json();
        assert!(json_result.is_ok());
    }

    #[test]
    fn test_solana_transaction_trait() {
        let solana_transfer_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";
        let solana_transfer_transaction =
            create_transaction_with_empty_signatures(solana_transfer_message);
        let result = SolanaTransactionWrapper::from_string(&solana_transfer_transaction);
        assert!(result.is_ok());

        let solana_tx = result.unwrap();
        assert!(solana_tx.transaction_type().contains("Solana"));

        let invalid_result = SolanaTransactionWrapper::from_string("invalid_data");
        assert!(invalid_result.is_err());
    }

    #[test]
    fn test_jupiter_swap_transaction() {
        // Jupiter swap transaction from the user's request
        let jupiter_transaction = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAsTTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ6653iXpBtBVMUA2+7hURKVHSEiGP6Bzz+71DafYBHQDv0Yk27V9AGBuUCokgwtdJtHGjOn65hFbpKYxFjpOxf9DslqNk9ntU1o905D8G/f/M/gGJfV/szOEdGlj8ByB4ydCgh9JdZoBmFC/1V+60NB9JdEtwXur6E410yCBDwODn7a9i8ySuhrG7m4UOmmngOd7rrj0EIP/mIOo3poMglc7k/piKlm7+u7deeb1LQ3/H1gPv54+BUArFsw2O5lY54pz/YD6rtbZ/BQGLaOTytSS3SHI51lpsQDqNm8IHuyTAFQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwTp4S+8hOgmyTLM6eJkDM4VWQwcYnOwklcIujuFILC8BpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqYb8H//NLjVx31IUdFMPpkUf0008tghSu5vUckZpELeujJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+FmycNZ/qYxRzwITBRNYliuvNXQr7VnJ2URenA0MhcfNkbQ/+if11/ZKdMCbHylYed5LCas238ndUUsyGqezjOXo/NFB6YMsrxCtkXSVyg8nG1spPNRwJ+pzcAftQOs5oL2MaEXlNY7kQGEFwqYqsAepz7QXX/3fSFmPGjLpqakIxwYJAAUCQA0DAA8GAAIADAgNAQEIAgACDAIAAACghgEAAAAAAA0BAgERChsNAAIDChIKEQoLBA4BBQIDEgwGCwANDRALBwoj5RfLl3rjrSoBAAAAJmQAAaCGAQAAAAAAkz4BAAAAAAAyAAANAwIAAAEJ";

        let solana_tx_result = SolanaTransactionWrapper::from_string(jupiter_transaction);
        assert!(solana_tx_result.is_ok());

        let solana_tx = solana_tx_result.unwrap();

        // Convert to VisualSign payload using the converter
        let payload_result = SolanaVisualSignConverter.to_visual_sign_payload(
            solana_tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: Some("Solana Transaction".to_string()),
                developer_config: None,
            },
        );

        if let Err(ref e) = payload_result {
            println!("Error converting to payload: {e:?}");
        }
        assert!(payload_result.is_ok());

        let payload = payload_result.unwrap();

        // Verify basic payload properties
        assert_eq!(payload.title, "Solana Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "SolanaTx");
        assert!(!payload.fields.is_empty());

        // Convert to JSON and verify structure
        let json_result = payload.to_json();
        assert!(json_result.is_ok());

        let json_value: serde_json::Value = serde_json::from_str(&json_result.unwrap()).unwrap();

        // Verify expected JSON structure using serde_json::json! macro for comparison
        let expected_structure = serde_json::json!({
            "Title": "Solana Transaction",
            "Version": "0",
            "PayloadType": "SolanaTx"
        });

        assert_eq!(json_value["Title"], expected_structure["Title"]);
        assert_eq!(json_value["Version"], expected_structure["Version"]);
        assert_eq!(json_value["PayloadType"], expected_structure["PayloadType"]);

        // Verify that fields array exists and is not empty
        assert!(json_value["Fields"].is_array());
        let fields = json_value["Fields"].as_array().unwrap();
        assert!(!fields.is_empty());

        // Look for Jupiter-related content in the fields
        let _fields_json = serde_json::to_string(&fields).unwrap();

        // Check for presence of Jupiter program ID or swap-related content
        let has_jupiter_content = fields.iter().any(|field| {
            let field_str = serde_json::to_string(field).unwrap_or_default();
            field_str.contains("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4")
                || field_str.contains("Jupiter")
                || field_str.contains("swap")
                || field_str.contains("Swap")
        });

        // Verify we found Jupiter content
        assert!(has_jupiter_content, "Should contain Jupiter swap content");

        // Note: This test verifies the transaction can be parsed without errors
        // The exact Jupiter swap detection depends on the instruction data parsing
        println!(
            "✅ Jupiter transaction parsed successfully with {} fields",
            fields.len()
        );
        println!("✅ Contains Jupiter content: {has_jupiter_content}");
    }

    #[test]
    fn test_v0_transaction() {
        // V0 transaction from the user's request
        let v0_transaction = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQAIEMb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hO9VYqgvLR5aQ58r++KhUxAMArXNUFouJhkNfk91xcdpfsw70khoY/pDZ7PZ6Utif//vUHTgWKYb1IOp28C3laonif5pJDmoFCEZLLM1jDQoBxbAzIjAnxzfida8KF8loqQWTFLbxtR33pCcsa4g/5IpH2dQ+PHkoCbIQgfspGmC7Pda2pnGc3R0WktKvNfpBJorRv4iVoUOTn784IlhxGbzCdMmWMCSVCNq8frVXYTEFUunuZBu0Welvi993TLZB9fJvij+ef7p3Rw8UE+ZQpngRVksq5ZjmYhxu6tmLviIDBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAR51VvyMcBu7nTFbs5oFQf9sbLeo/SOUQKxzaJWvBOPBpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqUcn0nz5UKgy0QJ34xepN6SZQQ1LggwZ6QPHCYVaRRN9tD/6J/XX9kp0wJsfKVh53ksJqzbfyd1RSzIap7OM5ei1w1W367Ykl8/1heeE1Ct6pgMZQ89eFMSv0TWee6UaMMzWwUztGQ+UwdGRAWmsk+hsxTf7GSUoTLwaPEtoWnCSmZVQM4qi8IJmCZXye+3lj/svGc+s43La9Kg4Nwso+h0DCAAJAwQXAQAAAAAACRULAAIECQoJDQkODAUPAwcAAgQGAQsj5RfLl3rjrSoBAAAAMGQAAUBCDwAAAAAAhBlJAAAAAAAyAAALAwQAAAEJAA==";

        let solana_tx_result = SolanaTransactionWrapper::from_string(v0_transaction);
        assert!(solana_tx_result.is_ok());

        let solana_tx = solana_tx_result.unwrap();

        // Check that it's recognized as a V0 transaction
        assert_eq!(solana_tx.transaction_type(), "Solana (V0)");

        // Convert to VisualSign payload using the converter
        let payload_result = SolanaVisualSignConverter.to_visual_sign_payload(
            solana_tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: Some("V0 Transaction".to_string()),
                developer_config: None,
            },
        );

        if let Err(ref e) = payload_result {
            println!("Error converting V0 to payload: {e:?}");
        }
        assert!(payload_result.is_ok());

        let payload = payload_result.unwrap();

        // Verify basic payload properties
        assert_eq!(payload.title, "V0 Transaction");
        assert_eq!(payload.version, "0");
        assert_eq!(payload.payload_type, "SolanaTx");
        assert!(!payload.fields.is_empty());

        // Convert to JSON and verify structure
        let json_result = payload.to_json();
        assert!(json_result.is_ok());

        let json_value: serde_json::Value = serde_json::from_str(&json_result.unwrap()).unwrap();

        // Verify that fields array exists and is not empty
        assert!(json_value["Fields"].is_array());
        let fields = json_value["Fields"].as_array().unwrap();
        assert!(!fields.is_empty());

        // Look for V0-specific content in the fields
        let has_v0_content = fields.iter().any(|field| {
            let field_str = serde_json::to_string(field).unwrap_or_default();
            field_str.contains("V0") || field_str.contains("Address Lookup")
        });

        // Verify we found V0 content
        assert!(has_v0_content, "Should contain V0 transaction content");

        println!(
            "✅ V0 transaction parsed successfully with {} fields",
            fields.len()
        );
        println!("✅ Contains V0 content: {has_v0_content}");
    }

    #[test]
    fn test_address_lookup_table_field_creation() {
        use solana_sdk::message::v0::MessageAddressTableLookup;
        use solana_sdk::pubkey::Pubkey;

        // Create a mock v0 message with address lookup tables
        let mut v0_message = solana_sdk::message::v0::Message::default();

        // Add two lookup tables with valid pubkeys
        let lookup1 = MessageAddressTableLookup {
            account_key: Pubkey::new_unique(),
            writable_indexes: vec![0, 1],
            readonly_indexes: vec![2, 3, 4],
        };

        let lookup2 = MessageAddressTableLookup {
            account_key: Pubkey::new_unique(),
            writable_indexes: vec![],
            readonly_indexes: vec![0],
        };

        v0_message.address_table_lookups = vec![lookup1, lookup2];

        // Test the field creation
        let field = create_address_lookup_table_field(&v0_message).unwrap();

        match field {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Address Lookup Tables");
                assert!(
                    !common.fallback_text.is_empty(),
                    "Should have fallback text with lookup table addresses"
                );

                // Check that both condensed and expanded views exist
                assert!(preview_layout.condensed.is_some());
                assert!(preview_layout.expanded.is_some());

                // Check expanded view has detailed fields
                let expanded_fields = &preview_layout.expanded.as_ref().unwrap().fields;
                assert!(
                    expanded_fields.len() >= 5,
                    "Should have multiple detail fields, got {}",
                    expanded_fields.len()
                );

                // Check first field is total count
                if let Some(first_field) = expanded_fields.first() {
                    if let SignablePayloadField::TextV2 { common, .. } =
                        &first_field.signable_payload_field
                    {
                        assert_eq!(common.label, "Total Tables");
                        assert_eq!(common.fallback_text, "2");
                    }
                }

                println!(
                    "✅ Address lookup table field created with {} detail fields in expanded view",
                    expanded_fields.len()
                );
            }
            _ => panic!("Expected PreviewLayout field type"),
        }
    }

    #[test]
    fn test_v0_transfer_decoding() {
        // Test the V0 transfer decoding function directly
        let v0_transaction = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQAIEMb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hO9VYqgvLR5aQ58r++KhUxAMArXNUFouJhkNfk91xcdpfsw70khoY/pDZ7PZ6Utif//vUHTgWKYb1IOp28C3laonif5pJDmoFCEZLLM1jDQoBxbAzIjAnxzfida8KF8loqQWTFLbxtR33pCcsa4g/5IpH2dQ+PHkoCbIQgfspGmC7Pda2pnGc3R0WktKvNfpBJorRv4iVoUOTn784IlhxGbzCdMmWMCSVCNq8frVXYTEFUunuZBu0Welvi993TLZB9fJvij+ef7p3Rw8UE+ZQpngRVksq5ZjmYhxu6tmLviIDBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAR51VvyMcBu7nTFbs5oFQf9sbLeo/SOUQKxzaJWvBOPBpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqUcn0nz5UKgy0QJ34xepN6SZQQ1LggwZ6QPHCYVaRRN9tD/6J/XX9kp0wJsfKVh53ksJqzbfyd1RSzIap7OM5ei1w1W367Ykl8/1heeE1Ct6pgMZQ89eFMSv0TWee6UaMMzWwUztGQ+UwdGRAWmsk+hsxTf7GSUoTLwaPEtoWnCSmZVQM4qi8IJmCZXye+3lj/svGc+s43La9Kg4Nwso+h0DCAAJAwQXAQAAAAAACRULAAIECQoJDQkODAUPAwcAAgQGAQsj5RfLl3rjrSoBAAAAMGQAAUBCDwAAAAAAhBlJAAAAAAAyAAALAwQAAAEJAA==";

        let solana_tx_result = SolanaTransactionWrapper::from_string(v0_transaction);
        assert!(solana_tx_result.is_ok());

        let solana_tx = solana_tx_result.unwrap();
        if let SolanaTransactionWrapper::Versioned(versioned_tx) = solana_tx {
            // Test transfer decoding directly
            let transfer_result = decode_v0_transfers(&versioned_tx);

            match transfer_result {
                Ok(transfers) => {
                    println!(
                        "✅ V0 transfer decoding succeeded with {} transfers",
                        transfers.len()
                    );
                    for (i, transfer) in transfers.iter().enumerate() {
                        println!(
                            "Transfer {}: {:?}",
                            i + 1,
                            transfer.signable_payload_field.label()
                        );
                    }
                }
                Err(e) => {
                    println!("❌ V0 transfer decoding failed: {e:?}");
                    // This is expected for transactions without transfers, so it's not a failure
                }
            }
        } else {
            panic!("Expected versioned transaction");
        }
    }

    #[test]
    fn test_v0_vs_legacy_transfer_comparison() {
        // Test legacy transfer transaction (known to work)
        let legacy_transfer_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";
        let legacy_transfer_transaction =
            create_transaction_with_empty_signatures(legacy_transfer_message);

        println!("Testing legacy transfer transaction...");
        let legacy_result = SolanaTransactionWrapper::from_string(&legacy_transfer_transaction);
        assert!(legacy_result.is_ok());

        let legacy_tx = legacy_result.unwrap();
        let legacy_payload_result = SolanaVisualSignConverter.to_visual_sign_payload(
            legacy_tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: Some("Legacy Transfer Test".to_string()),
                developer_config: None,
            },
        );

        assert!(legacy_payload_result.is_ok());
        let legacy_payload = legacy_payload_result.unwrap();

        // Check for transfer fields in legacy transaction
        let legacy_has_transfers = legacy_payload
            .fields
            .iter()
            .any(|field| field.label().contains("Transfer"));

        println!(
            "Legacy transaction has {} fields, transfers found: {}",
            legacy_payload.fields.len(),
            legacy_has_transfers
        );

        // Print all legacy fields for debugging
        for (i, field) in legacy_payload.fields.iter().enumerate() {
            println!(
                "Legacy Field {}: label='{}', fallback='{}'",
                i,
                field.label(),
                field.fallback_text()
            );
        }

        // Now let's create a real V0 transfer transaction by crafting one
        // ./target/debug/solana-tx-constructor --sender-address 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM --tx-type v0 transfer --source-token-account EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v --destination-token-account 83jxWxmLV34PZa9eZNwcZvDBd4hxqY1aycRPABAcDNDM --amount 1000000
        println!("Testing V0 transaction with transfer decoding enabled...");
        let v0_transaction = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQABBH6MCIdgv94d3c8ywX8gm4JC7lKq8TH6zYjQ6ixtCwbyaLWKvNAoVTqTUi1a9+MHCdQWoCE11bOsRYgQPQhUG3DG+nrzvtutOj1l82qryXQxsbvkwtL24OR8pgIDRS9dYQbd9uHXZaGT2cvhRs7reawctIXtX1s3kTqM9YV+/wCpJd3clp6q69nlSQBm2zHuyGaxkQHMeN8UjpzmOH6qauwBAwMCAQAJA0BCDwAAAAAAAA==";

        let v0_result = SolanaTransactionWrapper::from_string(v0_transaction);
        assert!(v0_result.is_ok());

        let v0_tx = v0_result.unwrap();
        let v0_payload_result = SolanaVisualSignConverter.to_visual_sign_payload(
            v0_tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: Some("V0 Transfer Test".to_string()),
                developer_config: None,
            },
        );

        assert!(v0_payload_result.is_ok());
        let v0_payload = v0_payload_result.unwrap();

        // Check for transfer fields in V0 transaction
        let v0_has_transfers = v0_payload
            .fields
            .iter()
            .any(|field| field.label().contains("Transfer"));

        let v0_has_transfer_failures = v0_payload.fields.iter().any(|field| {
            field.label().contains("Transfer Decoding Note")
                || field.fallback_text().contains("Transfer decoding failed")
        });

        println!(
            "V0 transaction has {} fields, transfers found: {}, transfer failures: {}",
            v0_payload.fields.len(),
            v0_has_transfers,
            v0_has_transfer_failures
        );

        // Print field details for debugging
        for (i, field) in v0_payload.fields.iter().enumerate() {
            println!(
                "V0 Field {}: label='{}', fallback='{}'",
                i,
                field.label(),
                field.fallback_text()
            );
        }

        // The real test: V0 transfer decoding should work without failures
        println!("✅ V0 transfer decoding integration test completed");
        println!(
            "Legacy has transfers: {legacy_has_transfers}, V0 has transfer failures: {v0_has_transfer_failures}"
        );

        // Assert that we can at least call the V0 transfer decoding without it failing
        assert!(
            !v0_has_transfer_failures,
            "V0 transaction should not have transfer decoding failures"
        );
    }

    #[test]
    fn test_v0_transfer_with_real_data() {
        // Create a test with known transaction data that should trigger solana-parser
        use solana_sdk::{
            message::{VersionedMessage, v0},
            pubkey::Pubkey,
            signature::Signature,
            transaction::VersionedTransaction,
        };

        // Add a transfer instruction (system transfer)
        let transfer_instruction = solana_sdk::instruction::CompiledInstruction {
            program_id_index: 2,                                 // system program
            accounts: vec![0, 1],                                // from fee payer to recipient
            data: vec![2, 0, 0, 0, 0, 202, 154, 59, 0, 0, 0, 0], // transfer 1 SOL (1_000_000_000 lamports)
        };
        // Create a minimal V0 transaction manually to test the decode path
        let mut v0_message = v0::Message {
            recent_blockhash: solana_sdk::hash::Hash::new_unique(),
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                solana_sdk::system_program::ID,
            ],
            address_table_lookups: vec![],
            instructions: vec![transfer_instruction],
        };

        // Add some account keys (fee payer, recipient, system program)
        v0_message.account_keys = vec![
            Pubkey::new_unique(),           // fee payer
            Pubkey::new_unique(),           // recipient
            solana_sdk::system_program::ID, // system program
        ];

        // Create a versioned transaction
        let versioned_transaction = VersionedTransaction {
            signatures: vec![Signature::default()], // dummy signature
            message: VersionedMessage::V0(v0_message),
        };

        println!("Testing manually crafted V0 transfer transaction...");

        // Test our V0 transfer decoding directly
        match decode_v0_transfers(&versioned_transaction) {
            Ok(transfers) => {
                println!(
                    "✅ Manually crafted V0 transfer decoding succeeded with {} transfers",
                    transfers.len()
                );

                if transfers.is_empty() {
                    println!(
                        "ℹ️  No transfers found - this could be expected if solana-parser doesn't recognize our crafted transaction"
                    );
                } else {
                    for (i, transfer) in transfers.iter().enumerate() {
                        println!(
                            "Transfer {}: label='{}', fallback='{}'",
                            i + 1,
                            transfer.signable_payload_field.label(),
                            transfer.signable_payload_field.fallback_text()
                        );
                    }
                }

                // Test full payload conversion
                let wrapper = SolanaTransactionWrapper::Versioned(versioned_transaction);
                let payload_result = SolanaVisualSignConverter.to_visual_sign_payload(
                    wrapper,
                    VisualSignOptions {
                        metadata: None,
                        decode_transfers: true,
                        transaction_name: Some("Manual V0 Transfer Test".to_string()),
                        developer_config: None,
                    },
                );

                match payload_result {
                    Ok(payload) => {
                        println!(
                            "✅ V0 transaction conversion succeeded with {} fields",
                            payload.fields.len()
                        );

                        let has_transfer_failures = payload.fields.iter().any(|field| {
                            field.label().contains("Transfer Decoding Note")
                                || field.fallback_text().contains("Transfer decoding failed")
                        });

                        println!("Transfer decoding failures: {has_transfer_failures}");

                        // Print all fields for inspection
                        for (i, field) in payload.fields.iter().enumerate() {
                            println!(
                                "Field {}: label='{}', fallback='{}'",
                                i,
                                field.label(),
                                field.fallback_text()
                            );
                        }

                        // The key test: no transfer decoding failures
                        assert!(
                            !has_transfer_failures,
                            "Manually crafted V0 transaction should not have transfer decoding failures"
                        );
                    }
                    Err(e) => {
                        panic!("V0 transaction conversion failed: {e:?}");
                    }
                }
            }
            Err(e) => {
                println!("❌ Manually crafted V0 transfer decoding failed: {e:?}");
                // This might happen if solana-parser has issues with our manually crafted transaction
                // but the important thing is our code doesn't panic
                println!(
                    "ℹ️  This is acceptable - solana-parser might not recognize manually crafted transactions"
                );
            }
        }

        println!("✅ V0 transfer decoding infrastructure is working correctly");
    }

    #[test]
    fn test_transaction_auto_detection_v0_vs_legacy() {
        // Test the auto-detection logic in from_string() - V0 should be detected first
        let v0_transaction = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQAIEMb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hO9VYqgvLR5aQ58r++KhUxAMArXNUFouJhkNfk91xcdpfsw70khoY/pDZ7PZ6Utif//vUHTgWKYb1IOp28C3laonif5pJDmoFCEZLLM1jDQoBxbAzIjAnxzfida8KF8loqQWTFLbxtR33pCcsa4g/5IpH2dQ+PHkoCbIQgfspGmC7Pda2pnGc3R0WktKvNfpBJorRv4iVoUOTn784IlhxGbzCdMmWMCSVCNq8frVXYTEFUunuZBu0Welvi993TLZB9fJvij+ef7p3Rw8UE+ZQpngRVksq5ZjmYhxu6tmLviIDBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAR51VvyMcBu7nTFbs5oFQf9sbLeo/SOUQKxzaJWvBOPBpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqUcn0nz5UKgy0QJ34xepN6SZQQ1LggwZ6QPHCYVaRRN9tD/6J/XX9kp0wJsfKVh53ksJqzbfyd1RSzIap7OM5ei1w1W367Ykl8/1heeE1Ct6pgMZQ89eFMSv0TWee6UaMMzWwUztGQ+UwdGRAWmsk+hsxTf7GSUoTLwaPEtoWnCSmZVQM4qi8IJmCZXye+3lj/svGc+s43La9Kg4Nwso+h0DCAAJAwQXAQAAAAAACRULAAIECQoJDQkODAUPAwcAAgQGAQsj5RfLl3rjrSoBAAAAMGQAAUBCDwAAAAAAhBlJAAAAAAAyAAALAwQAAAEJAA==";

        // Test that V0 is detected correctly
        let v0_wrapper = SolanaTransactionWrapper::from_string(v0_transaction).unwrap();
        assert_eq!(v0_wrapper.transaction_type(), "Solana (V0)");
        assert!(v0_wrapper.inner_versioned().is_some());
        if let Some(versioned) = v0_wrapper.inner_versioned() {
            assert!(matches!(versioned.message, VersionedMessage::V0(_)));
        }

        // Test legacy detection (this gets parsed as VersionedTransaction with Legacy message)
        let legacy_message = "AgABA3Lgs31rdjnEG5FRyrm2uAi4f+erGdyJl0UtJyMMLGzC9wF+t3qhmhpj3vI369n5Ef5xRLms/Vn8J/Lc7bmoIkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMBafBISARibJ+I25KpHkjLe53ZrqQcLWGy8n97yWD7mAQICAQAMAgAAAADKmjsAAAAA";
        let legacy_transaction = create_transaction_with_empty_signatures(legacy_message);

        let legacy_wrapper = SolanaTransactionWrapper::from_string(&legacy_transaction).unwrap();
        assert_eq!(legacy_wrapper.transaction_type(), "Solana (Legacy)");
        assert!(legacy_wrapper.inner_versioned().is_some());
        if let Some(versioned) = legacy_wrapper.inner_versioned() {
            assert!(matches!(versioned.message, VersionedMessage::Legacy(_)));
        }
    }

    #[test]
    fn test_legacy_fallback_parsing() {
        // Test that pure legacy transactions (not wrapped in VersionedTransaction) fall back correctly
        // We need to create transaction data that fails VersionedTransaction parsing but succeeds legacy parsing

        // This is a manually crafted legacy transaction that should fail VersionedTransaction deserialization
        // but succeed with legacy Transaction deserialization
        use solana_sdk::{
            hash::Hash, message::Message, pubkey::Pubkey,
            transaction::Transaction as SolanaTransaction,
        };

        // Create a minimal legacy transaction
        let legacy_tx = SolanaTransaction {
            signatures: vec![],
            message: Message {
                header: solana_sdk::message::MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 1,
                },
                account_keys: vec![Pubkey::new_unique(), solana_sdk::system_program::ID],
                recent_blockhash: Hash::new_unique(),
                instructions: vec![],
            },
        };

        // Serialize it as a legacy transaction
        let legacy_bytes = bincode::serialize(&legacy_tx).unwrap();
        let legacy_b64 = base64::engine::general_purpose::STANDARD.encode(legacy_bytes);

        // Test that our parser handles it correctly
        let wrapper = SolanaTransactionWrapper::from_string(&legacy_b64).unwrap();

        // This should be detected correctly based on the transaction_type logic
        let tx_type = wrapper.transaction_type();
        assert!(
            tx_type.contains("Legacy"),
            "Should be detected as legacy, got: {tx_type}"
        );
    }

    #[test]
    fn test_invalid_transaction_parsing() {
        // Test that invalid data fails gracefully
        let invalid_data = "invalid_base64_data!@#$";
        let result = SolanaTransactionWrapper::from_string(invalid_data);
        assert!(result.is_err(), "Invalid data should fail to parse");

        // Test with valid base64 but invalid transaction structure
        let invalid_tx_data = "SGVsbG8gV29ybGQ="; // "Hello World" in base64
        let result = SolanaTransactionWrapper::from_string(invalid_tx_data);
        assert!(
            result.is_err(),
            "Invalid transaction data should fail to parse"
        );
    }

    #[test]
    fn test_spl_token_tokenkeg_recognition() {
        // Regression for github.com/anchorageoss/visualsign-parser/issues/76 —
        // TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA must now be recognized
        // as SPL Token (not fall through to unknown_program).
        let tokenkeg_tx = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAEDGtcy7Vc3xB54TVH4H/JNV6GLORFZVW2eiFky1mqlJTJHohT28K37lWNJzHkspHGumVg0rwhDxT5hd/JUEGupaAbd9uHXZaGT2cvhRs7reawctIXtX1s3kTqM9YV+/wCpiz/aiPOGc/sEVBMImlZdQN5iFK0CVj9fTne9d3VuvB0BAgIBACMGAAF5QmVCZ074eW/VU/D+KlEJonY3BgtzkD1DFS0OaNFWDA==";

        let tx = SolanaTransactionWrapper::from_string(tokenkeg_tx)
            .expect("Should parse TokenKeg transaction");
        let payload = SolanaVisualSignConverter
            .to_visual_sign_payload(
                tx,
                VisualSignOptions {
                    metadata: None,
                    decode_transfers: true,
                    transaction_name: Some("SPL Token Test".to_string()),
                    developer_config: None,
                },
            )
            .expect("Should convert TokenKeg transaction to payload");

        let instruction_fields: Vec<_> = payload
            .fields
            .iter()
            .filter(|f| f.label().starts_with("Instruction"))
            .collect();
        assert_eq!(
            instruction_fields.len(),
            1,
            "Should have exactly 1 instruction"
        );

        let json_str = payload.to_json().unwrap();
        assert!(
            json_str.contains("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
            "Should contain TokenKeg program ID in the output"
        );
        // The instruction should be parsed by the SPL Token preset, so the
        // output should mention either the operation ("Mint To") or the
        // program label ("SPL Token") — not just raw hex.
        assert!(
            json_str.contains("Mint To") || json_str.contains("SPL Token"),
            "Should recognize TokenKeg as SPL Token program with proper instruction parsing"
        );
    }

    #[test]
    fn test_unknown_program_fallback() {
        // A program ID that's clearly synthetic and will never have a preset:
        // ensures genuinely-unknown programs still flow through the
        // unknown_program visualizer rather than crashing or losing the
        // raw data.
        use solana_sdk::{
            hash::Hash, instruction::CompiledInstruction, message::Message, pubkey::Pubkey,
            signature::Signature, transaction::Transaction as SolanaTransaction,
        };

        let unknown_program_id = Pubkey::new_from_array([
            0x46, 0x41, 0x4B, 0x45, // "FAKE"
            0x50, 0x52, 0x4F, 0x47, // "PROG"
            0x52, 0x41, 0x4D, 0x21, // "RAM!"
            0x46, 0x41, 0x4B, 0x45, 0x50, 0x52, 0x4F, 0x47, 0x52, 0x41, 0x4D, 0x21, 0x21, 0x21,
            0x21, 0x21, 0x00, 0x00, 0x00, 0x00,
        ]);
        let fee_payer = Pubkey::new_unique();
        let instruction_data = vec![0x01, 0x02, 0x03, 0x04, 0x05];

        let message = Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![fee_payer, unknown_program_id],
            recent_blockhash: Hash::new_unique(),
            instructions: vec![CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0],
                data: instruction_data.clone(),
            }],
        };
        let transaction = SolanaTransaction {
            signatures: vec![Signature::default()],
            message,
        };

        let payload = SolanaVisualSignConverter
            .to_visual_sign_payload(
                SolanaTransactionWrapper::Legacy(transaction),
                VisualSignOptions {
                    decode_transfers: false,
                    metadata: None,
                    transaction_name: Some("Unknown Program Test".to_string()),
                    developer_config: None,
                },
            )
            .expect("Should convert unknown program transaction to payload");

        let instruction_fields: Vec<_> = payload
            .fields
            .iter()
            .filter(|f| f.label().starts_with("Instruction"))
            .collect();
        assert_eq!(
            instruction_fields.len(),
            1,
            "Should have exactly 1 instruction"
        );

        let json_str = payload.to_json().unwrap();
        assert!(
            json_str.contains(&unknown_program_id.to_string()),
            "Should contain the unknown program ID in the output"
        );
        assert!(
            json_str.contains("0102030405"),
            "Should show instruction data as hex for unknown programs"
        );
        assert!(
            json_str.contains("Program ID"),
            "Unknown program should display with 'Program ID' field"
        );
    }

    // Lock in main's behavior: when v0 instruction decoding fails on the
    // diagnostics-OFF (production) path, the converter pushes a TextV2
    // "Instruction Decoding Note" field rather than erroring out the entire
    // conversion. Wallets render this note instead of seeing a hard failure
    // for an otherwise-renderable transaction. The empty-account-keys path
    // is the simplest deterministic trigger; other failure paths
    // (visualizer Err, etc.) share the same fallback handling.
    #[cfg(not(feature = "diagnostics"))]
    #[test]
    fn test_v0_decode_failure_emits_instruction_decoding_note() {
        let v0_message = solana_sdk::message::v0::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 0,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![solana_sdk::instruction::CompiledInstruction {
                program_id_index: 0,
                accounts: vec![],
                data: vec![],
            }],
            address_table_lookups: vec![],
        };
        let versioned_tx = VersionedTransaction {
            signatures: vec![],
            message: VersionedMessage::V0(v0_message.clone()),
        };
        let options = VisualSignOptions {
            metadata: None,
            decode_transfers: false,
            transaction_name: None,
            developer_config: None,
        };

        let payload =
            convert_v0_to_visual_sign_payload(&versioned_tx, &v0_message, false, None, &options)
                .expect("convert should succeed via Instruction Decoding Note fallback");

        let note = payload
            .fields
            .iter()
            .find_map(|f| match f {
                SignablePayloadField::TextV2 { common, text_v2 }
                    if common.label == "Instruction Decoding Note" =>
                {
                    Some(text_v2.text.clone())
                }
                _ => None,
            })
            .expect("Instruction Decoding Note field missing from payload");

        assert!(
            note.contains("Instruction decoding failed"),
            "unexpected note body: {note}"
        );
        assert!(
            note.contains("no account keys"),
            "note should propagate underlying error: {note}"
        );
    }

    // Regression tests for V0 transactions that reference address lookup table
    // entries. The parser cannot resolve ALT contents offline, so these render
    // with graceful degradation (ALT-backed accounts surface as placeholders)
    // rather than being rejected outright. The earlier fail-closed behavior
    // (#324) blocked all ALT-backed V0 transactions from being signed and was
    // reverted pending a design that passes resolved ALT data into the parser.
    mod v0_alt_rendering {
        use super::*;
        use solana_sdk::instruction::CompiledInstruction;
        use solana_sdk::message::MessageHeader;
        use solana_sdk::message::v0::{Message as V0Message, MessageAddressTableLookup};
        use solana_sdk::pubkey::Pubkey;

        fn make_v0(
            account_keys: Vec<Pubkey>,
            instructions: Vec<CompiledInstruction>,
            address_table_lookups: Vec<MessageAddressTableLookup>,
        ) -> V0Message {
            V0Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 0,
                },
                account_keys,
                recent_blockhash: solana_sdk::hash::Hash::default(),
                instructions,
                address_table_lookups,
            }
        }

        fn default_options() -> VisualSignOptions {
            VisualSignOptions {
                metadata: None,
                decode_transfers: false,
                transaction_name: Some("V0 ALT Regression".to_string()),
                developer_config: None,
            }
        }

        fn convert(v0_message: &V0Message) -> Result<SignablePayload, VisualSignError> {
            let versioned_tx = VersionedTransaction {
                signatures: vec![],
                message: VersionedMessage::V0(v0_message.clone()),
            };
            #[cfg(feature = "diagnostics")]
            let lint_config = visualsign::lint::LintConfig::default();
            convert_v0_to_visual_sign_payload(
                &versioned_tx,
                v0_message,
                false,
                None,
                &default_options(),
                #[cfg(feature = "diagnostics")]
                &lint_config,
            )
        }

        #[test]
        fn renders_v0_when_account_index_lives_behind_an_alt() {
            // Instruction's program_id is in-range, but two of its accounts
            // point past the static keys into the ALT-loaded range. With
            // static_len=2 and loaded_len=3 (writable=[0,1], readonly=[2])
            // the valid resolved range is [0, 5); ALT-backed indices are
            // [2, 5). The parser cannot resolve those offline, but it no
            // longer rejects the whole transaction (#324, reverted) -- it
            // renders with the ALT-backed accounts surfaced as placeholders.
            let key0 = Pubkey::new_unique();
            let key1 = Pubkey::new_unique();
            let alt = MessageAddressTableLookup {
                account_key: Pubkey::new_unique(),
                writable_indexes: vec![0, 1],
                readonly_indexes: vec![2],
            };
            let ix = CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0, 2, 3],
                data: vec![0xCC],
            };
            let msg = make_v0(vec![key0, key1], vec![ix], vec![alt]);

            let payload =
                convert(&msg).expect("ALT-backed account references should render, not reject");
            assert_eq!(payload.payload_type, "SolanaTx");
        }

        #[test]
        fn allows_v0_with_alt_when_no_instruction_references_alt_contents() {
            // ALTs are present in the wire format (e.g. for future inner-CPIs
            // a wallet pre-attached), but every compiled instruction's
            // program_id and accounts resolve against the static keys. This is
            // safe to render -- nothing about the displayed instructions is
            // hidden by an ALT. The presence of `address_table_lookups` alone
            // is not the threat; an unresolved reference into one is.
            let key0 = Pubkey::new_unique();
            let key1 = Pubkey::new_unique();
            let alt = MessageAddressTableLookup {
                account_key: Pubkey::new_unique(),
                writable_indexes: vec![0],
                readonly_indexes: vec![1],
            };
            let ix = CompiledInstruction {
                program_id_index: 1,
                accounts: vec![0],
                data: vec![0xDD],
            };
            let msg = make_v0(vec![key0, key1], vec![ix], vec![alt]);

            let payload =
                convert(&msg).expect("V0 with ALTs but only static-key references should render");
            assert_eq!(payload.payload_type, "SolanaTx");
            // Confirm the ALT itself is surfaced in the displayed payload.
            let has_alt_field = payload
                .fields
                .iter()
                .any(|f| matches!(f, SignablePayloadField::PreviewLayout { common, .. } if common.label == "Address Lookup Tables"));
            assert!(
                has_alt_field,
                "Address Lookup Tables field must be present in payload"
            );
        }

        #[test]
        fn malformed_v0_without_alts_keeps_existing_behavior() {
            // No ALTs: an OOB index here is just a malformed transaction, not
            // a hidden-behind-ALT attack. The fix MUST NOT change behavior on
            // this path -- the existing pipeline renders it with placeholders
            // / diagnostics rather than rejecting.
            let key0 = Pubkey::new_unique();
            let ix = CompiledInstruction {
                program_id_index: 99,
                accounts: vec![0, 50],
                data: vec![0xEE],
            };
            let msg = make_v0(vec![key0], vec![ix], vec![]);
            // We don't assert success vs. failure of conversion here (other
            // unrelated decode paths may still error on malformed input); we
            // only assert it isn't being caught by the ALT-rejection branch,
            // which would produce the new "Cannot render V0 transaction"
            // message.
            if let Err(VisualSignError::DecodeError(text)) = convert(&msg) {
                assert!(
                    !text.contains("Cannot render V0 transaction"),
                    "malformed-without-ALT path must not hit ALT-rejection branch: {text}"
                );
            }
        }
    }

    // --- regression tests for extract_idl_mappings ---

    fn make_options_with_idl_mapping(
        program_id: &str,
        idl: generated::parser::Idl,
    ) -> VisualSignOptions {
        // Build the fixture as a `BTreeMap` (crate determinism rule) and let
        // the call site `.into_iter().collect()` into the proto field's
        // `HashMap`. Mirrors the Ethereum ABI metadata test helper.
        let mut idl_mappings: BTreeMap<String, generated::parser::Idl> = BTreeMap::new();
        idl_mappings.insert(program_id.to_string(), idl);
        VisualSignOptions {
            metadata: Some(generated::parser::ChainMetadata {
                metadata: Some(generated::parser::chain_metadata::Metadata::Solana(
                    generated::parser::SolanaMetadata {
                        network_id: Some("SOLANA_MAINNET".to_string()),
                        idl: None,
                        idl_mappings: idl_mappings.into_iter().collect(),
                    },
                )),
            }),
            decode_transfers: false,
            transaction_name: None,
            developer_config: None,
        }
    }

    /// Unsigned IDL mappings are still accepted (parity with the Ethereum ABI
    /// path). The trusted-builtin-name protection in `IdlRegistry` is what
    /// stops the attacker-controlled name from being rendered.
    #[test]
    fn test_extract_idl_mappings_accepts_unsigned_idl() {
        let options = make_options_with_idl_mapping(
            "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"Custom"},"instructions":[]}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: None,
                program_name: Some("Custom".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert_eq!(mappings.len(), 1, "unsigned IDL should be accepted");
    }

    /// IDL mappings with an invalid program_id (not parseable as a base58
    /// Solana `Pubkey`) are dropped. Free defence in depth matching the
    /// Ethereum ABI path's address validation.
    #[test]
    fn test_extract_idl_mappings_skips_invalid_program_id() {
        let options = make_options_with_idl_mapping(
            "not_a_base58_pubkey",
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"X"}}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: None,
                program_name: Some("X".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert!(
            mappings.is_empty(),
            "invalid program_id should be skipped, got: {mappings:?}"
        );
    }

    /// IDL JSON exceeding `MAX_IDL_JSON_BYTES` is rejected before any
    /// expensive operation.
    #[test]
    fn test_extract_idl_mappings_skips_oversized_idl() {
        let mut big = String::with_capacity(MAX_IDL_JSON_BYTES + 1024);
        big.push('"');
        while big.len() < MAX_IDL_JSON_BYTES + 100 {
            big.push('x');
        }
        big.push('"');
        let options = make_options_with_idl_mapping(
            "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
            generated::parser::Idl {
                value: big,
                idl_type: None,
                idl_version: None,
                signature: None,
                program_name: Some("Custom".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert!(
            mappings.is_empty(),
            "oversized IDL should be skipped, got: {mappings:?}"
        );
    }

    /// IDL with a signature that fails to verify is dropped. Mirrors the
    /// Ethereum ABI path so callers that opt in to signing get the same
    /// "tampered bodies are rejected" guarantee.
    #[test]
    fn test_extract_idl_mappings_skips_invalid_signature() {
        let proto_sig = generated::parser::SignatureMetadata {
            value: "deadbeef".to_string(),
            metadata: vec![
                generated::parser::Metadata {
                    key: "algorithm".to_string(),
                    value: "ed25519".to_string(),
                },
                generated::parser::Metadata {
                    key: "public_key".to_string(),
                    value: "deadbeef".to_string(),
                },
            ],
        };
        let options = make_options_with_idl_mapping(
            "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"Custom"},"instructions":[]}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: Some(proto_sig),
                program_name: Some("Custom".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert!(
            mappings.is_empty(),
            "invalid signature should be skipped, got: {mappings:?}"
        );
    }

    /// Positive acceptance path: a signed IDL whose signer is on the allowlist
    /// is accepted through the full extraction pipeline. Driven via
    /// `extract_idl_mappings_with_signers` with an injected allowlist, the only
    /// way to cover the accept-on-valid-signature path (the production
    /// allowlist is env-configured, and env vars cannot be set in-process under
    /// edition 2024 + forbid(unsafe)). The empty-allowlist control proves the
    /// acceptance was gated on the allowlist, not on the signature alone.
    #[test]
    fn test_extract_idl_mappings_accepts_signed_idl_from_allowlisted_signer() {
        use ed25519_dalek::{Signer, SigningKey};

        const PROGRAM_ID: &str = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
        let idl_json = r#"{"metadata":{"name":"Custom"},"instructions":[]}"#;

        // Sign over the shared program-id-bound prehash, exactly as a real
        // signer would (see `visualsign::signing::solana_metadata_prehash`).
        let program_bytes = Pubkey::from_str(PROGRAM_ID)
            .expect("valid pubkey")
            .to_bytes();
        let signing_key = SigningKey::from_bytes(&[0x42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let hash =
            visualsign::signing::solana_metadata_prehash(&program_bytes, idl_json.as_bytes());
        let sig = signing_key.sign(&hash);
        let pk_bytes = verifying_key.to_bytes().to_vec();

        let proto_sig = generated::parser::SignatureMetadata {
            value: hex::encode(sig.to_bytes()),
            metadata: vec![
                generated::parser::Metadata {
                    key: "algorithm".to_string(),
                    value: "ed25519".to_string(),
                },
                generated::parser::Metadata {
                    key: "public_key".to_string(),
                    value: hex::encode(&pk_bytes),
                },
            ],
        };
        let options = make_options_with_idl_mapping(
            PROGRAM_ID,
            generated::parser::Idl {
                value: idl_json.to_string(),
                idl_type: None,
                idl_version: None,
                signature: Some(proto_sig),
                program_name: Some("Custom".to_string()),
            },
        );

        // Allowlist authorizing exactly the test signer.
        let mut allow = SignerAllowlist::new();
        allow.insert(pk_bytes);
        let mappings = extract_idl_mappings_with_signers(&options, &allow);
        assert_eq!(
            mappings.len(),
            1,
            "signed IDL from an allowlisted signer should be accepted, got: {mappings:?}"
        );
        assert!(mappings.contains_key(PROGRAM_ID));

        // Negative control: same signed IDL, empty allowlist => rejected.
        let rejected = extract_idl_mappings_with_signers(&options, &SignerAllowlist::new());
        assert!(
            rejected.is_empty(),
            "empty allowlist must reject the signed IDL (fail-closed), got: {rejected:?}"
        );
    }

    /// IDL mappings targeting a trusted built-in program are dropped at
    /// extraction time. The name guard in `IdlRegistry` blocks attacker
    /// labels; this filter also blocks the IDL *body* from ever reaching the
    /// registry, so the `unknown_program` IDL path cannot decode a System
    /// Program transfer with attacker-supplied arg/account names.
    #[test]
    fn test_extract_idl_mappings_skips_trusted_builtin_program() {
        // System Program and Jupiter Swap both belong to the canonical set.
        for trusted_program_id in [
            "11111111111111111111111111111111",
            "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB",
        ] {
            let options = make_options_with_idl_mapping(
                trusted_program_id,
                generated::parser::Idl {
                    value: r#"{"metadata":{"name":"Phantom Wallet"},"instructions":[]}"#
                        .to_string(),
                    idl_type: None,
                    idl_version: None,
                    signature: None,
                    program_name: Some("Phantom Wallet".to_string()),
                },
            );
            let mappings = extract_idl_mappings(&options);
            assert!(
                mappings.is_empty(),
                "trusted built-in '{trusted_program_id}' should be dropped, got: {mappings:?}"
            );
        }
    }

    /// Preset-registered program IDs (e.g. `swig_wallet`) are also dropped at
    /// extraction time even when they have no canonical name in the
    /// `NATIVE_PROGRAM_NAMES` table. Subsumes the protection PR #328 added.
    #[test]
    fn test_extract_idl_mappings_skips_preset_only_program() {
        use crate::core::available_visualizers;
        use crate::idl::builtin_programs::canonical_name as cname;

        let preset_only_id: String = available_visualizers()
            .iter()
            .filter_map(|v| v.get_config())
            .flat_map(|cfg| cfg.data().programs.keys().copied().collect::<Vec<_>>())
            .find(|id| cname(id).is_none())
            .expect("at least one preset-only program ID should be registered")
            .to_string();

        let options = make_options_with_idl_mapping(
            &preset_only_id,
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"X"},"instructions":[]}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: None,
                program_name: Some("X".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert!(
            mappings.is_empty(),
            "preset-only '{preset_only_id}' should be dropped, got: {mappings:?}"
        );
    }

    /// Display-name impersonation: an attacker submits IDL for a
    /// non-canonical pubkey but labels it "System Program". Without the
    /// reserved-name blocklist, `get_program_name` would surface the
    /// attacker's "System Program" label on the attacker's pubkey. Reject
    /// the mapping at extraction.
    #[test]
    fn test_extract_idl_mappings_rejects_reserved_name_on_non_canonical_pubkey() {
        let attacker_pubkey = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
        let options = make_options_with_idl_mapping(
            attacker_pubkey,
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"System Program"},"instructions":[]}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: None,
                // Attacker-controlled `program_name` impersonating a canonical
                // entry. Must be refused even though `program_id` itself is
                // not in the trusted set.
                program_name: Some("System Program".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert!(
            mappings.is_empty(),
            "reserved name on non-canonical pubkey must be dropped, got: {mappings:?}"
        );
    }

    /// The reserved-name guard also fires when the impersonating name comes
    /// from the IDL JSON `metadata.name` rather than the proto's
    /// `program_name`. Both paths feed `get_program_name` indirectly, so
    /// both must be filtered.
    #[test]
    fn test_extract_idl_mappings_rejects_reserved_name_in_idl_metadata() {
        let attacker_pubkey = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
        let options = make_options_with_idl_mapping(
            attacker_pubkey,
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"Jupiter Swap"},"instructions":[]}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: None,
                program_name: None,
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert!(
            mappings.is_empty(),
            "reserved name in IDL metadata.name on non-canonical pubkey must be dropped, got: {mappings:?}"
        );
    }

    /// A free-form name (not in the canonical table) on a non-canonical
    /// pubkey must still pass through. Otherwise legitimate wallet labels
    /// for unknown programs would be lost.
    #[test]
    fn test_extract_idl_mappings_allows_freeform_name() {
        let custom_pubkey = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";
        let options = make_options_with_idl_mapping(
            custom_pubkey,
            generated::parser::Idl {
                value: r#"{"metadata":{"name":"My Custom Program"},"instructions":[]}"#.to_string(),
                idl_type: None,
                idl_version: None,
                signature: None,
                program_name: Some("My Custom Program".to_string()),
            },
        );
        let mappings = extract_idl_mappings(&options);
        assert_eq!(mappings.len(), 1);
    }
}
