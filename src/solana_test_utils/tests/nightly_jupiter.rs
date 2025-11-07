use solana_test_utils::prelude::*;
use std::path::PathBuf;
use integration::Builder;

/// Main nightly test for Jupiter trading pairs
#[tokio::test]
#[ignore] // Run with --ignored for nightly
async fn test_nightly_jupiter_pairs() {
    async fn run_test(test_args: integration::TestArgs) {
        // Initialize tracing
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .try_init();

        // Check if Helius API key is available
        if !TransactionFetcher::is_available() {
            eprintln!("⚠️  HELIUS_API_KEY not set - skipping nightly tests");
            eprintln!("   Set HELIUS_API_KEY environment variable to run nightly tests");
            return;
        }

        tracing::info!("✓ Using Helius API for transaction data");

        // Load configuration (or use default)
        let config = load_config();

        // Create transaction fetcher (uses Helius API)
        let fetcher = TransactionFetcher::from_env()
            .expect("Failed to create transaction fetcher");

        // Set up fixture directory
        let fixture_dir = get_fixture_dir();
        std::fs::create_dir_all(&fixture_dir).expect("Failed to create fixture directory");

        // Create test runner with parser client
        let parser_client = Box::new(test_args.parser_client.unwrap());
        let mut runner = NightlyTestRunner::new(config, None, fetcher, fixture_dir.clone(), parser_client);

    // Run all pairs
    tracing::info!("🚀 Starting nightly Jupiter pairs test");
    let report = runner
        .run_all_pairs()
        .await
        .expect("Failed to run nightly tests");

    // Save reports
    let reports_dir = get_reports_dir();
    std::fs::create_dir_all(&reports_dir).expect("Failed to create reports directory");

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let json_path = reports_dir.join(format!("report_{}.json", timestamp));
    let md_path = reports_dir.join(format!("report_{}.md", timestamp));

    report
        .save_json(&json_path)
        .expect("Failed to save JSON report");
    report
        .save_markdown(&md_path)
        .expect("Failed to save Markdown report");

    // Print summary
    println!("\n{}", report.to_markdown());

    // Save latest report (overwrite)
    report
        .save_json(reports_dir.join("latest.json"))
        .expect("Failed to save latest JSON");
    report
        .save_markdown(reports_dir.join("latest.md"))
        .expect("Failed to save latest Markdown");

    tracing::info!("✓ Reports saved to:");
    tracing::info!("  JSON: {}", json_path.display());
    tracing::info!("  Markdown: {}", md_path.display());

    // Assert that we have some success
    assert!(
        report.total_transactions > 0,
        "No transactions were tested"
    );

    // Optionally fail if success rate is below threshold
    let min_success_rate = std::env::var("MIN_SUCCESS_RATE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(80.0);

    if report.success_rate() < min_success_rate {
        panic!(
            "Success rate {:.1}% is below minimum threshold of {:.1}%",
            report.success_rate(),
            min_success_rate
        );
    }

        tracing::info!("✓ Nightly test completed successfully!");
    }

    // Run test with parser infrastructure
    Builder::new().execute(run_test).await
}

/// Test configuration loading
#[test]
fn test_load_config() {
    let config = load_config();
    assert!(!config.pairs.is_empty());
    assert!(config.enabled_pairs().len() >= 5);
}

/// Test report generation
#[test]
fn test_report_generation() {
    let mut report = TestReport::new();

    let mut result = PairTestResult::new(
        "SOL-USDC".to_string(),
        "So11111111111111111111111111111111111111112".to_string(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
    );

    result.record_success();
    result.record_success();
    result.record_failure("sig123".to_string(), "Test error".to_string());

    report.add_pair_result(result);

    let markdown = report.to_markdown();
    assert!(markdown.contains("SOL-USDC"));
    assert!(markdown.contains("Jupiter Nightly Test Report"));

    let json = report.to_json().unwrap();
    assert!(json.contains("SOL-USDC"));
}

// Helper functions

fn load_config() -> PairsConfig {
    let config_path = get_config_path();

    if config_path.exists() {
        PairsConfig::from_file(&config_path).unwrap_or_else(|e| {
            eprintln!("⚠️  Failed to load config from {:?}: {}", config_path, e);
            eprintln!("   Using default configuration");
            PairsConfig::default_config()
        })
    } else {
        // Create default config file
        let config = PairsConfig::default_config();
        if let Err(e) = config.save_to_file(&config_path) {
            eprintln!("⚠️  Failed to save default config: {}", e);
        } else {
            tracing::info!("✓ Created default config at {:?}", config_path);
        }
        config
    }
}

fn get_config_path() -> PathBuf {
    std::env::var("PAIRS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("config")
                .join("pairs_config.toml")
        })
}

fn get_fixture_dir() -> PathBuf {
    std::env::var("FIXTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join("nightly")
        })
}

fn get_reports_dir() -> PathBuf {
    std::env::var("REPORTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("reports")
                .join("nightly")
        })
}
