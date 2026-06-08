use super::*;
use crate::core::VisualizerContext;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::{AccountMeta, CompiledInstruction, Instruction};
use solana_sdk::pubkey::Pubkey;
use spl_token::instruction as token_instruction;
use spl_token::instruction::AuthorityType;
use std::str::FromStr;
use visualsign::SignablePayloadField;

/// Flatten a single `Instruction` into the (compiled, account_keys) pair
/// expected by the post-#228 `VisualizerContext::new`. Program ID sits at
/// index 0 and each `AccountMeta` follows.
fn compile_for_test(instruction: &Instruction) -> (CompiledInstruction, Vec<Pubkey>) {
    let mut account_keys = vec![instruction.program_id];
    for meta in &instruction.accounts {
        account_keys.push(meta.pubkey);
    }
    let compiled = CompiledInstruction {
        program_id_index: 0,
        accounts: (1..=instruction.accounts.len() as u8).collect(),
        data: instruction.data.clone(),
    };
    (compiled, account_keys)
}

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
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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

            // Check expanded fields surface Account, Token Mint, and Freeze Authority with
            // the correct pubkey values.
            let expanded = preview_layout.expanded.as_ref().unwrap();
            let mint_field = expanded
                .fields
                .iter()
                .find_map(|f| match &f.signable_payload_field {
                    SignablePayloadField::TextV2 { common, text_v2 }
                        if common.label == "Token Mint" =>
                    {
                        Some(text_v2.text.clone())
                    }
                    _ => None,
                });
            assert_eq!(
                mint_field.as_deref(),
                Some(mint.to_string().as_str()),
                "Freeze Account should surface the mint pubkey"
            );
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
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

    // Visualize
    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    // Verify the result
    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            // Check title
            assert_eq!(preview_layout.title.as_ref().unwrap().text, "Thaw Account");

            // Check expanded fields surface the mint pubkey under the "Token Mint" label.
            let expanded = preview_layout.expanded.as_ref().unwrap();
            let mint_field = expanded
                .fields
                .iter()
                .find_map(|f| match &f.signable_payload_field {
                    SignablePayloadField::TextV2 { common, text_v2 }
                        if common.label == "Token Mint" =>
                    {
                        Some(text_v2.text.clone())
                    }
                    _ => None,
                });
            assert_eq!(
                mint_field.as_deref(),
                Some(mint.to_string().as_str()),
                "Thaw Account should surface the mint pubkey"
            );
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
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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
    assert!(matches!(parsed, TokenInstruction::SetAuthority { .. }));

    // Create a context
    let sender = SolanaAccount {
        account_key: current_authority.to_string(),
        signer: false,
        writable: false,
    };
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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
                if let SignablePayloadField::TextV2 { common, text_v2 } =
                    &field.signable_payload_field
                {
                    common.label == "Authority Type" && text_v2.text == "Mint Tokens"
                } else {
                    false
                }
            });
            assert!(has_authority_type, "Should have Authority Type field");

            // Should have New Authority field with the pubkey
            let has_new_authority = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } =
                    &field.signable_payload_field
                {
                    common.label == "New Authority" && text_v2.text == new_authority.to_string()
                } else {
                    false
                }
            });
            assert!(
                has_new_authority,
                "Should have New Authority field with pubkey"
            );
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
    assert!(matches!(parsed, TokenInstruction::SetAuthority { .. }));

    // Create a context
    let sender = SolanaAccount {
        account_key: current_authority.to_string(),
        signer: false,
        writable: false,
    };
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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
                if let SignablePayloadField::TextV2 { common, text_v2 } =
                    &field.signable_payload_field
                {
                    common.label == "Authority Type" && text_v2.text == "Freeze Account"
                } else {
                    false
                }
            });
            assert!(has_authority_type, "Should have Authority Type field");

            // Should have New Authority field with "None"
            let has_new_authority = expanded.fields.iter().any(|field| {
                if let SignablePayloadField::TextV2 { common, text_v2 } =
                    &field.signable_payload_field
                {
                    common.label == "New Authority" && text_v2.text == "None"
                } else {
                    false
                }
            });
            assert!(
                has_new_authority,
                "Should have New Authority field with None"
            );
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

fn run_visualization_test(
    instruction: solana_sdk::instruction::Instruction,
    expected_title_substr: &str,
    expected_expanded_labels: &[&str],
) {
    let sender = SolanaAccount {
        account_key: Pubkey::new_unique().to_string(),
        signer: false,
        writable: false,
    };
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

    let visualizer = SplTokenVisualizer;
    let result = visualizer.visualize_tx_commands(&context).unwrap();

    match result.signable_payload_field {
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            let title = &preview_layout.title.as_ref().unwrap().text;
            assert!(
                title.contains(expected_title_substr),
                "Title `{title}` should contain `{expected_title_substr}`"
            );

            let expanded = preview_layout.expanded.as_ref().unwrap();
            for label in expected_expanded_labels {
                let has_label = expanded.fields.iter().any(|field| {
                    matches!(
                        &field.signable_payload_field,
                        SignablePayloadField::TextV2 { common, .. } if common.label == *label
                    )
                });
                assert!(has_label, "Expected expanded field with label `{label}`");
            }
        }
        _ => panic!("Expected PreviewLayout"),
    }
}

#[test]
fn test_burn_visualization() {
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let instruction =
        token_instruction::burn(&spl_token::id(), &account, &mint, &owner, &[], 250).unwrap();
    run_visualization_test(instruction, "Burn", &["Program ID", "Amount", "Raw Data"]);
}

#[test]
fn test_burn_checked_visualization() {
    let account = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let instruction =
        token_instruction::burn_checked(&spl_token::id(), &account, &mint, &owner, &[], 250, 6)
            .unwrap();
    run_visualization_test(
        instruction,
        "Burn",
        &["Program ID", "Amount", "Decimals", "Raw Data"],
    );
}

#[test]
fn test_approve_visualization() {
    let source = Pubkey::new_unique();
    let delegate = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let instruction =
        token_instruction::approve(&spl_token::id(), &source, &delegate, &owner, &[], 10_000)
            .unwrap();
    run_visualization_test(
        instruction,
        "Approve",
        &["Program ID", "Amount", "Raw Data"],
    );
}

#[test]
fn test_approve_checked_visualization() {
    let source = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let delegate = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let instruction = token_instruction::approve_checked(
        &spl_token::id(),
        &source,
        &mint,
        &delegate,
        &owner,
        &[],
        10_000,
        6,
    )
    .unwrap();
    run_visualization_test(
        instruction,
        "Approve",
        &["Program ID", "Amount", "Decimals", "Raw Data"],
    );
}

#[test]
fn test_mint_to_checked_visualization() {
    let mint = Pubkey::new_unique();
    let account = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let instruction = token_instruction::mint_to_checked(
        &spl_token::id(),
        &mint,
        &account,
        &authority,
        &[],
        7_500,
        9,
    )
    .unwrap();
    run_visualization_test(
        instruction,
        "Mint To",
        &["Program ID", "Amount", "Decimals", "Raw Data"],
    );
}

#[test]
fn test_revoke_visualization() {
    // Revoke has no mint in its instruction (accounts: source, owner) — verify Source
    // and Owner surface and that no Token Mint field is rendered (the unchecked
    // variant's mint absence is reported via tracing::debug! instead).
    let source = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let instruction = token_instruction::revoke(&spl_token::id(), &source, &owner, &[]).unwrap();
    run_visualization_test(
        instruction,
        "Revoke",
        &["Program ID", "Instruction", "Source", "Owner", "Raw Data"],
    );
}

#[test]
fn test_close_account_visualization() {
    // CloseAccount has no mint in its instruction (accounts: account, destination, owner) —
    // verify Account/Destination/Owner are surfaced and no Token Mint field is rendered.
    let account = Pubkey::new_unique();
    let dest = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let instruction =
        token_instruction::close_account(&spl_token::id(), &account, &dest, &owner, &[]).unwrap();
    run_visualization_test(
        instruction,
        "Close Account",
        &[
            "Program ID",
            "Instruction",
            "Account",
            "Destination",
            "Owner",
            "Raw Data",
        ],
    );
}

#[test]
fn test_initialize_mint_visualization() {
    // InitializeMint surfaces the mint pubkey (account[0]), Decimals, Mint Authority,
    // and Freeze Authority (from instruction data).
    let mint = Pubkey::new_unique();
    let mint_authority = Pubkey::new_unique();
    let instruction =
        token_instruction::initialize_mint(&spl_token::id(), &mint, &mint_authority, None, 6)
            .unwrap();
    run_visualization_test(
        instruction,
        "Initialize Mint",
        &[
            "Program ID",
            "Instruction",
            "Decimals",
            "Mint Authority",
            "Freeze Authority",
            "Token Mint",
            "Raw Data",
        ],
    );
}

#[test]
fn test_unpack_rejects_empty_data() {
    // An empty data slice is the simplest malformed input the visualizer can
    // receive from the wire. Document that the upstream parser rejects it.
    assert!(TokenInstruction::unpack(&[]).is_err());
}

#[test]
fn test_unpack_rejects_truncated_transfer_data() {
    // Tag byte 3 = Transfer, which expects 8 amount bytes to follow. Only the
    // tag is present, so unpack must fail rather than read out-of-bounds.
    let truncated = [3u8];
    assert!(TokenInstruction::unpack(&truncated).is_err());
}

#[test]
fn test_unpack_rejects_unknown_opcode() {
    // 0xFF is well outside the defined TokenInstruction tag range.
    let unknown = [0xFFu8, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(TokenInstruction::unpack(&unknown).is_err());
}

#[test]
fn test_visualizer_renders_graceful_fallback_for_malformed_data() {
    // End-to-end guard: per the catch-all "partial rendering" contract the
    // visualizer must NEVER return Err for attacker-controlled bytes -- a
    // visualizer Err aborts the whole transaction in the non-diagnostics build.
    // Malformed input must degrade to a raw program_id + data layout.
    let instruction = Instruction {
        program_id: spl_token::id(),
        accounts: vec![],
        data: vec![],
    };
    let sender = SolanaAccount {
        account_key: Pubkey::new_unique().to_string(),
        signer: false,
        writable: false,
    };
    let (compiled, account_keys) = compile_for_test(&instruction);
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

    let field = SplTokenVisualizer
        .visualize_tx_commands(&context)
        .expect("malformed data must render a graceful fallback, not Err");

    let SignablePayloadField::PreviewLayout { preview_layout, .. } = field.signable_payload_field
    else {
        panic!("expected PreviewLayout");
    };
    let expanded = preview_layout.expanded.expect("expanded layout");
    let has_raw_data = expanded.fields.iter().any(|f| {
        matches!(
            &f.signable_payload_field,
            SignablePayloadField::TextV2 { common, .. } if common.label == "Raw Data"
        )
    });
    assert!(
        has_raw_data,
        "graceful fallback should still surface the Raw Data field"
    );
}

#[test]
fn test_v0_unresolved_accounts_render_placeholder_not_err() {
    // v0 + address-lookup-table: the TokenKeg program_id resolves (so spl_token
    // handles the instruction) but a token account index points into a lookup
    // table the parser has not resolved. This must degrade to an `unresolved(N)`
    // placeholder, never Err (which would erase the whole transaction).
    let program = spl_token::id();
    let mint = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    // account_keys holds indices 0..=2 only; index 50 below is unresolved.
    let account_keys = vec![program, mint, destination];

    // MintTo (tag 7) + 8-byte little-endian amount. Accounts: [mint, destination,
    // mintAuthority], with the authority pointing at the unresolved index 50.
    let mut data = vec![7u8];
    data.extend_from_slice(&1_000u64.to_le_bytes());
    let compiled = CompiledInstruction {
        program_id_index: 0,
        accounts: vec![1, 2, 50],
        data,
    };

    let sender = SolanaAccount {
        account_key: program.to_string(),
        signer: false,
        writable: false,
    };
    let idl_registry = crate::idl::IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

    let field = SplTokenVisualizer
        .visualize_tx_commands(&context)
        .expect("unresolved accounts must not abort the visualizer");

    let SignablePayloadField::PreviewLayout { preview_layout, .. } = field.signable_payload_field
    else {
        panic!("expected PreviewLayout");
    };
    let expanded = preview_layout.expanded.expect("expanded layout");
    let has_placeholder = expanded.fields.iter().any(|f| {
        matches!(
            &f.signable_payload_field,
            SignablePayloadField::TextV2 { text_v2, .. } if text_v2.text == "unresolved(50)"
        )
    });
    assert!(
        has_placeholder,
        "unresolved account must render an `unresolved(50)` placeholder"
    );
}

/// Load a transaction fixture and test field extraction
mod fixture_tests {
    use super::*;
    use serde_json::Value;

    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
            .unwrap_or_else(|e| panic!("Failed to read fixture {fixture_path}: {e}"));
        serde_json::from_str(&fixture_content)
            .unwrap_or_else(|e| panic!("Failed to parse fixture {fixture_path}: {e}"))
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
        let data = bs58::decode(&fixture.instruction_data).into_vec().unwrap();

        Instruction {
            program_id,
            accounts,
            data,
        }
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
            println!("Transaction Context: {note}");
        }
        println!();

        // Load the single relevant instruction — this is a UNIT test for
        // SPL Token parsing, not full-transaction integration.
        let instruction = create_instruction_from_fixture(&fixture);
        let (compiled, account_keys) = compile_for_test(&instruction);

        let sender = SolanaAccount {
            account_key: fixture.accounts.first().unwrap().pubkey.clone(),
            signer: false,
            writable: false,
        };
        let idl_registry = crate::idl::IdlRegistry::new();
        let context = VisualizerContext::new(&sender, &compiled, &account_keys, &idl_registry, 0);

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

                // Validate against expected fields. Each `expected_fields` entry must
                // be present in the rendered Expanded layout AND match its expected
                // value — collect every divergence and assert at the end so the test
                // surfaces all mismatches at once instead of bailing on the first.
                let expanded = preview_layout
                    .expanded
                    .as_ref()
                    .expect("Expected PreviewLayout to have an expanded layout");

                let mut failures: Vec<String> = Vec::new();
                for (key, expected_value) in &fixture.expected_fields {
                    let expected_str = expected_value
                        .as_str()
                        .unwrap_or_else(|| panic!("expected_fields[{key}] must be a JSON string"));

                    let mut matched_value: Option<String> = None;
                    for field in &expanded.fields {
                        let SignablePayloadField::TextV2 { common, text_v2 } =
                            &field.signable_payload_field
                        else {
                            continue;
                        };
                        if common.label.to_lowercase().replace(' ', "_") == key.to_lowercase() {
                            matched_value = Some(text_v2.text.clone());
                            break;
                        }
                    }

                    match matched_value {
                        Some(actual) if actual == expected_str => {
                            println!("✓ {key}: {expected_str} (matches)");
                        }
                        Some(actual) => {
                            failures
                                .push(format!("{key}: expected {expected_str:?}, got {actual:?}"));
                        }
                        None => failures.push(format!("{key}: field not found in expanded output")),
                    }
                }

                assert!(
                    failures.is_empty(),
                    "fixture validation failures:\n  - {}",
                    failures.join("\n  - ")
                );
            }
            _ => panic!("Expected PreviewLayout"),
        }
    }
}
