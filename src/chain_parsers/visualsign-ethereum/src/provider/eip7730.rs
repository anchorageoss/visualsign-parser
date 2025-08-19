use crate::registry;
use visualsign::{SignablePayload, SignablePayloadField};
use crate::decode_transaction_bytes; // reuse low-level decoder
use alloy_consensus::Transaction as _; // bring trait into scope for .input()

/// Decode a raw Ethereum transaction (RLP bytes) into SignablePayloadFields using the
/// embedded ERC-7730 registry. Returns None if decoding fails or no registry match.
pub fn decode_raw_transaction_to_fields(raw: &[u8]) -> Option<Vec<SignablePayloadField>> {
	let tx = decode_transaction_bytes(raw).ok()?;
	// Only legacy and EIP-1559 currently supported for calldata extraction here
	let input: Vec<u8> = match &tx {
		alloy_consensus::TypedTransaction::Legacy(t) => t.input().to_vec(),
		alloy_consensus::TypedTransaction::Eip1559(t) => t.input().to_vec(),
		_ => return None,
	};
	if input.is_empty() { return None; }
	registry::decode_calldata(&input)
}

/// Convenience function building a SignablePayload with fields derived from registry.
pub fn decode_raw_transaction_to_payload(raw: &[u8]) -> Option<SignablePayload> {
	let fields = decode_raw_transaction_to_fields(raw)?;
	Some(SignablePayload {
		fields,
		payload_type: "ethereum_tx".to_string(),
		subtitle: None,
		title: "Ethereum Transaction".to_string(),
		version: "1".to_string(),
	})
}
