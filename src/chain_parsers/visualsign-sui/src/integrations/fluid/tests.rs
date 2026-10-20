use super::*;

use super::config::{DEPLOYMENTS, SuiNetwork};
use crate::SuiVisualSignConverter;
use crate::core::{SuiModuleResolver, SuiTransactionWrapper};
use crate::intermediate::SuiIntermediateOutput;
use crate::utils::run_aggregated_fixture;

use base64::Engine;
use move_bytecode_utils::module_cache::SyncModuleCache;
use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::transaction::{
    Argument, CallArg, Command, ObjectArg, ProgrammableMoveCall, ProgrammableTransaction,
    TransactionData, TransactionDataAPI, TransactionKind, WithdrawFrom, WithdrawalTypeArg,
};
use visualsign::SignablePayload;
use visualsign::test_utils::check_signable_payload_field;
use visualsign::vsptrait::{VisualSignConverter, VisualSignOptions};

const FIXTURE: &str = include_str!("aggregated_test_data.json");

const SIGNER: &str = "0xa718613f515e74d6aa5facb07363640b2267d5cf5b2b466b954452b6e4357b65";
const TESTNET_USDC: &str =
    "0xa1ec7fc00a6f40db9693ad1415d0c193ad3906494428cf252621037bd7117e29::usdc::USDC";
const OTHER_ADDRESS: [u8; 32] = [0x77; 32];

// Real open-deposit-and-borrow PTB: 0 view::get_both_exchange_rate, 1 init_position,
// 2 option::some(NR(1,1)), 3 balance::redeem_funds<SUI>(I14), 4 balance::zero<USDC>,
// 5 structs::positive(I6 col), 6 structs::positive(I7 debt), 7 operate(.., R5, R6, ..),
// 8 option::destroy_some(NR(7,2)), 9 send_funds<SUI>(NR(7,0), I11),
// 10 send_funds<USDC>(NR(7,1), I12), 11 position::share(NR(1,0)),
// 12 TransferObjects([R8], I13).
const OPEN_BORROW_DIGEST: &str = "2mFygtTthoZZ5Z65rkNVNwY1ojRd7G2Yv1sFT4x8K61L";
const COL_AMOUNT_COMMAND: usize = 5;
const DEBT_AMOUNT_COMMAND: usize = 6;
const OPERATE_COMMAND: usize = 7;
const DESTROY_SOME_COMMAND: usize = 8;
const DEBT_SEND_COMMAND: usize = 10;
const CAP_TRANSFER_COMMAND: usize = 12;
const DEBT_AMOUNT_INPUT: u16 = 7;
const DEBT_RECIPIENT_INPUT: usize = 12;
const CAP_RECIPIENT_INPUT: usize = 13;

// Real open-deposit PTB, the same layout with a `none` debt amount and
// `send_funds<SUI>` to I10, `send_funds<USDC>` to I11 and the cap to I12.
const OPEN_DEPOSIT_DIGEST: &str = "A57eyYUASDKtX8V4xUDc2am9nijrBXMSUibuFZrSVX8t";
const OPEN_DEPOSIT_DEBT_RECIPIENT_INPUT: usize = 11;

// Real withdraw-all PTB on an existing position: operate is command 6.
const WITHDRAW_ALL_DIGEST: &str = "D8TG8ESNHykaka3tmthDBnrgTJaescSStea7o8TUp7Ui";
const WITHDRAW_ALL_OPERATE_COMMAND: usize = 6;

const CLOSE_DIGEST: &str = "HVC9Z7a9YsRaDjiinUvYMGTCFeCWHjPsbHasZx3r1khk";

// Real lending deposit: 0 balance::redeem_funds<USDC>(I7), 1 deposit(.., R0, I4 min_shares, I5),
// 2 send_funds<FToken<USDC>>(R1, I6).
const LEND_DEPOSIT_DIGEST: &str = "C4TXLrbyhQB8mmy25o1iUC9syg9hHMyB3Uh1kt9xi7rV";
const LENDING_COMMAND: usize = 1;
const LEND_FUNDS_WITHDRAWAL_INPUT: usize = 7;

// Real lending withdrawal: 0 redeem_funds<FToken<USDC>>(I8), 1 withdraw(.., I4 assets, R0, I5),
// 2 send_funds<USDC>(NR(1,0), I6), 3 send_funds<FToken<USDC>>(NR(1,1), I7).
const LEND_WITHDRAW_DIGEST: &str = "BA3RzuwbFjJeufx8h7NtfPjjniXWhbT5S7KKHuc6gJh4";
const UNUSED_SHARES_RECIPIENT_INPUT: usize = 7;

const LEND_REDEEM_DIGEST: &str = "HNn2xw45QyQrxyDSKZTAzrfhtAkibu5oDABkTP5xeinv";

fn fixture_tx(section: &str, function: &str, digest: &str) -> TransactionData {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let data = fixture[section][function]["operations"][digest]["data"]
        .as_str()
        .unwrap();
    bcs::from_bytes(
        &base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
    )
    .unwrap()
}

fn open_borrow_tx() -> TransactionData {
    fixture_tx("vaults", "operate", OPEN_BORROW_DIGEST)
}

fn lend_deposit_tx() -> TransactionData {
    fixture_tx("lending", "deposit", LEND_DEPOSIT_DIGEST)
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

/// Rewrites every type argument, funds withdrawal type and package id that
/// mentions `from` to mention `to` instead.
fn replace_everywhere(mut tx: TransactionData, from: &str, to: &str) -> TransactionData {
    let replace = |tag: &sui_types::TypeTag| {
        sui_types::parse_sui_type_tag(&tag.to_canonical_string(true).replace(from, to)).unwrap()
    };
    let pt = programmable(&mut tx);
    for input in &mut pt.inputs {
        if let CallArg::FundsWithdrawal(withdrawal) = input {
            let WithdrawalTypeArg::Balance(tag) = &withdrawal.type_arg;
            withdrawal.type_arg = WithdrawalTypeArg::Balance(replace(tag));
        }
    }
    for command in &mut pt.commands {
        if let Command::MoveCall(call) = command {
            if call.package.to_hex() == from {
                call.package = ObjectID::from_hex_literal(&format!("0x{to}")).unwrap();
            }
            for type_argument in &mut call.type_arguments {
                *type_argument = replace(&type_argument.to_type_tag().unwrap()).into();
            }
        }
    }
    tx
}

/// The same transaction against the mainnet packages.
fn on_mainnet(tx: TransactionData) -> TransactionData {
    let [mainnet, testnet] = DEPLOYMENTS else {
        panic!("expected a mainnet and a testnet deployment");
    };
    let hex = |literal: &str| ObjectID::from_hex_literal(literal).unwrap().to_hex();
    [
        (testnet.vaults_package, mainnet.vaults_package),
        (testnet.lending_package, mainnet.lending_package),
        (testnet.oracle_package, mainnet.oracle_package),
    ]
    .into_iter()
    .fold(tx, |tx, (from, to)| {
        replace_everywhere(tx, &hex(from), &hex(to))
    })
}

fn set_pure_input(tx: &mut TransactionData, index: usize, bytes: Vec<u8>) {
    programmable(tx).inputs[index] = CallArg::Pure(bytes);
}

fn visualize(
    tx: TransactionData,
    command_index: usize,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let block_data = SuiTransactionBlockData::try_from_with_module_cache(
        tx,
        &SyncModuleCache::new(SuiModuleResolver),
    )
    .unwrap();
    let SuiTransactionBlockKind::ProgrammableTransaction(pt) = block_data.transaction() else {
        panic!("expected a programmable transaction");
    };
    let context =
        VisualizerContext::new(block_data.sender(), command_index, &pt.commands, &pt.inputs);
    assert!(FluidVisualizer.can_handle(&context));
    FluidVisualizer.visualize_tx_commands(&context)
}

fn card(tx: TransactionData, command_index: usize) -> AnnotatedPayloadField {
    visualize(tx, command_index).unwrap().remove(0)
}

fn card_title(field: &AnnotatedPayloadField) -> String {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    preview_layout.title.as_ref().unwrap().text.clone()
}

fn condensed_labels(field: &AnnotatedPayloadField) -> Vec<String> {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    preview_layout
        .condensed
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|f| f.signable_payload_field.label().clone())
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
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    preview_layout
        .condensed
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .find_map(|f| match &f.signable_payload_field {
            SignablePayloadField::AddressV2 { common, address_v2 } if common.label == label => {
                Some(address_v2.name.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {label:?} address row"))
}

fn payload_of(tx: &TransactionData) -> SignablePayload {
    crate::utils::payload_from_b64(
        &base64::engine::general_purpose::STANDARD.encode(bcs::to_bytes(tx).unwrap()),
    )
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

fn set_amount(tx: &mut TransactionData, command: usize, function: &str) {
    let call = move_call(tx, command);
    call.function = function.parse().unwrap();
    if function == "max" || function == "none" {
        call.arguments.clear();
    }
}

#[test]
fn test_fluid_aggregated() {
    run_aggregated_fixture(FIXTURE, Box::new(FluidVisualizer));
}

#[test]
fn every_fixture_emits_intermediate_output_listing_each_command() {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let mut checked = 0;
    for (section, function) in [
        ("vaults", "operate"),
        ("vaults", "close_position"),
        ("lending", "deposit"),
        ("lending", "withdraw"),
        ("lending", "redeem"),
    ] {
        for digest in fixture[section][function]["operations"].as_object().unwrap().keys() {
            let tx = fixture_tx(section, function, digest);
            let TransactionKind::ProgrammableTransaction(pt) = tx.kind() else {
                panic!("expected a programmable transaction");
            };
            let commands = pt.commands.len();
            let options = VisualSignOptions {
                include_intermediate_output: true,
                ..VisualSignOptions::default()
            };
            let bytes = SuiVisualSignConverter
                .to_visual_sign_payload(SuiTransactionWrapper::new(tx), options)
                .unwrap()
                .intermediate_output
                .unwrap_or_else(|| panic!("no intermediate output for {digest}"));
            let output: SuiIntermediateOutput = borsh::from_slice(&bytes).unwrap();
            assert_eq!(output.commands.len(), commands, "{digest}");
            assert!(
                output
                    .commands
                    .iter()
                    .any(|command| command.module_name == "user" && command.function == function),
                "{digest} lists no user::{function} call"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 11);
}

#[test]
fn every_deployment_package_is_handled_in_the_form_it_is_called_with() {
    let config = Config::new();
    for deployment in DEPLOYMENTS {
        for (package, functions) in [
            (deployment.vaults_package, vec!["operate", "close_position"]),
            (
                deployment.lending_package,
                vec!["deposit", "withdraw", "redeem"],
            ),
        ] {
            let id = ObjectID::from_hex_literal(package).unwrap();
            assert_eq!(id.to_hex_literal(), package);
            for function in functions {
                assert!(config.can_handle(&id.to_hex_literal(), "user", function));
            }
            assert!(!config.can_handle(&id.to_hex_literal(), "user", "init_position"));
            assert_eq!(deployment_for(&id).unwrap().sui_network, deployment.sui_network);
        }
    }
}

#[test]
fn mainnet_lending_package_is_the_published_id() {
    let mainnet = &DEPLOYMENTS[0];
    assert_eq!(mainnet.sui_network, SuiNetwork::Mainnet);
    assert_eq!(
        ObjectID::from_hex_literal(mainnet.lending_package).unwrap(),
        ObjectID::from_hex_literal(
            "0x00207c696eaa1b80e6e001cb0739dff0a554b5561265008d33ca9b071fd1d202"
        )
        .unwrap()
    );
}

#[test]
fn vault_cards_are_titled_with_each_change() {
    for (digest, command, title) in [
        (
            OPEN_DEPOSIT_DIGEST,
            OPERATE_COMMAND,
            "Deposit 0.5 SUI collateral on Fluid (Sui Testnet)",
        ),
        (
            OPEN_BORROW_DIGEST,
            OPERATE_COMMAND,
            "Deposit 1 SUI collateral and borrow 0.1 USDC on Fluid (Sui Testnet)",
        ),
        (
            WITHDRAW_ALL_DIGEST,
            WITHDRAW_ALL_OPERATE_COMMAND,
            "Withdraw all SUI collateral on Fluid (Sui Testnet)",
        ),
    ] {
        let field = card(fixture_tx("vaults", "operate", digest), command);
        assert_eq!(card_title(&field), title);
    }
}

#[test]
fn repayment_is_a_negative_debt_amount() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, COL_AMOUNT_COMMAND, "none");
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "negative");

    let field = card(tx, OPERATE_COMMAND);
    assert_eq!(card_title(&field), "Repay 0.1 USDC on Fluid (Sui Testnet)");
    assert_eq!(field_value(&field, "Repayment"), "0.1");
    // Repaying pays nothing out, and unused USDC goes back to the signer.
    assert_eq!(
        condensed_labels(&field),
        vec!["Repayment", "Position", "Position Owner", "Network"]
    );
}

#[test]
fn max_debt_amount_repays_all_debt() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, COL_AMOUNT_COMMAND, "none");
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "max");

    let field = card(tx, OPERATE_COMMAND);
    assert_eq!(
        card_title(&field),
        "Repay all USDC debt on Fluid (Sui Testnet)"
    );
    assert_eq!(field_value(&field, "Repayment"), "All USDC debt");
}

#[test]
fn partial_collateral_withdrawal_shows_where_it_goes() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, COL_AMOUNT_COMMAND, "negative");
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "none");

    let field = card(tx, OPERATE_COMMAND);
    assert_eq!(
        card_title(&field),
        "Withdraw 1 SUI collateral on Fluid (Sui Testnet)"
    );
    assert_eq!(field_value(&field, "Collateral Withdrawal"), "1");
    assert_eq!(field_value(&field, "SUI Recipient"), SIGNER);
}

#[test]
fn zero_amounts_build_none() {
    let mut tx = open_borrow_tx();
    let zero = bcs::to_bytes(&0_u64).unwrap();
    set_pure_input(&mut tx, usize::from(DEBT_AMOUNT_INPUT), zero);

    let field = card(tx, OPERATE_COMMAND);
    assert_eq!(
        card_title(&field),
        "Deposit 1 SUI collateral on Fluid (Sui Testnet)"
    );
}

#[test]
fn operate_without_any_change_fails() {
    let mut tx = open_borrow_tx();
    set_amount(&mut tx, COL_AMOUNT_COMMAND, "none");
    set_amount(&mut tx, DEBT_AMOUNT_COMMAND, "none");

    expect_error(
        visualize(tx, OPERATE_COMMAND),
        "Fluid operate with neither a collateral nor a debt change aborts",
    );
}

#[test]
fn unused_coin_sent_to_someone_else_is_shown() {
    let mut tx = fixture_tx("vaults", "operate", OPEN_DEPOSIT_DIGEST);
    set_pure_input(
        &mut tx,
        OPEN_DEPOSIT_DEBT_RECIPIENT_INPUT,
        OTHER_ADDRESS.to_vec(),
    );

    let field = card(tx, OPERATE_COMMAND);
    assert_eq!(
        address_name(&field, "USDC Recipient"),
        "Not the signer's address"
    );
}

#[test]
fn borrowed_coin_and_position_owner_name_their_recipients() {
    let mut tx = open_borrow_tx();
    set_pure_input(&mut tx, DEBT_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());
    set_pure_input(&mut tx, CAP_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());

    let field = card(tx, OPERATE_COMMAND);
    let other = SuiAddress::from_bytes(OTHER_ADDRESS).unwrap().to_string();
    assert_eq!(field_value(&field, "USDC Recipient"), other);
    assert_eq!(
        address_name(&field, "USDC Recipient"),
        "Not the signer's address"
    );
    assert_eq!(field_value(&field, "Position Owner"), other);
    assert_eq!(
        address_name(&field, "Position Owner"),
        "Not the signer's address"
    );
}

#[test]
fn unknown_coins_are_shown_in_base_units_with_their_type() {
    let usdc = ObjectID::from_hex_literal(TESTNET_USDC.split("::").next().unwrap())
        .unwrap()
        .to_hex();
    let tx = replace_everywhere(open_borrow_tx(), &usdc, &ObjectID::from_single_byte(0x42).to_hex());

    let field = card(tx, OPERATE_COMMAND);
    assert_eq!(
        card_title(&field),
        "Deposit 1 SUI collateral and borrow 100000 base units of 0x42::usdc::USDC on Fluid (Sui Testnet)"
    );
}

#[test]
fn mainnet_calls_render_on_sui() {
    for (tx, command, title) in [
        (
            on_mainnet(open_borrow_tx()),
            OPERATE_COMMAND,
            "Deposit 1 SUI collateral and borrow 0.1 USDC on Fluid",
        ),
        (
            on_mainnet(fixture_tx("vaults", "close_position", CLOSE_DIGEST)),
            0,
            "Close vault position on Fluid",
        ),
        (
            on_mainnet(lend_deposit_tx()),
            LENDING_COMMAND,
            "Lend 10 USDC on Fluid",
        ),
        (
            on_mainnet(fixture_tx("lending", "withdraw", LEND_WITHDRAW_DIGEST)),
            LENDING_COMMAND,
            "Withdraw 5 USDC from lending on Fluid",
        ),
        (
            on_mainnet(fixture_tx("lending", "redeem", LEND_REDEEM_DIGEST)),
            LENDING_COMMAND,
            "Withdraw SUI from lending on Fluid",
        ),
    ] {
        let field = card(tx, command);
        assert_eq!(card_title(&field), title);
        assert_eq!(field_value(&field, "Network"), "Sui");
    }
}

#[test]
fn lending_payload_is_titled_with_the_action() {
    let payload = payload_of(&lend_deposit_tx());
    assert_eq!(payload.title, "Lend 10 USDC on Fluid (Sui Testnet)");
    assert_eq!(
        payload.subtitle.as_deref(),
        Some(format!("fTokens to {SIGNER}").as_str())
    );
}

#[test]
fn vault_payload_keeps_the_generic_title_while_the_cap_transfer_renders() {
    // The position owner cap goes back through `TransferObjects`, which the
    // generic transfer preview renders as a second card.
    let payload = payload_of(&open_borrow_tx());
    assert_eq!(payload.title, "Programmable Transaction");
}

#[test]
fn lending_payload_with_an_unrelated_command_keeps_the_generic_title() {
    let mut tx = lend_deposit_tx();
    insert_command(&mut tx, 0, tamper_call(vec![]));

    let payload = payload_of(&tx);
    assert_eq!(payload.title, "Programmable Transaction");
}

#[test]
fn unused_shares_sent_to_someone_else_are_shown() {
    let mut tx = fixture_tx("lending", "withdraw", LEND_WITHDRAW_DIGEST);
    let real = card(tx.clone(), LENDING_COMMAND);
    assert!(
        condensed_labels(&real)
            .iter()
            .all(|label| label != "Unused fToken Recipient")
    );

    set_pure_input(&mut tx, UNUSED_SHARES_RECIPIENT_INPUT, OTHER_ADDRESS.to_vec());
    let field = card(tx, LENDING_COMMAND);
    assert_eq!(
        address_name(&field, "Unused fToken Recipient"),
        "Not the signer's address"
    );
}

#[test]
fn borrowed_coin_not_sent_with_send_funds_fails() {
    let mut tx = open_borrow_tx();
    programmable(&mut tx).commands[DEBT_SEND_COMMAND] =
        tamper_call(vec![Argument::NestedResult(7, 1), Argument::Input(12)]);

    expect_error(
        visualize(tx, OPERATE_COMMAND),
        &format!(
            "The debt balance from command {OPERATE_COMMAND} is not sent with 0x2::balance::send_funds<{TESTNET_USDC}>"
        ),
    );
}

#[test]
fn borrowed_coin_used_twice_fails() {
    let mut tx = open_borrow_tx();
    insert_command(
        &mut tx,
        DEBT_SEND_COMMAND,
        tamper_call(vec![Argument::NestedResult(7, 1)]),
    );

    expect_error(
        visualize(tx, OPERATE_COMMAND),
        &format!(
            "The debt balance from command {OPERATE_COMMAND} must be used by exactly one later command"
        ),
    );
}

#[test]
fn amount_built_outside_fluid_fails() {
    let mut tx = open_borrow_tx();
    move_call(&mut tx, DEBT_AMOUNT_COMMAND).package = ObjectID::from_single_byte(0x42);

    expect_error(
        visualize(tx, OPERATE_COMMAND),
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
        visualize(tx, OPERATE_COMMAND + 1),
        &format!(
            "`debt_amount` is passed to command {DEBT_AMOUNT_COMMAND}, which may modify it before command {} consumes it",
            DEBT_AMOUNT_COMMAND + 1
        ),
    );
}

#[test]
fn recipient_modified_before_send_funds_fails() {
    let mut tx = open_borrow_tx();
    insert_command(
        &mut tx,
        DESTROY_SOME_COMMAND,
        tamper_call(vec![Argument::Input(12)]),
    );

    expect_error(
        visualize(tx, OPERATE_COMMAND),
        &format!(
            "`recipient` is passed to command {DESTROY_SOME_COMMAND}, which may modify it before command {} consumes it",
            DEBT_SEND_COMMAND + 1
        ),
    );
}

#[test]
fn cap_transferred_with_another_object_fails() {
    let mut tx = open_borrow_tx();
    let Command::TransferObjects(objects, _) =
        &mut programmable(&mut tx).commands[CAP_TRANSFER_COMMAND]
    else {
        panic!("expected the cap transfer");
    };
    objects.push(Argument::GasCoin);

    expect_error(
        visualize(tx, OPERATE_COMMAND),
        "The position owner cap is not returned through 0x1::option::destroy_some and TransferObjects",
    );
}

#[test]
fn lending_deposit_from_the_sponsor_fails() {
    let mut tx = lend_deposit_tx();
    let CallArg::FundsWithdrawal(withdrawal) =
        &mut programmable(&mut tx).inputs[LEND_FUNDS_WITHDRAWAL_INPUT]
    else {
        panic!("expected the funds withdrawal");
    };
    withdrawal.withdraw_from = WithdrawFrom::Sponsor;

    expect_error(
        visualize(tx, LENDING_COMMAND),
        &format!(
            "`assets` is not a 0x2::balance::redeem_funds<{TESTNET_USDC}> of the signer's address balance"
        ),
    );
}

#[test]
fn lending_deposit_from_a_coin_object_fails() {
    let mut tx = lend_deposit_tx();
    let pt = programmable(&mut tx);
    pt.inputs.push(CallArg::Object(ObjectArg::ImmOrOwnedObject((
        ObjectID::from_single_byte(0x55),
        sui_types::base_types::SequenceNumber::from_u64(1),
        sui_types::digests::ObjectDigest::new([0x55; 32]),
    ))));
    let coin = Argument::Input(u16::try_from(pt.inputs.len() - 1).unwrap());
    let call = move_call(&mut tx, 0);
    call.module = "coin".parse().unwrap();
    call.function = "into_balance".parse().unwrap();
    call.arguments = vec![coin];

    expect_error(
        visualize(tx, LENDING_COMMAND),
        &format!(
            "`assets` is not a 0x2::balance::redeem_funds<{TESTNET_USDC}> of the signer's address balance"
        ),
    );
}

#[test]
fn format_units_trims_trailing_zeros() {
    assert_eq!(format_units(500_000_000, 9), "0.5");
    assert_eq!(format_units(100_000, 6), "0.1");
    assert_eq!(format_units(19_999_998, 6), "19.999998");
    assert_eq!(format_units(10_000_000, 6), "10");
    assert_eq!(format_units(1, 9), "0.000000001");
    assert_eq!(format_units(0, 8), "0");
}
