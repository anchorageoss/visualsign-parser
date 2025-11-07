use solana_test_utils::prelude::*;
use solana_sdk::signature::{Keypair, Signer};
#[allow(deprecated)]
use solana_sdk::system_instruction;

#[tokio::test]
#[ignore] // Requires surfpool to be installed and running
async fn test_surfpool_basic_transfer() {
    // Start Surfpool
    let config = SurfpoolConfig::default();
    let surfpool = SurfpoolManager::start(config).await.unwrap();
    let client = surfpool.rpc_client();

    // Create test accounts
    let payer = Keypair::new();
    let recipient = Keypair::new();

    // Airdrop some SOL to payer
    surfpool
        .airdrop(&payer.pubkey(), 10_000_000_000)
        .await
        .unwrap();

    // Build and execute transfer transaction
    let signature = SolanaTransactionBuilder::new()
        .add_instruction(system_instruction::transfer(
            &payer.pubkey(),
            &recipient.pubkey(),
            1_000_000_000,
        ))
        .set_payer(payer)
        .execute(&client)
        .await
        .unwrap();

    println!("Transfer transaction: {}", signature);

    // Verify recipient balance
    let balance = client.get_balance(&recipient.pubkey()).unwrap();
    assert_eq!(balance, 1_000_000_000);
}

#[tokio::test]
#[ignore] // Requires surfpool to be installed
async fn test_fixture_manager() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let fixture_manager = FixtureManager::new(temp_dir.path());

    // Create a test fixture
    let fixture = SolanaTestFixture::new(
        "Test system transfer",
        "11111111111111111111111111111111", // System program
        "AgAAAAAAAAA=",                       // Base58 encoded data
    )
    .with_cluster("localnet")
    .with_expected_field("program", serde_json::json!("System"))
    .with_expected_field("instruction", serde_json::json!("Transfer"));

    // Save fixture
    fixture_manager.save("test_transfer", &fixture).unwrap();

    // Load fixture back
    let loaded: SolanaTestFixture = fixture_manager.load("test_transfer").unwrap();
    assert_eq!(loaded.description, "Test system transfer");
    assert_eq!(loaded.cluster, Some("localnet".to_string()));

    // List fixtures
    let fixtures = fixture_manager.list_fixtures().unwrap();
    assert_eq!(fixtures, vec!["test_transfer"]);
}

#[test]
fn test_transaction_validation() {
    use solana_sdk::{hash::Hash, message::Message};

    let payer = Keypair::new();
    let recipient = Keypair::new();

    let instruction = system_instruction::transfer(&payer.pubkey(), &recipient.pubkey(), 1_000_000);

    let message = Message::new(&[instruction], Some(&payer.pubkey()));
    let mut transaction = solana_sdk::transaction::Transaction::new_unsigned(message);
    transaction.sign(&[&payer], Hash::default());

    // Validate transaction using assertions
    let result = TransactionValidator::new(&transaction)
        .validate_signed()
        .validate_instruction_count(1)
        .validate_signer(&payer.pubkey())
        .validate_account_present(&recipient.pubkey())
        .complete();

    assert!(result.is_ok());
}

#[test]
fn test_transaction_assertions() {
    use solana_sdk::{hash::Hash, message::Message};

    let payer = Keypair::new();
    let recipient = Keypair::new();

    let instruction = system_instruction::transfer(&payer.pubkey(), &recipient.pubkey(), 1_000_000);

    let message = Message::new(&[instruction], Some(&payer.pubkey()));
    let mut transaction = solana_sdk::transaction::Transaction::new_unsigned(message);
    transaction.sign(&[&payer], Hash::default());

    // Use trait-based assertions
    transaction
        .assert_signed()
        .assert_instruction_count(1)
        .assert_signer(&payer.pubkey())
        .assert_account_present(&recipient.pubkey());
}

#[test]
fn test_known_tokens() {
    use solana_test_utils::common::known_mints;

    assert_eq!(
        known_mints::sol().to_string(),
        "So11111111111111111111111111111111111111112"
    );

    assert_eq!(
        known_mints::usdc().to_string(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    );
}

#[test]
fn test_format_token_amount() {
    use solana_test_utils::common::format_token_amount;

    assert_eq!(format_token_amount(1_000_000, 6), "1");
    assert_eq!(format_token_amount(1_500_000, 6), "1.5");
    assert_eq!(format_token_amount(1_234_567, 6), "1.234567");
}
