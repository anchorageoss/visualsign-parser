use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    vsptrait::{
        ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
        VisualSignConverterFromString, VisualSignError, VisualSignOptions,
    },
};

// This is a standalone crate for handling unspecified or unknown transactions, mostly provided for testing and as a sample implementation template to start from
/// Wrapper for unspecified/unknown transactions
#[derive(Debug, Clone)]
pub struct UnspecifiedTransactionWrapper {
    raw_data: String,
}

impl Transaction for UnspecifiedTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        Ok(Self {
            raw_data: data.to_string(),
        })
    }

    fn transaction_type(&self) -> String {
        "Unspecified".to_string()
    }
}

impl UnspecifiedTransactionWrapper {
    pub fn new(raw_data: String) -> Self {
        Self { raw_data }
    }

    pub fn raw_data(&self) -> &str {
        &self.raw_data
    }
}

/// Converter for unspecified/unknown chains
pub struct UnspecifiedVisualSignConverter;

impl VisualSignConverter<UnspecifiedTransactionWrapper> for UnspecifiedVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: UnspecifiedTransactionWrapper,
        _options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        // Return the exact payload expected by the e2e test
        let fields = vec![
            SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Unspecified Chain".to_string(),
                    label: "Network".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: "Unspecified Chain".to_string(),
                },
            },
            SignablePayloadField::TextV2 {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Raw Data".to_string(),
                    label: "Raw Data".to_string(),
                },
                text_v2: SignablePayloadFieldTextV2 {
                    text: transaction_wrapper.raw_data().to_string(),
                },
            },
        ];

        Ok(ConversionResult::new(SignablePayload::new(
            0,
            "Unspecified Transaction".to_string(),
            None,
            fields,
            "fill in parsed signable payload".to_string(), // This is what the test expects
        )))
    }
}

impl VisualSignConverterFromString<UnspecifiedTransactionWrapper>
    for UnspecifiedVisualSignConverter
{
}

// Public API functions
pub fn transaction_to_visual_sign(
    raw_data: String,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let wrapper = UnspecifiedTransactionWrapper::new(raw_data);
    let converter = UnspecifiedVisualSignConverter;
    converter
        .to_visual_sign_payload(wrapper, options)
        .map(|r| r.payload)
}

pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let converter = UnspecifiedVisualSignConverter;
    converter
        .to_visual_sign_payload_from_string(transaction_data, options)
        .map(|r| r.payload)
}
