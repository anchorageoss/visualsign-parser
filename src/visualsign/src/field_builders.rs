use crate::errors;
use crate::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldAmountV2,
    SignablePayloadFieldCommon, SignablePayloadFieldNumber, SignablePayloadFieldTextV2,
};

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

pub fn create_number_field(
    label: &str,
    number: &str,
    unit: &str,
) -> Result<AnnotatedPayloadField, errors::VisualSignError> {
    // Validate that the number string is a valid f64.
    match number.parse::<f64>() {
        Ok(n) if n.is_nan() => {
            return Err(errors::VisualSignError::InvalidNumberField(
                number.to_string(),
            ));
        }
        Err(_) => {
            return Err(errors::VisualSignError::InvalidNumberField(
                number.to_string(), // for debugging, we don't want to show this to users as-is
            ));
        }
        Ok(_) => {} // valid number, continue
    }

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
    if amount.parse::<f64>().is_err() {
        return Err(errors::VisualSignError::InvalidNumberField(
            amount.to_string(),
        ));
    }
    // unlike number field, we do want amount fields to have a valid symbol
    if abbreviation.is_empty() {
        return Err(errors::VisualSignError::MissingField(
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
            VisualSignError::MissingField(ref s) if s.is_empty() => {}
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
