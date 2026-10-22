use super::*;

use super::config::DEPLOYMENTS;
use crate::SuiVisualSignConverter;
use crate::core::SuiTransactionWrapper;
use crate::types::{SuiTransactionBlockData, SuiTransactionBlockKind};
use crate::intermediate::SuiIntermediateOutput;
use crate::utils::run_aggregated_fixture;

use base64::Engine;
use sui_types::base_types::ObjectID;
use sui_types::transaction::{
    Argument, CallArg, Command, ProgrammableMoveCall, ProgrammableTransaction, TransactionData,
    TransactionDataAPI, TransactionKind,
};
use visualsign::test_utils::check_signable_payload_field;
use visualsign::vsptrait::{VisualSignConverter, VisualSignOptions};

const FIXTURE: &str = include_str!("aggregated_test_data.json");
const REJECTED: &str = include_str!("rejected_test_data.json");

const OTHER_ADDRESS: [u8; 32] = [0x77; 32];

// Real mainnet open-deposit-and-borrow PTB: 0 view::quote, 1 init_position,
// 2 option::some(NR(1,1)), 3 balance::redeem_funds<SUI>(I12), 4 balance::zero<USDC>,
// 5 structs::positive(I3 col), 6 structs::positive(I4 debt), 7 operate(.., R5, R6, ..),
// 8 option::destroy_some(NR(7,2)), 9 balance::send_funds<SUI>(NR(7,0), I9),
// 10 balance::send_funds<USDC>(NR(7,1), I10), 11 position::share(NR(1,0)),
// 12 TransferObjects([R8], I11).
const OPEN_BORROW_DIGEST: &str = "2avMUdAzFBRMBD4WisprLqj2pNs5XgbVBuMmUTRWV8Q8";
const ZERO_USDC_COMMAND: usize = 4;
const COL_AMOUNT_COMMAND: usize = 5;
const DEBT_AMOUNT_COMMAND: usize = 6;
const OPERATE_COMMAND: usize = 7;
const DESTROY_SOME_COMMAND: usize = 8;
const DEBT_SEND_COMMAND: usize = 10;
const CAP_TRANSFER_COMMAND: usize = 12;
const DEBT_AMOUNT_INPUT: u16 = 4;
const DEBT_RECIPIENT_INPUT: u16 = 10;
const CAP_RECIPIENT_INPUT: usize = 11;

// Real mainnet repayment funded from owned coins, with no cap passed:
// .. 10 operate, 11 option::destroy_none(NR(10,2)), 12 balance::join<SUI>(R0, NR(10,0)),
// 13 coin::from_balance<SUI>(R0), 14 TransferObjects([R13], I9),
// 15 coin::from_balance<USDC>(NR(10,1)), 16 TransferObjects([R15], I10).
const REPAY_DIGEST: &str = "22yRPaDKGhjaQRbpTYXJifK2pacFav1znMZ98ZpVrj8K";
const REPAY_OPERATE_COMMAND: usize = 10;
const REPAY_FROM_BALANCE_COMMAND: usize = 13;
const REPAY_SUI_RECIPIENT_INPUT: usize = 9;

fn decode(data: &str) -> TransactionData {
    bcs::from_bytes(
        &base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
    )
    .unwrap()
}

// Historical types remain test-only builders. Decode their real/tampered BCS
// through the SDK before exercising the production view and converter.
fn sdk_transaction(tx: &TransactionData) -> sui_sdk_types::Transaction {
    bcs::from_bytes(&bcs::to_bytes(tx).unwrap()).unwrap()
}

fn fixture_tx(digest: &str) -> TransactionData {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    decode(
        fixture["vaults"]["operate"]["operations"][digest]["data"]
            .as_str()
            .unwrap(),
    )
}

fn open_borrow_tx() -> TransactionData {
    fixture_tx(OPEN_BORROW_DIGEST)
}

fn programmable(tx: &mut TransactionData) -> &mut ProgrammableTransaction {
    match tx.kind_mut() {
        TransactionKind::ProgrammableTransaction(pt) => pt,
        _ => panic!("expected a programmable transaction"),
    }
}

fn move_call(tx: &mut TransactionData, index: usize) -> &mut ProgrammableMoveCall {
    match &mut programmable(tx).commands[index] {
        Command::MoveCall(call) => call,
        other => panic!("expected a move call at command {index}, got {other:?}"),
    }
}

fn tamper_call(args: Vec<Argument>) -> Command {
    Command::move_call(
        ObjectID::from_single_byte(0x42),
        "tamper".parse().unwrap(),
        "tamper".parse().unwrap(),
        vec![],
        args,
    )
}

/// Inserts `command` at `index`, shifting later commands and every `Result`
/// reference to them so the rest of the PTB keeps pointing where it did.
fn insert_command(tx: &mut TransactionData, index: usize, command: Command) {
    let pt = programmable(tx);
    let shift = |argument: &mut Argument| match argument {
        Argument::Result(target) | Argument::NestedResult(target, _)
            if usize::from(*target) >= index =>
        {
            *target += 1;
        }
        _ => {}
    };
    for existing in &mut pt.commands {
        match existing {
            Command::MoveCall(call) => call.arguments.iter_mut().for_each(shift),
            Command::TransferObjects(objects, recipient) => {
                objects.iter_mut().for_each(shift);
                shift(recipient);
            }
            Command::SplitCoins(coin, amounts) | Command::MergeCoins(coin, amounts) => {
                shift(coin);
                amounts.iter_mut().for_each(shift);
            }
            Command::MakeMoveVec(_, elements) => elements.iter_mut().for_each(shift),
            Command::Upgrade(_, _, _, ticket) => shift(ticket),
            Command::Publish(_, _) => {}
        }
    }
    pt.commands.insert(index, command);
}

fn set_pure_input(tx: &mut TransactionData, index: usize, bytes: Vec<u8>) {
    programmable(tx).inputs[index] = CallArg::Pure(bytes);
}

fn set_amount(tx: &mut TransactionData, command: usize, function: &str) {
    let call = move_call(tx, command);
    call.function = function.parse().unwrap();
    if function == "max" || function == "none" {
        call.arguments.clear();
    }
}

fn visualize(
    tx: &TransactionData,
    command_index: usize,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let transaction = sdk_transaction(tx);
    let block_data = SuiTransactionBlockData::try_from(&transaction).unwrap();
    let SuiTransactionBlockKind::ProgrammableTransaction(pt) = block_data.transaction() else {
        panic!("expected a programmable transaction");
    };
    let context =
        VisualizerContext::new(block_data.sender(), command_index, &pt.commands, &pt.inputs);
    assert!(FluidVisualizer.can_handle(&context));
    FluidVisualizer.visualize_tx_commands(&context)
}

fn card(tx: &TransactionData, command_index: usize) -> AnnotatedPayloadField {
    visualize(tx, command_index).unwrap().remove(0)
}

fn preview_layout(field: &AnnotatedPayloadField) -> &SignablePayloadFieldPreviewLayout {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    preview_layout
}

fn card_title(field: &AnnotatedPayloadField) -> String {
    preview_layout(field).title.as_ref().unwrap().text.clone()
}

fn condensed_labels(field: &AnnotatedPayloadField) -> Vec<String> {
    preview_layout(field)
        .condensed
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|field| field.signable_payload_field.label().clone())
        .collect()
}

/// The value of the `label` row, which the expanded list repeats.
fn field_value(field: &AnnotatedPayloadField, label: &str) -> String {
    let (found, values) = check_signable_payload_field(&field.signable_payload_field, label);
    assert!(found, "no {label:?} field rendered");
    assert!(
        values.windows(2).all(|pair| pair[0] == pair[1]),
        "{label:?} rows disagree: {values:?}"
    );
    values[0].clone()
}

fn address_name(field: &AnnotatedPayloadField, label: &str) -> String {
    preview_layout(field)
        .condensed
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .find_map(|field| match &field.signable_payload_field {
            SignablePayloadField::AddressV2 { common, address_v2 } if common.label == label => {
                Some(address_v2.name.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {label:?} address row"))
}

fn other_address() -> String {
    SuiAddress::from_bytes(OTHER_ADDRESS).unwrap().to_string()
}

fn expect_error(result: Result<Vec<AnnotatedPayloadField>, VisualSignError>, expected: &str) {
    match result {
        Err(error) => assert!(
            error.to_string().ends_with(expected),
            "expected error ending with {expected:?}, got {error}"
        ),
        Ok(fields) => {
            panic!("expected an error ending with {expected:?}, got a preview: {fields:?}")
        }
    }
}

#[test]
fn test_fluid_aggregated() {
    run_aggregated_fixture(FIXTURE, Box::new(FluidVisualizer));
}

#[test]
fn fixture_titles_name_each_change() {
    for (digest, command, title) in [
        (
            OPEN_BORROW_DIGEST,
            OPERATE_COMMAND,
            "Deposit 0.9 SUI collateral and borrow 0.3555 USDC on Fluid",
        ),
        (
            "9EbDd7nTDteLJ8CKEsmNkdY6EcAtvr9xP5pVs6z2hY2U",
            6,
            "Borrow 0.1 USDC on Fluid",
        ),
        (REPAY_DIGEST, REPAY_OPERATE_COMMAND, "Repay 0.05 USDC on Fluid"),
        (
            "BfLC6C9fdLPfwpUNNBD4u9RWsniPM4K4hKREg9t8yU3y",
            8,
            "Repay all USDC debt on Fluid",
        ),
        (
            "GAd991E2QHe1efuU44vbXo4uoSbmEN5WhKoQ63dcpxcm",
            8,
            "Withdraw 0.1 SUI collateral on Fluid",
        ),
        (
            "6PxtgEk85iArTFWgYrfugS7UuFQA6uM5CqTdv9nFw6Nh",
            8,
            "Withdraw all SUI collateral and repay all USDC debt on Fluid",
        ),
    ] {
        assert_eq!(card_title(&card(&fixture_tx(digest), command)), title);
    }
}

#[test]
fn deposit_without_a_cap_shows_no_position_owner() {
    let field = card(&fixture_tx("EJk1pMgkJpGHTjdAU2Nx8tcKo1JBp6tHDB6x3roQBC4V"), 7);
    assert_eq!(
        condensed_labels(&field),
        vec!["Collateral Deposit", "Position", "Network"]
    );
}

#[test]
fn every_fixture_emits_intermediate_output_listing_each_command() {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let operations = fixture["vaults"]["operate"]["operations"]
        .as_object()
        .unwrap();
    for digest in operations.keys() {
        let tx = fixture_tx(digest);
        let TransactionKind::ProgrammableTransaction(pt) = tx.kind() else {
            panic!("expected a programmable transaction");
        };
        let commands = pt.commands.len();
        let options = VisualSignOptions {
            include_intermediate_output: true,
            ..VisualSignOptions::default()
        };
        let bytes = SuiVisualSignConverter
            .to_visual_sign_payload(SuiTransactionWrapper::new(sdk_transaction(&tx)), options)
            .unwrap()
            .intermediate_output
            .unwrap_or_else(|| panic!("no intermediate output for {digest}"));
        let output: SuiIntermediateOutput = borsh::from_slice(&bytes).unwrap();
        assert_eq!(output.commands.len(), commands, "{digest}");
        assert!(
            output
                .commands
                .iter()
                .any(|command| command.module_name == "user" && command.function == "operate"),
            "{digest} lists no user::operate call"
        );
    }
    assert_eq!(operations.len(), 10);
}

#[test]
fn every_deployment_package_is_handled_in_the_form_it_is_called_with() {
    let config = Config::new();
    for deployment in DEPLOYMENTS {
        let id = crate::types::ObjectID::from_hex_literal(deployment.vaults_package).unwrap();
        assert_eq!(id.to_hex_literal(), deployment.vaults_package);
        assert!(config.can_handle(&id.to_hex_literal(), "user", "operate"));
        for function in ["init_position", "close_position", "liquidate"] {
            assert!(!config.can_handle(&id.to_hex_literal(), "user", function));
        }
        assert!(deployment_for(&id).is_some());
    }
}

#[test]
fn leverage_through_a_flash_loan_fails() {
    let rejected: serde_json::Value = serde_json::from_str(REJECTED).unwrap();
    let leverage = &rejected["leverage"];
    expect_error(
        visualize(
            &decode(leverage["data"].as_str().unwrap()),
            usize::try_from(leverage["command_index"].as_u64().unwrap()).unwrap(),
        ),
        "Argument `col_amount` is not a transaction input",
    );
}

#[test]
fn repayment_is_a_negative_debt_amount() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, COL_AMOUNT_COMMAND, "none");
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "negative");

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(card_title(&field), "Repay 0.3555 USDC on Fluid");
    assert_eq!(field_value(&field, "Repayment"), "0.3555");
    assert_eq!(
        condensed_labels(&field),
        vec!["Repayment", "Position", "Position Owner", "Network"]
    );
}

#[test]
fn zero_amounts_build_none() {
    let mut tx = open_borrow_tx();
    let zero = bcs::to_bytes(&0_u64).unwrap();
    set_pure_input(&mut tx, usize::from(DEBT_AMOUNT_INPUT), zero);

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(card_title(&field), "Deposit 0.9 SUI collateral on Fluid");
}

#[test]
fn operate_without_any_change_fails() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, COL_AMOUNT_COMMAND, "none");
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "none");

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        "Fluid operate with neither a collateral nor a debt change aborts",
    );
}

#[test]
fn unused_coin_sent_to_someone_else_is_shown() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "none");
    set_pure_input(
        &mut tx,
        usize::from(DEBT_RECIPIENT_INPUT),
        OTHER_ADDRESS.to_vec(),
    );

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(field_value(&field, "USDC Recipient"), other_address());
    assert_eq!(
        address_name(&field, "USDC Recipient"),
        "Not the signer's address"
    );
}

#[test]
fn borrowed_coin_and_position_owner_name_their_recipients() {
    let mut tx = open_borrow_tx();
    set_pure_input(
        &mut tx,
        usize::from(DEBT_RECIPIENT_INPUT),
        OTHER_ADDRESS.to_vec(),
    );
    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(field_value(&field, "USDC Recipient"), other_address());
    assert_eq!(field_value(&field, "Position Owner"), other_address());
    assert_eq!(
        address_name(&field, "Position Owner"),
        "Not the signer's address"
    );
}

#[test]
fn coin_joined_into_another_balance_follows_that_balance() {
    let mut tx = fixture_tx(REPAY_DIGEST);
    set_pure_input(&mut tx, REPAY_SUI_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, REPAY_OPERATE_COMMAND);
    assert_eq!(field_value(&field, "SUI Recipient"), other_address());
    assert_eq!(
        address_name(&field, "SUI Recipient"),
        "Not the signer's address"
    );
}

#[test]
fn joined_balance_used_again_after_it_is_sent_fails() {
    let mut tx = fixture_tx(REPAY_DIGEST);
    insert_command(
        &mut tx,
        REPAY_FROM_BALANCE_COMMAND + 1,
        tamper_call(vec![Argument::Result(0)]),
    );

    expect_error(
        visualize(&tx, REPAY_OPERATE_COMMAND),
        &format!(
            "The collateral balance from command {REPAY_OPERATE_COMMAND} must be used by exactly one later command"
        ),
    );
}

#[test]
fn unknown_coins_are_shown_in_base_units_with_their_type() {
    let mut tx = open_borrow_tx();
    let other = sui_types::parse_sui_type_tag("0x42::usdc::USDC").unwrap();
    for index in [ZERO_USDC_COMMAND, OPERATE_COMMAND, DEBT_SEND_COMMAND] {
        let call = move_call(&mut tx, index);
        let last = call.type_arguments.len() - 1;
        call.type_arguments[last] = other.clone().into();
    }

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        card_title(&field),
        "Deposit 0.9 SUI collateral and borrow 355500 base units of 0x42::usdc::USDC on Fluid"
    );
}

#[test]
fn borrowed_coin_not_sent_to_an_address_fails() {
    let mut tx = open_borrow_tx();
    programmable(&mut tx).commands[DEBT_SEND_COMMAND] = tamper_call(vec![
        Argument::NestedResult(7, 1),
        Argument::Input(DEBT_RECIPIENT_INPUT),
    ]);

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        &format!(
            "The debt balance from command {OPERATE_COMMAND} is not sent to an address through 0x2::balance or 0x2::coin"
        ),
    );
}

#[test]
fn borrowed_coin_used_twice_fails() {
    let mut tx = open_borrow_tx();
    insert_command(
        &mut tx,
        DEBT_SEND_COMMAND + 1,
        tamper_call(vec![Argument::NestedResult(7, 1)]),
    );

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        &format!(
            "The debt balance from command {OPERATE_COMMAND} must be used by exactly one later command"
        ),
    );
}

#[test]
fn borrowed_coin_sent_as_another_coin_type_fails() {
    let mut tx = open_borrow_tx();
    move_call(&mut tx, DEBT_SEND_COMMAND).type_arguments[0] =
        sui_types::parse_sui_type_tag("0x2::sui::SUI").unwrap().into();

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        &format!(
            "The debt balance from command {OPERATE_COMMAND} is not sent to an address through 0x2::balance or 0x2::coin"
        ),
    );
}

#[test]
fn amount_built_outside_fluid_fails() {
    let mut tx = open_borrow_tx();
    move_call(&mut tx, DEBT_AMOUNT_COMMAND).package = ObjectID::from_single_byte(0x42);

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        "`debt_amount` is not built by Fluid's structs::{none, positive, negative, max}",
    );
}

#[test]
fn amount_input_modified_before_its_constructor_fails() {
    let mut tx = open_borrow_tx();
    insert_command(
        &mut tx,
        DEBT_AMOUNT_COMMAND,
        tamper_call(vec![Argument::Input(DEBT_AMOUNT_INPUT)]),
    );

    expect_error(
        visualize(&tx, OPERATE_COMMAND + 1),
        &format!(
            "`debt_amount` is passed to command {DEBT_AMOUNT_COMMAND}, which may modify it before command {} consumes it",
            DEBT_AMOUNT_COMMAND + 1
        ),
    );
}

#[test]
fn recipient_modified_before_it_is_paid_fails() {
    let mut tx = open_borrow_tx();
    insert_command(
        &mut tx,
        DESTROY_SOME_COMMAND,
        tamper_call(vec![Argument::Input(DEBT_RECIPIENT_INPUT)]),
    );

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        &format!(
            "`recipient` is passed to command {DESTROY_SOME_COMMAND}, which may modify it before command {} consumes it",
            DEBT_SEND_COMMAND + 1
        ),
    );
}

#[test]
fn cap_not_transferred_fails() {
    let mut tx = open_borrow_tx();
    programmable(&mut tx).commands[CAP_TRANSFER_COMMAND] = tamper_call(vec![Argument::Result(8)]);

    expect_error(
        visualize(&tx, OPERATE_COMMAND),
        "The position owner cap is not returned through 0x1::option::destroy_some and TransferObjects, or 0x1::option::destroy_none",
    );
}

#[test]
fn format_units_trims_trailing_zeros() {
    assert_eq!(format_units(900_000_000, 9), "0.9");
    assert_eq!(format_units(355_500, 6), "0.3555");
    assert_eq!(format_units(3_000_000_000, 9), "3");
    assert_eq!(format_units(1, 9), "0.000000001");
    assert_eq!(format_units(0, 8), "0");
}

#[test]
fn type_matching_preserves_canonical_coin_identity() {
    let padded_sui = format!("0x{}2::sui::SUI", "0".repeat(63));
    for (actual, expected) in [
        ("0x2::sui::SUI", padded_sui.as_str()),
        ("vector<0x2::sui::SUI>", "vector<0x0002::sui::SUI>"),
        ("u64", "u64"),
    ] {
        assert!(type_matches(actual, expected));
        assert_eq!(
            sui_types::parse_sui_type_tag(actual).unwrap(),
            sui_types::parse_sui_type_tag(expected).unwrap(),
        );
    }
    for (actual, expected) in [
        ("0x2::sui::SUI", "0x42::sui::SUI"),
        ("0x2::sui::SUI", "0x2::sui::OTHER"),
        ("vector<u64>", "vector<u8>"),
        ("not a type", "not a type"),
    ] {
        assert!(!type_matches(actual, expected));
    }
}
