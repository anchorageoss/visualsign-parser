# Jupiter Swap Testing with Surfpool

This example shows how to use `solana_test_utils` to test Jupiter swap parsing with Surfpool.

## Basic Setup

```rust
use solana_test_utils::prelude::*;
use visualsign_solana::presets::jupiter_swap::JupiterSwapVisualizer;

#[tokio::test]
#[ignore] // Requires surfpool installation
async fn test_jupiter_with_surfpool() {
    // 1. Start Surfpool with mainnet fork
    let config = SurfpoolConfig::default()
        .with_fork_url("https://api.mainnet-beta.solana.com");

    let surfpool = SurfpoolManager::start(config).await.unwrap();
    let client = surfpool.rpc_client();

    // 2. Load a test fixture
    let fixture_manager = FixtureManager::new("tests/fixtures/jupiter_swap");
    let fixture: SolanaTestFixture = fixture_manager
        .load("sample_route")
        .unwrap();

    // 3. Build instruction from fixture
    let instruction_data = fixture.decode_instruction_data().unwrap();
    let program_id = Pubkey::from_str(&fixture.program_id).unwrap();
    let accounts = fixture.accounts.iter()
        .map(|a| a.to_account_meta())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    // 4. Create and execute transaction
    let payer = Keypair::new();
    surfpool.airdrop(&payer.pubkey(), 10_000_000_000).await.unwrap();

    let signature = SolanaTransactionBuilder::new()
        .add_program_instruction(program_id, instruction_data, accounts)
        .set_payer(payer)
        .execute(&client)
        .await
        .unwrap();

    println!("Transaction executed: {}", signature);

    // 5. Validate transaction
    let transaction = client.get_transaction(&signature, UiTransactionEncoding::Json).unwrap();
    // Parse and validate with Jupiter visualizer
}
```

## Generating Fixtures from Surfpool

```rust
use solana_test_utils::prelude::*;

async fn generate_jupiter_fixture() -> Result<()> {
    // Start Surfpool
    let surfpool = SurfpoolManager::start(SurfpoolConfig::default()).await?;

    // Execute a Jupiter swap transaction
    let tx = execute_jupiter_swap(&surfpool).await?;

    // Extract instruction data
    let instruction = &tx.message.instructions[0];
    let instruction_data = bs58::encode(&instruction.data).into_string();

    // Create fixture
    let fixture = SolanaTestFixture::new(
        "Jupiter SOL to USDC swap",
        "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
        instruction_data,
    )
    .with_cluster("mainnet-beta")
    .with_accounts(
        instruction.accounts.iter()
            .map(|meta| TestAccount::from(meta))
            .collect()
    )
    .with_expected_field("input_token", "So11111111111111111111111111111111111111112")
    .with_expected_field("output_token", "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

    // Save fixture
    let fixture_manager = FixtureManager::new("tests/fixtures");
    fixture_manager.save("jupiter_sol_usdc", &fixture)?;

    Ok(())
}
```

## Validating Parser Output

```rust
use solana_test_utils::prelude::*;
use solana_test_utils::validation::validate_instruction_fields;

#[tokio::test]
async fn test_jupiter_parser_with_fixture() {
    let fixture_manager = FixtureManager::new("tests/fixtures");
    let fixture: SolanaTestFixture = fixture_manager.load("sample_route").unwrap();

    // Parse instruction using Jupiter visualizer
    let parsed_fields = parse_jupiter_instruction(&fixture).unwrap();

    // Validate against expected fields
    validate_instruction_fields(&parsed_fields, &fixture.expected_fields).unwrap();
}
```

## Testing Different Route Types

```rust
#[tokio::test]
async fn test_different_jupiter_routes() {
    let surfpool = SurfpoolManager::start(SurfpoolConfig::default()).await.unwrap();

    let test_cases = vec![
        ("Route", "basic_route"),
        ("ExactOutRoute", "exact_out_route"),
        ("SharedAccountsRoute", "shared_accounts_route"),
    ];

    for (route_type, fixture_name) in test_cases {
        println!("Testing {}", route_type);

        let fixture_manager = FixtureManager::new("tests/fixtures/jupiter_swap");
        let fixture: SolanaTestFixture = fixture_manager.load(fixture_name).unwrap();

        // Test with real mainnet state via Surfpool
        // ... execute and validate
    }
}
```

## Benefits of This Approach

1. **Real Mainnet State**: Surfpool provides actual mainnet account data
2. **No Manual Setup**: Accounts are fetched on-demand
3. **Reproducible**: Fixtures ensure consistent test behavior
4. **Type-Safe**: Rust's type system catches errors early
5. **Reusable**: Same framework works for all Solana programs

## Installation

1. Install Surfpool:
   ```bash
   cargo install surfpool
   ```

2. Add dependency to your test:
   ```toml
   [dev-dependencies]
   solana_test_utils = { path = "../solana_test_utils" }
   ```

3. Run tests:
   ```bash
   cargo test --test jupiter_infra -- --ignored
   ```
