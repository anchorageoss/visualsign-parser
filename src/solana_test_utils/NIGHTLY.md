# Nightly Jupiter Trading Pairs Tests

Automated testing suite for Jupiter swap parser using real mainnet transactions.

## Overview

The nightly test suite:
- Fetches recent Jupiter swap transactions for common trading pairs via Helius API
- Tests the parser against real-world transaction data
- Generates detailed reports with success rates
- Runs automatically every night via GitHub Actions

## Quick Start

### Prerequisites

1. **Helius API Key**: Get one at https://helius.dev
2. **Surfpool (optional)**: For enhanced testing with mainnet fork

```bash
# Set API key
export HELIUS_API_KEY=your_key_here

# Optional: Enable Surfpool
export SURFPOOL_ENABLED=1
```

### Running Locally

```bash
cd src
cargo test -p solana_test_utils --test nightly_jupiter -- --ignored --nocapture
```

### Configuration

Edit `solana_test_utils/config/pairs_config.toml` to customize trading pairs:

```toml
[[pairs]]
name = "SOL-USDC"
input_mint = "So11111111111111111111111111111111111111112"
output_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
enabled = true
min_transactions = 5
```

## Trading Pairs

### Default Enabled Pairs (Top 5)
- **SOL-USDC**: Native SOL to USDC stablecoin
- **SOL-USDT**: Native SOL to USDT stablecoin
- **USDC-USDT**: Stablecoin arbitrage
- **SOL-mSOL**: Liquid staking derivative
- **BONK-SOL**: Meme token to SOL

### Additional Pairs (Disabled by Default)
- JUP-SOL, PYTH-SOL, ORCA-SOL, RAY-SOL, WIF-SOL

Enable by setting `enabled = true` in config file.

## Reports

Reports are saved to `solana_test_utils/reports/nightly/`:

- **latest.json**: Most recent run (JSON format)
- **latest.md**: Most recent run (Markdown)
- **report_YYYYMMDD_HHMMSS.json**: Timestamped archives
- **report_YYYYMMDD_HHMMSS.md**: Timestamped archives

### Report Format

```markdown
# Jupiter Nightly Test Report

**Timestamp:** 2025-01-15T02:00:00Z
**Duration:** 45.23s

## Summary
- **Total Pairs Tested:** 5
- **Successful Pairs:** 5
- **Failed Pairs:** 0
- **Total Transactions:** 25
- **Success Rate:** 100.0%

## Results by Trading Pair
| Pair | Tested | Passed | Failed | Success Rate |
|------|--------|--------|--------|--------------|
| ✅ SOL-USDC | 5 | 5 | 0 | 100.0% |
| ✅ SOL-USDT | 5 | 5 | 0 | 100.0% |
...
```

## GitHub Actions

### Scheduled Run

The workflow runs automatically daily at 2 AM UTC.

### Manual Trigger

1. Go to **Actions** → **Nightly Jupiter Trading Pairs Test**
2. Click **Run workflow**
3. Optionally set minimum success rate threshold

### Secrets Required

Add these to your repository secrets:

- `HELIUS_API_KEY` (required): Your Helius API key
- `SURFPOOL_ENABLED` (optional): Set to `1` to enable Surfpool
- `SLACK_WEBHOOK_URL` (optional): For failure notifications

### Artifacts

Test reports are uploaded as workflow artifacts and retained for 30 days.

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `HELIUS_API_KEY` | Helius API key (required) | - |
| `SURFPOOL_ENABLED` | Enable Surfpool integration | `0` |
| `MIN_SUCCESS_RATE` | Minimum pass rate to succeed | `80` |
| `PAIRS_CONFIG` | Path to pairs config file | `config/pairs_config.toml` |
| `FIXTURE_DIR` | Directory for test fixtures | `fixtures/nightly` |
| `REPORTS_DIR` | Directory for test reports | `reports/nightly` |
| `RUST_LOG` | Logging level | `info` |

## Architecture

```
solana_test_utils/src/nightly/
├── config.rs       # Trading pairs configuration
├── fetcher.rs      # Helius API transaction fetching
├── runner.rs       # Test execution logic
└── report.rs       # Report generation

solana_test_utils/tests/
└── nightly_jupiter.rs  # Main test entry point

solana_test_utils/config/
└── pairs_config.toml   # Trading pairs configuration

.github/workflows/
└── nightly-jupiter.yml  # GitHub Actions workflow
```

## How It Works

1. **Load Configuration**: Read trading pairs from TOML config
2. **Fetch Transactions**: Query Helius API for recent Jupiter swaps
3. **Test Each Pair**:
   - Fetch 5+ recent transactions per pair
   - Validate transaction structure and signature format
   - Confirm transaction exists on mainnet via Helius
   - Record success/failure
4. **Generate Report**: Create JSON and Markdown reports
5. **Upload Artifacts**: Save reports for review

## Current Testing Scope

The nightly test currently validates:
- ✅ Real Jupiter transactions exist on mainnet
- ✅ Transactions can be fetched and deserialized
- ✅ Transaction signatures are valid
- ✅ Transactions are from the correct program (Jupiter)

**Not yet validated:**
- ❌ Parsed transaction contains correct token mints
- ❌ Actual gRPC parser integration for full validation
- ❌ Jupiter-specific instruction fields

To fully test mint address extraction and Jupiter visualization, the test needs to:
1. Connect to the running parser via gRPC
2. Send transaction hex to the parser
3. Validate the response contains expected mints and Jupiter fields

This is marked as a TODO for integration with the gRPC parser interface.

## Troubleshooting

### "HELIUS_API_KEY not set"

```bash
export HELIUS_API_KEY=your_key_here
```

### "No transactions found for this pair"

The pair may have low trading volume. Try:
- Decreasing `min_transactions` in config
- Using a more popular trading pair
- Checking Helius API rate limits

### Parser Failures

Check the report for:
- Failed transaction signatures
- Error messages
- Success rate per pair

Investigate specific transactions on Solscan:
```
https://solscan.io/tx/<signature>
```

### Surfpool Issues

Surfpool is optional. Tests will run without it but won't use mainnet fork.

To install Surfpool from source:
```bash
# Clone the repository
git clone https://github.com/txtx/surfpool.git

# Navigate to the directory
cd surfpool

# Compile and install
cargo xtask install

# Verify installation
surfpool -V

# Make sure surfpool is on your PATH
```

## Adding New Pairs

1. Edit `config/pairs_config.toml`:

```toml
[[pairs]]
name = "NEW-TOKEN"
input_mint = "address..."
output_mint = "address..."
enabled = true
min_transactions = 5
```

2. Run tests to verify:

```bash
cargo test -p solana_test_utils --test nightly_jupiter -- --ignored
```

## Best Practices

1. **Start with default pairs**: Use the top 5 pairs initially
2. **Monitor success rates**: Aim for >95% success rate
3. **Review failed transactions**: Check Solscan for unusual patterns
4. **Update fixtures**: Failed transactions can become new test fixtures
5. **Adjust thresholds**: Tune `MIN_SUCCESS_RATE` based on your needs

## CI/CD Integration

### Local Pre-commit Check

```bash
# Quick smoke test with 1 transaction per pair
MIN_TRANSACTIONS=1 cargo test -p solana_test_utils --test nightly_jupiter -- --ignored
```

### Pull Request Check (optional)

Add to your CI:
```yaml
- name: Quick Jupiter test
  env:
    HELIUS_API_KEY: ${{ secrets.HELIUS_API_KEY }}
  run: |
    cd src
    cargo test -p solana_test_utils test_load_config test_report_generation
```

## Future Enhancements

- [ ] Support for ExactOutRoute and SharedAccountsRoute variants
- [ ] Integration with Jupiter parser for actual validation
- [ ] Performance metrics tracking
- [ ] Historical trend analysis
- [ ] Automatic fixture generation from failures
- [ ] Multi-hop route testing
- [ ] Slippage variation testing

## Support

- Issues: https://github.com/tkhq/visualsign-parser/issues
- Documentation: See README.md in solana_test_utils/

## License

Same as parent project.
