use super::config::BitcoinNetwork;
use super::*;

use crate::core::SuiModuleResolver;
use crate::utils::run_aggregated_fixture;

use base64::Engine;
use move_bytecode_utils::module_cache::SyncModuleCache;
use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::transaction::{
    Argument, CallArg, Command, ProgrammableMoveCall, ProgrammableTransaction, TransactionData,
    TransactionDataAPI, TransactionKind,
};
use visualsign::test_utils::check_signable_payload_field;

const FIXTURE: &str = include_str!("aggregated_test_data.json");
const HASHI_PACKAGE: &str = "0x8f7efd743897fde48cc35b6203cd72c7ad4248f0eb02a9ad378e4a2d39cc2c7e";
const DEPOSIT_DIGEST: &str = "9oBYx5DMJCLBj6LALo3shpwUJSp6dNvZic6c2FpaF7jo";

// Real deposit PTB: 0 utxo_id(I0 txid, I1 vout), 1 utxo(R0, I2 amount, I3 path),
// 2 deposit(I4 hashi, R1, I5 clock).
const UTXO_ID_COMMAND: usize = 0;
const UTXO_COMMAND: usize = 1;
const DEPOSIT_COMMAND: usize = 2;
const TXID_INPUT: u16 = 0;
const VOUT_INPUT: u16 = 1;
const DEPOSIT_AMOUNT_INPUT: u16 = 2;
const DERIVATION_PATH_INPUT: usize = 3;

const STDLIB: u8 = 1;

fn encoded_fixture_tx(section: &str, function: &str, digest: &str) -> String {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    fixture[section][function]["operations"][digest]["data"]
        .as_str()
        .unwrap()
        .to_string()
}

fn fixture_tx(section: &str, function: &str, digest: &str) -> TransactionData {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded_fixture_tx(section, function, digest))
        .unwrap();
    bcs::from_bytes(&bytes).unwrap()
}

fn deposit_tx() -> TransactionData {
    fixture_tx("deposit", "deposit", DEPOSIT_DIGEST)
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

fn result(index: usize) -> Argument {
    Argument::Result(u16::try_from(index).unwrap())
}

fn framework_call(
    package: u8,
    module: &str,
    function: &str,
    type_arg: Option<&str>,
    args: Vec<Argument>,
) -> Command {
    Command::move_call(
        ObjectID::from_single_byte(package),
        module.parse().unwrap(),
        function.parse().unwrap(),
        type_arg
            .map(|tag| vec![sui_types::parse_sui_type_tag(tag).unwrap()])
            .unwrap_or_default(),
        args,
    )
}

fn push_pure_input(tx: &mut TransactionData, bytes: Vec<u8>) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(CallArg::Pure(bytes));
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn tamper_call(function: &str, args: Vec<Argument>) -> Command {
    Command::move_call(
        ObjectID::from_single_byte(0x42),
        "tamper".parse().unwrap(),
        function.parse().unwrap(),
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
    assert!(HashiVisualizer.can_handle(&context));
    HashiVisualizer.visualize_tx_commands(&context)
}

fn preview_text(field: &AnnotatedPayloadField) -> (String, String) {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    (
        preview_layout.title.as_ref().unwrap().text.clone(),
        preview_layout.subtitle.as_ref().unwrap().text.clone(),
    )
}

fn field_values(field: &AnnotatedPayloadField, label: &str) -> Vec<String> {
    let (found, values) = check_signable_payload_field(&field.signable_payload_field, label);
    assert!(found, "no {label:?} field rendered");
    values
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
fn test_hashi_aggregated() {
    run_aggregated_fixture(FIXTURE, Box::new(HashiVisualizer));
}

#[test]
fn deposit_title_names_amount_and_recipient() {
    let fields = visualize(deposit_tx(), DEPOSIT_COMMAND).unwrap();
    assert_eq!(
        preview_text(&fields[0]),
        (
            "Hashi Deposit 0.15409915 BTC (Bitcoin Signet)".to_string(),
            "To 0x9746979c122e2d6fdab7fa09fb7234f5a3043eab3fa2fe0a59b08e5c09553cc3".to_string()
        )
    );
}

#[test]
fn deposit_without_derivation_path_warns_in_subtitle_and_summary() {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[DERIVATION_PATH_INPUT] =
        CallArg::Pure(bcs::to_bytes(&None::<[u8; 32]>).unwrap());

    let fields = visualize(tx, DEPOSIT_COMMAND).unwrap();
    assert_eq!(
        preview_text(&fields[0]),
        (
            "Hashi Deposit 0.15409915 BTC (Bitcoin Signet)".to_string(),
            "Warning: no hBTC recipient".to_string()
        )
    );
    assert_eq!(
        field_values(&fields[0], "hBTC Recipient"),
        vec!["None: no hBTC will be credited".to_string()]
    );
    assert_eq!(
        field_values(&fields[0], "Summary"),
        vec![
            "Warning: this deposit names no recipient. The Bitcoin Signet transaction output e9d9379417481e04cd2d7d37a505f9ce89fceaf39ee11f5435af7b1b4cfb2647:1, declared as 0.15409915 BTC, will not be credited to anyone.".to_string()
        ]
    );
}

#[test]
fn bitcoin_networks_have_distinct_prefixes_and_names() {
    assert_eq!(
        (
            BitcoinNetwork::Mainnet.bech32_hrp(),
            BitcoinNetwork::Mainnet.display_name()
        ),
        ("bc", "Bitcoin")
    );
    assert_eq!(
        (
            BitcoinNetwork::Signet.bech32_hrp(),
            BitcoinNetwork::Signet.display_name()
        ),
        ("tb", "Bitcoin Signet")
    );
}

#[test]
fn deposit_with_a_txid_that_is_not_32_bytes_fails() {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[usize::from(TXID_INPUT)] = CallArg::Pure(vec![0u8; 31]);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Bitcoin txid must be 32 bytes, got 31",
    );
}

#[test]
fn deposit_with_an_invalid_derivation_path_encoding_fails() {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[DERIVATION_PATH_INPUT] = CallArg::Pure(vec![2]);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Invalid derivation_path encoding: expected option type",
    );
}

#[test]
fn deposit_whose_utxo_comes_from_another_hashi_function_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, DEPOSIT_COMMAND).arguments[1] = result(UTXO_ID_COMMAND);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Expected argument to come from Hashi `utxo::utxo`",
    );
}

#[test]
fn deposit_whose_utxo_comes_from_a_foreign_package_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, UTXO_COMMAND).package = ObjectID::from_single_byte(0x42);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Expected argument to come from Hashi `utxo::utxo`",
    );
}

#[test]
fn deposit_whose_utxo_id_comes_from_a_foreign_package_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, UTXO_ID_COMMAND).package = ObjectID::from_single_byte(0x42);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Expected argument to come from Hashi `utxo::utxo_id`",
    );
}

#[test]
fn deposit_with_utxo_passed_as_input_fails() {
    let mut tx = deposit_tx();
    let unrelated = push_pure_input(&mut tx, bcs::to_bytes(&0u64).unwrap());
    move_call(&mut tx, DEPOSIT_COMMAND).arguments[1] = unrelated;

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "`utxo` is not the result of a previous command",
    );
}

#[test]
fn deposit_referring_to_its_own_result_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, DEPOSIT_COMMAND).arguments[1] = result(DEPOSIT_COMMAND);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "`utxo` in command 2 refers to the result of command 2, which does not precede it",
    );
}

#[test]
fn deposit_referring_to_a_later_result_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, DEPOSIT_COMMAND).arguments[1] = result(DEPOSIT_COMMAND + 1);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "`utxo` in command 2 refers to the result of command 3, which does not precede it",
    );
}

#[test]
fn deposit_recipient_rewritten_by_an_earlier_command_fails() {
    let mut tx = deposit_tx();
    let attacker = push_pure_input(&mut tx, bcs::to_bytes(&[0xAAu8; 32]).unwrap());
    insert_command(
        &mut tx,
        0,
        framework_call(
            STDLIB,
            "option",
            "swap",
            Some("address"),
            vec![
                Argument::Input(u16::try_from(DERIVATION_PATH_INPUT).unwrap()),
                attacker,
            ],
        ),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`derivation_path` is passed to command 0, which may modify it before command 2 consumes it",
    );
}

#[test]
fn deposit_utxo_rewritten_before_the_deposit_fails() {
    let mut tx = deposit_tx();
    insert_command(
        &mut tx,
        DEPOSIT_COMMAND,
        tamper_call("replace_utxo", vec![result(UTXO_COMMAND)]),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`utxo` is passed to command 2, which may modify it before command 3 consumes it",
    );
}

#[test]
fn batched_deposits_sharing_txid_amount_and_recipient_inputs_render() {
    let mut tx = deposit_tx();
    let second_vout = push_pure_input(&mut tx, bcs::to_bytes(&2u32).unwrap());
    let hashi = ObjectID::from_hex_literal(HASHI_PACKAGE).unwrap();
    let deposit_call = move_call(&mut tx, DEPOSIT_COMMAND).clone();
    let hashi_input = deposit_call.arguments[0];
    let clock_input = deposit_call.arguments[2];
    let commands = &mut programmable(&mut tx).commands;
    commands.push(Command::move_call(
        hashi,
        "utxo".parse().unwrap(),
        "utxo_id".parse().unwrap(),
        vec![],
        vec![Argument::Input(TXID_INPUT), second_vout],
    ));
    commands.push(Command::move_call(
        hashi,
        "utxo".parse().unwrap(),
        "utxo".parse().unwrap(),
        vec![],
        vec![
            result(DEPOSIT_COMMAND + 1),
            Argument::Input(DEPOSIT_AMOUNT_INPUT),
            Argument::Input(u16::try_from(DERIVATION_PATH_INPUT).unwrap()),
        ],
    ));
    commands.push(Command::move_call(
        hashi,
        "deposit".parse().unwrap(),
        "deposit".parse().unwrap(),
        vec![],
        vec![hashi_input, result(DEPOSIT_COMMAND + 2), clock_input],
    ));

    for (command, vout) in [(DEPOSIT_COMMAND, "1"), (DEPOSIT_COMMAND + 3, "2")] {
        let fields = visualize(tx.clone(), command).unwrap();
        assert_eq!(
            preview_text(&fields[0]),
            (
                "Hashi Deposit 0.15409915 BTC (Bitcoin Signet)".to_string(),
                "To 0x9746979c122e2d6fdab7fa09fb7234f5a3043eab3fa2fe0a59b08e5c09553cc3".to_string()
            )
        );
        assert_eq!(
            field_values(&fields[0], "Bitcoin Transaction"),
            vec!["e9d9379417481e04cd2d7d37a505f9ce89fceaf39ee11f5435af7b1b4cfb2647".to_string()]
        );
        assert_eq!(
            field_values(&fields[0], "Bitcoin Output Index"),
            vec![vout.to_string()]
        );
    }
}

#[test]
fn deposit_recipient_passed_to_a_foreign_utxo_module_fails() {
    let mut tx = deposit_tx();
    insert_command(
        &mut tx,
        0,
        Command::move_call(
            ObjectID::from_single_byte(0x42),
            "utxo".parse().unwrap(),
            "utxo".parse().unwrap(),
            vec![],
            vec![Argument::Input(
                u16::try_from(DERIVATION_PATH_INPUT).unwrap(),
            )],
        ),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`derivation_path` is passed to command 0, which may modify it before command 2 consumes it",
    );
}

#[test]
fn deposit_amount_rewritten_by_an_earlier_command_fails() {
    let mut tx = deposit_tx();
    insert_command(
        &mut tx,
        0,
        tamper_call("bump", vec![Argument::Input(DEPOSIT_AMOUNT_INPUT)]),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`amount` is passed to command 0, which may modify it before command 2 consumes it",
    );
}

#[test]
fn deposit_txid_rewritten_by_an_earlier_command_fails() {
    let mut tx = deposit_tx();
    insert_command(
        &mut tx,
        0,
        tamper_call("replace_txid", vec![Argument::Input(TXID_INPUT)]),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`txid` is passed to command 0, which may modify it before command 1 consumes it",
    );
}

#[test]
fn deposit_vout_rewritten_by_an_earlier_command_fails() {
    let mut tx = deposit_tx();
    insert_command(
        &mut tx,
        0,
        tamper_call("bump", vec![Argument::Input(VOUT_INPUT)]),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`vout` is passed to command 0, which may modify it before command 1 consumes it",
    );
}

#[test]
fn deposit_utxo_id_rewritten_before_the_utxo_is_built_fails() {
    let mut tx = deposit_tx();
    insert_command(
        &mut tx,
        UTXO_COMMAND,
        tamper_call("replace_utxo_id", vec![result(UTXO_ID_COMMAND)]),
    );

    expect_error(
        visualize(tx, DEPOSIT_COMMAND + 1),
        "`utxo_id` is passed to command 1, which may modify it before command 2 consumes it",
    );
}

#[test]
fn deposit_with_a_wrong_width_amount_fails() {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[usize::from(DEPOSIT_AMOUNT_INPUT)] =
        CallArg::Pure(bcs::to_bytes(&7u32).unwrap());

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Invalid u64 value: could not convert slice to array",
    );
}

#[test]
fn deposit_with_a_txid_that_is_not_an_input_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, UTXO_ID_COMMAND).arguments[0] = Argument::GasCoin;

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Argument `txid` is not a transaction input",
    );
}

#[test]
fn deposit_with_a_missing_utxo_argument_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, UTXO_COMMAND).arguments.truncate(2);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Argument `derivation_path` not found",
    );
}

#[test]
fn title_suffix_is_empty_on_mainnet_and_names_signet() {
    assert_eq!(BitcoinNetwork::Mainnet.title_suffix(), "");
    assert_eq!(BitcoinNetwork::Signet.title_suffix(), " (Bitcoin Signet)");
}

#[test]
fn configured_packages_and_deployment_rows_match_exactly() {
    let config = HASHI_CONFIG.get_or_init(Config::new);
    let configured: Vec<ObjectID> = config
        .data()
        .packages
        .keys()
        .map(|package| ObjectID::from_hex_literal(package).unwrap())
        .collect();
    let rows: Vec<ObjectID> = config::DEPLOYMENTS
        .iter()
        .map(|deployment| ObjectID::from_hex_literal(deployment.package_id).unwrap())
        .collect();

    let mut sorted_configured = configured.clone();
    sorted_configured.sort();
    let mut sorted_rows = rows.clone();
    sorted_rows.sort();
    sorted_rows.dedup();
    assert_eq!(rows.len(), sorted_rows.len(), "duplicate deployment row");
    assert_eq!(sorted_configured, sorted_rows);
    for package in &configured {
        assert!(deployment_for(package).unwrap().type_origin().is_some());
    }
}

#[test]
fn unconfigured_package_has_no_deployment() {
    assert!(deployment_for(&ObjectID::from_single_byte(0x42)).is_none());
}

#[test]
fn full_payload_includes_the_hashi_preview() {
    let payload =
        crate::utils::payload_from_b64(&encoded_fixture_tx("deposit", "deposit", DEPOSIT_DIGEST));
    let hashi_fields = payload
        .fields
        .iter()
        .filter(|field| field.label() == "Hashi Deposit")
        .count();
    assert_eq!(hashi_fields, 1, "Hashi Deposit missing from full payload");
}

#[test]
fn format_btc_trims_trailing_zeros() {
    assert_eq!(format_btc(0), "0");
    assert_eq!(format_btc(546), "0.00000546");
    assert_eq!(format_btc(100_000_000), "1");
    assert_eq!(format_btc(150_000_000), "1.5");
    assert_eq!(format_btc(u64::MAX), "184467440737.09551615");
}
