use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configuration for nightly trading pairs tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairsConfig {
    pub pairs: Vec<TradingPair>,
}

/// A trading pair to test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingPair {
    /// Human-readable name (e.g., "SOL-USDC")
    pub name: String,
    /// Input token mint address
    pub input_mint: String,
    /// Output token mint address
    pub output_mint: String,
    /// Whether this pair is enabled for testing
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional: minimum transactions to fetch
    #[serde(default = "default_min_transactions")]
    pub min_transactions: usize,
}

fn default_enabled() -> bool {
    true
}

fn default_min_transactions() -> usize {
    5
}

impl PairsConfig {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.as_ref().display()))
    }

    /// Load default configuration
    pub fn default_config() -> Self {
        Self {
            pairs: vec![
                TradingPair {
                    name: "SOL-USDC".to_string(),
                    input_mint: "So11111111111111111111111111111111111111112".to_string(),
                    output_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                    enabled: true,
                    min_transactions: 5,
                },
                TradingPair {
                    name: "SOL-USDT".to_string(),
                    input_mint: "So11111111111111111111111111111111111111112".to_string(),
                    output_mint: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
                    enabled: true,
                    min_transactions: 5,
                },
                TradingPair {
                    name: "USDC-USDT".to_string(),
                    input_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                    output_mint: "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string(),
                    enabled: true,
                    min_transactions: 5,
                },
                TradingPair {
                    name: "SOL-mSOL".to_string(),
                    input_mint: "So11111111111111111111111111111111111111112".to_string(),
                    output_mint: "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So".to_string(),
                    enabled: true,
                    min_transactions: 5,
                },
                TradingPair {
                    name: "BONK-SOL".to_string(),
                    input_mint: "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263".to_string(),
                    output_mint: "So11111111111111111111111111111111111111112".to_string(),
                    enabled: true,
                    min_transactions: 5,
                },
            ],
        }
    }

    /// Get only enabled pairs
    pub fn enabled_pairs(&self) -> Vec<&TradingPair> {
        self.pairs.iter().filter(|p| p.enabled).collect()
    }

    /// Save configuration to a TOML file
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .context("Failed to serialize config to TOML")?;

        std::fs::write(path.as_ref(), content)
            .with_context(|| format!("Failed to write config file: {}", path.as_ref().display()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = PairsConfig::default_config();
        assert_eq!(config.pairs.len(), 5);
        assert_eq!(config.enabled_pairs().len(), 5);
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("pairs_config.toml");

        let config = PairsConfig::default_config();
        config.save_to_file(&path).unwrap();

        let loaded = PairsConfig::from_file(&path).unwrap();
        assert_eq!(loaded.pairs.len(), config.pairs.len());
    }

    #[test]
    fn test_enabled_filtering() {
        let mut config = PairsConfig::default_config();
        config.pairs[0].enabled = false;
        config.pairs[1].enabled = false;

        assert_eq!(config.enabled_pairs().len(), 3);
    }
}
