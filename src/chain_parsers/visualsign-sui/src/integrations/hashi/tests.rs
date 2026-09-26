use super::*;

use crate::core::SuiModuleResolver;
use crate::utils::run_aggregated_fixture;

use base64::Engine;
use move_bytecode_utils::module_cache::SyncModuleCache;
use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::transaction::{
    Argument, CallArg, Command, FundsWithdrawalArg, ProgrammableMoveCall, ProgrammableTransaction,
    TransactionData, TransactionDataAPI, TransactionKind,
};
use visualsign::test_utils::check_signable_payload_field;

const FIXTURE: &str = include_str!("aggregated_test_data.json");
const HASHI_PACKAGE: &str = "0x8f7efd743897fde48cc35b6203cd72c7ad4248f0eb02a9ad378e4a2d39cc2c7e";
const HBTC_TYPE_ORIGIN: &str = "0xfcea10cadbb553c4874201584abf68771592678952efd957b2e82c010c7f4360";
const DEPOSIT_DIGEST: &str = "9oBYx5DMJCLBj6LALo3shpwUJSp6dNvZic6c2FpaF7jo";
const WITHDRAWAL_DIGEST: &str = "4BESv9z6m2PRHUXg4uniDXQpeMtKsk9vxc7h2jgogVga";

// Real deposit PTB: 0 utxo_id(I0 txid, I1 vout), 1 utxo(R0, I2 amount, I3 path),
// 2 deposit(I4 hashi, R1, I5 clock).
const UTXO_ID_COMMAND: usize = 0;
const UTXO_COMMAND: usize = 1;
const DEPOSIT_COMMAND: usize = 2;
const TXID_INPUT: u16 = 0;
const VOUT_INPUT: u16 = 1;
const DEPOSIT_AMOUNT_INPUT: u16 = 2;
const DERIVATION_PATH_INPUT: usize = 3;

// Real withdrawal PTB: 0 MergeCoins(I0, [I1]), 1 SplitCoins(I0, [I2 amount]),
// 2 into_balance<BTC>(NR(1,0)), 3 request_withdrawal(I3 hashi, I4 clock, R2, I5 addr).
const SPLIT_COMMAND: usize = 1;
const INTO_BALANCE_COMMAND: usize = 2;
const WITHDRAWAL_COMMAND: usize = 3;
const HBTC_COIN_INPUT: u16 = 0;
const SPLIT_AMOUNT_INPUT: u16 = 2;
const BITCOIN_ADDRESS_INPUT: u16 = 5;

// Real address-balance withdrawal: 0 balance::redeem_funds<BTC>(I3 funds withdrawal),
// 1 request_withdrawal(I0 hashi, I1 clock, R0, I2 addr).
const REDEEM_WITHDRAWAL_DIGEST: &str = "E7fvzD239zzTLnhTVsuXVKPKKkFUKSRvdR43oJ8Je8cD";
const REDEEM_COMMAND: usize = 0;
const REDEEM_WITHDRAWAL_COMMAND: usize = 1;
const FUNDS_WITHDRAWAL_INPUT: u16 = 3;

// Real top-up withdrawal: 0 coin::redeem_funds<BTC>(I4), 1 MergeCoins(I3, [R0]),
// 2 SplitCoins(I3, [I5 amount]), 3 into_balance(NR(2,0)), 4 send_funds(I3, I6),
// 5 request_withdrawal(I0 hashi, I1 clock, NR(3,0), I2 addr).
const TOP_UP_WITHDRAWAL_DIGEST: &str = "8cxs733Zm4LYaVnJBMUptjvr3CXxp2ETwBGpfxnR7kcf";
const TOP_UP_FUNDS_WITHDRAWAL_INPUT: usize = 4;
const TOP_UP_HBTC_COIN_INPUT: u16 = 3;
const TOP_UP_SPLIT_AMOUNT_INPUT: u16 = 5;

// Top-up inputs rebuilt as: 0 coin::redeem_funds<BTC>(I4), 1 into_balance(R0),
// 2 request_withdrawal(I0 hashi, I1 clock, R1, I2 addr).
const REDEEMED_COIN_WITHDRAWAL_COMMAND: usize = 2;
const REDEEMED_COIN_INTO_BALANCE_COMMAND: usize = 1;

const STDLIB: u8 = 1;
const SUI_FRAMEWORK: u8 = 2;

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

fn withdrawal_tx() -> TransactionData {
    fixture_tx("withdraw", "request_withdrawal", WITHDRAWAL_DIGEST)
}

fn redeem_withdrawal_tx() -> TransactionData {
    fixture_tx("withdraw", "request_withdrawal", REDEEM_WITHDRAWAL_DIGEST)
}

fn top_up_withdrawal_tx() -> TransactionData {
    fixture_tx("withdraw", "request_withdrawal", TOP_UP_WITHDRAWAL_DIGEST)
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

fn hbtc_type() -> String {
    format!("{HBTC_TYPE_ORIGIN}::btc::BTC")
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

fn expected_withdrawal_preview() -> (String, String) {
    (
        "Hashi Withdraw 0.24185763 hBTC (Bitcoin Signet)".to_string(),
        "To tb1p57gd244cdg4zjh57x265l6alts5c8620l4rk92wqpxj3pk0eaczsau385x".to_string(),
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
fn withdrawal_title_names_amount_and_full_bitcoin_recipient() {
    let fields = visualize(withdrawal_tx(), WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(preview_text(&fields[0]), expected_withdrawal_preview());
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
fn withdrawal_to_p2wpkh_encodes_bech32_v0() {
    let mut tx = withdrawal_tx();
    programmable(&mut tx).inputs[usize::from(BITCOIN_ADDRESS_INPUT)] =
        CallArg::Pure(bcs::to_bytes(&vec![0x75u8; 20]).unwrap());

    let fields = visualize(tx, WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        field_values(&fields[0], "Bitcoin Recipient"),
        vec!["tb1qw46h2at4w46h2at4w46h2at4w46h2at4qy2ul6".to_string()]
    );
}

#[test]
fn mainnet_addresses_match_bip173_and_bip350_vectors() {
    let p2wpkh =
        bcs::to_bytes(&hex::decode("751e76e8199196d454941c45d1b3a323f1433bd6").unwrap()).unwrap();
    let p2tr = bcs::to_bytes(
        &hex::decode("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798").unwrap(),
    )
    .unwrap();
    assert_eq!(
        encode_bitcoin_address(&p2wpkh, BitcoinNetwork::Mainnet).unwrap(),
        "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
    );
    assert_eq!(
        encode_bitcoin_address(&p2tr, BitcoinNetwork::Mainnet).unwrap(),
        "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"
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
fn withdrawal_with_unsupported_address_length_fails() {
    let mut tx = withdrawal_tx();
    programmable(&mut tx).inputs[usize::from(BITCOIN_ADDRESS_INPUT)] =
        CallArg::Pure(bcs::to_bytes(&vec![0u8; 25]).unwrap());

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Bitcoin address must be a 20-byte P2WPKH or 32-byte P2TR program, got 25 bytes",
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
fn withdrawal_address_rewritten_by_an_earlier_command_fails() {
    let mut tx = withdrawal_tx();
    insert_command(
        &mut tx,
        0,
        framework_call(
            STDLIB,
            "vector",
            "reverse",
            Some("u8"),
            vec![Argument::Input(BITCOIN_ADDRESS_INPUT)],
        ),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "`bitcoin_address` is passed to command 0, which may modify it before command 4 consumes it",
    );
}

#[test]
fn withdrawal_split_amount_rewritten_by_an_earlier_command_fails() {
    let mut tx = withdrawal_tx();
    insert_command(
        &mut tx,
        0,
        tamper_call("bump", vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "`split amount` is passed to command 0, which may modify it before command 2 consumes it",
    );
}

#[test]
fn withdrawal_coin_merged_after_the_split_fails() {
    let mut tx = withdrawal_tx();
    let split_coin = Argument::NestedResult(u16::try_from(SPLIT_COMMAND).unwrap(), 0);
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND,
        Command::MergeCoins(split_coin, vec![Argument::Input(HBTC_COIN_INPUT)]),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "`withdrawal coin` is passed to command 2, which may modify it before command 3 consumes it",
    );
}

#[test]
fn withdrawal_coin_resplit_after_the_split_fails() {
    let mut tx = withdrawal_tx();
    let split_coin = Argument::NestedResult(u16::try_from(SPLIT_COMMAND).unwrap(), 0);
    let resplit_amount = push_pure_input(&mut tx, bcs::to_bytes(&1u64).unwrap());
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND,
        Command::SplitCoins(split_coin, vec![resplit_amount]),
    );
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND + 1,
        Command::MergeCoins(
            Argument::Input(HBTC_COIN_INPUT),
            vec![Argument::NestedResult(
                u16::try_from(INTO_BALANCE_COMMAND).unwrap(),
                0,
            )],
        ),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 2),
        "`withdrawal coin` is passed to command 2, which may modify it before command 4 consumes it",
    );
}

#[test]
fn withdrawal_balance_passed_to_an_unknown_call_before_the_withdrawal_fails() {
    let mut tx = withdrawal_tx();
    insert_command(
        &mut tx,
        WITHDRAWAL_COMMAND,
        tamper_call("skim", vec![result(INTO_BALANCE_COMMAND)]),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "`btc` is passed to command 3, which may modify it before command 4 consumes it",
    );
}

#[test]
fn withdrawal_balance_read_by_balance_value_before_the_withdrawal_renders() {
    let mut tx = withdrawal_tx();
    insert_command(
        &mut tx,
        WITHDRAWAL_COMMAND,
        framework_call(
            SUI_FRAMEWORK,
            "balance",
            "value",
            Some(&hbtc_type()),
            vec![result(INTO_BALANCE_COMMAND)],
        ),
    );

    let fields = visualize(tx, WITHDRAWAL_COMMAND + 1).unwrap();
    assert_eq!(preview_text(&fields[0]), expected_withdrawal_preview());
    assert_eq!(
        field_values(&fields[0], "Withdrawal Amount (sats)"),
        vec!["24185763".to_string()]
    );
}

#[test]
fn withdrawal_balance_passed_to_a_foreign_balance_value_fails() {
    let mut tx = withdrawal_tx();
    insert_command(
        &mut tx,
        WITHDRAWAL_COMMAND,
        framework_call(
            0x42,
            "balance",
            "value",
            Some(&hbtc_type()),
            vec![result(INTO_BALANCE_COMMAND)],
        ),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "`btc` is passed to command 3, which may modify it before command 4 consumes it",
    );
}

#[test]
fn withdrawal_coin_read_by_coin_value_before_into_balance_renders() {
    let mut tx = withdrawal_tx();
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND,
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "value",
            Some(&hbtc_type()),
            vec![Argument::NestedResult(
                u16::try_from(SPLIT_COMMAND).unwrap(),
                0,
            )],
        ),
    );

    let fields = visualize(tx, WITHDRAWAL_COMMAND + 1).unwrap();
    assert_eq!(preview_text(&fields[0]), expected_withdrawal_preview());
}

#[test]
fn withdrawal_split_amount_reused_by_value_before_the_split_renders() {
    let mut tx = withdrawal_tx();
    let other_coin = push_pure_input(&mut tx, bcs::to_bytes(&0u64).unwrap());
    insert_command(
        &mut tx,
        0,
        Command::MakeMoveVec(None, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
    );
    insert_command(
        &mut tx,
        1,
        Command::SplitCoins(other_coin, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
    );

    let fields = visualize(tx, WITHDRAWAL_COMMAND + 2).unwrap();
    assert_eq!(preview_text(&fields[0]), expected_withdrawal_preview());
}

#[test]
fn withdrawal_address_modified_after_the_withdrawal_renders() {
    let mut tx = withdrawal_tx();
    programmable(&mut tx).commands.push(framework_call(
        STDLIB,
        "vector",
        "reverse",
        Some("u8"),
        vec![Argument::Input(BITCOIN_ADDRESS_INPUT)],
    ));

    let fields = visualize(tx, WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(preview_text(&fields[0]), expected_withdrawal_preview());
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

// `Result(n)` and `NestedResult(n, 0)` name the same single-output coin, so a
// merge through one spelling counts as a second use of the other.
#[test]
fn withdrawal_coin_aliased_by_result_and_nested_result_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).arguments[0] = result(SPLIT_COMMAND);
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND,
        Command::MergeCoins(
            Argument::Input(HBTC_COIN_INPUT),
            vec![Argument::NestedResult(
                u16::try_from(SPLIT_COMMAND).unwrap(),
                0,
            )],
        ),
    );

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "`withdrawal coin` is passed to command 2, which may modify it before command 3 consumes it",
    );
}

#[test]
fn withdrawal_balance_from_a_foreign_into_balance_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).package = ObjectID::from_single_byte(0x42);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn withdrawal_balance_from_a_foreign_module_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).module = "balance".to_string();

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn withdrawal_funded_by_the_gas_coin_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).arguments[0] = Argument::GasCoin;

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "The gas coin cannot fund an hBTC withdrawal",
    );
}

#[test]
fn withdrawal_split_from_the_gas_coin_fails() {
    let mut tx = withdrawal_tx();
    let Command::SplitCoins(source, _) = &mut programmable(&mut tx).commands[SPLIT_COMMAND] else {
        panic!("expected SplitCoins at command {SPLIT_COMMAND}");
    };
    *source = Argument::GasCoin;

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "The gas coin cannot fund an hBTC withdrawal",
    );
}

#[test]
fn withdrawal_split_from_a_coin_split_off_the_gas_coin_fails() {
    let mut tx = withdrawal_tx();
    let pt = programmable(&mut tx);
    pt.commands[0] =
        Command::SplitCoins(Argument::GasCoin, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]);
    let Command::SplitCoins(source, _) = &mut pt.commands[SPLIT_COMMAND] else {
        panic!("expected SplitCoins at command {SPLIT_COMMAND}");
    };
    *source = Argument::NestedResult(0, 0);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "The gas coin cannot fund an hBTC withdrawal",
    );
}

#[test]
fn withdrawal_split_from_a_redeemed_coin_renders_the_split_amount() {
    let mut tx = top_up_withdrawal_tx();
    let split_amount = push_pure_input(&mut tx, bcs::to_bytes(&100_000_000u64).unwrap());
    let pt = programmable(&mut tx);
    let redeemed = result(0);
    pt.commands = vec![
        pt.commands[0].clone(),
        Command::SplitCoins(redeemed, vec![split_amount]),
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "into_balance",
            Some(&hbtc_type()),
            vec![Argument::NestedResult(1, 0)],
        ),
        {
            let Command::MoveCall(mut call) = pt.commands[5].clone() else {
                panic!("expected request_withdrawal at command 5");
            };
            call.arguments[2] = result(2);
            Command::MoveCall(call)
        },
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "send_funds",
            Some(&hbtc_type()),
            vec![redeemed, Argument::Input(6)],
        ),
    ];

    let fields = visualize(tx, 3).unwrap();
    assert_eq!(
        preview_text(&fields[0]),
        (
            "Hashi Withdraw 1 hBTC (Bitcoin Signet)".to_string(),
            "To tb1p4ctlhtn9k8qvlk90ukpresd5xx78mycevcdnw9hdgyuvxvf47dtq2epyfv".to_string()
        )
    );
}

#[test]
fn address_balance_withdrawal_shows_the_reserved_amount() {
    let fields = visualize(redeem_withdrawal_tx(), REDEEM_WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        preview_text(&fields[0]),
        (
            "Hashi Withdraw 0.1 hBTC (Bitcoin Signet)".to_string(),
            "To tb1qht7wmzjggxl9je0zw7fl42rjjqh9w4ka2lwv3u".to_string()
        )
    );
    assert_eq!(
        field_values(&fields[0], "Withdrawal Amount (sats)"),
        vec!["10000000".to_string()]
    );
}

#[test]
fn address_balance_withdrawal_of_a_foreign_coin_type_fails() {
    let mut tx = redeem_withdrawal_tx();
    programmable(&mut tx).inputs[usize::from(FUNDS_WITHDRAWAL_INPUT)] =
        CallArg::FundsWithdrawal(FundsWithdrawalArg::balance_from_sender(
            10_000_000,
            sui_types::parse_sui_type_tag("0x2::sui::SUI").unwrap(),
        ));

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        &format!("Funds withdrawal is not denominated in {HBTC_TYPE_ORIGIN}::btc::BTC"),
    );
}

#[test]
fn address_balance_withdrawal_from_the_sponsor_fails() {
    let mut tx = redeem_withdrawal_tx();
    programmable(&mut tx).inputs[usize::from(FUNDS_WITHDRAWAL_INPUT)] =
        CallArg::FundsWithdrawal(FundsWithdrawalArg::balance_from_sponsor(
            10_000_000,
            sui_types::parse_sui_type_tag(&hbtc_type()).unwrap(),
        ));

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        "Withdrawing from the sponsor's address balance is not supported",
    );
}

#[test]
fn address_balance_withdrawal_narrowed_before_redeem_fails() {
    let mut tx = redeem_withdrawal_tx();
    let sub_limit = push_pure_input(&mut tx, vec![0u8; 32]);
    insert_command(
        &mut tx,
        REDEEM_COMMAND,
        framework_call(
            SUI_FRAMEWORK,
            "funds_accumulator",
            "withdrawal_split",
            Some(&format!("0x2::balance::Balance<{}>", hbtc_type())),
            vec![Argument::Input(FUNDS_WITHDRAWAL_INPUT), sub_limit],
        ),
    );

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND + 1),
        "`withdrawal` is passed to command 0, which may modify it before command 1 consumes it",
    );
}

#[test]
fn address_balance_withdrawal_widened_before_redeem_fails() {
    let mut tx = redeem_withdrawal_tx();
    let extra = push_pure_input(&mut tx, vec![0u8; 32]);
    insert_command(
        &mut tx,
        REDEEM_COMMAND,
        framework_call(
            SUI_FRAMEWORK,
            "funds_accumulator",
            "withdrawal_join",
            Some(&format!("0x2::balance::Balance<{}>", hbtc_type())),
            vec![Argument::Input(FUNDS_WITHDRAWAL_INPUT), extra],
        ),
    );

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND + 1),
        "`withdrawal` is passed to command 0, which may modify it before command 1 consumes it",
    );
}

#[test]
fn address_balance_withdrawal_redeemed_as_a_foreign_type_fails() {
    let mut tx = redeem_withdrawal_tx();
    move_call(&mut tx, REDEEM_COMMAND).type_arguments[0] =
        sui_types::parse_sui_type_tag("0x2::sui::SUI")
            .unwrap()
            .into();

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        &format!("Withdrawal balance is not a Balance<{HBTC_TYPE_ORIGIN}::btc::BTC>"),
    );
}

#[test]
fn withdrawal_balance_from_an_unsupported_framework_call_with_a_foreign_type_fails() {
    let mut tx = withdrawal_tx();
    let call = move_call(&mut tx, INTO_BALANCE_COMMAND);
    call.function = "into_balance_lossy".to_string();
    call.type_arguments[0] = sui_types::parse_sui_type_tag("0x2::sui::SUI")
        .unwrap()
        .into();

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn address_balance_withdrawal_redeeming_a_pure_input_fails() {
    let mut tx = redeem_withdrawal_tx();
    let not_a_withdrawal = push_pure_input(&mut tx, bcs::to_bytes(&10_000_000u64).unwrap());
    move_call(&mut tx, REDEEM_COMMAND).arguments[0] = not_a_withdrawal;

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        "Redeemed withdrawal is not a funds withdrawal input",
    );
}

#[test]
fn address_balance_withdrawal_redeemed_through_a_foreign_package_fails() {
    let mut tx = redeem_withdrawal_tx();
    move_call(&mut tx, REDEEM_COMMAND).package = ObjectID::from_single_byte(0x42);

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn address_balance_withdrawal_balance_skimmed_before_the_withdrawal_fails() {
    let mut tx = redeem_withdrawal_tx();
    insert_command(
        &mut tx,
        REDEEM_WITHDRAWAL_COMMAND,
        tamper_call("skim", vec![result(REDEEM_COMMAND)]),
    );

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND + 1),
        "`btc` is passed to command 1, which may modify it before command 2 consumes it",
    );
}

#[test]
fn title_suffix_is_empty_on_mainnet_and_names_signet() {
    assert_eq!(BitcoinNetwork::Mainnet.title_suffix(), "");
    assert_eq!(BitcoinNetwork::Signet.title_suffix(), " (Bitcoin Signet)");
}

#[test]
fn withdrawal_balance_not_from_coin_into_balance_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).function = "into_balance_lossy".to_string();

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn withdrawal_of_a_whole_coin_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).arguments[0] = Argument::Input(HBTC_COIN_INPUT);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawing a whole coin is not supported: its amount is not part of the transaction",
    );
}

#[test]
fn withdrawal_coin_not_produced_by_split_coins_fails() {
    let mut tx = withdrawal_tx();
    let hbtc = hbtc_type();
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND,
        framework_call(SUI_FRAMEWORK, "coin", "zero", Some(&hbtc), vec![]),
    );
    move_call(&mut tx, INTO_BALANCE_COMMAND + 1).arguments[0] = result(INTO_BALANCE_COMMAND);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND + 1),
        "Withdrawal coin is not produced by SplitCoins or 0x2::coin::redeem_funds",
    );
}

fn redeemed_coin_withdrawal_tx(withdrawal: FundsWithdrawalArg) -> TransactionData {
    let mut tx = top_up_withdrawal_tx();
    let pt = programmable(&mut tx);
    pt.inputs[TOP_UP_FUNDS_WITHDRAWAL_INPUT] = CallArg::FundsWithdrawal(withdrawal);
    pt.commands = vec![
        pt.commands[0].clone(),
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "into_balance",
            Some(&hbtc_type()),
            vec![result(0)],
        ),
        {
            let Command::MoveCall(mut call) = pt.commands[5].clone() else {
                panic!("expected request_withdrawal at command 5");
            };
            call.arguments[2] = result(1);
            Command::MoveCall(call)
        },
    ];
    tx
}

fn hbtc_from_sender(amount: u64) -> FundsWithdrawalArg {
    FundsWithdrawalArg::balance_from_sender(
        amount,
        sui_types::parse_sui_type_tag(&hbtc_type()).unwrap(),
    )
}

#[test]
fn withdrawal_of_a_redeemed_coin_shows_the_reserved_amount() {
    let fields = visualize(
        redeemed_coin_withdrawal_tx(hbtc_from_sender(25_000_000)),
        REDEEMED_COIN_WITHDRAWAL_COMMAND,
    )
    .unwrap();
    assert_eq!(
        preview_text(&fields[0]).0,
        "Hashi Withdraw 0.25 hBTC (Bitcoin Signet)"
    );
    assert_eq!(
        field_values(&fields[0], "Withdrawal Amount (sats)"),
        vec!["25000000".to_string()]
    );
}

#[test]
fn withdrawal_of_a_redeemed_coin_of_a_foreign_type_fails() {
    let tx = redeemed_coin_withdrawal_tx(FundsWithdrawalArg::balance_from_sender(
        25_000_000,
        sui_types::parse_sui_type_tag("0x2::sui::SUI").unwrap(),
    ));

    expect_error(
        visualize(tx, REDEEMED_COIN_WITHDRAWAL_COMMAND),
        &format!("Funds withdrawal is not denominated in {HBTC_TYPE_ORIGIN}::btc::BTC"),
    );
}

#[test]
fn withdrawal_of_a_redeemed_coin_split_before_use_fails() {
    let mut tx = redeemed_coin_withdrawal_tx(hbtc_from_sender(25_000_000));
    insert_command(
        &mut tx,
        REDEEMED_COIN_INTO_BALANCE_COMMAND,
        Command::SplitCoins(result(0), vec![Argument::Input(TOP_UP_SPLIT_AMOUNT_INPUT)]),
    );

    expect_error(
        visualize(tx, REDEEMED_COIN_WITHDRAWAL_COMMAND + 1),
        "`withdrawal coin` is passed to command 1, which may modify it before command 2 consumes it",
    );
}

#[test]
fn withdrawal_of_a_redeemed_coin_merged_into_before_use_fails() {
    let mut tx = redeemed_coin_withdrawal_tx(hbtc_from_sender(25_000_000));
    insert_command(
        &mut tx,
        REDEEMED_COIN_INTO_BALANCE_COMMAND,
        Command::MergeCoins(result(0), vec![Argument::Input(TOP_UP_HBTC_COIN_INPUT)]),
    );

    expect_error(
        visualize(tx, REDEEMED_COIN_WITHDRAWAL_COMMAND + 1),
        "`withdrawal coin` is passed to command 1, which may modify it before command 2 consumes it",
    );
}

#[test]
fn withdrawal_of_a_redeemed_coin_from_the_sponsor_fails() {
    let tx = redeemed_coin_withdrawal_tx(FundsWithdrawalArg::balance_from_sponsor(
        25_000_000,
        sui_types::parse_sui_type_tag(&hbtc_type()).unwrap(),
    ));

    expect_error(
        visualize(tx, REDEEMED_COIN_WITHDRAWAL_COMMAND),
        "Withdrawing from the sponsor's address balance is not supported",
    );
}

#[test]
fn withdrawal_with_a_split_amount_index_out_of_range_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).arguments[0] =
        Argument::NestedResult(u16::try_from(SPLIT_COMMAND).unwrap(), 1);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "SplitCoins amount for the withdrawal coin not found",
    );
}

#[test]
fn withdrawal_of_a_foreign_coin_type_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).type_arguments[0] =
        sui_types::parse_sui_type_tag("0x2::sui::SUI")
            .unwrap()
            .into();

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        &format!("Withdrawal balance is not a Balance<{HBTC_TYPE_ORIGIN}::btc::BTC>"),
    );
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
fn a_call_to_the_disabled_first_package_version_is_not_claimed() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, WITHDRAWAL_COMMAND).package =
        ObjectID::from_hex_literal(HBTC_TYPE_ORIGIN).unwrap();
    let block_data = SuiTransactionBlockData::try_from_with_module_cache(
        tx,
        &SyncModuleCache::new(SuiModuleResolver),
    )
    .unwrap();
    let SuiTransactionBlockKind::ProgrammableTransaction(pt) = block_data.transaction() else {
        panic!("expected a programmable transaction");
    };
    let context = VisualizerContext::new(
        block_data.sender(),
        WITHDRAWAL_COMMAND,
        &pt.commands,
        &pt.inputs,
    );

    assert!(!HashiVisualizer.can_handle(&context));
}

#[test]
fn full_payload_includes_the_hashi_preview() {
    for (section, function, digest, label) in [
        ("deposit", "deposit", DEPOSIT_DIGEST, "Hashi Deposit"),
        (
            "withdraw",
            "request_withdrawal",
            WITHDRAWAL_DIGEST,
            "Hashi Withdrawal",
        ),
    ] {
        let payload =
            crate::utils::payload_from_b64(&encoded_fixture_tx(section, function, digest));
        let hashi_fields = payload
            .fields
            .iter()
            .filter(|field| field.label() == label)
            .count();
        assert_eq!(
            hashi_fields, 1,
            "{label} missing from full payload for {digest}"
        );
    }
}

#[test]
fn format_btc_trims_trailing_zeros() {
    assert_eq!(format_btc(0), "0");
    assert_eq!(format_btc(546), "0.00000546");
    assert_eq!(format_btc(100_000_000), "1");
    assert_eq!(format_btc(150_000_000), "1.5");
    assert_eq!(format_btc(u64::MAX), "184467440737.09551615");
}
