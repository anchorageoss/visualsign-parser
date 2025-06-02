//! Parsing endpoint for `VisualSign`

use generated::{
    google::rpc::Code,
    parser::{
        Metadata, ParseRequest, ParseResponse, ParsedTransaction, ParsedTransactionPayload,
        Signature, SignatureScheme,
    },
};
use qos_crypto::sha_256;
use qos_p256::P256Pair;
use std::string::ToString;

use crate::errors::GrpcError;

pub fn parse(
    parse_request: ParseRequest,
    ephemeral_key: &P256Pair,
) -> Result<ParseResponse, GrpcError> {
    let unsigned_payload = parse_request.unsigned_payload;
    if unsigned_payload.is_empty() {
        return Err(GrpcError::new(
            Code::InvalidArgument,
            "unsigned transaction is empty",
        ));
    }

    let payload = ParsedTransactionPayload {
        transaction_metadata: vec![Metadata {
            key: "tx_foo".to_string(),
            value: "tx_bar".to_string(),
        }],
        method_metadata: vec![Metadata {
            key: "method_baz".to_string(),
            value: "method_quux".to_string(),
        }],
        unsigned_payload,
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
