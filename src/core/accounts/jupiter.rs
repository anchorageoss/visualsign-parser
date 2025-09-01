// Handles Jupiter swap preset logic for Solana

use visualsign::{AnnotatedPayloadField, SignablePayloadField};

#[derive(Debug, Clone)]
pub struct JupiterSwapInfo {
    pub in_token: Option<String>,
    pub out_token: Option<String>,
    pub amount: u64,
    pub slippage_bps: u16,
}

pub fn format_jupiter_swap(info: &JupiterSwapInfo) -> String {
    format!(
        "Jupiter Swap: {} -> {} (amount: {}, slippage: {}bps)",
        info.in_token.as_deref().unwrap_or("Unknown"),
        info.out_token.as_deref().unwrap_or("Unknown"),
        info.amount,
        info.slippage_bps
    )
}

// Add more helpers as needed for expanded fields, etc.
