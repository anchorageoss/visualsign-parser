use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Overall test report for nightly run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    pub timestamp: String,
    pub total_pairs: usize,
    pub successful_pairs: usize,
    pub failed_pairs: usize,
    pub total_transactions: usize,
    pub successful_transactions: usize,
    pub failed_transactions: usize,
    pub duration: Duration,
    pub pair_results: Vec<PairTestResult>,
}

/// Test result for a single trading pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairTestResult {
    pub pair_name: String,
    pub input_mint: String,
    pub output_mint: String,
    pub transactions_tested: usize,
    pub transactions_passed: usize,
    pub transactions_failed: usize,
    pub success_rate: f64,
    pub failed_signatures: Vec<String>,
    pub errors: Vec<String>,
}

impl TestReport {
    pub fn new() -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            total_pairs: 0,
            successful_pairs: 0,
            failed_pairs: 0,
            total_transactions: 0,
            successful_transactions: 0,
            failed_transactions: 0,
            duration: Duration::from_secs(0),
            pair_results: Vec::new(),
        }
    }

    pub fn add_pair_result(&mut self, result: PairTestResult) {
        self.total_pairs += 1;
        self.total_transactions += result.transactions_tested;
        self.successful_transactions += result.transactions_passed;
        self.failed_transactions += result.transactions_failed;

        if result.transactions_failed == 0 {
            self.successful_pairs += 1;
        } else {
            self.failed_pairs += 1;
        }

        self.pair_results.push(result);
    }

    pub fn set_duration(&mut self, duration: Duration) {
        self.duration = duration;
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_transactions == 0 {
            return 0.0;
        }
        (self.successful_transactions as f64 / self.total_transactions as f64) * 100.0
    }

    pub fn to_json(&self) -> anyhow::Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| anyhow::anyhow!("Failed to serialize report: {}", e))
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Jupiter Nightly Test Report\n\n");
        md.push_str(&format!("**Timestamp:** {}\n\n", self.timestamp));
        md.push_str(&format!("**Duration:** {:.2}s\n\n", self.duration.as_secs_f64()));

        md.push_str("## Summary\n\n");
        md.push_str(&format!("- **Total Pairs Tested:** {}\n", self.total_pairs));
        md.push_str(&format!("- **Successful Pairs:** {}\n", self.successful_pairs));
        md.push_str(&format!("- **Failed Pairs:** {}\n", self.failed_pairs));
        md.push_str(&format!("- **Total Transactions:** {}\n", self.total_transactions));
        md.push_str(&format!("- **Success Rate:** {:.1}%\n\n", self.success_rate()));

        md.push_str("## Results by Trading Pair\n\n");
        md.push_str("| Pair | Tested | Passed | Failed | Success Rate |\n");
        md.push_str("|------|--------|--------|--------|-------------|\n");

        for result in &self.pair_results {
            let status = if result.transactions_failed == 0 {
                "✅"
            } else {
                "❌"
            };

            md.push_str(&format!(
                "| {} {} | {} | {} | {} | {:.1}% |\n",
                status,
                result.pair_name,
                result.transactions_tested,
                result.transactions_passed,
                result.transactions_failed,
                result.success_rate
            ));
        }

        // Add details for failed pairs
        let failed_pairs: Vec<_> = self
            .pair_results
            .iter()
            .filter(|r| r.transactions_failed > 0)
            .collect();

        if !failed_pairs.is_empty() {
            md.push_str("\n## Failed Pairs Details\n\n");

            for result in failed_pairs {
                md.push_str(&format!("### {} ({} failures)\n\n", result.pair_name, result.transactions_failed));

                if !result.failed_signatures.is_empty() {
                    md.push_str("**Failed Signatures:**\n\n");
                    for sig in &result.failed_signatures {
                        md.push_str(&format!("- `{}`\n", sig));
                    }
                    md.push('\n');
                }

                if !result.errors.is_empty() {
                    md.push_str("**Errors:**\n\n");
                    for error in &result.errors {
                        md.push_str(&format!("- {}\n", error));
                    }
                    md.push('\n');
                }
            }
        }

        md
    }

    pub fn save_json(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let json = self.to_json()?;
        std::fs::write(path.as_ref(), json)?;
        Ok(())
    }

    pub fn save_markdown(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let md = self.to_markdown();
        std::fs::write(path.as_ref(), md)?;
        Ok(())
    }
}

impl Default for TestReport {
    fn default() -> Self {
        Self::new()
    }
}

impl PairTestResult {
    pub fn new(pair_name: String, input_mint: String, output_mint: String) -> Self {
        Self {
            pair_name,
            input_mint,
            output_mint,
            transactions_tested: 0,
            transactions_passed: 0,
            transactions_failed: 0,
            success_rate: 0.0,
            failed_signatures: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn record_success(&mut self) {
        self.transactions_tested += 1;
        self.transactions_passed += 1;
        self.update_success_rate();
    }

    pub fn record_failure(&mut self, signature: String, error: String) {
        self.transactions_tested += 1;
        self.transactions_failed += 1;
        self.failed_signatures.push(signature);
        self.errors.push(error);
        self.update_success_rate();
    }

    fn update_success_rate(&mut self) {
        if self.transactions_tested == 0 {
            self.success_rate = 0.0;
        } else {
            self.success_rate =
                (self.transactions_passed as f64 / self.transactions_tested as f64) * 100.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_creation() {
        let mut report = TestReport::new();
        assert_eq!(report.total_pairs, 0);
        assert_eq!(report.success_rate(), 0.0);

        let mut pair_result = PairTestResult::new(
            "SOL-USDC".to_string(),
            "So11111111111111111111111111111111111111112".to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
        );

        pair_result.record_success();
        pair_result.record_success();
        pair_result.record_failure("sig1".to_string(), "Error 1".to_string());

        report.add_pair_result(pair_result);

        assert_eq!(report.total_pairs, 1);
        assert_eq!(report.total_transactions, 3);
        assert_eq!(report.successful_transactions, 2);
        assert_eq!(report.failed_transactions, 1);
        assert!((report.success_rate() - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_markdown_generation() {
        let mut report = TestReport::new();
        let mut pair_result = PairTestResult::new(
            "SOL-USDC".to_string(),
            "So11111111111111111111111111111111111111112".to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
        );

        pair_result.record_success();
        pair_result.record_success();
        report.add_pair_result(pair_result);

        let markdown = report.to_markdown();
        assert!(markdown.contains("# Jupiter Nightly Test Report"));
        assert!(markdown.contains("SOL-USDC"));
        assert!(markdown.contains("100.0%"));
    }
}
