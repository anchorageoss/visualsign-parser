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
    parse_request: ParseRequest,
    ephemeral_key: &P256Pair,
) -> Result<ParseResponse, GrpcError> {
    let request_payload = parse_request.unsigned_payload;
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
        .convert_transaction(&registry_chain, request_payload.as_str(), options)
        .map_err(|e| GrpcError::new(Code::InvalidArgument, &format!("{e}")))?;

    // Convert SignablePayload to String (assuming you want JSON)
    let parsed_payload_str = serde_json::to_string(&signable_payload_str).map_err(|e| {
        GrpcError::new(Code::Internal, &format!("Failed to serialize payload: {e}"))
    })?;

    // Metadata can be empty; if so, we use an empty vec for hashing to avoid having to deal with
    // optional types in ParsedTransactionPayload.
    let metadata_bytes = if let Some(metadata) = parse_request.chain_metadata {
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
