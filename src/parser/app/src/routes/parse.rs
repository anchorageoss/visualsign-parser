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

    // todo: make these request args or metadata
    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
    };
    let registry = create_registry();
    let proto_chain = ProtoChain::from_i32(parse_request.chain)
        .ok_or_else(|| GrpcError::new(Code::InvalidArgument, "invalid chain"))?;
    let registry_chain: VisualSignRegistryChain = chain_conversion::proto_to_registry(proto_chain);

    let signable_payload_str = registry
        .convert_transaction(&registry_chain, request_payload.as_str(), options)
        .map_err(|e| GrpcError::new(Code::InvalidArgument, &format!("{e}")))?;

    // Convert SignablePayload to String (assuming you want JSON)
    let signable_payload = serde_json::to_string(&signable_payload_str).map_err(|e| {
        GrpcError::new(Code::Internal, &format!("Failed to serialize payload: {e}"))
    })?;

    let payload = ParsedTransactionPayload { signable_payload };

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
