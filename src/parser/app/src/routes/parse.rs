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

use visualsign::registry::Chain as VisualSignRegistryChain;
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
        abi_registry: None,
    };
    let registry = create_registry();
    let proto_chain = ProtoChain::from_i32(parse_request.chain)
        .ok_or_else(|| GrpcError::new(Code::InvalidArgument, "invalid chain"))?;
    let registry_chain: VisualSignRegistryChain = chain_conversion::proto_to_registry(proto_chain);

    let signable_payload_str = registry
        .convert_transaction(&registry_chain, request_payload, options)
        .map_err(|e| GrpcError::new(Code::InvalidArgument, &format!("{e}")))?;

    // Convert SignablePayload to String (assuming you want JSON)
    let parsed_payload_str = serde_json::to_string(&signable_payload_str).map_err(|e| {
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
    };

    let digest = sha_256(&borsh::to_vec(&payload).expect("payload implements borsh::Serialize"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use generated::parser::{Abi, ChainMetadata, EthereumMetadata, chain_metadata};

    /// Verify that metadata_digest is deterministic regardless of abi_mappings insertion order.
    /// This guards against accidental reintroduction of HashMap-backed map fields.
    #[test]
    fn metadata_digest_is_deterministic_across_insertion_orders() {
        let abi_a = Abi {
            value: r#"[{"name":"transfer"}]"#.to_string(),
            signature: None,
        };
        let abi_b = Abi {
            value: r#"[{"name":"approve"}]"#.to_string(),
            signature: None,
        };

        // Insert in order A, B
        let mut mappings_ab = std::collections::BTreeMap::new();
        mappings_ab.insert("0xaaaa".to_string(), abi_a.clone());
        mappings_ab.insert("0xbbbb".to_string(), abi_b.clone());

        // Insert in order B, A
        let mut mappings_ba = std::collections::BTreeMap::new();
        mappings_ba.insert("0xbbbb".to_string(), abi_b);
        mappings_ba.insert("0xaaaa".to_string(), abi_a);

        let metadata_ab = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: mappings_ab,
            })),
        };
        let metadata_ba = ChainMetadata {
            metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                network_id: Some("ETHEREUM_MAINNET".to_string()),
                abi: None,
                abi_mappings: mappings_ba,
            })),
        };

        let bytes_ab = borsh::to_vec(&metadata_ab).unwrap();
        let bytes_ba = borsh::to_vec(&metadata_ba).unwrap();
        assert_eq!(
            sha_256(&bytes_ab),
            sha_256(&bytes_ba),
            "metadata_digest must be identical regardless of map insertion order"
        );
    }
}
