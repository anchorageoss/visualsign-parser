use crate::errors;
use crate::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAmountV2,
    SignablePayloadFieldCommon, SignablePayloadFieldNumber, SignablePayloadFieldTextV2,
};

use regex::Regex;
// thread-safe static initialization for regex
use std::sync::LazyLock;

// Regex to validate signed proper numbers (e.g., -123.45, +678.90)
// A signed proper number is defined as a string that starts with an optional sign (+ or - or -),
// followed by one or more digits, a decimal point, and one or more digits.
// Examples of valid signed proper numbers: "123.45", "-123.45", "+678.90"
// Examples of invalid signed proper numbers: "123", "-.45", "123.", "abc", "12.3.4"
// Note: This regex does not allow leading zeros unless the number is exactly "0" or "0.0".
// It also does not allow numbers with multiple decimal points or non-numeric characters.
// It allows numbers like "0.0", "-0.0", "+0.0"
// which are valid representations of zero.
// The reason it's implemented this way is to avoid adding a large dependency like bignum on this library which could be used in a wide range of applications.
// The regex is designed to be simple and efficient for the common use case of validating signed decimal. We don't yet use it as a numeric type yet, if that ever changes, this will be refactored.
static SIGNED_PROPER_NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([-+]?[0-9]+\.[0-9]+|[-+]?0)$")
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
        format!("{} {}", number, unit)
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
    let fallback_text = format!("{} {}", amount, abbreviation);
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
        .map(|byte| format!("{:02x}", byte))
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
                _ => panic!("Expected InvalidNumberField error for {}", num),
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
        match err {
            VisualSignError::EmptyField(ref s) if s.is_empty() => {}
            _ => panic!("Expected MissingField error"),
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
}
