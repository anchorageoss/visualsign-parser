use solana_test_utils::prelude::*;
use std::path::PathBuf;
use integration::Builder;

/// Surfpool-based Jupiter test with controlled environment
#[tokio::test]
#[ignore] // Run with --ignored, requires surfpool installed
async fn test_jupiter_with_surfpool() {
    async fn run_test(test_args: integration::TestArgs) {
        // Initialize tracing
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .try_init();

        // Start Surfpool with mainnet fork
        let config = SurfpoolConfig::default();
        let surfpool = match SurfpoolManager::start(config).await {
            Ok(s) => {
                tracing::info!("✓ Surfpool started successfully");
                s
            }
            Err(e) => {
                eprintln!("✗ Failed to start Surfpool: {}", e);
                eprintln!("  Make sure surfpool is installed: https://github.com/txtx/surfpool");
                eprintln!("  Install with: cargo xtask install");
                return;
            }
        };

        tracing::info!("✓ Using Surfpool for controlled testing environment");

        // Load configuration
        let config = PairsConfig::default_config();

        // Create transaction fetcher (uses Helius to fetch real transactions as fixtures)
        let fetcher = match TransactionFetcher::from_env() {
            Ok(f) => {
                tracing::info!("✓ Helius API available - will use for fixture data");
                f
            }
            Err(_) => {
                tracing::warn!("⚠️  HELIUS_API_KEY not set - skipping fixture fetching");
                tracing::warn!("   Set HELIUS_API_KEY to fetch real transactions as test fixtures");
                return;
            }
        };

        // Set up fixture directory
        let fixture_dir = get_fixture_dir();
        std::fs::create_dir_all(&fixture_dir).expect("Failed to create fixture directory");

        // Create test runner with parser client and surfpool
        let parser_client = Box::new(test_args.parser_client.unwrap());
        let mut runner = NightlyTestRunner::new(config, Some(surfpool), fetcher, fixture_dir.clone(), parser_client);

        tracing::info!("🚀 Starting Surfpool Jupiter test");
        let report = runner
            .run_all_pairs()
            .await
            .expect("Failed to run Surfpool tests");

        // Save reports
        let reports_dir = get_reports_dir();
        std::fs::create_dir_all(&reports_dir).expect("Failed to create reports directory");

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let json_path = reports_dir.join(format!("surfpool_report_{}.json", timestamp));
        let md_path = reports_dir.join(format!("surfpool_report_{}.md", timestamp));

        report
            .save_json(&json_path)
            .expect("Failed to save JSON report");
        report
            .save_markdown(&md_path)
            .expect("Failed to save Markdown report");

        // Print summary
        println!("\n{}", report.to_markdown());

        tracing::info!("✓ Reports saved to:");
        tracing::info!("  JSON: {}", json_path.display());
        tracing::info!("  Markdown: {}", md_path.display());

        // Assert that we have some success
        assert!(
            report.total_transactions > 0,
            "No transactions were tested"
        );

        tracing::info!("✓ Surfpool test completed successfully!");
    }

    // Run test with parser infrastructure
    Builder::new().execute(run_test).await
}

// Helper functions

fn get_fixture_dir() -> PathBuf {
    std::env::var("FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join("surfpool")
        })
}

fn get_reports_dir() -> PathBuf {
    std::env::var("REPORTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("reports")
                .join("surfpool")
        })
}
