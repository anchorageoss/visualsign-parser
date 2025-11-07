use super::{
    config::{PairsConfig, TradingPair},
    fetcher::TransactionFetcher,
    report::{PairTestResult, TestReport},
};
use crate::{FixtureManager, SurfpoolManager};
use anyhow::{Context, Result};
use std::time::Instant;

/// Type alias for parser client (can be any type that implements the parser interface)
pub type ParserClient = Box<dyn std::any::Any + Send + Sync>;

/// Runner for nightly Jupiter trading pair tests
///
/// Can be used with or without Surfpool:
/// - Without Surfpool: Uses live RPC (Helius) for real mainnet transactions
/// - With Surfpool: Uses local Solana fork for controlled testing environment
pub struct NightlyTestRunner {
    config: PairsConfig,
    surfpool: Option<SurfpoolManager>,
    fetcher: TransactionFetcher,
    fixture_manager: FixtureManager,
    parser_client: Option<ParserClient>,
}

impl NightlyTestRunner {
    /// Create a new test runner
    pub fn new(
        config: PairsConfig,
        surfpool: Option<SurfpoolManager>,
        fetcher: TransactionFetcher,
        fixture_dir: impl Into<std::path::PathBuf>,
        parser_client: ParserClient,
    ) -> Self {
        Self {
            config,
            surfpool,
            fetcher,
            fixture_manager: FixtureManager::new(fixture_dir),
            parser_client: Some(parser_client),
        }
    }

    /// Create a new test runner without parser client (for basic tests)
    pub fn new_without_parser(
        config: PairsConfig,
        surfpool: Option<SurfpoolManager>,
        fetcher: TransactionFetcher,
        fixture_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            config,
            surfpool,
            fetcher,
            fixture_manager: FixtureManager::new(fixture_dir),
            parser_client: None,
        }
    }

    /// Get RPC client if Surfpool is available
    pub fn rpc_client(&self) -> Option<solana_client::rpc_client::RpcClient> {
        self.surfpool.as_ref().map(|s| s.rpc_client())
    }

    /// Run tests for all enabled trading pairs
    pub async fn run_all_pairs(&mut self) -> Result<TestReport> {
        let start = Instant::now();
        let mut report = TestReport::new();

        let enabled_pairs = self.config.enabled_pairs();
        tracing::info!("Running tests for {} enabled pairs", enabled_pairs.len());

        if self.surfpool.is_some() {
            tracing::info!("✓ Using Surfpool for local testing");
        }

        for pair in enabled_pairs {
            tracing::info!("Testing pair: {}", pair.name);

            match self.test_pair(pair).await {
                Ok(result) => {
                    report.add_pair_result(result);
                }
                Err(e) => {
                    tracing::error!("Failed to test pair {}: {}", pair.name, e);
                    let mut result = PairTestResult::new(
                        pair.name.clone(),
                        pair.input_mint.clone(),
                        pair.output_mint.clone(),
                    );
                    result.record_failure(
                        "N/A".to_string(),
                        format!("Failed to fetch transactions: {}", e),
                    );
                    report.add_pair_result(result);
                }
            }
        }

        report.set_duration(start.elapsed());

        Ok(report)
    }

    /// Test a single trading pair
    async fn test_pair(&self, pair: &TradingPair) -> Result<PairTestResult> {
        let mut result = PairTestResult::new(
            pair.name.clone(),
            pair.input_mint.clone(),
            pair.output_mint.clone(),
        );

        // Fetch recent transactions for this pair
        let transactions = self
            .fetcher
            .fetch_recent_swaps(&pair.input_mint, &pair.output_mint, pair.min_transactions)
            .await
            .context("Failed to fetch transactions")?;

        tracing::info!(
            "Fetched {} transactions for pair {}",
            transactions.len(),
            pair.name
        );

        if transactions.is_empty() {
            result.record_failure(
                "N/A".to_string(),
                "No transactions found for this pair".to_string(),
            );
            return Ok(result);
        }

        // Test each transaction
        for tx_data in transactions {
            match self.test_transaction(&tx_data).await {
                Ok(_) => {
                    result.record_success();
                    tracing::debug!("✓ Transaction {} passed", tx_data.signature);
                }
                Err(e) => {
                    result.record_failure(tx_data.signature.clone(), e.to_string());
                    tracing::warn!("✗ Transaction {} failed: {}", tx_data.signature, e);
                }
            }
        }

        Ok(result)
    }

    /// Test a single transaction
    async fn test_transaction(
        &self,
        tx_data: &super::fetcher::TransactionData,
    ) -> Result<()> {
        // Validate signature format
        use std::str::FromStr;
        solana_sdk::signature::Signature::from_str(&tx_data.signature)
            .context("Invalid signature format")?;

        // Validate transaction structure
        if tx_data.transaction.message.is_null() {
            anyhow::bail!("Transaction message is null");
        }

        // Validate transaction is serializable
        let tx_bytes = serde_json::to_vec(&tx_data.transaction)
            .context("Failed to serialize transaction")?;

        // If parser client is available, validate actual parsing
        // Note: The parser client is passed via gRPC from the test harness
        // For now, we validate structure. Full validation requires implementing
        // the gRPC call in the test harness with the parser client.
        if !self.parser_client.is_none() {
            tracing::debug!(
                "Parser client available for transaction {}, but gRPC integration pending",
                tx_data.signature
            );
        }

        // Helius already validated this is a real transaction from the chain
        tracing::trace!(
            "Validated transaction {} (slot: {}, size: {} bytes)",
            tx_data.signature,
            tx_data.slot,
            tx_bytes.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nightly::config::PairsConfig;
    use tempfile::TempDir;

    #[tokio::test]
    #[ignore] // Requires HELIUS_API_KEY and surfpool
    async fn test_runner_basic() {
        if !TransactionFetcher::is_available() {
            println!("Skipping: HELIUS_API_KEY not set");
            return;
        }

        let config = PairsConfig::default_config();
        let fetcher = TransactionFetcher::from_env().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let mut runner =
            NightlyTestRunner::new_without_parser(config, None, fetcher, temp_dir.path().to_path_buf());

        let report = runner.run_all_pairs().await.unwrap();

        println!("{}", report.to_markdown());
        assert!(report.total_pairs > 0);
    }
}
