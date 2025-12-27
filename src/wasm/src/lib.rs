use wasm_bindgen::prelude::*;

// Set panic hook for better error messages in console
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Parse an Ethereum transaction to VisualSign JSON
///
/// # Arguments
/// * `tx_data` - Hex-encoded transaction (with or without 0x prefix) or base64-encoded
///
/// # Returns
/// JSON string with human-readable transaction details
#[wasm_bindgen]
pub fn parse_ethereum_transaction(tx_data: &str) -> Result<String, JsValue> {
    use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
    use visualsign_ethereum::{EthereumTransactionWrapper, EthereumVisualSignConverter};

    let tx = EthereumTransactionWrapper::from_string(tx_data)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse transaction: {}", e)))?;

    let converter = EthereumVisualSignConverter::new();
    let payload = converter
        .to_visual_sign_payload(
            tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .map_err(|e| JsValue::from_str(&format!("Failed to convert transaction: {}", e)))?;

    payload
        .to_validated_json()
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize JSON: {}", e)))
}

/// Parse a Solana transaction to VisualSign JSON
///
/// # Arguments
/// * `tx_data` - Base58 or base64-encoded transaction
///
/// # Returns
/// JSON string with human-readable transaction details
#[wasm_bindgen]
pub fn parse_solana_transaction(tx_data: &str) -> Result<String, JsValue> {
    use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
    use visualsign_solana::{SolanaTransactionWrapper, SolanaVisualSignConverter};

    let tx = SolanaTransactionWrapper::from_string(tx_data)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse transaction: {}", e)))?;

    let payload = SolanaVisualSignConverter
        .to_visual_sign_payload(
            tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .map_err(|e| JsValue::from_str(&format!("Failed to convert transaction: {}", e)))?;

    payload
        .to_validated_json()
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize JSON: {}", e)))
}

#[cfg(not(target_arch = "wasm32"))]
/// Parse a Sui transaction to VisualSign JSON
///
/// # Arguments
/// * `tx_data` - Base64-encoded transaction
///
/// # Returns
/// JSON string with human-readable transaction details
#[wasm_bindgen]
pub fn parse_sui_transaction(tx_data: &str) -> Result<String, JsValue> {
    use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
    use visualsign_sui::{SuiTransactionWrapper, SuiVisualSignConverter};

    let tx = SuiTransactionWrapper::from_string(tx_data)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse transaction: {}", e)))?;

    let payload = SuiVisualSignConverter
        .to_visual_sign_payload(
            tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .map_err(|e| JsValue::from_str(&format!("Failed to convert transaction: {}", e)))?;

    payload
        .to_validated_json()
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize JSON: {}", e)))
}

/// Parse a Tron transaction to VisualSign JSON
///
/// # Arguments
/// * `tx_data` - Hex or base64-encoded transaction
///
/// # Returns
/// JSON string with human-readable transaction details
#[wasm_bindgen]
pub fn parse_tron_transaction(tx_data: &str) -> Result<String, JsValue> {
    use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};
    use visualsign_tron::{TronTransactionWrapper, TronVisualSignConverter};

    let tx = TronTransactionWrapper::from_string(tx_data)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse transaction: {}", e)))?;

    let payload = TronVisualSignConverter
        .to_visual_sign_payload(
            tx,
            VisualSignOptions {
                metadata: None,
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .map_err(|e| JsValue::from_str(&format!("Failed to convert transaction: {}", e)))?;

    payload
        .to_validated_json()
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize JSON: {}", e)))
}

/// Parse a transaction with automatic chain detection
///
/// # Arguments
/// * `tx_data` - Encoded transaction data (format will be auto-detected)
///
/// # Returns
/// JSON string with human-readable transaction details
#[wasm_bindgen]
pub fn parse_transaction(tx_data: &str) -> Result<String, JsValue> {
    // Try each chain parser in sequence
    // This is a simple approach - could be improved with format detection

    if let Ok(result) = parse_ethereum_transaction(tx_data) {
        return Ok(result);
    }

    if let Ok(result) = parse_solana_transaction(tx_data) {
        return Ok(result);
    }

    if let Ok(result) = parse_tron_transaction(tx_data) {
        return Ok(result);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(result) = parse_sui_transaction(tx_data) {
            return Ok(result);
        }
    }

    Err(JsValue::from_str("Could not parse transaction with any supported chain parser"))
}
