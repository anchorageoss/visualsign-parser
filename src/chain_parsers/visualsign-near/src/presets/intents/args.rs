//! JSON deserialization of execute_intents args.

use defuse_core::payload::multi::MultiPayload;

use super::NearIntentsError;

/// The `execute_intents(signed: Vec<MultiPayload>)` argument object. NEAR
/// serializes method args as a JSON object keyed by parameter name, so the raw
/// args bytes are `{ "signed": [ ... ] }`.
#[derive(serde::Deserialize)]
struct ExecuteIntentsArgs {
    signed: Vec<MultiPayload>,
}

/// Decode raw `execute_intents` args into the signed payload list.
///
/// `MultiPayload` is a defuse type; it stays `pub(crate)` and never crosses the
/// public crate boundary.
pub(crate) fn decode_args(args: &[u8]) -> Result<Vec<MultiPayload>, NearIntentsError> {
    let parsed: ExecuteIntentsArgs =
        serde_json::from_slice(args).map_err(|e| NearIntentsError::ArgsNotJson(e.to_string()))?;
    Ok(parsed.signed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_json() {
        assert!(decode_args(b"not json").is_err());
    }

    #[test]
    fn decodes_empty_signed_array() {
        let payloads = decode_args(br#"{"signed":[]}"#).expect("decode");
        assert!(payloads.is_empty());
    }
}
