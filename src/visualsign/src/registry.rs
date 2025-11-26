use std::collections::HashMap;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use crate::{
    vsptrait::{
        Transaction, VisualSignConverter, VisualSignConverterFromString, VisualSignError,
        VisualSignOptions,
    },
    SignablePayload,
};

/// Supported blockchain types
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Chain {
    Unspecified,
    Solana,
    Ethereum,
    Bitcoin,
    Sui,
    Aptos,
    Polkadot,
    Tron,
    // Add other chains as needed
    Custom(String), // For extensibility without modifying the enum
}

impl Chain {
    pub fn as_str(&self) -> &str {
        match self {
            Chain::Unspecified => "Unspecified",
            Chain::Solana => "Solana",
            Chain::Ethereum => "Ethereum",
            Chain::Bitcoin => "Bitcoin",
            Chain::Sui => "Sui",
            Chain::Aptos => "Aptos",
            Chain::Polkadot => "Polkadot",
            Chain::Tron => "Tron",
            Chain::Custom(name) => name.as_str(),
        }
    }
}

impl FromStr for Chain {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "unspecified" => Chain::Unspecified,
            "solana" => Chain::Solana,
            "ethereum" => Chain::Ethereum,
            "bitcoin" => Chain::Bitcoin,
            "sui" => Chain::Sui,
            "aptos" => Chain::Aptos,
            "polkadot" => Chain::Polkadot,
            "tron" => Chain::Tron,
            _ => Chain::Custom(s.to_string()),
        })
    }
}

/// Type-erased trait for converters that can be stored in the registry
pub trait VisualSignConverterAny: Send + Sync {
    fn to_visual_sign_payload_from_string_any(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError>;

    fn supports_format(&self, transaction_data: &str) -> bool;
}

// Create a wrapper type to hold both the converter and a marker for the transaction type
struct ConverterWrapper<T, C>
where
    T: Transaction + Send + Sync,
    C: VisualSignConverter<T> + VisualSignConverterFromString<T>,
{
    converter: C,
    _phantom: PhantomData<T>,
}

impl<T, C> ConverterWrapper<T, C>
where
    T: Transaction + Send + Sync,
    C: VisualSignConverter<T> + VisualSignConverterFromString<T>,
{
    fn new(converter: C) -> Self {
        Self {
            converter,
            _phantom: PhantomData,
        }
    }
}

// Implement VisualSignConverterAny for the wrapper
impl<T, C> VisualSignConverterAny for ConverterWrapper<T, C>
where
    T: Transaction + Send + Sync, // Add Send + Sync bounds to T for thread safety
    C: VisualSignConverter<T> + VisualSignConverterFromString<T> + Send + Sync,
{
    fn to_visual_sign_payload_from_string_any(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        self.converter
            .to_visual_sign_payload_from_string(transaction_data, options)
    }

    fn supports_format(&self, transaction_data: &str) -> bool {
        // Try to parse and see if it succeeds
        T::from_string(transaction_data).is_ok()
    }
}

/// Registry for transaction converters
pub struct TransactionConverterRegistry {
    converters: HashMap<Chain, Box<dyn VisualSignConverterAny>>,
}

impl Default for TransactionConverterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionConverterRegistry {
    pub fn new() -> Self {
        Self {
            converters: HashMap::new(),
        }
    }

    pub fn register<T, C>(&mut self, chain: Chain, converter: C)
    where
        T: Transaction + Send + Sync + 'static,
        C: VisualSignConverter<T> + VisualSignConverterFromString<T> + Send + Sync + 'static,
    {
        self.converters
            .insert(chain, Box::new(ConverterWrapper::<T, C>::new(converter)));
    }

    pub fn get_converter(&self, chain: &Chain) -> Option<&dyn VisualSignConverterAny> {
        self.converters.get(chain).map(|c| c.as_ref())
    }

    pub fn convert_transaction(
        &self,
        chain: &Chain,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        match self.get_converter(chain) {
            Some(converter) => {
                converter.to_visual_sign_payload_from_string_any(transaction_data, options)
            }
            None => Err(VisualSignError::ConversionError(format!(
                "No converter registered for chain: {}",
                chain.as_str()
            ))),
        }
    }

    pub fn auto_detect_and_convert(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<(Chain, SignablePayload), VisualSignError> {
        // Try each converter to see if it can parse the transaction
        for (chain, converter) in &self.converters {
            if converter.supports_format(transaction_data) {
                match converter
                    .to_visual_sign_payload_from_string_any(transaction_data, options.clone())
                {
                    Ok(payload) => return Ok((chain.clone(), payload)),
                    Err(_) => continue, // Try next converter
                }
            }
        }

        Err(VisualSignError::ConversionError(
            "Could not detect transaction type or no compatible converter found".to_string(),
        ))
    }

    pub fn supported_chains(&self) -> Vec<Chain> {
        self.converters.keys().cloned().collect()
    }
}

/// Generic layered registry for combining global and request-scoped data.
///
/// This struct enables efficient per-request registry overlays without cloning.
/// The global registry is shared via `Arc` (O(1) clone), while wallet-provided
/// data is owned and dropped after the request completes.
///
/// # Usage Pattern
///
/// ```ignore
/// // Global registry created once at startup
/// let global = Arc::new(MyRegistry::with_defaults());
///
/// // Per-request: create layered registry with optional wallet data
/// let layered = if let Some(wallet_data) = request.wallet_metadata {
///     LayeredRegistry::with_request(Arc::clone(&global), wallet_data)
/// } else {
///     LayeredRegistry::new(Arc::clone(&global))
/// };
///
/// // Use layered registry for lookups (checks request first, then global)
/// let symbol = layered.lookup(|r| r.get_token_symbol(chain_id, address));
/// ```
///
/// # Type Parameter
///
/// `R` - The registry type (e.g., `ContractRegistry` for Ethereum)
pub struct LayeredRegistry<R> {
    /// Request-scoped data (checked first during lookups)
    request: Option<R>,
    /// Global registry shared across requests via Arc
    global: Arc<R>,
}

impl<R> LayeredRegistry<R> {
    /// Creates a layered registry with only the global layer.
    ///
    /// Use this when no request-specific data is available.
    pub fn new(global: Arc<R>) -> Self {
        Self {
            request: None,
            global,
        }
    }

    /// Creates a layered registry with both global and request-scoped layers.
    ///
    /// Lookups will check the request layer first, then fall back to global.
    pub fn with_request(global: Arc<R>, request: R) -> Self {
        Self {
            request: Some(request),
            global,
        }
    }

    /// Returns a reference to the global registry.
    pub fn global(&self) -> &R {
        &self.global
    }

    /// Returns a reference to the request-scoped registry, if present.
    pub fn request(&self) -> Option<&R> {
        self.request.as_ref()
    }

    /// Performs a layered lookup: checks request first, then global.
    ///
    /// The closure `f` is called on the request registry first (if present).
    /// If it returns `None`, the closure is called on the global registry.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let symbol = layered.lookup(|r| r.get_token_symbol(chain_id, address));
    /// ```
    pub fn lookup<T, F>(&self, f: F) -> Option<T>
    where
        F: Fn(&R) -> Option<T>,
    {
        // Check request layer first
        if let Some(ref request) = self.request {
            if let Some(result) = f(request) {
                return Some(result);
            }
        }
        // Fall back to global
        f(&self.global)
    }

    /// Performs a layered lookup that returns a Result.
    ///
    /// Similar to `lookup`, but for fallible operations. Checks request first,
    /// then global. Returns the first `Ok` result, or the last `Err` if both fail.
    pub fn lookup_result<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: Fn(&R) -> Result<T, E>,
    {
        // Check request layer first
        if let Some(ref request) = self.request {
            if let ok @ Ok(_) = f(request) {
                return ok;
            }
        }
        // Fall back to global
        f(&self.global)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};
    // Import TransactionParseError only in tests where it's actually used
    use crate::vsptrait::TransactionParseError;

    // Mock transactions for different chains with realistic format detection
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct MockSolanaTransaction {
        data: Vec<u8>,
    }

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct MockEthereumTransaction {
        data: Vec<u8>,
    }

    // Implement Transaction trait with format detection without external crates
    impl Transaction for MockSolanaTransaction {
        fn from_string(data: &str) -> Result<Self, TransactionParseError> {
            // Simplified detection logic - Solana transactions start with byte 0x01
            // For testing purposes, we'll use a pattern where "01" at the start indicates Solana
            if data.starts_with("01") {
                // Simple hex decode without using hex crate
                let bytes = match decode_hex(data) {
                    Ok(b) => b,
                    Err(_) => {
                        return Err(TransactionParseError::DecodeError(
                            "Invalid hex".to_string(),
                        ))
                    }
                };

                if !bytes.is_empty() && bytes[0] == 0x01 {
                    return Ok(Self { data: bytes });
                }
            }

            Err(TransactionParseError::InvalidFormat(
                "Not a Solana transaction".to_string(),
            ))
        }

        fn transaction_type(&self) -> String {
            "Solana".to_string()
        }
    }

    impl Transaction for MockEthereumTransaction {
        fn from_string(data: &str) -> Result<Self, TransactionParseError> {
            // Simplified detection logic - Ethereum transactions start with byte 0x02
            // For testing purposes, we'll use a pattern where "02" at the start indicates Ethereum
            if data.starts_with("02") {
                // Simple hex decode without using hex crate
                let bytes = match decode_hex(data) {
                    Ok(b) => b,
                    Err(_) => {
                        return Err(TransactionParseError::DecodeError(
                            "Invalid hex".to_string(),
                        ))
                    }
                };

                if !bytes.is_empty() && bytes[0] == 0x02 {
                    return Ok(Self { data: bytes });
                }
            }

            Err(TransactionParseError::InvalidFormat(
                "Not an Ethereum transaction".to_string(),
            ))
        }

        fn transaction_type(&self) -> String {
            "Ethereum".to_string()
        }
    }

    // Simple hex decoder function to avoid dependency on hex crate
    fn decode_hex(s: &str) -> Result<Vec<u8>, &'static str> {
        if s.len() % 2 != 0 {
            return Err("Hex string must have even length");
        }

        let mut result = Vec::with_capacity(s.len() / 2);
        let mut chars = s.chars();

        while let (Some(a), Some(b)) = (chars.next(), chars.next()) {
            let byte = match (a.to_digit(16), b.to_digit(16)) {
                (Some(high), Some(low)) => ((high as u8) << 4) | (low as u8),
                _ => return Err("Invalid hex character"),
            };
            result.push(byte);
        }

        Ok(result)
    }

    // Mock converter implementations
    struct MockSuccessConverter<T> {
        phantom: PhantomData<T>,
    }

    impl<T> MockSuccessConverter<T> {
        fn new() -> Self {
            Self {
                phantom: PhantomData,
            }
        }
    }

    impl<T: Transaction> VisualSignConverter<T> for MockSuccessConverter<T> {
        fn to_visual_sign_payload(
            &self,
            _transaction: T,
            _options: VisualSignOptions,
        ) -> Result<SignablePayload, VisualSignError> {
            // Create a simple payload using SignablePayload::new
            Ok(SignablePayload::new(
                0,
                "Test Transaction".to_string(),
                None,
                vec![SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Test".to_string(),
                        label: "Test Label".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Test Value".to_string(),
                    },
                }],
                "Test Source".to_string(),
            ))
        }
    }

    impl<T: Transaction> VisualSignConverterFromString<T> for MockSuccessConverter<T> {}

    struct MockFailingConverter<T> {
        phantom: PhantomData<T>,
    }

    impl<T> MockFailingConverter<T> {
        fn new() -> Self {
            Self {
                phantom: PhantomData,
            }
        }
    }

    impl<T: Transaction> VisualSignConverter<T> for MockFailingConverter<T> {
        fn to_visual_sign_payload(
            &self,
            _transaction: T,
            _options: VisualSignOptions,
        ) -> Result<SignablePayload, VisualSignError> {
            Err(VisualSignError::ConversionError(
                "Mock conversion failed".to_string(),
            ))
        }
    }

    impl<T: Transaction> VisualSignConverterFromString<T> for MockFailingConverter<T> {}

    #[test]
    fn test_auto_detect_solana_success() {
        let mut registry = TransactionConverterRegistry::new();

        registry.register::<MockSolanaTransaction, _>(Chain::Solana, MockSuccessConverter::new());

        registry
            .register::<MockEthereumTransaction, _>(Chain::Ethereum, MockSuccessConverter::new());

        let result =
            registry.auto_detect_and_convert("01abcdef1234567890", VisualSignOptions::default());

        assert!(result.is_ok());
        let (chain, _) = result.unwrap();
        assert_eq!(chain, Chain::Solana);
    }

    #[test]
    fn test_auto_detect_ethereum_success() {
        let mut registry = TransactionConverterRegistry::new();

        registry.register::<MockSolanaTransaction, _>(Chain::Solana, MockSuccessConverter::new());
        registry
            .register::<MockEthereumTransaction, _>(Chain::Ethereum, MockSuccessConverter::new());

        let result =
            registry.auto_detect_and_convert("02abcdef1234567890", VisualSignOptions::default());

        assert!(result.is_ok());
        let (chain, _) = result.unwrap();
        assert_eq!(chain, Chain::Ethereum);
    }

    #[test]
    fn test_auto_detect_no_matching_converter() {
        let mut registry = TransactionConverterRegistry::new();

        registry.register::<MockSolanaTransaction, _>(Chain::Solana, MockSuccessConverter::new());
        registry
            .register::<MockEthereumTransaction, _>(Chain::Ethereum, MockSuccessConverter::new());

        let result = registry.auto_detect_and_convert(
            "03abcdef1234567890", // Starts with 03, not supported
            VisualSignOptions::default(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_auto_detect_format_supported_but_conversion_fails() {
        let mut registry = TransactionConverterRegistry::new();

        registry.register::<MockSolanaTransaction, _>(Chain::Solana, MockFailingConverter::new());

        registry
            .register::<MockEthereumTransaction, _>(Chain::Ethereum, MockSuccessConverter::new());

        let result = registry.auto_detect_and_convert(
            "01abcdef1234567890", // Solana format but conversion will fail
            VisualSignOptions::default(),
        );

        // Should try Ethereum after Solana fails, but Ethereum won't match the format
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_registry() {
        let registry = TransactionConverterRegistry::new();

        let result =
            registry.auto_detect_and_convert("01abcdef1234567890", VisualSignOptions::default());

        assert!(result.is_err());
    }

    #[test]
    fn test_chain_from_str() {
        assert_eq!(Chain::from_str("solana"), Ok(Chain::Solana));
        assert_eq!(Chain::from_str("SOLANA"), Ok(Chain::Solana)); // Case insensitive
        assert_eq!(Chain::from_str("ethereum"), Ok(Chain::Ethereum));
        assert_eq!(Chain::from_str("bitcoin"), Ok(Chain::Bitcoin));
        assert_eq!(Chain::from_str("sui"), Ok(Chain::Sui));
        assert_eq!(Chain::from_str("aptos"), Ok(Chain::Aptos));
        assert_eq!(Chain::from_str("polkadot"), Ok(Chain::Polkadot));
        assert_eq!(Chain::from_str("tron"), Ok(Chain::Tron));
        assert_eq!(
            Chain::from_str("unknown"),
            Ok(Chain::Custom("unknown".to_string()))
        );
    }

    #[test]
    fn test_chain_as_str() {
        assert_eq!(Chain::Solana.as_str(), "Solana");
        assert_eq!(Chain::Ethereum.as_str(), "Ethereum");
        assert_eq!(Chain::Bitcoin.as_str(), "Bitcoin");
        assert_eq!(Chain::Sui.as_str(), "Sui");
        assert_eq!(Chain::Aptos.as_str(), "Aptos");
        assert_eq!(Chain::Polkadot.as_str(), "Polkadot");
        assert_eq!(Chain::Tron.as_str(), "Tron");
        assert_eq!(Chain::Custom("MyChain".to_string()).as_str(), "MyChain");
    }
}
