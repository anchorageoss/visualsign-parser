//! Parsing endpoint for `VisualSign`

use crate::{chain_conversion, errors::GrpcError, registry::create_registry};
use generated::parser::Chain as ProtoChain;
use generated::{
    google::rpc::Code,
    parser::{
        ParseRequest, ParseResponse, ParsedTransaction, ParsedTransactionPayload, Signature,
        SignatureScheme,
    },
};
use qos_crypto::sha_256;
use qos_p256::P256Pair;

use visualsign::errors::VisualSignError;
use visualsign::registry::{Chain as VisualSignRegistryChain, TransactionConverterRegistry};
use visualsign::vsptrait::VisualSignOptions;

/// Parses an unsigned transaction payload and returns a signed parsed response.
///
/// # Panics
///
/// Panics if the `ParsedTransactionPayload` cannot be serialized to Borsh format.
/// This should never happen as the payload type implements `borsh::BorshSerialize`.
pub fn parse(
    parse_request: &ParseRequest,
    ephemeral_key: &P256Pair,
) -> Result<ParseResponse, GrpcError> {
    let registry = create_registry();
    parse_with_registry(parse_request, ephemeral_key, &registry)
}

/// Same as [`parse`] but accepts a caller-provided registry. Exists primarily as
/// a test seam so unit tests can inject stub converters and exercise the
/// `parse()` pipeline without depending on the full production registry.
pub(crate) fn parse_with_registry(
    parse_request: &ParseRequest,
    ephemeral_key: &P256Pair,
    registry: &TransactionConverterRegistry,
) -> Result<ParseResponse, GrpcError> {
    let request_payload = parse_request.unsigned_payload.as_str();
    if request_payload.is_empty() {
        return Err(GrpcError::new(
            Code::InvalidArgument,
            "unsigned transaction is empty",
        ));
    }

    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: parse_request.chain_metadata.clone(),
        developer_config: None, // Production API: only accept unsigned transactions
        include_intermediate_output: parse_request.include_intermediate_output,
    };
    let proto_chain = ProtoChain::try_from(parse_request.chain)
        .map_err(|_| GrpcError::new(Code::InvalidArgument, "invalid chain"))?;
    let registry_chain: VisualSignRegistryChain = chain_conversion::proto_to_registry(proto_chain);

    let conversion = registry
        .convert_transaction(&registry_chain, request_payload, options)
        .map_err(|e| GrpcError::new(Code::InvalidArgument, &format!("{e}")))?;
    let signable_payload = conversion.payload;
    let intermediate_output = conversion.intermediate_output;

    // Defense-in-depth: validate the charset of the SignablePayload unconditionally
    // on the signing path, regardless of which converter produced it. Per-converter
    // validation can be skipped by chain-specific overrides of
    // `to_visual_sign_payload_from_string` (e.g. Ethereum), so we enforce here too.
    // Caller-supplied metadata (Ethereum `abi_mappings`, Solana `idl_mappings`)
    // could otherwise smuggle bidi controls or zero-width characters into the
    // displayed strings.
    //
    // `validate_charset` may also return non-validation errors (e.g.
    // `SerializationError` if internal JSON serialization fails). Those are
    // server-side bugs, not client input problems, so map them to `Internal`
    // and reserve `InvalidArgument` for genuine validation rejections.
    signable_payload.validate_charset().map_err(|e| match e {
        VisualSignError::ValidationError(_) => {
            GrpcError::new(Code::InvalidArgument, &format!("{e}"))
        }
        _ => GrpcError::new(Code::Internal, &format!("{e}")),
    })?;

    // Convert SignablePayload to String (assuming you want JSON)
    let parsed_payload_str = serde_json::to_string(&signable_payload).map_err(|e| {
        GrpcError::new(Code::Internal, &format!("Failed to serialize payload: {e}"))
    })?;

    // Metadata can be empty; if so, we use an empty vec for hashing to avoid having to deal with
    // optional types in ParsedTransactionPayload.
    let metadata_bytes = if let Some(metadata) = parse_request.chain_metadata.as_ref() {
        borsh::to_vec(&metadata).expect("chain_metadata implements borsh::Serialize")
    } else {
        vec![]
    };

    let payload = ParsedTransactionPayload {
        parsed_payload: parsed_payload_str.clone(),
        input_payload_digest: qos_hex::encode(&sha_256(request_payload.as_bytes())),
        metadata_digest: qos_hex::encode(&sha_256(&metadata_bytes)),
        // TODO: remove me once clients have migrated and rely on the fields above
        signable_payload: parsed_payload_str,
        // Move the converter's bytes in verbatim. The digest below is computed
        // from this same field (see `signing_digest_bytes`), so the bytes that
        // are transported are, by construction, the bytes that are signed.
        intermediate_output: intermediate_output.unwrap_or_default(),
    };

    let digest = sha_256(&signing_digest_bytes(&payload));
    let sig = ephemeral_key
        .sign(&digest)
        .map_err(|e| GrpcError::new(Code::Internal, &format!("{e:?}")))?;

    let signature = Signature {
        public_key: qos_hex::encode(&ephemeral_key.public_key().to_bytes()),
        signature: qos_hex::encode(&sig),
        message: qos_hex::encode(&digest),
        scheme: SignatureScheme::TurnkeyP256EphemeralKey as i32,
    };

    Ok(ParseResponse {
        parsed_transaction: Some(ParsedTransaction {
            payload: Some(payload),
            signature: Some(signature),
        }),
    })
}

/// Compute the bytes the ephemeral key signs over for a `ParsedTransactionPayload`.
///
/// This is the single source of truth for the signed digest, derived entirely
/// from the `payload` argument — so the `intermediate_output` that is signed is
/// exactly the field carried on the response, never a separately-held copy.
///
/// Backward-compatibility contract: `intermediate_output` is excluded from the
/// derived Borsh encoding (`#[borsh(skip)]`, applied by codegen). When it is
/// empty, this returns exactly `borsh(payload)` — byte-for-byte identical to
/// the pre-feature four-field encoding, so existing signatures/consumers are
/// unaffected. When it is non-empty, its Borsh `Vec<u8>` encoding (u32-LE
/// length prefix followed by the raw bytes, matching a downstream borsh
/// encoder) is appended. Presence is out-of-band (no tag byte), so an
/// intermediate-unaware consumer that also appends nothing agrees on the empty
/// case and correctly rejects the non-empty case it cannot verify.
fn signing_digest_bytes(payload: &ParsedTransactionPayload) -> Vec<u8> {
    let mut bytes = borsh::to_vec(payload).expect("payload implements borsh::Serialize");
    if !payload.intermediate_output.is_empty() {
        bytes.extend_from_slice(
            &borsh::to_vec(&payload.intermediate_output)
                .expect("Vec<u8> implements borsh::Serialize"),
        );
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use generated::parser::{Abi, ChainMetadata, EthereumMetadata, chain_metadata};
    use std::collections::BTreeMap;
    use visualsign::vsptrait::{
        ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
        VisualSignConverterFromString,
    };
    use visualsign::{
        SignablePayload, SignablePayloadField, SignablePayloadFieldCommon,
        SignablePayloadFieldTextV2,
    };

    /// Verify that `metadata_digest` is deterministic for identical metadata,
    /// including non-empty `abi_mappings` — the proto map field is now a `BTreeMap`
    /// (after the tonic 0.10 / `tonic_build::Builder::btree_map(["."])` config), so
    /// borsh serialization sees keys in a consistent order regardless of insertion
    /// order. This test exercises that contract.
    #[test]
    fn metadata_digest_is_deterministic() {
        let transfer_abi = Abi {
            value: r#"[{"name":"transfer"}]"#.to_string(),
            signature: None,
            ..Default::default()
        };
        let approve_abi = Abi {
            value: r#"[{"name":"approve"}]"#.to_string(),
            signature: None,
            ..Default::default()
        };

        // Build two maps with the SAME entries but OPPOSITE insertion orders. The
        // proto field is a `BTreeMap` (forced via `btree_map(["."])`), so both must
        // serialize to identical borsh bytes regardless of the order keys were
        // inserted -- that is the determinism contract this test pins.
        let mut abi_mappings_forward = BTreeMap::new();
        abi_mappings_forward.insert("0xaaaa".to_string(), transfer_abi.clone());
        abi_mappings_forward.insert("0xbbbb".to_string(), approve_abi.clone());

        let mut abi_mappings_reverse = BTreeMap::new();
        abi_mappings_reverse.insert("0xbbbb".to_string(), approve_abi);
        abi_mappings_reverse.insert("0xaaaa".to_string(), transfer_abi);

        let metadata_a = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: abi_mappings_forward,
            })),
        };
        let metadata_b = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi_mappings: abi_mappings_reverse,
            })),
        };

        let bytes_a = borsh::to_vec(&metadata_a).expect("borsh serialization");
        let bytes_b = borsh::to_vec(&metadata_b).expect("borsh serialization");
        assert_eq!(
            sha_256(&bytes_a),
            sha_256(&bytes_b),
            "metadata_digest must be identical regardless of map insertion order"
        );
    }

    // ----------------------------------------------------------------------
    // Charset-bypass regression test infrastructure
    // ----------------------------------------------------------------------
    //
    // The fix under test is: `parse_with_registry` must invoke
    // `SignablePayload::validate_charset` unconditionally on every signing
    // path, regardless of which converter ran.
    //
    // Per-converter validation can be skipped by chain-specific overrides of
    // `to_visual_sign_payload_from_string` (e.g. Ethereum's override at
    // `chain_parsers/visualsign-ethereum/src/lib.rs` calls
    // `to_visual_sign_payload` directly, bypassing the default's
    // `to_validated_visual_sign_payload`).
    //
    // To prove the property holds regardless of converter, the tests below
    // register a stub converter that intentionally emits a `SignablePayload`
    // containing a non-ASCII character (U+202E RIGHT-TO-LEFT OVERRIDE) in
    // its field strings. Without the unconditional check in
    // `parse_with_registry`, this poisoned payload would be signed verbatim.

    /// Stub transaction whose `from_string` always succeeds; the converter
    /// decides what payload to emit based on construction args, not on tx data.
    #[derive(Debug, Clone)]
    struct StubTransaction;

    impl Transaction for StubTransaction {
        fn from_string(_data: &str) -> Result<Self, TransactionParseError> {
            Ok(Self)
        }

        fn transaction_type(&self) -> String {
            "Stub".to_string()
        }
    }

    /// Stub converter that emits a `SignablePayload` whose label contains
    /// the configured marker string. Uses the default
    /// `to_visual_sign_payload_from_string` impl (so the default
    /// `VisualSignConverterFromString` path is exercised) but overrides
    /// `to_validated_visual_sign_payload` below to skip its inner
    /// `validate_charset` call. This is a different bypass surface than
    /// `BypassingConverter`, which overrides `to_visual_sign_payload_from_string`
    /// itself, and makes the regression test fail closed if the unconditional
    /// check in `parse_with_registry` is removed.
    struct StubConverter {
        label_text: String,
    }

    impl VisualSignConverter<StubTransaction> for StubConverter {
        fn to_visual_sign_payload(
            &self,
            _transaction: StubTransaction,
            _options: VisualSignOptions,
        ) -> Result<ConversionResult, VisualSignError> {
            Ok(ConversionResult::new(SignablePayload::new(
                0,
                "Stub Transaction".to_string(),
                None,
                vec![SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: self.label_text.clone(),
                        label: self.label_text.clone(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: self.label_text.clone(),
                    },
                }],
                "StubTx".to_string(),
            )))
        }

        /// Intentionally skips the default's `validate_charset()` call so the
        /// default `to_visual_sign_payload_from_string` path delivers an
        /// unvalidated payload to `parse_with_registry`. Without the
        /// unconditional check there, the poisoned payload would reach signing.
        fn to_validated_visual_sign_payload(
            &self,
            transaction: StubTransaction,
            options: VisualSignOptions,
        ) -> Result<ConversionResult, VisualSignError> {
            self.to_visual_sign_payload(transaction, options)
        }
    }

    /// Converter whose `to_visual_sign_payload_from_string` bypasses the
    /// default's `to_validated_visual_sign_payload` wrapper, mirroring the
    /// production Ethereum converter's override. This is the discriminating
    /// surface: without the unconditional check in `parse_with_registry`,
    /// payloads from this converter would reach signing unvalidated.
    struct BypassingConverter {
        label_text: String,
    }

    impl VisualSignConverter<StubTransaction> for BypassingConverter {
        fn to_visual_sign_payload(
            &self,
            _transaction: StubTransaction,
            _options: VisualSignOptions,
        ) -> Result<ConversionResult, VisualSignError> {
            Ok(ConversionResult::new(SignablePayload::new(
                0,
                "Stub Transaction".to_string(),
                None,
                vec![SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: self.label_text.clone(),
                        label: self.label_text.clone(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: self.label_text.clone(),
                    },
                }],
                "StubTx".to_string(),
            )))
        }
    }

    impl VisualSignConverterFromString<StubTransaction> for BypassingConverter {
        fn to_visual_sign_payload_from_string(
            &self,
            transaction_data: &str,
            options: VisualSignOptions,
        ) -> Result<ConversionResult, VisualSignError> {
            // NOTE: intentionally calls `to_visual_sign_payload` directly,
            // skipping `to_validated_visual_sign_payload`. Mirrors the
            // production Ethereum override.
            let transaction = StubTransaction::from_string(transaction_data)
                .map_err(VisualSignError::ParseError)?;
            self.to_visual_sign_payload(transaction, options)
        }
    }

    impl VisualSignConverterFromString<StubTransaction> for StubConverter {}

    /// Builds a request targeting the `Tron` chain slot. We pick `Tron` because
    /// `chain_conversion::proto_to_registry` maps `ProtoChain::Tron` to
    /// `RegistryChain::Tron`, which is what we register the stub under.
    fn stub_request() -> ParseRequest {
        ParseRequest {
            include_intermediate_output: false,
            unsigned_payload: "stub".to_string(),
            chain: ProtoChain::Tron as i32,
            chain_metadata: None,
        }
    }

    /// Regression: when the converter overrides
    /// `to_visual_sign_payload_from_string` to bypass charset validation (as
    /// the production Ethereum converter does), `parse_with_registry` must
    /// still reject payloads containing non-ASCII characters before signing.
    #[test]
    fn parse_rejects_non_ascii_payload_when_converter_skips_validation() {
        // U+202E RIGHT-TO-LEFT OVERRIDE: a bidi control that flips display
        // order and is the canonical spoofing primitive.
        let poisoned = "transfer\u{202E}approve";

        let mut registry = TransactionConverterRegistry::new();
        registry.register::<StubTransaction, _>(
            VisualSignRegistryChain::Tron,
            BypassingConverter {
                label_text: poisoned.to_string(),
            },
        );

        let key = P256Pair::generate().expect("generate ephemeral key");
        let err = parse_with_registry(&stub_request(), &key, &registry).expect_err(
            "parse_with_registry must reject payloads whose strings contain \
             non-ASCII characters, even when the converter skipped its own \
             charset validation",
        );
        assert_eq!(
            err.code,
            Code::InvalidArgument,
            "charset validation failure should map to InvalidArgument, got: {err:?}",
        );
    }

    /// Sanity counterpart: a benign ASCII-only payload through the same
    /// bypassing converter must still sign successfully. Confirms the
    /// unconditional `validate_charset` call doesn't reject legitimate input.
    #[test]
    fn parse_accepts_ascii_payload_when_converter_skips_validation() {
        let mut registry = TransactionConverterRegistry::new();
        registry.register::<StubTransaction, _>(
            VisualSignRegistryChain::Tron,
            BypassingConverter {
                label_text: "benign label".to_string(),
            },
        );

        let key = P256Pair::generate().expect("generate ephemeral key");
        let response = parse_with_registry(&stub_request(), &key, &registry).expect(
            "benign ASCII payload must still parse successfully through a \
             converter that skipped its own charset validation",
        );
        assert!(response.parsed_transaction.is_some());
    }

    /// Regression: covers a second bypass surface. `StubConverter`
    /// uses the default `to_visual_sign_payload_from_string` impl but
    /// overrides `to_validated_visual_sign_payload` to skip the inner
    /// `validate_charset` call. Without the unconditional check in
    /// `parse_with_registry`, the poisoned payload would reach signing
    /// through this path. Pairs with the `BypassingConverter` test, which
    /// covers the other bypass surface (overriding
    /// `to_visual_sign_payload_from_string` itself).
    #[test]
    fn parse_rejects_non_ascii_payload_via_default_converter_path() {
        let poisoned = "transfer\u{202E}approve";

        let mut registry = TransactionConverterRegistry::new();
        registry.register::<StubTransaction, _>(
            VisualSignRegistryChain::Tron,
            StubConverter {
                label_text: poisoned.to_string(),
            },
        );

        let key = P256Pair::generate().expect("generate ephemeral key");
        let err = parse_with_registry(&stub_request(), &key, &registry).expect_err(
            "parse_with_registry must reject non-ASCII payloads even when the \
             converter skips its inner validate_charset call",
        );
        assert_eq!(err.code, Code::InvalidArgument);
    }

    fn sample_payload(intermediate_output: Vec<u8>) -> ParsedTransactionPayload {
        ParsedTransactionPayload {
            parsed_payload: "parsed".to_string(),
            input_payload_digest: "input".to_string(),
            metadata_digest: "meta".to_string(),
            signable_payload: "signable".to_string(),
            intermediate_output,
        }
    }

    /// Backward-compatibility: with an empty `intermediate_output`, the bytes we
    /// sign are byte-for-byte identical to the legacy four-field Borsh encoding.
    /// A parser/HSM that predates the field computes the same digest, so existing
    /// signatures keep verifying and rollout is incremental.
    #[test]
    fn signing_digest_is_backward_compatible_when_intermediate_empty() {
        let payload = sample_payload(vec![]);
        // `intermediate_output` is `#[borsh(skip)]`, so `borsh::to_vec` yields the
        // exact pre-feature four-field encoding.
        let legacy = borsh::to_vec(&payload).expect("borsh");
        assert_eq!(
            signing_digest_bytes(&payload),
            legacy,
            "empty intermediate_output must not change the signed bytes",
        );
    }

    /// When present, the intermediate bytes are appended as a Borsh `Vec<u8>`
    /// (u32-LE length prefix + raw bytes) after the unchanged four-field prefix.
    #[test]
    fn signing_digest_appends_intermediate_when_present() {
        let base = signing_digest_bytes(&sample_payload(vec![]));
        let payload = sample_payload(vec![1, 2, 3, 4]);

        // The four-field Borsh prefix is unchanged by the skipped field.
        assert_eq!(
            borsh::to_vec(&payload).expect("borsh"),
            base,
            "the derived Borsh encoding must ignore intermediate_output",
        );

        let mut expected = base.clone();
        expected.extend_from_slice(&borsh::to_vec(&payload.intermediate_output).expect("borsh"));
        assert_eq!(
            signing_digest_bytes(&payload),
            expected,
            "present intermediate_output must be appended as borsh(Vec<u8>)",
        );
        assert!(signing_digest_bytes(&payload).len() > base.len());
    }
}
