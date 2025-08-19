// Generated compile-time data (GenField, GenFormat, SELECTOR_MAP)
include!(concat!(env!("OUT_DIR"), "/erc7730_registry_gen.rs"));
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

/// Given calldata bytes, attempt to produce SignablePayloadFields using the registry.
/// Current implementation is heuristic and does not ABI-decode parameters; it surfaces field
/// labels and paths as plain text fields. Future improvements can plug proper ABI decoding.
pub fn decode_calldata(calldata: &[u8]) -> Option<Vec<SignablePayloadField>> {
    if calldata.len() < 4 {
        return None;
    }
    let selector_hex = format!(
        "0x{:08x}",
        u32::from_be_bytes([calldata[0], calldata[1], calldata[2], calldata[3]])
    );
    let formats = SELECTOR_MAP.get(&*selector_hex)?;
    let format = formats.first()?;
    let mut fields = Vec::new();
    for f in format.fields.iter() {
        let label = f.label.to_string();
        let text_content = f.path.to_string();
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text_content.clone(),
                label,
            },
            text_v2: SignablePayloadFieldTextV2 { text: text_content },
        });
    }
    if fields.is_empty() {
        // Provide at least a raw calldata field if nothing matched
        fields.push(SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: selector_hex.clone(),
                label: "Function Selector".into(),
            },
            text_v2: SignablePayloadFieldTextV2 { text: selector_hex },
        });
    }
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex::FromHex;

    // Helper to build calldata from selector hex string like "0x04e45aaf"
    fn calldata_from_selector(selector: &str) -> Vec<u8> {
        let clean = selector.trim_start_matches("0x");
        let bytes = <[u8; 4]>::from_hex(clean).unwrap();
        bytes.to_vec() // no args appended for these tests
    }

    #[test]
    fn registry_is_populated() {
        assert!(
            SELECTOR_MAP.entries().len() > 0,
            "Registry map should not be empty"
        );
    }

    #[test]
    fn known_uniswap_selector_present() {
        // From calldata-UniswapV3Router02.json: selector 0x04e45aaf (exactInputSingle)
        let selector = "0x04e45aaf";
        let formats = SELECTOR_MAP.get(selector).expect("selector present");
        assert!(
            !formats.is_empty(),
            "Registered formats list should not be empty"
        );
        let first = formats[0];
        assert!(
            first.fields.iter().any(|f| f.label == "Send"),
            "Expected a field with label 'Send'"
        );
    }

    #[test]
    fn decode_calldata_returns_fields_for_known_selector() {
        let selector = "0x04e45aaf"; // exactInputSingle
        let calldata = calldata_from_selector(selector);
        let fields = decode_calldata(&calldata).expect("Should decode fields");
        // Expect at least the number of fields defined in spec (we know some exist)
        assert!(
            fields.len() >= 3,
            "Expected at least 3 fields, got {}",
            fields.len()
        );
        // Ensure labels preserved
        let labels: Vec<_> = fields.iter().map(|f| f.label().clone()).collect();
        assert!(
            labels.iter().any(|l| l == "Send"),
            "Missing 'Send' label in decoded fields: {:?}",
            labels
        );
    }

    #[test]
    fn decode_calldata_unknown_selector_returns_none() {
        // Random selector unlikely to exist
        let calldata = calldata_from_selector("0xdeadbeef");
        assert!(decode_calldata(&calldata).is_none());
    }

    #[test]
    fn decode_calldata_short_input_returns_none() {
        assert!(decode_calldata(&[0x01, 0x02, 0x03]).is_none());
    }

    #[test]
    fn decode_calldata_with_additional_arguments() {
        // Use known selector and append arbitrary bytes simulating encoded params
        let selector = "0x04e45aaf"; // exactInputSingle
        let mut calldata = calldata_from_selector(selector);
        // Append 32 bytes (typical ABI word) of zeros
        calldata.extend_from_slice(&[0u8; 32]);
        let fields = decode_calldata(&calldata).expect("Should decode with extra args");
        assert!(
            fields.len() >= 3,
            "Expected at least 3 fields with args present"
        );
    }

    #[test]
    fn decode_calldata_is_deterministic() {
        let selector = "0x04e45aaf";
        let calldata = calldata_from_selector(selector);
        let a = decode_calldata(&calldata).unwrap();
        let b = decode_calldata(&calldata).unwrap();
        assert_eq!(a, b, "Decoding same calldata should be deterministic");
    }

    #[test]
    fn first_format_field_count_matches_decoded_count_for_all_selectors() {
        // For every selector ensure decode returns exactly the number of fields in the first format
        // (current implementation uses the first format only)
        for (selector, formats) in SELECTOR_MAP.entries() {
            if formats.is_empty() {
                continue;
            }
            let first = formats[0];
            // Build calldata bytes
            let mut raw = Vec::new();
            let hex = selector.trim_start_matches("0x");
            let bytes = <[u8; 4]>::from_hex(hex).expect("valid selector hex");
            raw.extend_from_slice(&bytes);
            let decoded = decode_calldata(&raw).expect("should decode");
            assert_eq!(
                decoded.len(),
                first.fields.len(),
                "selector {selector} field count mismatch"
            );
        }
    }

    #[test]
    fn decoded_fields_have_non_empty_labels_and_fallback_text() {
        // Sample up to first 25 selectors to keep test lean
        for (i, (selector, _)) in SELECTOR_MAP.entries().enumerate() {
            if i >= 25 {
                break;
            }
            let mut raw = Vec::new();
            let hex = selector.trim_start_matches("0x");
            let bytes = <[u8; 4]>::from_hex(hex).expect("valid selector hex");
            raw.extend_from_slice(&bytes);
            let decoded = decode_calldata(&raw).expect("decode");
            for f in decoded {
                let label_empty = f.label().is_empty();
                let fb_empty = f.fallback_text().is_empty();
                assert!(
                    !(label_empty && fb_empty),
                    "selector {selector} has both empty label and fallback_text"
                );
            }
        }
    }
}
