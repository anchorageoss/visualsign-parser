use thiserror::Error;

// Our library's custom, top-level error type.
#[derive(Error, Debug)]
pub enum VisualSignError {
    #[error("Missing required field '{0}'")]
    MissingField(String),
    #[error("Invalid number field: '{0}' contains non-numeric characters or is empty.")]
    InvalidNumberField(String),
}
