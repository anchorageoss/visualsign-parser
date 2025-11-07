use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Well-known token mint addresses on Solana mainnet
pub mod known_mints {
    use super::*;

    pub fn sol() -> Pubkey {
        Pubkey::from_str("So11111111111111111111111111111111111111112")
            .expect("Invalid SOL mint")
    }

    pub fn usdc() -> Pubkey {
        Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
            .expect("Invalid USDC mint")
    }

    pub fn usdt() -> Pubkey {
        Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB")
            .expect("Invalid USDT mint")
    }

    pub fn bonk() -> Pubkey {
        Pubkey::from_str("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263")
            .expect("Invalid BONK mint")
    }

    pub fn jup() -> Pubkey {
        Pubkey::from_str("JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN")
            .expect("Invalid JUP mint")
    }
}

/// Format token amount with decimals
pub fn format_token_amount(amount: u64, decimals: u8) -> String {
    let divisor = 10u64.pow(decimals as u32);
    let whole = amount / divisor;
    let fractional = amount % divisor;

    if fractional == 0 {
        format!("{}", whole)
    } else {
        let fractional_str = format!("{:0width$}", fractional, width = decimals as usize);
        let trimmed = fractional_str.trim_end_matches('0');
        format!("{}.{}", whole, trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_token_amount() {
        assert_eq!(format_token_amount(1_000_000, 6), "1");
        assert_eq!(format_token_amount(1_500_000, 6), "1.5");
        assert_eq!(format_token_amount(1_234_567, 6), "1.234567");
        assert_eq!(format_token_amount(0, 6), "0");
    }

    #[test]
    fn test_known_mints() {
        assert_eq!(
            known_mints::sol().to_string(),
            "So11111111111111111111111111111111111111112"
        );
        assert_eq!(
            known_mints::usdc().to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
        );
    }
}
