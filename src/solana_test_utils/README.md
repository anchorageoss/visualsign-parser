# Solana Test Utils

A reusable testing framework for Solana programs with Surfpool integration.

## Overview

`solana_test_utils` provides a comprehensive testing infrastructure for Solana programs, including:

- **Surfpool Integration**: Manage Surfpool instances for testing with mainnet-forked state
- **Transaction Builders**: Fluent API for constructing and executing transactions
- **Fixture Management**: Load and save test fixtures in JSON format
- **Validation Utilities**: Assert and validate transaction properties
- **Common Utilities**: Token helpers, account management, and more

## Features

### 1. Surfpool Manager

Automatically manages the lifecycle of a Surfpool validator instance:

```rust
use solana_test_utils::prelude::*;

#[tokio::test]
async fn test_with_surfpool() {
    let config = SurfpoolConfig::default()
        .with_fork_url("https://api.mainnet-beta.solana.com");

    let surfpool = SurfpoolManager::start(config).await.unwrap();
    let client = surfpool.rpc_client();

    // Use the client for testing
    // Surfpool automatically shuts down when dropped
}
```

### 2. Transaction Builder

Build and execute transactions with a fluent API:

```rust
use solana_test_utils::prelude::*;
use solana_sdk::system_instruction;

let signature = SolanaTransactionBuilder::new()
    .add_instruction(system_instruction::transfer(
        &payer.pubkey(),
        &recipient.pubkey(),
        1_000_000,
    ))
    .set_payer(payer)
    .execute(&client)
    .await
    .unwrap();
```

### 3. Fixture Management

Load and save test fixtures:

```rust
use solana_test_utils::prelude::*;

let fixture_manager = FixtureManager::new("tests/fixtures");

// Load a fixture
let fixture: SolanaTestFixture = fixture_manager
    .load("jupiter_swap")
    .unwrap();

// Create and save a new fixture
let new_fixture = SolanaTestFixture::new(
    "My test case",
    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
    "base58_encoded_data",
)
.with_expected_field("input_amount", "1000000");

fixture_manager.save("my_test", &new_fixture).unwrap();
```

### 4. Transaction Validation

Validate transaction properties:

```rust
use solana_test_utils::prelude::*;

// Using assertions (panics on failure)
transaction
    .assert_signed()
    .assert_instruction_count(1)
    .assert_signer(&payer.pubkey());

// Using validator (collects errors)
TransactionValidator::new(&transaction)
    .validate_signed()
    .validate_instruction_count(1)
    .validate_signer(&payer.pubkey())
    .complete()
    .unwrap();
```

## Usage in Tests

### Basic Example

```rust
use solana_test_utils::prelude::*;

#[tokio::test]
#[ignore] // Requires surfpool installation
async fn test_jupiter_swap() {
    // Start Surfpool with mainnet fork
    let surfpool = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .unwrap();

    // Load test fixture
    let fixture_manager = FixtureManager::new("tests/fixtures");
    let fixture: SolanaTestFixture = fixture_manager
        .load("jupiter_swap")
        .unwrap();

    // Build and execute transaction
    let instruction_data = fixture.decode_instruction_data().unwrap();
    let accounts = fixture.accounts.iter()
        .map(|a| a.to_account_meta())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let client = surfpool.rpc_client();
    // ... execute transaction and validate
}
```

### With Jupiter Parser

```rust
use solana_test_utils::prelude::*;
use visualsign_solana::presets::jupiter_swap::JupiterSwapVisualizer;

#[tokio::test]
async fn test_jupiter_with_surfpool() {
    let surfpool = SurfpoolManager::start(SurfpoolConfig::default())
        .await
        .unwrap();

    // Generate transaction with Surfpool
    // Parse with Jupiter visualizer
    // Validate output
}
```

## Installation

Add to your `Cargo.toml`:

```toml
[dev-dependencies]
solana_test_utils = { path = "../solana_test_utils" }
```

## Requirements

- **Surfpool**: Must be installed to run tests marked with `#[ignore]`
  - Install from: https://docs.surfpool.run
  - Or use: `cargo install surfpool`

## Common Utilities

### Token Helpers

```rust
use solana_test_utils::common::{known_mints, format_token_amount};

let sol_mint = known_mints::sol();
let usdc_mint = known_mints::usdc();

let formatted = format_token_amount(1_500_000, 6); // "1.5"
```

### Account Helpers

```rust
use solana_test_utils::common::TestAccount;

let account = TestAccount::new(
    "So11111111111111111111111111111111111111112",
    false, // not a signer
    true,  // writable
).with_description("SOL token mint");

let account_meta = account.to_account_meta().unwrap();
```

## Testing Strategy

1. **Unit Tests**: Test individual components without Surfpool
2. **Integration Tests**: Use Surfpool for realistic testing with mainnet state
3. **Fixture-Based Tests**: Create reusable test cases in JSON format

## Examples

See `tests/surfpool_tests.rs` for complete examples.

## Contributing

When adding new utilities:

1. Add to appropriate module (`surfpool`, `fixtures`, `validation`, `common`)
2. Include unit tests
3. Update this README with examples
4. Consider if the utility should be exported in `prelude`

## Architecture

```
solana_test_utils/
├── src/
│   ├── surfpool/       # Surfpool lifecycle management
│   ├── fixtures/       # Fixture loading and transaction building
│   ├── validation/     # Transaction validation utilities
│   └── common/         # Shared utilities (tokens, accounts)
└── tests/
    └── surfpool_tests.rs  # Integration tests
```

## License

Same as parent project.
