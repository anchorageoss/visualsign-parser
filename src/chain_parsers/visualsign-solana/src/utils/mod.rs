use std::collections::BTreeMap;

// Constants
const ADDRESS_TRUNCATION_LENGTH: usize = 8;
/// Helper function to create a complete Solana transaction from a message with empty signatures.
/// Used by test code in this crate and by integration tests.
#[allow(clippy::unwrap_used)]
pub fn create_transaction_with_empty_signatures(message_base64: &str) -> String {
    use base64::Engine;
    // Decode the message
    let message_bytes = base64::engine::general_purpose::STANDARD
        .decode(message_base64)
        .unwrap();

    // Create a complete Solana transaction with empty signatures
    let mut transaction_bytes = Vec::new();

    // Add compact array length for signatures (0 signatures)
    transaction_bytes.push(0u8);

    // Add the message
    transaction_bytes.extend_from_slice(&message_bytes);

    // Encode the complete transaction back to base64
    base64::engine::general_purpose::STANDARD.encode(transaction_bytes)
}

#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
}

/// Static lookup table for common Solana token addresses
pub fn get_token_lookup_table() -> BTreeMap<&'static str, TokenInfo> {
    let mut tokens = BTreeMap::new();

    // SOL (native)
    tokens.insert(
        "11111111111111111111111111111112",
        TokenInfo {
            symbol: "SOL",
            name: "Solana",
            decimals: 9,
        },
    );

    // USDC
    tokens.insert(
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        TokenInfo {
            symbol: "USDC",
            name: "USD Coin",
            decimals: 6,
        },
    );

    // USDT
    tokens.insert(
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        TokenInfo {
            symbol: "USDT",
            name: "Tether USD",
            decimals: 6,
        },
    );

    // The entries below were verified on 2026-09-23 against mainnet
    // `getTokenSupply` (decimals) and the Jupiter token list
    // (`https://lite-api.jup.ag/tokens/v2/search?query=<mint>`, symbol/name).

    // USDG (Global Dollar, Token-2022)
    tokens.insert(
        "2u1tszSeqZ3qBWF3uNGPFc8TzMk2tdiwknnRMWGWjGWH",
        TokenInfo {
            symbol: "USDG",
            name: "Global Dollar",
            decimals: 6,
        },
    );

    // JupUSD (Jupiter USD)
    tokens.insert(
        "JuprjznTrTSp2UFa3ZBUFgwdAmtZCq4MQCwysN55USD",
        TokenInfo {
            symbol: "JupUSD",
            name: "Jupiter USD",
            decimals: 6,
        },
    );

    // Jupiter Lend Earn receipt tokens (fTokens), one per lending market. The
    // mint is derived by the program from the underlying asset mint, so these
    // are stable identifiers.
    tokens.insert(
        "9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D",
        TokenInfo {
            symbol: "jlUSDC",
            name: "Jupiter Lend USDC",
            decimals: 6,
        },
    );
    tokens.insert(
        "Cmn4v2wipYV41dkakDvCgFJpxhtaaKt11NyWV8pjSE8A",
        TokenInfo {
            symbol: "jlUSDT",
            name: "Jupiter Lend USDT",
            decimals: 6,
        },
    );
    tokens.insert(
        "9fvHrYNw1A8Evpcj7X2yy4k4fT7nNHcA9L6UsamNHAif",
        TokenInfo {
            symbol: "jlUSDG",
            name: "Jupiter Lend USDG",
            decimals: 6,
        },
    );
    // The JupUSD market's receipt token is listed as JUICED, not "jlJupUSD".
    tokens.insert(
        "7GxATsNMnaC88vdwd2t3mwrFuQwwGvmYPrUQ4D6FotXk",
        TokenInfo {
            symbol: "JUICED",
            name: "JUICED",
            decimals: 6,
        },
    );

    tokens
}

/// Looks a mint up in the static token table.
pub fn lookup_token(mint: &str) -> Option<TokenInfo> {
    get_token_lookup_table().get(mint).cloned()
}

/// Shortens a base58 address for display when no symbol is known:
/// `EPjFWdd5...` becomes `EPjF...Dt1v`.
/// "abcd...wxyz" for anything longer than `ADDRESS_TRUNCATION_LENGTH` bytes.
/// Slices only on char boundaries: an input that cannot be cut cleanly (not a
/// base58 address) comes back whole instead of panicking.
pub fn truncate_address(address: &str) -> String {
    if address.len() <= ADDRESS_TRUNCATION_LENGTH {
        return address.to_string();
    }
    match (address.get(..4), address.get(address.len() - 4..)) {
        (Some(head), Some(tail)) => format!("{head}...{tail}"),
        _ => address.to_string(),
    }
}

/// Helper function to format token amounts.
///
/// Defensive against attacker-controlled `decimals`: uses `checked_pow` so an
/// out-of-range value (>= 20) returns the raw amount rather than triggering a
/// divide-by-zero panic (10^64 wraps to 0 in `u64`, etc.). Callers that ingest
/// `decimals` from untrusted transaction bytes should additionally validate the
/// value up front and surface a parse error to the user.
pub fn format_token_amount(amount: u64, decimals: u8) -> String {
    let Some(divisor) = 10_u64.checked_pow(decimals as u32) else {
        // decimals is out of the representable range for u64; render as raw.
        return amount.to_string();
    };
    if divisor == 0 {
        // Belt and braces: should be unreachable given checked_pow above.
        return amount.to_string();
    }
    let whole = amount / divisor;
    let fractional = amount % divisor;

    if fractional == 0 {
        format!("{whole}")
    } else {
        let fractional_str = format!("{:0width$}", fractional, width = decimals as usize);
        let trimmed = fractional_str.trim_end_matches('0');
        if trimmed.is_empty() {
            format!("{whole}")
        } else {
            format!("{whole}.{trimmed}")
        }
    }
}

/// Enhanced swap instruction with token information
#[derive(Debug, Clone)]
pub struct SwapTokenInfo {
    pub address: String,
    pub symbol: String,
    pub name: String,
    pub decimals: u8,
    pub amount: u64,
    pub human_readable_amount: String,
}

/// Helper function to get token info from address
pub fn get_token_info(address: &str, amount: u64) -> SwapTokenInfo {
    let token_lookup = get_token_lookup_table();

    if let Some(token_info) = token_lookup.get(address) {
        SwapTokenInfo {
            address: address.to_string(),
            symbol: token_info.symbol.to_string(),
            name: token_info.name.to_string(),
            decimals: token_info.decimals,
            amount,
            human_readable_amount: format_token_amount(amount, token_info.decimals),
        }
    } else {
        // Unknown token - show truncated address
        let truncated = truncate_address(address);

        SwapTokenInfo {
            address: address.to_string(),
            symbol: truncated.clone(),
            name: format!("Unknown Token ({truncated})"),
            decimals: 0,
            amount,
            human_readable_amount: amount.to_string(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_address() {
        assert_eq!(
            truncate_address("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
            "EPjF...Dt1v"
        );
        assert_eq!(truncate_address("short"), "short");
        // Multi-byte input must not panic on a byte-index slice. A cut that
        // would land inside a char returns the input whole; a clean cut works.
        assert_eq!(truncate_address("abcéfghijklmn"), "abcéfghijklmn");
        assert_eq!(truncate_address("abcdéfghijkl"), "abcd...ijkl");
    }

    #[test]
    fn test_format_token_amount_typical_decimals() {
        // 9 decimals (SOL)
        assert_eq!(format_token_amount(1_000_000_000, 9), "1");
        assert_eq!(format_token_amount(1_500_000_000, 9), "1.5");
        // 6 decimals (USDC)
        assert_eq!(format_token_amount(1_000_000, 6), "1");
        assert_eq!(format_token_amount(1_234_567, 6), "1.234567");
        // 0 decimals
        assert_eq!(format_token_amount(42, 0), "42");
    }

    #[test]
    fn test_format_token_amount_zero_amount() {
        assert_eq!(format_token_amount(0, 9), "0");
        assert_eq!(format_token_amount(0, 0), "0");
        // Out-of-range decimals with zero amount should still return "0", not panic.
        assert_eq!(format_token_amount(0, 64), "0");
    }

    /// Regression: decimals >= 20 must not trigger a divide-by-zero
    /// panic. `10_u64.pow(20)` overflows in debug and wraps in release; for
    /// `decimals == 64` the wrapped value is exactly `0` because `10^64 mod
    /// 2^64 == 0`, which used to panic on division.
    #[test]
    fn test_format_token_amount_decimals_out_of_range_does_not_panic() {
        // Each call must return without panicking.
        for decimals in [20u8, 21, 38, 63, 64, 100, 200, u8::MAX] {
            let formatted = format_token_amount(12_345_678_u64, decimals);
            // Fallback path: render the raw amount.
            assert_eq!(
                formatted, "12345678",
                "decimals={decimals} should fall back to raw amount"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub mod test_utils {
    use crate::core::VisualizerContext;
    use crate::transaction_string_to_visual_sign;
    use solana_parser::solana::structs::SolanaAccount;
    use solana_sdk::instruction::{CompiledInstruction, Instruction};
    use solana_sdk::pubkey::Pubkey;
    use visualsign::SignablePayload;
    use visualsign::vsptrait::VisualSignOptions;

    pub fn payload_from_b64(data: &str) -> SignablePayload {
        transaction_string_to_visual_sign(
            data,
            VisualSignOptions {
                include_intermediate_output: false,
                metadata: None,
                decode_transfers: true,
                transaction_name: None,
                developer_config: None,
            },
        )
        .expect("Failed to visualize tx commands")
    }

    /// Owned wire data for one `VisualizerContext`: the instruction's program
    /// at `account_keys[0]`, its accounts after it, and the first instruction
    /// account as the sender. Presets' instruction-level tests build contexts
    /// from a resolved `Instruction` through this instead of hand-rolling it.
    pub struct InstructionTestContext {
        sender: SolanaAccount,
        compiled: CompiledInstruction,
        account_keys: Vec<Pubkey>,
        registry: crate::idl::IdlRegistry,
    }

    impl InstructionTestContext {
        pub fn from_instruction(instruction: &Instruction) -> Self {
            let mut account_keys = vec![instruction.program_id];
            account_keys.extend(instruction.accounts.iter().map(|m| m.pubkey));
            let compiled = CompiledInstruction {
                program_id_index: 0,
                accounts: (1..=instruction.accounts.len() as u8).collect(),
                data: instruction.data.clone(),
            };
            let sender = SolanaAccount {
                account_key: instruction
                    .accounts
                    .first()
                    .map(|m| m.pubkey.to_string())
                    .unwrap_or_default(),
                signer: true,
                writable: true,
            };
            Self {
                sender,
                compiled,
                account_keys,
                registry: crate::idl::IdlRegistry::new(),
            }
        }

        /// The compiled instruction, for tests that need to point an account
        /// at an index outside `account_keys` (an unresolved ALT entry).
        pub fn compiled_mut(&mut self) -> &mut CompiledInstruction {
            &mut self.compiled
        }

        pub fn context(&self) -> VisualizerContext<'_> {
            VisualizerContext::new(
                &self.sender,
                &self.compiled,
                &self.account_keys,
                &self.registry,
                0,
            )
        }
    }

    pub fn assert_has_field(payload: &SignablePayload, label: &str) {
        payload
            .fields
            .iter()
            .find(|f| f.label() == label)
            .unwrap_or_else(|| panic!("Should have a {label} field"));
    }
}
