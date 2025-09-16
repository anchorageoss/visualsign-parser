use thiserror::Error;

/// Errors that can occur during transaction parsing
#[derive(Debug, Eq, PartialEq, Error)]
pub enum TransactionParseError {
    #[error("Invalid transaction format: {0}")]
    InvalidFormat(String),
    #[error("Decode error: {0}")]
    DecodeError(String),
    #[error("Unsupported transaction version: {0}")]
    UnsupportedVersion(String),
    #[error("Unsupported encoding format: {0}")]
    UnsupportedEncoding(String),
}

// Our library's custom, top-level error type.
#[derive(Debug, Eq, PartialEq, Error)]
pub enum VisualSignError {
    #[error("Failed to parse transaction")]
    ParseError(#[from] TransactionParseError),
    #[error("Failed to decode instruction: {0}")]
    DecodeError(String),
    #[error("Missing required data: {0}")]
    MissingData(String),
    // Consider adding more specific error types
    #[error("Conversion failed: {0}")]
    ConversionError(String),
    #[error("Missing required field '{0}'")]
    MissingField(String),
    #[error("Invalid number field: '{0}' contains non-numeric characters or is empty.")]
    InvalidNumberField(String),
    #[error("Empty field provided")]
    EmptyField(String),
    #[error("Validation failed: {0}")]
    ValidationError(String),
    #[error("Serialization failed: {0}")]
    SerializationError(String),
}
