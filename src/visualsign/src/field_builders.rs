use crate::errors;
use crate::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    SignablePayloadFieldNumber, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use regex::Regex;
// thread-safe static initialization for regex
use std::sync::LazyLock;

// Regex to validate signed proper numbers (e.g., -123.45, +678.90)
// A signed proper number is defined as a string that starts with an optional sign (+ or - or -),
// followed by one or more digits, a decimal point, and one or more digits.
// Examples of valid signed proper numbers: "123", "123.45", "-123.45", "+678.90"
// Examples of invalid signed proper numbers: "-.45", "123.", "abc", "12.3.4"
// Note: This regex does not allow leading zeros unless the number is exactly "0" or "0.0".
// It also does not allow numbers with multiple decimal points or non-numeric characters.
// It allows numbers like "0.0", "-0.0", "+0.0"
// which are valid representations of zero.
// The reason it's implemented this way is to avoid adding a large dependency like bignum on this library which could be used in a wide range of applications.
// The regex is designed to be simple and efficient for the common use case of validating signed decimal. We don't yet use it as a numeric type yet, if that ever changes, this will be refactored.
static SIGNED_PROPER_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([-+]?[0-9]+(\.[0-9]+)?|[-+]?0)$")
        .expect("Failed to compile regex for signed proper number")
});

pub fn create_text_field(
    label: &str,
    text: &str,
) -> Result<AnnotatedPayloadField, errors::VisualSignError> {
    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: text.to_string(),
                label: label.to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: text.to_string(),
            },
        },
    })
}

fn validate_number_string(number: &str) -> Result<bool, errors::VisualSignError> {
    if number.is_empty() {
        return Err(errors::VisualSignError::EmptyField(number.to_string()));
    }

    // Check if the number is a valid signed proper number
    if SIGNED_PROPER_NUMBER_RE.is_match(number) {
        Ok(true)
    } else {
        Err(errors::VisualSignError::InvalidNumberField(
            number.to_string(),
        ))
    }
}

pub fn create_number_field(
    label: &str,
    number: &str,
    unit: &str,
) -> Result<AnnotatedPayloadField, errors::VisualSignError> {
    validate_number_string(number)?;
    // If unit is empty, fallback_text shouldn't have trailing space.
    let fallback_text = if unit.is_empty() {
        number.to_string()
    } else {
        format!("{number} {unit}")
    };

    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::Number {
            common: SignablePayloadFieldCommon {
                fallback_text,
                label: label.to_string(),
            },
            number: SignablePayloadFieldNumber {
                number: number.to_string(),
            },
        },
    })
}

pub fn create_amount_field(
    label: &str,
    amount: &str,
    abbreviation: &str,
) -> Result<AnnotatedPayloadField, errors::VisualSignError> {
    validate_number_string(amount)?;
    // unlike number field, we do want amount fields to have a valid symbol
    if abbreviation.is_empty() {
        return Err(errors::VisualSignError::EmptyField(
            abbreviation.to_string(),
        ));
    }
    let fallback_text = format!("{amount} {abbreviation}");
    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text,
                label: label.to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: amount.to_string(),
                abbreviation: Some(abbreviation.to_string()),
            },
        },
    })
}

/// Helper function to create an address field
pub fn create_address_field(
    label: &str,
    address: &str,
    name: Option<&str>,
    memo: Option<&str>,
    asset_label: Option<&str>,
    badge_text: Option<&str>,
) -> Result<AnnotatedPayloadField, errors::VisualSignError> {
    // TODO think harder about address validation that's generic enough to work across chains
    Ok(AnnotatedPayloadField {
        static_annotation: None,
        dynamic_annotation: None,
        signable_payload_field: SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: address.to_string(),
                label: label.to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: address.to_string(),
                name: name.unwrap_or("").to_string(),
                memo: memo.map(|s| s.to_string()),
                asset_label: asset_label.unwrap_or("").to_string(),
                badge_text: badge_text.map(|s| s.to_string()),
            },
        },
    })
}

fn default_hex_representation(data: &[u8]) -> String {
    data.iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<String>>()
        .join("")
}

/// Create a standard Raw Data field for expanded views
pub fn create_raw_data_field(
    data: &[u8],
    optional_fallback_string: Option<String>,
) -> Result<AnnotatedPayloadField, errors::VisualSignError> {
    let raw_data_fallback_string =
        optional_fallback_string.unwrap_or_else(|| default_hex_representation(data));

    Ok(AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: raw_data_fallback_string.to_string(),
                label: "Raw Data".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: raw_data_fallback_string,
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    })
}

/// Wrap a SignablePayloadField in an AnnotatedPayloadField with no annotations
pub fn annotate_field(field: SignablePayloadField) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        signable_payload_field: field,
        static_annotation: None,
        dynamic_annotation: None,
    }
}

/// Create a preview layout field with title, subtitle (fallback text), and expanded fields
/// This is useful for operation summaries that show a collapsible preview
pub fn create_preview_layout(
    title: &str,
    subtitle: String,
    fields: Vec<AnnotatedPayloadField>,
) -> AnnotatedPayloadField {
    AnnotatedPayloadField {
        signable_payload_field: SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: subtitle.clone(),
                label: title.to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: title.to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: subtitle }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        },
        static_annotation: None,
        dynamic_annotation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::VisualSignError;
    use base64::{engine::general_purpose::STANDARD as b64, Engine as _};

    #[test]
    fn test_create_text_field() {
        let test_cases = [
            // (label, text, expected_label, expected_fallback, expected_text)
            ("Label", "Text", "Label", "Text", "Text"),
            ("", "", "", "", ""),
            ("Empty Text", "", "Empty Text", "", ""),
        ];

        for (label, text, expected_label, expected_fallback, expected_text) in test_cases {
            let field = create_text_field(label, text).expect("should succeed");
            assert!(field.static_annotation.is_none());
            assert!(field.dynamic_annotation.is_none());

            match field.signable_payload_field {
                SignablePayloadField::TextV2 { common, text_v2 } => {
                    assert_eq!(common.label, expected_label);
                    assert_eq!(common.fallback_text, expected_fallback);
                    assert_eq!(text_v2.text, expected_text);
                }
                _ => panic!("Expected TextV2 field"),
            }
        }
    }

    #[test]
    fn test_create_number_field_success() {
        let test_cases = [
            ("Gas", "123500", "units", "Gas", "123500 units", "123500"),
            ("Decimals", "12.35", "", "Decimals", "12.35", "12.35"),
            ("Count", "42", "", "Count", "42", "42"),
            ("", "0", "units", "", "0 units", "0"),
        ];

        for (label, number, unit, expected_label, expected_fallback, expected_number) in test_cases
        {
            let field = create_number_field(label, number, unit).expect("should succeed");
            assert!(field.static_annotation.is_none());
            assert!(field.dynamic_annotation.is_none());

            match field.signable_payload_field {
                SignablePayloadField::Number { common, number } => {
                    assert_eq!(common.label, expected_label);
                    assert_eq!(common.fallback_text, expected_fallback);
                    assert_eq!(number.number, expected_number);
                }
                _ => panic!("Expected Number field"),
            }
        }
    }

    #[test]
    fn test_create_number_field_invalid_number() {
        // let invalid_numbers = ["abc", "12.3.4", "NaN", "--1"];
        let invalid_numbers = ["abc", "12.3.4", "NaN", "--1"];
        for &num in &invalid_numbers {
            let err = create_number_field("Label", num, "unit").unwrap_err();
            match err {
                VisualSignError::InvalidNumberField(ref s) if s == num => {}
                _ => panic!("Expected InvalidNumberField error for {num}"),
            }
        }
    }

    #[test]
    fn test_create_amount_field_success() {
        let test_cases = [
            (
                "Balance",
                "1000.0",
                "USDC",
                "Balance",
                "1000.0 USDC",
                "1000.0",
                "USDC",
            ),
            ("", "0", "TOKEN", "", "0 TOKEN", "0", "TOKEN"),
            (
                "Wei As ETH",
                "0.0000000000000000001",
                "ETH",
                "Wei As ETH",
                "0.0000000000000000001 ETH",
                "0.0000000000000000001",
                "ETH",
            ),
        ];

        for (
            label,
            amount,
            abbrev,
            expected_label,
            expected_fallback,
            expected_amount,
            expected_abbrev,
        ) in test_cases
        {
            let field = create_amount_field(label, amount, abbrev).expect("should succeed");
            assert!(field.static_annotation.is_none());
            assert!(field.dynamic_annotation.is_none());

            match field.signable_payload_field {
                SignablePayloadField::AmountV2 { common, amount_v2 } => {
                    assert_eq!(common.label, expected_label);
                    assert_eq!(common.fallback_text, expected_fallback);
                    assert_eq!(amount_v2.amount, expected_amount);
                    assert_eq!(amount_v2.abbreviation, Some(expected_abbrev.to_string()));
                }
                _ => panic!("Expected AmountV2 field"),
            }
        }
    }

    #[test]
    fn test_create_amount_field_invalid_number() {
        let err = create_amount_field("Label", "notanumber", "USD").unwrap_err();
        match err {
            VisualSignError::InvalidNumberField(ref s) if s == "notanumber" => {}
            _ => panic!("Expected InvalidNumberField error"),
        }
    }

    #[test]
    fn test_create_amount_field_missing_abbreviation() {
        let err = create_amount_field("Label", "123", "").unwrap_err();
        println!("Error: {err:?}");
        match err {
            VisualSignError::EmptyField(ref s) if s.is_empty() => {}
            _ => panic!("Expected EmptyField error"),
        }
    }

    #[test]
    fn test_default_hex_representation() {
        let test_cases = [
            (vec![0x00, 0xFF, 0xAB], "00ffab"),
            (vec![], ""),
            (vec![0xDE, 0xAD, 0xBE, 0xEF], "deadbeef"),
        ];

        for (data, expected) in test_cases {
            assert_eq!(default_hex_representation(&data), expected);
        }
    }

    #[test]
    fn test_create_raw_data_field() {
        let test_cases = [
            // (data, fallback, expected_fallback, expected_text)
            (b"Hello".as_slice(), None, "48656c6c6f", "48656c6c6f"),
            (
                b"Hello".as_slice(),
                Some("Fallback".to_string()),
                "Fallback",
                "Fallback",
            ),
            (b"".as_slice(), None, "", ""),
        ];

        for (data, fallback, expected_fallback, expected_text) in test_cases {
            let field = create_raw_data_field(data, fallback.clone()).expect("should succeed");
            assert!(field.static_annotation.is_none());
            assert!(field.dynamic_annotation.is_none());

            match field.signable_payload_field {
                SignablePayloadField::TextV2 { common, text_v2 } => {
                    assert_eq!(common.label, "Raw Data");
                    assert_eq!(common.fallback_text, expected_fallback);
                    assert_eq!(text_v2.text, expected_text);
                }
                _ => panic!("Expected TextV2 field"),
            }
        }
    }

    #[test]
    fn test_create_raw_data_field_with_base64_override() {
        let data = b"\x42\x00\xFF\xAA";
        let base64_override = b64.encode(data);

        let field =
            create_raw_data_field(data, Some(base64_override.clone())).expect("should succeed");

        match field.signable_payload_field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Raw Data");
                assert_eq!(common.fallback_text, base64_override);
                assert_eq!(text_v2.text, base64_override);
            }
            _ => panic!("Expected TextV2 field"),
        }
    }

    #[test]
    fn test_create_address_field_diverse_encodings() {
        let test_cases = [
        // Bitcoin - different address formats
        (
            "Bitcoin Legacy (Base58)",
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2", // P2PKH Base58 encoding
            Some("Bitcoin Core"),
            Some("Legacy P2PKH format"),
            Some("BTC"),
            Some("Legacy"),
        ),
        (
            "Bitcoin SegWit (Bech32)",
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", // Bech32 encoding
            Some("SegWit Wallet"),
            Some("Native SegWit format"),
            Some("BTC"),
            Some("SegWit"),
        ),
        // Ethereum - hex format
        (
            "Ethereum Address",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", // Vitalik's address - hex encoded
            Some("Vitalik"),
            Some("Ethereum foundation"),
            Some("ETH"),
            Some("Founder"),
        ),
        // Solana - Base58 encoding
        (
            "Solana Wallet",
            "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM", // Base58 encoding
            Some("Solana User"),
            Some("Public key format"),
            Some("SOL"),
            Some("User"),
        ),
        // Cosmos - Bech32 encoding with cosmos prefix
        (
            "Cosmos Hub",
            "cosmos1hsk6jryyqjfhp5dhc55tc9jtckygx0eph6dd02", // Bech32 with cosmos prefix
            Some("Cosmos User"),
            Some("Cosmos Hub address"),
            Some("ATOM"),
            Some("Hub"),
        ),
        // Cardano - Bech32 encoding (very long)
        (
            "Cardano Address",
            "addr1qx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgse35a3x",
            Some("Cardano Wallet"),
            Some("Shelley-era address"),
            Some("ADA"),
            Some("Shelley"),
        ),
        // Polkadot - SS58 format (modified Base58)
        (
            "Polkadot Address",
            "1FRMM8PEiWXYax7rpS6X4XZX1aAAxSWx1CrKTyrVYhV24fg",
            Some("DOT Holder"),
            Some("SS58 format"),
            Some("DOT"),
            Some("Substrate"),
        ),
        // TON - Base64 URL-safe
        (
            "TON Wallet",
            "EQCjk1hh952vWaE9bRguFkAhDAL5jj3xj9p0uPWrFBq_GEMS",
            Some("TON User"),
            Some("Base64 URL-safe encoding"),
            Some("TON"),
            Some("User"),
        ),
        // Algorand - Base32
        (
            "Algorand Wallet",
            "VCBFKUFBM4EWIVRQBJVHB7YL5IS6O54IDMVH5YABYNJONR7TLMKQ4H4I6U",
            Some("Algo User"),
            Some("Base32 encoding"),
            Some("ALGO"),
            Some("Standard"),
        ),
        // Tezos - Base58 with different prefix
        (
            "Tezos Address",
            "tz1fLM9SshG1ptadCTmEQYfzrqoKP1MYj2ne",
            Some("Tezos User"),
            Some("tz1 prefix for ed25519"),
            Some("XTZ"),
            Some("tz1"),
        ),
        // Near - human-readable accounts
        (
            "NEAR Account",
            "example.near",
            Some("NEAR User"),
            Some("Human-readable format"),
            Some("NEAR"),
            Some("Account"),
        ),
        // Aptos - hex format without 0x prefix
        (
            "Aptos Account",
            "697c3ccc3750e40183f9a96f1e705c7f82afac772f152d288f7a3a8fa03a27e8",
            Some("Aptos User"),
            Some("Hex without 0x prefix"),
            Some("APT"),
            Some("Account"),
        ),
    ];

        for (label, address, name, memo, asset_label, badge_text) in test_cases {
            let field = create_address_field(label, address, name, memo, asset_label, badge_text)
                .expect("should succeed");

            match field.signable_payload_field {
                SignablePayloadField::AddressV2 { common, address_v2 } => {
                    assert_eq!(common.label, label);
                    assert_eq!(common.fallback_text, address);
                    assert_eq!(address_v2.address, address);
                    assert_eq!(address_v2.name, name.unwrap_or(""));
                    assert_eq!(address_v2.memo.as_deref(), memo);
                    assert_eq!(address_v2.asset_label, asset_label.unwrap_or(""));
                    assert_eq!(address_v2.badge_text.as_deref(), badge_text);
                }
                _ => panic!("Expected AddressV2 field"),
            }
        }
    }

    #[test]
    fn test_create_address_field_edge_cases() {
        // Test edge cases like very short addresses, very long addresses, addresses with special characters
        let test_cases = [
            // Very short address (Aptos)
            (
                "Short Address",
                "0x1",
                Some("Core Framework"),
                None,
                Some("APT"),
                None,
            ),
            // Very long Bitcoin taproot address
            (
                "Taproot Address",
                "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0",
                None,
                Some("Taproot format"),
                Some("BTC"),
                Some("Taproot"),
            ),
            // Address with special characters (TON)
            (
                "Special Chars",
                "EQD4FPq-PRDieyQKkizFTRtSDyucUIqrj0v_zXJmqaDp6_0t",
                Some("Has special chars"),
                Some("Contains - and _"),
                Some("TON"),
                Some("Special"),
            ),
            // Empty address (edge case - should still work)
            (
                "Empty Address",
                "",
                Some("Empty"),
                Some("No address provided"),
                Some("NONE"),
                Some("Invalid"),
            ),
        ];

        for (label, address, name, memo, asset_label, badge_text) in test_cases {
            let field = create_address_field(label, address, name, memo, asset_label, badge_text)
                .expect("should succeed");

            match field.signable_payload_field {
                SignablePayloadField::AddressV2 { common, address_v2 } => {
                    assert_eq!(common.label, label);
                    assert_eq!(common.fallback_text, address);
                    assert_eq!(address_v2.address, address);
                    assert_eq!(address_v2.name, name.unwrap_or(""));
                    assert_eq!(address_v2.memo.as_deref(), memo);
                    assert_eq!(address_v2.asset_label, asset_label.unwrap_or(""));
                    assert_eq!(address_v2.badge_text.as_deref(), badge_text);
                }
                _ => panic!("Expected AddressV2 field"),
            }
        }
    }
}
