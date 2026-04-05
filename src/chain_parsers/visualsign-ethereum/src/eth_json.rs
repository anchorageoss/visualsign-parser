use alloy_consensus::{TxEip1559, TxLegacy, TypedTransaction};
use alloy_primitives::{Address, Bytes, TxKind, U256};
use serde::Deserialize;

use crate::EthereumParserError;

const DEFAULT_GAS_LIMIT: u64 = 21_000;
// chainId is always required — no default. Silently defaulting to mainnet
// is too dangerous for a signing service.

/// Maximum hex-encoded data length accepted (512 KB of hex = 256 KB decoded).
/// Real Ethereum calldata is bounded by block gas limits to much less than this.
const MAX_HEX_DATA_LEN: usize = 512 * 1024;

/// Maximum raw JSON input size before deserialization (1 MB).
/// A valid Ethereum transaction JSON is at most a few KB; this is generous.
const MAX_JSON_INPUT_LEN: usize = 1024 * 1024;

/// Maximum length of a raw input value to include in error messages.
/// Prevents leaking large calldata payloads into logs or error responses.
const ERROR_PREVIEW_LEN: usize = 64;

/// Tagged envelope for JSON transaction input.
/// Extensible for future JSON entry types (EIP-712 typed data, ERC-7730 clear signing).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum EthJsonInput {
    #[serde(rename = "transaction")]
    Transaction(EthJsonTransaction),
}

/// Ethereum transaction fields matching JSON-RPC `eth_sendTransaction` format.
/// All fields except `chainId` are optional with sensible defaults. `chainId`
/// must always be provided and has no default; defaulting to mainnet is too
/// dangerous for a signing service. Numeric values are hex strings with
/// optional `0x` prefix.
///
/// Uses `deny_unknown_fields` to reject typos (e.g., `"chainID"` vs `"chainId"`)
/// that would silently fall back to defaults -- dangerous for a signing service.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EthJsonTransaction {
    from: Option<String>,
    to: Option<String>,
    data: Option<String>,
    value: Option<String>,
    nonce: Option<String>,
    gas: Option<String>,
    gas_price: Option<String>,
    max_fee_per_gas: Option<String>,
    max_priority_fee_per_gas: Option<String>,
    chain_id: Option<String>,
}

/// Returns true if the input looks like JSON (starts with '{' after trimming whitespace).
///
/// This is a safe heuristic: '{' is not a valid character in hex strings or standard
/// base64 encoding, so no legitimate RLP-encoded transaction can be misrouted.
pub(crate) fn is_json_input(data: &str) -> bool {
    data.trim_start().starts_with('{')
}

/// Parse a JSON string into a TypedTransaction.
pub(crate) fn decode_json_transaction(data: &str) -> Result<TypedTransaction, EthereumParserError> {
    if data.len() > MAX_JSON_INPUT_LEN {
        return Err(EthereumParserError::FailedToParseJsonTransaction(format!(
            "JSON input too large: {} bytes (max {})",
            data.len(),
            MAX_JSON_INPUT_LEN,
        )));
    }
    let input: EthJsonInput = serde_json::from_str(data)
        .map_err(|e| EthereumParserError::FailedToParseJsonTransaction(e.to_string()))?;
    match input {
        EthJsonInput::Transaction(tx) => build_transaction(tx),
    }
}

fn build_transaction(tx: EthJsonTransaction) -> Result<TypedTransaction, EthereumParserError> {
    if let Some(ref from_str) = tx.from {
        let _ = parse_address(from_str)?;
        log::debug!("JSON transaction contains 'from' field which is accepted but ignored");
    }

    // Reject contradictory gas pricing fields
    if tx.gas_price.is_some() && tx.max_fee_per_gas.is_some() {
        return Err(EthereumParserError::FailedToParseJsonTransaction(
            "Cannot specify both 'gasPrice' (legacy) and 'maxFeePerGas' (EIP-1559)".to_string(),
        ));
    }

    // Reject maxPriorityFeePerGas without maxFeePerGas -- almost certainly a user mistake
    if tx.max_priority_fee_per_gas.is_some() && tx.max_fee_per_gas.is_none() {
        return Err(EthereumParserError::FailedToParseJsonTransaction(
            "'maxPriorityFeePerGas' requires 'maxFeePerGas' to be set".to_string(),
        ));
    }

    let to = match &tx.to {
        Some(addr) => TxKind::Call(parse_address(addr)?),
        None => TxKind::Create,
    };
    let value = tx
        .value
        .as_deref()
        .map(parse_hex_u256)
        .transpose()?
        .unwrap_or(U256::ZERO);
    let nonce = tx
        .nonce
        .as_deref()
        .map(parse_hex_u64)
        .transpose()?
        .unwrap_or(0);
    let gas_limit = match tx.gas.as_deref() {
        Some(raw) => {
            let parsed = parse_hex_u64(raw)?;
            if parsed == 0 {
                return Err(EthereumParserError::FailedToParseJsonTransaction(
                    "'gas' must be greater than 0; omit the field to use the default (21000)"
                        .to_string(),
                ));
            }
            parsed
        }
        None => DEFAULT_GAS_LIMIT,
    };
    let chain_id = match tx.chain_id.as_deref() {
        Some(raw) => {
            let parsed = parse_hex_u64(raw)?;
            if parsed == 0 {
                return Err(EthereumParserError::FailedToParseJsonTransaction(
                    "'chainId' must be greater than 0".to_string(),
                ));
            }
            parsed
        }
        None => {
            return Err(EthereumParserError::FailedToParseJsonTransaction(
                "'chainId' is required".to_string(),
            ));
        }
    };
    let input_data = tx
        .data
        .as_deref()
        .map(parse_hex_bytes)
        .transpose()?
        .unwrap_or_default();

    if let Some(ref fee) = tx.max_fee_per_gas {
        let max_fee_per_gas = parse_hex_u128(fee)?;
        let max_priority_fee_per_gas = tx
            .max_priority_fee_per_gas
            .as_deref()
            .map(parse_hex_u128)
            .transpose()?
            .unwrap_or(0);

        if max_priority_fee_per_gas > max_fee_per_gas {
            return Err(EthereumParserError::FailedToParseJsonTransaction(
                "'maxPriorityFeePerGas' cannot exceed 'maxFeePerGas'".to_string(),
            ));
        }

        Ok(TypedTransaction::Eip1559(TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            input: input_data,
            // accessList is not supported via JSON input in v1; defaults to empty.
            access_list: Default::default(),
        }))
    } else {
        let gas_price = tx
            .gas_price
            .as_deref()
            .map(parse_hex_u128)
            .transpose()?
            .unwrap_or(0);

        if gas_price == 0 {
            log::warn!("Legacy transaction has gas_price=0; will not be mined on most networks");
        }

        Ok(TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(chain_id),
            nonce,
            gas_limit,
            gas_price,
            to,
            value,
            input: input_data,
        }))
    }
}

/// Truncate a string for safe inclusion in error messages.
/// Uses char boundaries to avoid panicking on multi-byte UTF-8 input.
fn error_preview(s: &str) -> &str {
    truncate_at_char_boundary(s, ERROR_PREVIEW_LEN)
}

/// Like `error_preview` but reserves 3 chars for a trailing "..." ellipsis,
/// so the total output (preview + "...") stays within `ERROR_PREVIEW_LEN`.
fn error_preview_with_ellipsis(s: &str) -> &str {
    truncate_at_char_boundary(s, ERROR_PREVIEW_LEN.saturating_sub(3))
}

fn truncate_at_char_boundary(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn strip_hex_prefix(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

macro_rules! parse_hex_int {
    ($name:ident, $ty:ty, $zero:expr, $label:literal) => {
        fn $name(s: &str) -> Result<$ty, EthereumParserError> {
            let hex = strip_hex_prefix(s);
            if hex.is_empty() {
                return Ok($zero);
            }
            <$ty>::from_str_radix(hex, 16).map_err(|e| {
                EthereumParserError::FailedToParseJsonTransaction(format!(
                    "Invalid hex {} '{}': {}",
                    $label,
                    error_preview(s),
                    e,
                ))
            })
        }
    };
}

parse_hex_int!(parse_hex_u64, u64, 0, "u64");
parse_hex_int!(parse_hex_u128, u128, 0, "u128");
parse_hex_int!(parse_hex_u256, U256, U256::ZERO, "U256");

fn parse_address(s: &str) -> Result<Address, EthereumParserError> {
    s.parse::<Address>().map_err(|e| {
        EthereumParserError::FailedToParseJsonTransaction(format!(
            "Invalid address '{}': {}",
            error_preview(s),
            e,
        ))
    })
}

fn parse_hex_bytes(s: &str) -> Result<Bytes, EthereumParserError> {
    let hex = strip_hex_prefix(s);
    if hex.is_empty() {
        return Ok(Bytes::new());
    }
    if hex.len() > MAX_HEX_DATA_LEN {
        return Err(EthereumParserError::FailedToParseJsonTransaction(format!(
            "Hex data too large: {} chars (max {})",
            hex.len(),
            MAX_HEX_DATA_LEN,
        )));
    }
    let decoded = hex::decode(hex).map_err(|e| {
        let preview = if s.len() > ERROR_PREVIEW_LEN {
            format!("{}...", error_preview_with_ellipsis(s))
        } else {
            s.to_string()
        };
        EthereumParserError::FailedToParseJsonTransaction(format!(
            "Invalid hex data '{preview}': {e}",
        ))
    })?;
    Ok(Bytes::from(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Transaction as _;

    #[test]
    fn test_eip1559_json() {
        let json = r#"{
            "type": "transaction",
            "to": "0x000000000000000000000000000000000000dEaD",
            "value": "0xde0b6b3a7640000",
            "nonce": "0x2a",
            "gas": "0x5208",
            "maxFeePerGas": "0x4a817c800",
            "maxPriorityFeePerGas": "0x3b9aca00",
            "chainId": "0x1",
            "data": "0x"
        }"#;

        let tx = decode_json_transaction(json).unwrap();
        match &tx {
            TypedTransaction::Eip1559(inner) => {
                assert_eq!(inner.chain_id, 1);
                assert_eq!(inner.nonce, 42);
                assert_eq!(inner.gas_limit, 21000);
                assert_eq!(inner.max_fee_per_gas, 20_000_000_000);
                assert_eq!(inner.max_priority_fee_per_gas, 1_000_000_000);
                assert_eq!(inner.value, U256::from(0xde0b6b3a7640000u64));
                assert_eq!(
                    inner.to,
                    TxKind::Call(
                        "0x000000000000000000000000000000000000dEaD"
                            .parse()
                            .unwrap()
                    )
                );
                assert!(inner.input.is_empty());
            }
            _ => panic!("Expected EIP-1559 transaction"),
        }
    }

    #[test]
    fn test_legacy_json() {
        let json = r#"{
            "type": "transaction",
            "to": "0x000000000000000000000000000000000000dEaD",
            "value": "0xde0b6b3a7640000",
            "nonce": "0x0",
            "gas": "0x5208",
            "gasPrice": "0x4a817c800",
            "chainId": "0x1"
        }"#;

        let tx = decode_json_transaction(json).unwrap();
        match &tx {
            TypedTransaction::Legacy(inner) => {
                assert_eq!(inner.chain_id, Some(1));
                assert_eq!(inner.nonce, 0);
                assert_eq!(inner.gas_limit, 21000);
                assert_eq!(inner.gas_price, 20_000_000_000);
                assert_eq!(inner.value, U256::from(0xde0b6b3a7640000u64));
            }
            _ => panic!("Expected Legacy transaction"),
        }
    }

    #[test]
    fn test_requires_chain_id() {
        let json = r#"{"type": "transaction"}"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("chainId"));
                assert!(msg.contains("required"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_minimal_json_with_chain_id() {
        let json = r#"{"type": "transaction", "chainId": "0x1"}"#;
        let tx = decode_json_transaction(json).unwrap();
        match &tx {
            TypedTransaction::Legacy(inner) => {
                assert_eq!(inner.chain_id, Some(1));
                assert_eq!(inner.nonce, 0);
                assert_eq!(inner.gas_limit, DEFAULT_GAS_LIMIT);
                assert_eq!(inner.gas_price, 0);
                assert_eq!(inner.value, U256::ZERO);
                assert_eq!(inner.to, TxKind::Create);
                assert!(inner.input.is_empty());
            }
            _ => panic!("Expected Legacy transaction with defaults"),
        }
    }

    #[test]
    fn test_hex_without_prefix() {
        let json = r#"{
            "type": "transaction",
            "nonce": "2a",
            "gas": "5208",
            "chainId": "1"
        }"#;
        let tx = decode_json_transaction(json).unwrap();
        assert_eq!(tx.nonce(), 42);
        assert_eq!(tx.gas_limit(), 21000);
    }

    #[test]
    fn test_invalid_hex() {
        let json = r#"{
            "type": "transaction",
            "value": "0xGHIJ"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        assert!(matches!(
            err,
            EthereumParserError::FailedToParseJsonTransaction(_)
        ));
    }

    #[test]
    fn test_unknown_type() {
        let json = r#"{"type": "unknown"}"#;
        let err = decode_json_transaction(json).unwrap_err();
        assert!(matches!(
            err,
            EthereumParserError::FailedToParseJsonTransaction(_)
        ));
    }

    #[test]
    fn test_from_field_accepted() {
        let json = r#"{
            "type": "transaction",
            "from": "0x742d35Cc6634C0532925a3b844Bc9e7595f2bD28",
            "to": "0x000000000000000000000000000000000000dEaD",
            "value": "0x0",
            "chainId": "0x1"
        }"#;
        let tx = decode_json_transaction(json).unwrap();
        assert_eq!(tx.value(), U256::ZERO);
    }

    #[test]
    fn test_to_absent_is_create() {
        let json = r#"{
            "type": "transaction",
            "data": "0x6060604052",
            "chainId": "0x1"
        }"#;
        let tx = decode_json_transaction(json).unwrap();
        match &tx {
            TypedTransaction::Legacy(inner) => {
                assert_eq!(inner.to, TxKind::Create);
                assert_eq!(
                    inner.input.as_ref(),
                    &hex::decode("6060604052").unwrap()[..]
                );
            }
            _ => panic!("Expected Legacy"),
        }
    }

    #[test]
    fn test_large_value_u256() {
        let json = r#"{
            "type": "transaction",
            "value": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "chainId": "0x1"
        }"#;
        let tx = decode_json_transaction(json).unwrap();
        assert_eq!(tx.value(), U256::MAX);
    }

    #[test]
    fn test_is_json_input() {
        assert!(is_json_input(r#"{"type": "transaction"}"#));
        assert!(is_json_input(r#"  {"type": "transaction"}"#));
        assert!(!is_json_input("0x02f903f8"));
        assert!(!is_json_input("SGVsbG8="));
        assert!(!is_json_input(""));
    }

    #[test]
    fn test_empty_hex_prefix_value_and_nonce_are_zero() {
        let json = r#"{
            "type": "transaction",
            "value": "0x",
            "nonce": "0x",
            "chainId": "0x1"
        }"#;
        let tx = decode_json_transaction(json).unwrap();
        assert_eq!(tx.value(), U256::ZERO);
        assert_eq!(tx.nonce(), 0);
        assert_eq!(tx.gas_limit(), DEFAULT_GAS_LIMIT);
    }

    #[test]
    fn test_gas_zero_rejected() {
        let json = r#"{
            "type": "transaction",
            "gas": "0x0",
            "chainId": "0x1"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("gas"));
                assert!(msg.contains("greater than 0"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }

        // "0x" (empty hex) also parses to 0 and should be rejected
        let json2 = r#"{
            "type": "transaction",
            "gas": "0x",
            "chainId": "0x1"
        }"#;
        let err2 = decode_json_transaction(json2).unwrap_err();
        assert!(matches!(
            err2,
            EthereumParserError::FailedToParseJsonTransaction(_)
        ));
    }

    #[test]
    fn test_chain_id_zero_rejected() {
        let json = r#"{
            "type": "transaction",
            "chainId": "0x0"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("chainId"));
                assert!(msg.contains("greater than 0"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }

        // "0x" (empty hex) also parses to 0 and should be rejected
        let json2 = r#"{
            "type": "transaction",
            "chainId": "0x"
        }"#;
        let err2 = decode_json_transaction(json2).unwrap_err();
        assert!(matches!(
            err2,
            EthereumParserError::FailedToParseJsonTransaction(_)
        ));
    }

    #[test]
    fn test_conflicting_gas_fields_rejected() {
        let json = r#"{
            "type": "transaction",
            "gasPrice": "0x4a817c800",
            "maxFeePerGas": "0x4a817c800"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("gasPrice"));
                assert!(msg.contains("maxFeePerGas"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_priority_fee_without_max_fee_rejected() {
        let json = r#"{
            "type": "transaction",
            "maxPriorityFeePerGas": "0x3b9aca00"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("maxPriorityFeePerGas"));
                assert!(msg.contains("maxFeePerGas"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_priority_fee_exceeds_max_fee_rejected() {
        let json = r#"{
            "type": "transaction",
            "maxFeePerGas": "0x3b9aca00",
            "maxPriorityFeePerGas": "0x4a817c800",
            "chainId": "0x1"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("maxPriorityFeePerGas"));
                assert!(msg.contains("cannot exceed"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_eip1559_also_requires_chain_id() {
        let json = r#"{
            "type": "transaction",
            "maxFeePerGas": "0x4a817c800"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("chainId"));
                assert!(msg.contains("required"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_invalid_address() {
        let json = r#"{
            "type": "transaction",
            "to": "not-an-address"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        assert!(matches!(
            err,
            EthereumParserError::FailedToParseJsonTransaction(_)
        ));
    }

    #[test]
    fn test_unknown_fields_rejected() {
        let json = r#"{
            "type": "transaction",
            "chainID": "0x1"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        assert!(matches!(
            err,
            EthereumParserError::FailedToParseJsonTransaction(_)
        ));
    }

    #[test]
    fn test_data_size_limit() {
        // Construct hex data just over MAX_HEX_DATA_LEN, but keep overall JSON under
        // MAX_JSON_INPUT_LEN so we hit the hex data-length guard, not the JSON size guard.
        let over_limit_hex_len = MAX_HEX_DATA_LEN + 2;
        let repeat_count = over_limit_hex_len / 2;
        let huge_hex = format!("0x{}", "aa".repeat(repeat_count));
        let json = format!(r#"{{"type": "transaction", "chainId": "0x1", "data": "{huge_hex}"}}"#);
        assert!(json.len() < MAX_JSON_INPUT_LEN);
        let err = decode_json_transaction(&json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("Hex data too large"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_from_field_invalid_rejected() {
        let json = r#"{
            "type": "transaction",
            "from": "not-an-address",
            "chainId": "0x1"
        }"#;
        let err = decode_json_transaction(json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("Invalid address"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_json_input_size_limit() {
        let padding = "a".repeat(MAX_JSON_INPUT_LEN);
        let json = format!(r#"{{"type": "transaction", "data": "{padding}"}}"#);
        assert!(json.len() > MAX_JSON_INPUT_LEN);
        let err = decode_json_transaction(&json).unwrap_err();
        match err {
            EthereumParserError::FailedToParseJsonTransaction(msg) => {
                assert!(msg.contains("too large"));
            }
            _ => panic!("Expected FailedToParseJsonTransaction"),
        }
    }

    #[test]
    fn test_error_preview_multibyte_safe() {
        // 4-byte emoji repeated to exceed ERROR_PREVIEW_LEN bytes
        let s = "\u{1F600}".repeat(20); // 80 bytes
        let preview = error_preview(&s);
        assert!(preview.len() <= ERROR_PREVIEW_LEN);
        // Verify the preview is valid UTF-8 (won't panic when used as &str)
        assert!(std::str::from_utf8(preview.as_bytes()).is_ok());
    }
}
