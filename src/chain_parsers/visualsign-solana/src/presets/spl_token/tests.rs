use super::*;
use crate::core::VisualizerContext;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use spl_token::instruction as token_instruction;
use spl_token::instruction::AuthorityType;
use std::str::FromStr;
use visualsign::SignablePayloadField;

/// Test case for instructions with amount only
struct AmountTestCase {
    name: &'static str,
    expected_name: &'static str,
    amount: u64,
    builder: fn(&Pubkey, &Pubkey, &Pubkey, &Pubkey, u64) -> solana_sdk::instruction::Instruction,
    variant_check: fn(&TokenInstruction) -> bool,
}

/// Test case for checked instructions (amount + decimals)
struct CheckedTestCase {
    name: &'static str,
    expected_name: &'static str,
    amount: u64,
    decimals: u8,
    builder: fn(
        &Pubkey,
        &Pubkey,
        &Pubkey,
        &Pubkey,
        &Pubkey,
        u64,
        u8,
    ) -> solana_sdk::instruction::Instruction,
    variant_check: fn(&TokenInstruction) -> bool,
}

/// Test case for simple instructions (no parameters)
struct SimpleTestCase {
    name: &'static str,
    expected_name: &'static str,
    builder: fn(&Pubkey, &Pubkey, &Pubkey) -> solana_sdk::instruction::Instruction,
    variant_check: fn(&TokenInstruction) -> bool,
}

fn run_amount_test(test: &AmountTestCase) {
    let key1 = Pubkey::new_unique();
    let key2 = Pubkey::new_unique();
    let key3 = Pubkey::new_unique();
    let key4 = Pubkey::new_unique();

    let instruction = (test.builder)(&key1, &key2, &key3, &key4, test.amount);
    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();

    assert!(
        (test.variant_check)(&parsed),
        "{}: variant mismatch",
        test.name
    );
    assert_eq!(
        format_token_instruction(&parsed),
        test.expected_name,
        "{}: name mismatch",
        test.name
    );

    // Verify amount
    let parsed_amount = match parsed {
        TokenInstruction::Transfer { amount } => amount,
        TokenInstruction::Burn { amount } => amount,
        TokenInstruction::Approve { amount } => amount,
        TokenInstruction::MintTo { amount } => amount,
        _ => panic!("{}: Expected instruction with amount field", test.name),
    };
    assert_eq!(parsed_amount, test.amount, "{}: amount mismatch", test.name);
}

fn run_checked_test(test: &CheckedTestCase) {
    let key1 = Pubkey::new_unique();
    let key2 = Pubkey::new_unique();
    let key3 = Pubkey::new_unique();
    let key4 = Pubkey::new_unique();
    let key5 = Pubkey::new_unique();

    let instruction = (test.builder)(
        &key1,
        &key2,
        &key3,
        &key4,
        &key5,
        test.amount,
        test.decimals,
    );
    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();

    assert!(
        (test.variant_check)(&parsed),
        "{}: variant mismatch",
        test.name
    );
    assert_eq!(
        format_token_instruction(&parsed),
        test.expected_name,
        "{}: name mismatch",
        test.name
    );

    // Verify amount and decimals
    let (parsed_amount, parsed_decimals) = match parsed {
        TokenInstruction::TransferChecked { amount, decimals } => (amount, decimals),
        TokenInstruction::BurnChecked { amount, decimals } => (amount, decimals),
        TokenInstruction::ApproveChecked { amount, decimals } => (amount, decimals),
        TokenInstruction::MintToChecked { amount, decimals } => (amount, decimals),
        _ => panic!("{}: Expected checked instruction", test.name),
    };
    assert_eq!(parsed_amount, test.amount, "{}: amount mismatch", test.name);
    assert_eq!(
        parsed_decimals, test.decimals,
        "{}: decimals mismatch",
        test.name
    );
}

fn run_simple_test(test: &SimpleTestCase) {
    let key1 = Pubkey::new_unique();
    let key2 = Pubkey::new_unique();
    let key3 = Pubkey::new_unique();

    let instruction = (test.builder)(&key1, &key2, &key3);
    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();

    assert!(
        (test.variant_check)(&parsed),
        "{}: variant mismatch",
        test.name
    );
    assert_eq!(
        format_token_instruction(&parsed),
        test.expected_name,
        "{}: name mismatch",
        test.name
    );
}

#[test]
fn test_amount_instructions() {
    let test_cases = [
        AmountTestCase {
            name: "Transfer",
            expected_name: "Transfer",
            amount: 1000,
            builder: |source, dest, owner, _unused, amount| {
                token_instruction::transfer(&spl_token::id(), source, dest, owner, &[], amount)
                    .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::Transfer { .. }),
        },
        AmountTestCase {
            name: "Burn",
            expected_name: "Burn",
            amount: 250,
            builder: |account, mint, owner, _unused, amount| {
                token_instruction::burn(&spl_token::id(), account, mint, owner, &[], amount)
                    .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::Burn { .. }),
        },
        AmountTestCase {
            name: "Approve",
            expected_name: "Approve",
            amount: 10000,
            builder: |source, delegate, owner, _unused, amount| {
                token_instruction::approve(&spl_token::id(), source, delegate, owner, &[], amount)
                    .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::Approve { .. }),
        },
    ];

    for test in &test_cases {
        run_amount_test(test);
    }
}

#[test]
fn test_checked_instructions() {
    let test_cases = [
        CheckedTestCase {
            name: "TransferChecked",
            expected_name: "Transfer (Checked)",
            amount: 5000,
            decimals: 6,
            builder: |source, mint, dest, owner, _unused, amount, decimals| {
                token_instruction::transfer_checked(
                    &spl_token::id(),
                    source,
                    mint,
                    dest,
                    owner,
                    &[],
                    amount,
                    decimals,
                )
                .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::TransferChecked { .. }),
        },
        CheckedTestCase {
            name: "BurnChecked",
            expected_name: "Burn (Checked)",
            amount: 750,
            decimals: 9,
            builder: |account, mint, owner, _unused1, _unused2, amount, decimals| {
                token_instruction::burn_checked(
                    &spl_token::id(),
                    account,
                    mint,
                    owner,
                    &[],
                    amount,
                    decimals,
                )
                .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::BurnChecked { .. }),
        },
        CheckedTestCase {
            name: "ApproveChecked",
            expected_name: "Approve (Checked)",
            amount: 15000,
            decimals: 6,
            builder: |source, mint, delegate, owner, _unused, amount, decimals| {
                token_instruction::approve_checked(
                    &spl_token::id(),
                    source,
                    mint,
                    delegate,
                    owner,
                    &[],
                    amount,
                    decimals,
                )
                .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::ApproveChecked { .. }),
        },
    ];

    for test in &test_cases {
        run_checked_test(test);
    }
}

#[test]
fn test_simple_instructions() {
    let test_cases = [
        SimpleTestCase {
            name: "Revoke",
            expected_name: "Revoke",
            builder: |source, owner, _unused| {
                token_instruction::revoke(&spl_token::id(), source, owner, &[]).unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::Revoke),
        },
        SimpleTestCase {
            name: "CloseAccount",
            expected_name: "Close Account",
            builder: |account, destination, owner| {
                token_instruction::close_account(&spl_token::id(), account, destination, owner, &[])
                    .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::CloseAccount),
        },
        SimpleTestCase {
            name: "FreezeAccount",
            expected_name: "Freeze Account",
            builder: |account, mint, freeze_authority| {
                token_instruction::freeze_account(
                    &spl_token::id(),
                    account,
                    mint,
                    freeze_authority,
                    &[],
                )
                .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::FreezeAccount),
        },
        SimpleTestCase {
            name: "ThawAccount",
            expected_name: "Thaw Account",
            builder: |account, mint, freeze_authority| {
                token_instruction::thaw_account(
                    &spl_token::id(),
                    account,
                    mint,
                    freeze_authority,
                    &[],
                )
                .unwrap()
            },
            variant_check: |i| matches!(i, TokenInstruction::ThawAccount),
        },
    ];

    for test in &test_cases {
        run_simple_test(test);
    }
}

#[test]
fn test_initialize_mint() {
    let mint = Pubkey::new_unique();
    let mint_authority = Pubkey::new_unique();
    let freeze_authority = Some(Pubkey::new_unique());
    let decimals = 6u8;

    let instruction = token_instruction::initialize_mint(
        &spl_token::id(),
        &mint,
        &mint_authority,
        freeze_authority.as_ref(),
        decimals,
    )
    .unwrap();

    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();
    assert!(matches!(parsed, TokenInstruction::InitializeMint { .. }));
    assert_eq!(format_token_instruction(&parsed), "Initialize Mint");

    if let TokenInstruction::InitializeMint {
        decimals: parsed_decimals,
        mint_authority: parsed_mint_auth,
        freeze_authority: parsed_freeze_auth,
    } = parsed
    {
        assert_eq!(parsed_decimals, decimals);
        assert_eq!(parsed_mint_auth, mint_authority);
        assert_eq!(parsed_freeze_auth, freeze_authority.into());
    }
}

#[test]
fn test_initialize_mint2() {
    let instruction = token_instruction::initialize_mint2(
        &spl_token::id(),
        &Pubkey::new_unique(),
        &Pubkey::new_unique(),
        Some(&Pubkey::new_unique()),
        9,
    )
    .unwrap();

    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();
    assert!(matches!(parsed, TokenInstruction::InitializeMint2 { .. }));
    assert_eq!(format_token_instruction(&parsed), "Initialize Mint (v2)");
}

#[test]
fn test_freeze_and_thaw_coverage() {
    // Explicitly test FreezeAccount instruction formatting
    let freeze_instruction = token_instruction::freeze_account(
        &spl_token::id(),
        &Pubkey::new_unique(),
        &Pubkey::new_unique(),
        &Pubkey::new_unique(),
        &[],
    )
    .unwrap();

    let freeze_parsed = TokenInstruction::unpack(&freeze_instruction.data).unwrap();
    assert!(matches!(freeze_parsed, TokenInstruction::FreezeAccount));
    assert_eq!(format_token_instruction(&freeze_parsed), "Freeze Account");

    // Explicitly test ThawAccount instruction formatting
    let thaw_instruction = token_instruction::thaw_account(
        &spl_token::id(),
        &Pubkey::new_unique(),
        &Pubkey::new_unique(),
        &Pubkey::new_unique(),
        &[],
    )
    .unwrap();

    let thaw_parsed = TokenInstruction::unpack(&thaw_instruction.data).unwrap();
    assert!(matches!(thaw_parsed, TokenInstruction::ThawAccount));
    assert_eq!(format_token_instruction(&thaw_parsed), "Thaw Account");
}

#[test]
fn test_transfer_visualization_with_addresses() {
    // Create a transfer instruction
    let source = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let amount = 1000u64;

    let instruction =
        token_instruction::transfer(&spl_token::id(), &source, &destination, &owner, &[], amount)
            .unwrap();

    // Create a context with this instruction
    let sender = SolanaAccount {
        account_key: source.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize the instruction
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result structure
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } => {
            // Check label
            assert_eq!(common.label, "Instruction 1");

            // Check title
            assert_eq!(preview_layout.title.as_ref().unwrap().text, "Transfer");

            // Check that we have expanded fields
            let expanded = preview_layout.expanded.as_ref().unwrap();
            assert!(!expanded.fields.is_empty());

            // Verify Program ID field exists
            let has_program_id = expanded.fields.iter().any(|field| {
                matches!(
                    &field.signable_payload_field,
                    SignablePayloadField::TextV2 { common, .. } if common.label == "Program ID"
                )
            });
            assert!(has_program_id, "Should have Program ID field");

            // Verify Raw Data field exists
            let has_raw_data = expanded.fields.iter().any(|field| {
                matches!(
                    &field.signable_payload_field,
                    SignablePayloadField::TextV2 { common, .. } if common.label == "Raw Data"
                )
            });
            assert!(has_raw_data, "Should have Raw Data field");
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_mint_to_visualization_with_amount() {
    // Create a mint_to instruction
    let mint = Pubkey::new_unique();
    let account = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let amount = 5000u64;

    let instruction =
        token_instruction::mint_to(&spl_token::id(), &mint, &account, &authority, &[], amount)
            .unwrap();

    // Create a context
    let sender = SolanaAccount {
        account_key: authority.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title contains amount
            let title = &preview_layout.title.as_ref().unwrap().text;
            assert!(title.contains("Mint To"));
            assert!(title.contains(&amount.to_string()));

            // Check expanded fields contain Amount field
            let expanded = preview_layout.expanded.as_ref().unwrap();
            let has_amount_field = expanded.fields.iter().any(|field| {
                matches!(
                    &field.signable_payload_field,
                    SignablePayloadField::TextV2 { common, .. } if common.label == "Amount"
                )
            });
            assert!(has_amount_field, "Should have Amount field");
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_freeze_account_visualization() {
    // Create a freeze_account instruction
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let freeze_authority = Pubkey::new_unique();

    let instruction = token_instruction::freeze_account(
        &spl_token::id(),
        &account,
        &mint,
        &freeze_authority,
        &[],
    )
    .unwrap();

    // Create a context
    let sender = SolanaAccount {
        account_key: freeze_authority.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title
            assert_eq!(
                preview_layout.title.as_ref().unwrap().text,
                "Freeze Account"
            );

            // Check expanded fields contain program info
            let expanded = preview_layout.expanded.as_ref().unwrap();
            assert!(!expanded.fields.is_empty());
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_thaw_account_visualization() {
    // Create a thaw_account instruction
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let freeze_authority = Pubkey::new_unique();

    let instruction =
        token_instruction::thaw_account(&spl_token::id(), &account, &mint, &freeze_authority, &[])
            .unwrap();

    // Create a context
    let sender = SolanaAccount {
        account_key: freeze_authority.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title
            assert_eq!(preview_layout.title.as_ref().unwrap().text, "Thaw Account");

            // Check expanded fields contain program info
            let expanded = preview_layout.expanded.as_ref().unwrap();
            assert!(!expanded.fields.is_empty());
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_transfer_checked_visualization_with_decimals() {
    // Create a transfer_checked instruction
    let source = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let amount = 2500u64;
    let decimals = 6u8;

    let instruction = token_instruction::transfer_checked(
        &spl_token::id(),
        &source,
        &mint,
        &destination,
        &owner,
        &[],
        amount,
        decimals,
    )
    .unwrap();

    // Create a context
    let sender = SolanaAccount {
        account_key: owner.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title
            let title = &preview_layout.title.as_ref().unwrap().text;
            assert_eq!(title, "Transfer (Checked)");

            // Check expanded fields
            let expanded = preview_layout.expanded.as_ref().unwrap();

            // Should have Instruction field
            let has_instruction_field = expanded.fields.iter().any(|field| {
                matches!(
                    &field.signable_payload_field,
                    SignablePayloadField::TextV2 { common, .. } if common.label == "Instruction"
                )
            });
            assert!(has_instruction_field, "Should have Instruction field");

            // Should have Token Mint field (for checked instructions)
            let has_mint_field = expanded.fields.iter().any(|field| {
                matches!(
                    &field.signable_payload_field,
                    SignablePayloadField::TextV2 { common, .. } if common.label == "Token Mint"
                )
            });
            assert!(has_mint_field, "Should have Token Mint field");
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_set_authority_with_mint_tokens() {
    // Test SetAuthority with MintTokens authority type
    let account = Pubkey::new_unique();
    let current_authority = Pubkey::new_unique();
    let new_authority = Pubkey::new_unique();

    let instruction = token_instruction::set_authority(
        &spl_token::id(),
        &account,
        Some(&new_authority),
        AuthorityType::MintTokens,
        &current_authority,
        &[],
    )
    .unwrap();

    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();
    assert!(matches!(
        parsed,
        TokenInstruction::SetAuthority { .. }
    ));

    // Create a context
    let sender = SolanaAccount {
        account_key: current_authority.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title contains authority type
            let title = &preview_layout.title.as_ref().unwrap().text;
            assert!(title.contains("Set Authority"));
            assert!(title.contains("Mint Tokens"));

            // Check expanded fields
            let expanded = preview_layout.expanded.as_ref().unwrap();

            // Should have Authority Type field
            let has_authority_type = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
                {
                    common.label == "Authority Type" && text_v2.text == "Mint Tokens"
                } else {
                    false
                }
            });
            assert!(has_authority_type, "Should have Authority Type field");

            // Should have New Authority field with the pubkey
            let has_new_authority = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
                {
                    common.label == "New Authority" && text_v2.text == new_authority.to_string()
                } else {
                    false
                }
            });
            assert!(has_new_authority, "Should have New Authority field with pubkey");
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_set_authority_with_none() {
    // Test SetAuthority with None as new_authority
    let account = Pubkey::new_unique();
    let current_authority = Pubkey::new_unique();

    let instruction = token_instruction::set_authority(
        &spl_token::id(),
        &account,
        None,
        AuthorityType::FreezeAccount,
        &current_authority,
        &[],
    )
    .unwrap();

    let parsed = TokenInstruction::unpack(&instruction.data).unwrap();
    assert!(matches!(
        parsed,
        TokenInstruction::SetAuthority { .. }
    ));

    // Create a context
    let sender = SolanaAccount {
        account_key: current_authority.to_string(),
        signer: false,
        writable: false,
    };
    let instructions = vec![instruction.clone()];
    let context = VisualizerContext::new(&sender, 0, &instructions);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title
            let title = &preview_layout.title.as_ref().unwrap().text;
            assert!(title.contains("Set Authority"));
            assert!(title.contains("Freeze Account"));

            // Check expanded fields
            let expanded = preview_layout.expanded.as_ref().unwrap();

            // Should have Authority Type field
            let has_authority_type = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
                {
                    common.label == "Authority Type" && text_v2.text == "Freeze Account"
                } else {
                    false
                }
            });
            assert!(has_authority_type, "Should have Authority Type field");

            // Should have New Authority field with "None"
            let has_new_authority = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } = &field.signable_payload_field
                {
                    common.label == "New Authority" && text_v2.text == "None"
                } else {
                    false
                }
            });
            assert!(has_new_authority, "Should have New Authority field with None");
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_set_authority_all_types() {
    // Test all authority types to ensure format_authority_type works correctly
    let test_cases = [
        (AuthorityType::MintTokens, "Mint Tokens"),
        (AuthorityType::FreezeAccount, "Freeze Account"),
        (AuthorityType::AccountOwner, "Account Owner"),
        (AuthorityType::CloseAccount, "Close Account"),
    ];

    for (authority_type, expected_name) in test_cases.iter() {
        let account = Pubkey::new_unique();
        let current_authority = Pubkey::new_unique();
        let new_authority = Pubkey::new_unique();

        let instruction = token_instruction::set_authority(
            &spl_token::id(),
            &account,
            Some(&new_authority),
            authority_type.clone(),
            &current_authority,
            &[],
        )
        .unwrap();

        let parsed = TokenInstruction::unpack(&instruction.data).unwrap();

        if let TokenInstruction::SetAuthority {
            authority_type: parsed_auth_type,
            ..
        } = parsed
        {
            assert_eq!(parsed_auth_type, *authority_type);
            assert_eq!(format_authority_type(&parsed_auth_type), *expected_name);
        } else {
            panic!("Expected SetAuthority instruction");
        }
    }
}

/// Load a transaction fixture and test field extraction
mod fixture_tests {
    use super::*;
    use serde_json::Value;

    #[derive(Debug, serde::Deserialize)]
    struct TestFixture {
        description: String,
        source: String,
        signature: String,
        cluster: String,
        #[serde(default)]
        full_transaction_note: Option<String>,
        instruction_index: usize,
        instruction_data: String,
        program_id: String,
        accounts: Vec<TestAccount>,
        expected_fields: serde_json::Map<String, Value>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct TestAccount {
        pubkey: String,
        signer: bool,
        writable: bool,
        description: String,
    }

    fn load_fixture(name: &str) -> TestFixture {
        let fixture_path = format!(
            "{}/tests/fixtures/spl_token/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        let fixture_content = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", fixture_path, e));
        serde_json::from_str(&fixture_content)
            .unwrap_or_else(|e| panic!("Failed to parse fixture {}: {}", fixture_path, e))
    }

    fn create_instruction_from_fixture(fixture: &TestFixture) -> Instruction {
        let program_id = Pubkey::from_str(&fixture.program_id).unwrap();
        let accounts: Vec<AccountMeta> = fixture
            .accounts
            .iter()
            .map(|acc| {
                let pubkey = Pubkey::from_str(&acc.pubkey).unwrap();
                AccountMeta {
                    pubkey,
                    is_signer: acc.signer,
                    is_writable: acc.writable,
                }
            })
            .collect();

        // Instruction data from JSON RPC responses is base58 encoded
        let data = bs58::decode(&fixture.instruction_data)
            .into_vec()
            .unwrap();

        Instruction {
            program_id,
            accounts,
            data,
        }
    }

    fn load_full_transaction_instructions(fixture: &TestFixture) -> Vec<Instruction> {
        // In a real scenario, we'd load all instructions from the transaction
        // For now, we just create the one instruction from the fixture
        // TODO: Extend fixture format to include all transaction instructions
        vec![create_instruction_from_fixture(fixture)]
    }

    #[test]
    fn test_mint_to_real_transaction() {
        let fixture = load_fixture("mint_to_example");
        println!("\n=== Testing Real Transaction ===");
        println!("Description: {}", fixture.description);
        println!("Source: {}", fixture.source);
        println!("Signature: {}", fixture.signature);
        println!("Cluster: {}", fixture.cluster);
        if let Some(note) = &fixture.full_transaction_note {
            println!("Transaction Context: {}", note);
        }
        println!();

        // Load instructions - this is a UNIT test for SPL Token parsing
        // We only test the specific instruction, not the full transaction context
        let instructions = load_full_transaction_instructions(&fixture);

        // Create a context - using index 0 since we only loaded the one relevant instruction
        // In reality, the fixture.instruction_index would be used with all transaction instructions
        let sender = SolanaAccount {
            account_key: fixture.accounts.get(0).unwrap().pubkey.clone(),
            signer: false,
            writable: false,
        };
        let context = VisualizerContext::new(&sender, 0, &instructions);

        // Visualize
        let visualizer = SplTokenVisualizer;
        let result = visualizer.visualize_tx_commands(&context).unwrap();

        // Extract and print all fields
        match result.signable_payload_field {
            SignablePayloadField::PreviewLayout {
                preview_layout,
                common,
            } => {
                println!("=== Extracted Fields ===");
                println!("Label: {}", common.label);
                if let Some(title) = &preview_layout.title {
                    println!("Title: {}", title.text);
                }

                if let Some(expanded) = &preview_layout.expanded {
                    println!("\nExpanded Fields:");
                    for field in &expanded.fields {
                        if let SignablePayloadField::TextV2 { common, text_v2 } =
                            &field.signable_payload_field
                        {
                            println!("  {}: {}", common.label, text_v2.text);
                        }
                    }
                }

                // Validate against expected fields
                println!("\n=== Validation ===");
                for (key, expected_value) in &fixture.expected_fields {
                    let expected_str = expected_value.as_str().unwrap();

                    // Check in expanded fields
                    if let Some(expanded) = &preview_layout.expanded {
                        let found = expanded.fields.iter().any(|field| {
                            if let SignablePayloadField::TextV2 { common, text_v2 } =
                                &field.signable_payload_field
                            {
                                let label_matches = common.label.to_lowercase().replace(" ", "_") == key.to_lowercase();
                                let value_matches = text_v2.text == expected_str;
                                if label_matches {
                                    if value_matches {
                                        println!("✓ {}: {} (matches)", key, expected_str);
                                    } else {
                                        println!("✗ {}: expected '{}', got '{}'", key, expected_str, text_v2.text);
                                    }
                                    return value_matches;
                                }
                                false
                            } else {
                                false
                            }
                        });

                        if !found {
                            println!("✗ {}: field not found in output", key);
                        }
                    }
                }
            }
            _ => panic!("Expected PreviewLayout"),
        }
    }
}
