//! The L1 action hash (`connectionId`), computed from the action bytes as received.
//!
//! Mirrors `action_hash` in hyperliquid-python-sdk `utils/signing.py`:
//!
//! ```text
//! keccak256( msgpack(action)
//!          ‖ nonce as u64 big-endian
//!          ‖ 0x00                         (no vault)
//!            | 0x01 ‖ 20-byte vault address
//!          ‖ [0x00 ‖ expiresAfter as u64 big-endian]   (only when expiresAfter is set) )
//! ```
//!
//! Python's `msgpack.packb(dict)` writes keys in dict insertion order, which is the
//! order of the JSON the action was serialised to. The action is therefore transcoded
//! straight from its raw JSON text to msgpack, so key order, integer widths and string
//! formats are exactly what was received. Nothing is re-ordered or normalised.

use alloy_primitives::keccak256;

/// Transcode raw action JSON to msgpack, keeping key order.
///
/// Callers must already have refused floats: JSON numbers that are not integers
/// would otherwise be written as msgpack float64.
pub fn action_msgpack(raw_action_json: &str) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut de = serde_json::Deserializer::from_str(raw_action_json);
    let mut ser = rmp_serde::Serializer::new(&mut buf);
    serde_transcode::transcode(&mut de, &mut ser)
        .map_err(|e| format!("cannot transcode action to msgpack: {e}"))?;
    de.end()
        .map_err(|e| format!("trailing data after action: {e}"))?;
    Ok(buf)
}

/// Compute `connectionId` from the msgpack-encoded action and the envelope fields.
pub fn connection_id(
    action_msgpack: &[u8],
    nonce: u64,
    vault_address: Option<&[u8; 20]>,
    expires_after: Option<u64>,
) -> [u8; 32] {
    let mut data = Vec::with_capacity(action_msgpack.len() + 8 + 21 + 9);
    data.extend_from_slice(action_msgpack);
    data.extend_from_slice(&nonce.to_be_bytes());
    match vault_address {
        None => data.push(0x00),
        Some(vault) => {
            data.push(0x01);
            data.extend_from_slice(vault);
        }
    }
    if let Some(expires_after) = expires_after {
        data.push(0x00);
        data.extend_from_slice(&expires_after.to_be_bytes());
    }
    keccak256(&data).0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Python msgpack writes the smallest integer format that fits, and `str`
    /// (not `bin`) for strings. These bytes are what `msgpack.packb` produces
    /// for the same dict under msgpack 1.2.4.
    #[test]
    fn transcoding_matches_python_msgpack_formats() {
        let packed = action_msgpack(
            r#"{"b":1,"a":200,"c":70000,"d":-1,"e":-200,"f":"x","g":true,"h":null,"i":[]}"#,
        )
        .unwrap();
        assert_eq!(
            hex::encode(packed),
            concat!(
                "89", // fixmap, 9 entries, in received order
                "a162",
                "01", // "b": positive fixint
                "a161",
                "ccc8", // "a": uint8 200
                "a163",
                "ce00011170", // "c": uint32 70000
                "a164",
                "ff", // "d": negative fixint
                "a165",
                "d1ff38", // "e": int16 -200
                "a166",
                "a178", // "f": fixstr
                "a167",
                "c3", // "g": true
                "a168",
                "c0", // "h": nil
                "a169",
                "90", // "i": empty fixarray
            )
        );
    }

    #[test]
    fn strings_of_32_bytes_or_more_use_str8() {
        let s = "x".repeat(40);
        let packed = action_msgpack(&format!(r#"{{"k":"{s}"}}"#)).unwrap();
        // fixmap(1), fixstr "k", str8 len 40
        assert_eq!(&packed[..5], &[0x81, 0xa1, b'k', 0xd9, 40]);
    }

    #[test]
    fn rejects_trailing_data() {
        assert!(action_msgpack(r#"{"a":1} {"b":2}"#).is_err());
    }
}
