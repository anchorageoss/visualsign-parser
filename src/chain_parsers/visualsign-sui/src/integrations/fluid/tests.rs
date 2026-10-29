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
const PREVIOUS_DEPLOYMENT: &str = include_str!("previous_deployment_test_data.json");
const EDGE_CASES: &str = include_str!("edge_case_test_data.json");

// Real transactions against Fluid's previous vaults package, whose `operate` takes the
// same arguments; `repoint` sends them to the current package.
const PREVIOUS_VAULTS_PACKAGE: &str =
    "0xf0f5a9c105384d5b79e212a9c9d3adfa5d030ef4c0cf9e22fa48674f9ad3652a";

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
const COL_RECIPIENT_INPUT: usize = 9;
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

// Real mainnet deposit paid from a gas-coin split: 1 coin::into_balance, 4 balance::split(R1)
// funds operate 9, then 11 balance::join(R1, NR(9,0)), 14 coin::from_balance(R1),
// 15 TransferObjects([R14], I13).
const REMAINDER_DEPOSIT_DIGEST: &str = "D8Kp7gyZfJuTD2CHBZohYeuk2Q14wKFB4ufYGWw3gnwd";
const REMAINDER_OPERATE_COMMAND: usize = 9;
const REMAINDER_RECIPIENT_INPUT: usize = 13;

// Real mainnet transaction with two operates (9 and 21) whose caps go out together in
// 26 TransferObjects([R10, R22], I12).
const TWO_OPERATES_DIGEST: &str = "G45Hd39QdH44PBNNPeKZu1J7DQf7syDcAsDmYWLynHEJ";
const TWO_OPERATES_CAP_RECIPIENT_INPUT: usize = 12;

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

fn repoint(mut tx: TransactionData) -> TransactionData {
    let previous = ObjectID::from_hex_literal(PREVIOUS_VAULTS_PACKAGE).unwrap();
    let current = ObjectID::from_hex_literal(DEPLOYMENTS[0].vaults_package).unwrap();
    for command in &mut programmable(&mut tx).commands {
        if let Command::MoveCall(call) = command {
            if call.package == previous {
                call.package = current;
            }
            for type_argument in &mut call.type_arguments {
                let tag = type_argument
                    .to_type_tag()
                    .unwrap()
                    .to_canonical_string(true)
                    .replace(&previous.to_hex(), &current.to_hex());
                *type_argument = sui_types::parse_sui_type_tag(&tag).unwrap().into();
            }
        }
    }
    tx
}

fn operations(fixture: &str) -> serde_json::Map<String, serde_json::Value> {
    let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    fixture["vaults"]["operate"]["operations"]
        .as_object()
        .unwrap()
        .clone()
}

fn fixture_tx(digest: &str) -> TransactionData {
    let operation = operations(FIXTURE)
        .remove(digest)
        .or_else(|| operations(PREVIOUS_DEPLOYMENT).remove(digest))
        .unwrap();
    repoint(decode(operation["data"].as_str().unwrap()))
}

/// The previous deployment's fixture, pointed at the current package.
fn repointed_previous_deployment() -> String {
    let mut fixture: serde_json::Value = serde_json::from_str(PREVIOUS_DEPLOYMENT).unwrap();
    let operations = fixture["vaults"]["operate"]["operations"]
        .as_object_mut()
        .unwrap();
    for operation in operations.values_mut() {
        let tx = repoint(decode(operation["data"].as_str().unwrap()));
        operation["data"] = base64::engine::general_purpose::STANDARD
            .encode(bcs::to_bytes(&tx).unwrap())
            .into();
        operation["asserts"]["Vaults Package"] = DEPLOYMENTS[0].vaults_package.into();
    }
    fixture.to_string()
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

fn push_pure_input(tx: &mut TransactionData, bytes: Vec<u8>) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(CallArg::Pure(bytes));
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn framework_call(module: &str, function: &str, type_arg: &str, args: Vec<Argument>) -> Command {
    Command::move_call(
        ObjectID::from_single_byte(2),
        module.parse().unwrap(),
        function.parse().unwrap(),
        vec![sui_types::parse_sui_type_tag(type_arg).unwrap()],
        args,
    )
}

/// Inserts `SplitCoins(GasCoin, [amount])` at `index` and returns the new coin.
fn insert_gas_split(tx: &mut TransactionData, index: usize, amount: u64) -> Argument {
    let amount = push_pure_input(tx, bcs::to_bytes(&amount).unwrap());
    insert_command(tx, index, Command::SplitCoins(Argument::GasCoin, vec![amount]));
    Argument::NestedResult(u16::try_from(index).unwrap(), 0)
}

fn result(index: usize) -> Argument {
    Argument::Result(u16::try_from(index).unwrap())
}

const SUI: &str = "0x2::sui::SUI";
const ONE_SUI: u64 = 1_000_000_000;

/// After the real open-and-borrow's operate (command 7): a 1 SUI gas split with 5 SUI
/// merged in, as a balance at the returned command. Five commands are inserted from 8.
fn insert_one_plus_five_sui_balance(tx: &mut TransactionData) -> usize {
    let one = insert_gas_split(tx, 8, ONE_SUI);
    let five = insert_gas_split(tx, 9, 5 * ONE_SUI);
    insert_command(tx, 10, Command::MergeCoins(one, vec![five]));
    insert_command(tx, 11, framework_call("coin", "into_balance", SUI, vec![one]));
    11
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
fn test_fluid_previous_deployment_aggregated() {
    run_aggregated_fixture(&repointed_previous_deployment(), Box::new(FluidVisualizer));
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
    let digests: Vec<String> = [FIXTURE, PREVIOUS_DEPLOYMENT]
        .into_iter()
        .flat_map(|fixture| operations(fixture).keys().cloned().collect::<Vec<_>>())
        .collect();
    for digest in &digests {
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
    assert_eq!(digests.len(), 14);
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

fn visualize_edge_case(name: &str) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let edge_cases: serde_json::Value = serde_json::from_str(EDGE_CASES).unwrap();
    let entry = &edge_cases[name];
    visualize(
        &repoint(decode(entry["data"].as_str().unwrap())),
        usize::try_from(entry["command_index"].as_u64().unwrap()).unwrap(),
    )
}

#[test]
fn leverage_through_a_flash_loan_fails() {
    expect_error(
        visualize_edge_case("leverage"),
        "Argument `col_amount` is not a transaction input",
    );
}

#[test]
fn other_funds_joined_into_an_output_sent_elsewhere_are_shown() {
    let field = visualize_edge_case("mixed_join").unwrap().remove(0);
    assert_eq!(card_title(&field), "Repay 0.05 USDC on Fluid");
    assert_eq!(field_value(&field, "SUI Recipient"), other_address());
    assert_eq!(
        field_value(&field, "SUI Recipient Also Receives"),
        "5 SUI split from the gas coin"
    );
}

#[test]
fn payment_remainder_joined_into_an_output_sent_elsewhere_is_shown() {
    let mut tx = fixture_tx(REMAINDER_DEPOSIT_DIGEST);
    set_pure_input(&mut tx, REMAINDER_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, REMAINDER_OPERATE_COMMAND);
    assert_eq!(
        field_value(&field, "SUI Recipient Also Receives"),
        "what remains of 0.1 SUI split from the gas coin"
    );
}

#[test]
fn payment_remainder_returned_to_the_signer_adds_no_row() {
    let field = card(&fixture_tx(REMAINDER_DEPOSIT_DIGEST), REMAINDER_OPERATE_COMMAND);
    assert!(
        condensed_labels(&field)
            .iter()
            .all(|label| !label.ends_with("Also Receives"))
    );
}

#[test]
fn funds_joined_into_an_output_are_shown() {
    let mut tx = open_borrow_tx();
    let amount = push_pure_input(&mut tx, bcs::to_bytes(&2_000_000_000_u64).unwrap());
    let split = DESTROY_SOME_COMMAND;
    insert_command(
        &mut tx,
        split,
        Command::SplitCoins(Argument::GasCoin, vec![amount]),
    );
    insert_command(
        &mut tx,
        split + 1,
        framework_call("coin", "into_balance", "0x2::sui::SUI", vec![Argument::NestedResult(
            u16::try_from(split).unwrap(),
            0,
        )]),
    );
    insert_command(
        &mut tx,
        split + 2,
        framework_call("balance", "join", "0x2::sui::SUI", vec![
            Argument::NestedResult(7, 0),
            Argument::Result(u16::try_from(split + 1).unwrap()),
        ]),
    );
    set_pure_input(&mut tx, COL_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(field_value(&field, "SUI Recipient"), other_address());
    assert_eq!(
        field_value(&field, "SUI Recipient Also Receives"),
        "2 SUI split from the gas coin"
    );
}

#[test]
fn balance_with_funds_added_before_the_output_joins_it_shows_every_addition() {
    let mut tx = open_borrow_tx();
    let small = insert_gas_split(&mut tx, 8, ONE_SUI / 10);
    insert_command(&mut tx, 9, framework_call("coin", "into_balance", SUI, vec![small]));
    let large = insert_gas_split(&mut tx, 10, 5 * ONE_SUI);
    insert_command(&mut tx, 11, framework_call("coin", "into_balance", SUI, vec![large]));
    insert_command(&mut tx, 12, framework_call("balance", "join", SUI, vec![result(9), result(11)]));
    insert_command(
        &mut tx,
        13,
        framework_call("balance", "join", SUI, vec![result(9), Argument::NestedResult(7, 0)]),
    );
    move_call(&mut tx, 15).arguments[0] = result(9);
    set_pure_input(&mut tx, COL_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        field_value(&field, "SUI Recipient Also Receives"),
        "0.1 SUI split from the gas coin plus 5 SUI split from the gas coin"
    );
}

#[test]
fn merged_coin_joined_into_the_output_shows_the_merge() {
    let mut tx = open_borrow_tx();
    let balance = insert_one_plus_five_sui_balance(&mut tx);
    insert_command(
        &mut tx,
        balance + 1,
        framework_call("balance", "join", SUI, vec![Argument::NestedResult(7, 0), result(balance)]),
    );
    set_pure_input(&mut tx, COL_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        field_value(&field, "SUI Recipient Also Receives"),
        "1 SUI split from the gas coin plus 5 SUI split from the gas coin"
    );
}

#[test]
fn merged_coin_sent_with_the_cap_shows_the_merge() {
    let mut tx = open_borrow_tx();
    let one = insert_gas_split(&mut tx, CAP_TRANSFER_COMMAND, ONE_SUI);
    let five = insert_gas_split(&mut tx, CAP_TRANSFER_COMMAND + 1, 5 * ONE_SUI);
    insert_command(&mut tx, CAP_TRANSFER_COMMAND + 2, Command::MergeCoins(one, vec![five]));
    let Command::TransferObjects(objects, _) =
        &mut programmable(&mut tx).commands[CAP_TRANSFER_COMMAND + 3]
    else {
        panic!("expected the cap transfer");
    };
    objects.push(one);
    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        field_value(&field, "Position Owner Also Receives"),
        "1 SUI split from the gas coin plus 5 SUI split from the gas coin"
    );
}

#[test]
fn coin_changed_by_an_unknown_call_shows_no_amount() {
    let mut tx = open_borrow_tx();
    let one = insert_gas_split(&mut tx, CAP_TRANSFER_COMMAND, ONE_SUI);
    insert_command(&mut tx, CAP_TRANSFER_COMMAND + 1, tamper_call(vec![one]));
    let Command::TransferObjects(objects, _) =
        &mut programmable(&mut tx).commands[CAP_TRANSFER_COMMAND + 2]
    else {
        panic!("expected the cap transfer");
    };
    objects.push(one);
    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        field_value(&field, "Position Owner Also Receives"),
        "1 SUI split from the gas coin, changed by later commands (amount not in transaction)"
    );
}

#[test]
fn coin_both_split_and_merged_shows_no_amount() {
    let mut tx = open_borrow_tx();
    let one = insert_gas_split(&mut tx, CAP_TRANSFER_COMMAND, ONE_SUI);
    let five = insert_gas_split(&mut tx, CAP_TRANSFER_COMMAND + 1, 5 * ONE_SUI);
    insert_command(&mut tx, CAP_TRANSFER_COMMAND + 2, Command::MergeCoins(one, vec![five]));
    let amount = push_pure_input(&mut tx, bcs::to_bytes(&ONE_SUI).unwrap());
    insert_command(&mut tx, CAP_TRANSFER_COMMAND + 3, Command::SplitCoins(one, vec![amount]));
    let Command::TransferObjects(objects, _) =
        &mut programmable(&mut tx).commands[CAP_TRANSFER_COMMAND + 4]
    else {
        panic!("expected the cap transfer");
    };
    objects.push(one);
    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        field_value(&field, "Position Owner Also Receives"),
        "1 SUI split from the gas coin, changed by later commands (amount not in transaction)"
    );
}

#[test]
fn both_operates_in_one_transaction_render() {
    for command in [9, 21] {
        let field = card(&fixture_tx(TWO_OPERATES_DIGEST), command);
        assert_eq!(card_title(&field), "Deposit 0.3 SUI collateral on Fluid");
    }
}

#[test]
fn caps_sent_together_to_someone_else_are_shown() {
    let mut tx = fixture_tx(TWO_OPERATES_DIGEST);
    set_pure_input(&mut tx, TWO_OPERATES_CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, 9);
    assert_eq!(field_value(&field, "Position Owner"), other_address());
    assert_eq!(
        field_value(&field, "Position Owner Also Receives"),
        "an object returned by option::destroy_some at command 22"
    );
}

#[test]
fn object_sent_with_the_cap_is_shown_only_when_it_leaves_the_signer() {
    let mut tx = open_borrow_tx();
    let Command::TransferObjects(objects, _) =
        &mut programmable(&mut tx).commands[CAP_TRANSFER_COMMAND]
    else {
        panic!("expected the cap transfer");
    };
    objects.push(Argument::GasCoin);
    let to_signer = card(&tx, OPERATE_COMMAND);
    assert!(
        condensed_labels(&to_signer)
            .iter()
            .all(|label| !label.ends_with("Also Receives"))
    );

    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());
    let to_other = card(&tx, OPERATE_COMMAND);
    assert_eq!(
        field_value(&to_other, "Position Owner Also Receives"),
        "the gas coin's whole balance"
    );
}

#[test]
fn cap_sent_with_public_transfer_is_followed() {
    let mut tx = open_borrow_tx();
    programmable(&mut tx).commands[CAP_TRANSFER_COMMAND] = framework_call(
        "transfer",
        "public_transfer",
        &format!("{}::position::PositionOwnerCap", DEPLOYMENTS[0].vaults_package),
        vec![Argument::Result(8), Argument::Input(11)],
    );
    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(&tx, OPERATE_COMMAND);
    assert_eq!(field_value(&field, "Position Owner"), other_address());
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
            "The debt balance from command {OPERATE_COMMAND} is not sent to an address through 0x2::balance, 0x2::coin or 0x2::transfer"
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
            "The debt balance from command {OPERATE_COMMAND} is not sent to an address through 0x2::balance, 0x2::coin or 0x2::transfer"
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
        "The position owner cap is not returned through 0x1::option::destroy_some and a transfer, or 0x1::option::destroy_none",
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
