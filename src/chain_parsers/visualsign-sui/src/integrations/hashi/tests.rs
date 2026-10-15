use super::*;

use super::config::SuiNetwork;
use crate::core::SuiModuleResolver;
use crate::utils::run_aggregated_fixture;

use base64::Engine;
use move_bytecode_utils::module_cache::SyncModuleCache;
use sui_json_rpc_types::{
    SuiTransactionBlockData, SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::base_types::SequenceNumber;
use sui_types::transaction::{
    Argument, CallArg, Command, FundsWithdrawalArg, ObjectArg, ProgrammableMoveCall,
    ProgrammableTransaction, SharedObjectMutability, TransactionData, TransactionDataAPI,
    TransactionKind,
};
use visualsign::SignablePayload;
use visualsign::test_utils::check_signable_payload_field;
use visualsign::vsptrait::VisualSignOptions;

const FIXTURE: &str = include_str!("aggregated_test_data.json");
const HASHI_PACKAGE: &str = "0x5e0b99c31f691742acea5de4ca1bf53c398d5639fed6503a51ffe8035dc68485";
// Version 1 of the package, so it also defines `btc::BTC`.
const HBTC_TYPE_ORIGIN: &str = HASHI_PACKAGE;
const DEPOSIT_DIGEST: &str = "4WGVP4rYLHxHCoA9jFjFFCoomNMxmKAVnSJsdGhMUSBF";

// Real deposit PTB: 0 utxo_id(I0 txid, I1 vout), 1 utxo(R0, I2 amount, I3 path),
// 2 deposit(I4 hashi, R1, I5 clock).
const UTXO_ID_COMMAND: usize = 0;
const UTXO_COMMAND: usize = 1;
const DEPOSIT_COMMAND: usize = 2;
const TXID_INPUT: u16 = 0;
const VOUT_INPUT: u16 = 1;
const DEPOSIT_AMOUNT_INPUT: u16 = 2;
const DERIVATION_PATH_INPUT: usize = 3;

// Coin-funded withdrawal, built from the real address-balance withdrawal by
// `withdrawal_tx`: 0 MergeCoins(I0, [I1]), 1 SplitCoins(I0, [I2 amount]),
// 2 into_balance<BTC>(NR(1,0)), 3 request_withdrawal(I3 hashi, I4 clock, R2, I5 addr).
const SPLIT_COMMAND: usize = 1;
const INTO_BALANCE_COMMAND: usize = 2;
const WITHDRAWAL_COMMAND: usize = 3;
const HBTC_COIN_INPUT: u16 = 0;
const SPLIT_AMOUNT_INPUT: u16 = 2;
const BITCOIN_ADDRESS_INPUT: u16 = 5;

// Real address-balance withdrawal: 0 balance::redeem_funds<BTC>(I3 funds withdrawal),
// 1 request_withdrawal(I0 hashi, I1 clock, R0, I2 addr).
const REDEEM_WITHDRAWAL_DIGEST: &str = "58XSeQkQffKNz1TJ4vJmNaeMvEm6N6LCN889YRi8yhhW";
const REDEEM_COMMAND: usize = 0;
const REDEEM_WITHDRAWAL_COMMAND: usize = 1;
const FUNDS_WITHDRAWAL_INPUT: u16 = 3;

// Withdrawal from a coin topped up from the address balance, built from the real
// address-balance withdrawal by `top_up_withdrawal_tx`: 0 coin::redeem_funds<BTC>(I4),
// 1 MergeCoins(I3, [R0]), 2 SplitCoins(I3, [I5 amount]), 3 into_balance(NR(2,0)),
// 4 send_funds(I3, I6), 5 request_withdrawal(I0 hashi, I1 clock, NR(3,0), I2 addr).
const TOP_UP_WITHDRAWAL_COMMAND: usize = 5;
const TOP_UP_FUNDS_WITHDRAWAL_INPUT: usize = 4;
const TOP_UP_HBTC_COIN_INPUT: u16 = 3;
const TOP_UP_SPLIT_AMOUNT_INPUT: u16 = 5;
const TOP_UP_CHANGE_RECIPIENT_INPUT: usize = 6;

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

// Amounts and coins of the coin-funded withdrawals built below. Every other
// input (the `Hashi` object, the clock, the Bitcoin address, the sender and gas)
// is the real address-balance withdrawal's.
const SPLIT_AMOUNT_SATS: u64 = 24_185_763;
const TOP_UP_AMOUNT_SATS: u64 = 143_809_724;
const TOP_UP_REDEEMED_SATS: u64 = 138_017_594;

fn owned_coin(byte: u8) -> CallArg {
    CallArg::Object(ObjectArg::ImmOrOwnedObject((
        ObjectID::from_single_byte(byte),
        SequenceNumber::from_u64(1),
        sui_types::digests::ObjectDigest::new([byte; 32]),
    )))
}

fn real_request_withdrawal(tx: &TransactionData) -> ProgrammableMoveCall {
    let TransactionKind::ProgrammableTransaction(pt) = tx.kind() else {
        panic!("expected a programmable transaction");
    };
    let Command::MoveCall(call) = &pt.commands[REDEEM_WITHDRAWAL_COMMAND] else {
        panic!("expected request_withdrawal at command {REDEEM_WITHDRAWAL_COMMAND}");
    };
    (**call).clone()
}

/// The real withdrawal's inputs are `I0 hashi, I1 clock, I2 address,
/// I3 funds withdrawal`.
fn real_withdrawal_inputs(tx: &TransactionData) -> (CallArg, CallArg, CallArg, CallArg) {
    let TransactionKind::ProgrammableTransaction(pt) = tx.kind() else {
        panic!("expected a programmable transaction");
    };
    (
        pt.inputs[0].clone(),
        pt.inputs[1].clone(),
        pt.inputs[2].clone(),
        pt.inputs[usize::from(FUNDS_WITHDRAWAL_INPUT)].clone(),
    )
}

fn withdrawal_tx() -> TransactionData {
    let mut tx = redeem_withdrawal_tx();
    let (hashi, clock, address, _) = real_withdrawal_inputs(&tx);
    let mut request = real_request_withdrawal(&tx);
    request.arguments = vec![
        Argument::Input(3),
        Argument::Input(4),
        result(INTO_BALANCE_COMMAND),
        Argument::Input(BITCOIN_ADDRESS_INPUT),
    ];
    let pt = programmable(&mut tx);
    pt.inputs = vec![
        owned_coin(0xa1),
        owned_coin(0xa2),
        CallArg::Pure(bcs::to_bytes(&SPLIT_AMOUNT_SATS).unwrap()),
        hashi,
        clock,
        address,
    ];
    pt.commands = vec![
        Command::MergeCoins(
            Argument::Input(HBTC_COIN_INPUT),
            vec![Argument::Input(1)],
        ),
        Command::SplitCoins(
            Argument::Input(HBTC_COIN_INPUT),
            vec![Argument::Input(SPLIT_AMOUNT_INPUT)],
        ),
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "into_balance",
            Some(&hbtc_type()),
            vec![Argument::NestedResult(1, 0)],
        ),
        Command::MoveCall(Box::new(request)),
    ];
    tx
}

fn redeem_withdrawal_tx() -> TransactionData {
    fixture_tx("withdraw", "request_withdrawal", REDEEM_WITHDRAWAL_DIGEST)
}

fn top_up_withdrawal_tx() -> TransactionData {
    let mut tx = redeem_withdrawal_tx();
    let (hashi, clock, address, _) = real_withdrawal_inputs(&tx);
    let sender = tx.sender();
    let mut request = real_request_withdrawal(&tx);
    request.arguments = vec![
        Argument::Input(0),
        Argument::Input(1),
        Argument::NestedResult(3, 0),
        Argument::Input(2),
    ];
    let pt = programmable(&mut tx);
    pt.inputs = vec![
        hashi,
        clock,
        address,
        owned_coin(0xb3),
        CallArg::FundsWithdrawal(hbtc_from_sender(TOP_UP_REDEEMED_SATS)),
        CallArg::Pure(bcs::to_bytes(&TOP_UP_AMOUNT_SATS).unwrap()),
        CallArg::Pure(bcs::to_bytes(&sender).unwrap()),
    ];
    pt.commands = vec![
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "redeem_funds",
            Some(&hbtc_type()),
            vec![Argument::Input(4)],
        ),
        Command::MergeCoins(
            Argument::Input(TOP_UP_HBTC_COIN_INPUT),
            vec![result(0)],
        ),
        Command::SplitCoins(
            Argument::Input(TOP_UP_HBTC_COIN_INPUT),
            vec![Argument::Input(TOP_UP_SPLIT_AMOUNT_INPUT)],
        ),
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "into_balance",
            Some(&hbtc_type()),
            vec![Argument::NestedResult(2, 0)],
        ),
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "send_funds",
            Some(&hbtc_type()),
            vec![
                Argument::Input(TOP_UP_HBTC_COIN_INPUT),
                Argument::Input(6),
            ],
        ),
        Command::MoveCall(Box::new(request)),
    ];
    tx
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

fn card_headline(field: &AnnotatedPayloadField) -> (String, String, String) {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    let condensed = &preview_layout.condensed.as_ref().unwrap().fields;
    let value_of = |label_prefix: &str| {
        let row = condensed
            .iter()
            .map(|f| &f.signable_payload_field)
            .find(|f| f.label().starts_with(label_prefix))
            .unwrap_or_else(|| panic!("no collapsed {label_prefix:?} row"));
        check_signable_payload_field(row, row.label()).1.remove(0)
    };
    (
        preview_layout.title.as_ref().unwrap().text.clone(),
        value_of("Amount"),
        value_of("Recipient Address"),
    )
}

fn headline(title: &str, amount: &str, recipient: &str) -> (String, String, String) {
    (title.to_string(), amount.to_string(), recipient.to_string())
}

fn field_values(field: &AnnotatedPayloadField, label: &str) -> Vec<String> {
    let (found, values) = check_signable_payload_field(&field.signable_payload_field, label);
    assert!(found, "no {label:?} field rendered");
    values
}

const WITHDRAWAL_TITLE: &str = "Bridge hBTC from Sui Testnet to Bitcoin Signet via Hashi";
const DEPOSIT_TITLE: &str = "Bridge BTC from Bitcoin Signet to Sui Testnet via Hashi";
const DEPOSIT_SUBTITLE: &str =
    "0.00097291 BTC to 0xd5a05ab3ae8731246fa0f3bb0d91725e4f13f778c864290c7553216a56e2e900";
const WITHDRAWAL_SUBTITLE: &str =
    "0.24185763 hBTC to tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk";
const DEPOSIT_OUTPUT: &str = "ae41124c332b7e50e9b47c4304af3fa6768f2386f27880eaff3b650baa1f3f0a:4";

fn expected_deposit_headline() -> (String, String, String) {
    headline(
        DEPOSIT_TITLE,
        "0.00097291",
        "0xd5a05ab3ae8731246fa0f3bb0d91725e4f13f778c864290c7553216a56e2e900",
    )
}

fn expected_withdrawal_headline() -> (String, String, String) {
    headline(
        WITHDRAWAL_TITLE,
        "0.24185763",
        "tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk",
    )
}

fn layout_labels(field: &AnnotatedPayloadField) -> (Vec<String>, Vec<String>) {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    let labels = |list: &Option<visualsign::SignablePayloadFieldListLayout>| {
        list.as_ref()
            .unwrap()
            .fields
            .iter()
            .map(|f| f.signable_payload_field.label().clone())
            .collect()
    };
    (
        labels(&preview_layout.condensed),
        labels(&preview_layout.expanded),
    )
}

fn payload_of(tx: &TransactionData) -> SignablePayload {
    crate::utils::payload_from_b64(
        &base64::engine::general_purpose::STANDARD.encode(bcs::to_bytes(tx).unwrap()),
    )
}

fn top_level_labels(payload: &SignablePayload) -> Vec<String> {
    payload
        .fields
        .iter()
        .map(|field| field.label().clone())
        .collect()
}

fn address_name(fields: &[SignablePayloadField], label: &str) -> Option<String> {
    fields.iter().find_map(|field| match field {
        SignablePayloadField::AddressV2 { common, address_v2 } if common.label == label => {
            Some(address_v2.name.clone())
        }
        SignablePayloadField::PreviewLayout { preview_layout, .. } => {
            [&preview_layout.condensed, &preview_layout.expanded]
                .into_iter()
                .flatten()
                .find_map(|list| {
                    let nested: Vec<SignablePayloadField> = list
                        .fields
                        .iter()
                        .map(|f| f.signable_payload_field.clone())
                        .collect();
                    address_name(&nested, label)
                })
        }
        _ => None,
    })
}

fn deposit_to(recipient: Option<[u8; 32]>) -> TransactionData {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[DERIVATION_PATH_INPUT] =
        CallArg::Pure(bcs::to_bytes(&recipient).unwrap());
    tx
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
    assert_eq!(card_headline(&fields[0]), expected_deposit_headline());
}

#[test]
fn taproot_withdrawal_address_renders_as_a_full_bech32m_address() {
    // A 32-byte witness program is paid as P2TR, so it renders with the
    // bech32m `tb1p` prefix.
    let program: Vec<u8> =
        hex::decode("a790d556b86a2a295e9e32b54febbf5c2983e94ffd4762a9c009a510d9f9ee05").unwrap();
    let mut tx = withdrawal_tx();
    programmable(&mut tx).inputs[usize::from(BITCOIN_ADDRESS_INPUT)] =
        CallArg::Pure(bcs::to_bytes(&program).unwrap());
    let fields = visualize(tx, WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(
            WITHDRAWAL_TITLE,
            "0.24185763",
            "tb1p57gd244cdg4zjh57x265l6alts5c8620l4rk92wqpxj3pk0eaczsau385x"
        )
    );
}

#[test]
fn withdrawal_title_names_amount_and_full_bitcoin_recipient() {
    let fields = visualize(withdrawal_tx(), WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(card_headline(&fields[0]), expected_withdrawal_headline());
}

#[test]
fn deposit_without_derivation_path_warns_in_the_title_and_recipient_row() {
    let fields = visualize(deposit_to(None), DEPOSIT_COMMAND).unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(
            "Warning: Hashi deposit from Bitcoin Signet credits no one",
            "0.00097291",
            "None: no hBTC will be credited"
        )
    );
    assert_eq!(layout_labels(&fields[0]).0, DEPOSIT_KEY_ROWS);
}

#[test]
fn withdrawal_to_p2wpkh_encodes_bech32_v0() {
    let mut tx = withdrawal_tx();
    programmable(&mut tx).inputs[usize::from(BITCOIN_ADDRESS_INPUT)] =
        CallArg::Pure(bcs::to_bytes(&vec![0x75u8; 20]).unwrap());

    let fields = visualize(tx, WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        field_values(&fields[0], "Recipient Address"),
        ["tb1qw46h2at4w46h2at4w46h2at4w46h2at4qy2ul6"; 2]
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
fn sui_network_display_names() {
    assert_eq!(SuiNetwork::Mainnet.display_name(), "Sui");
    assert_eq!(SuiNetwork::Testnet.display_name(), "Sui Testnet");
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
    assert_eq!(card_headline(&fields[0]), expected_withdrawal_headline());
    assert_eq!(field_values(&fields[0], "Amount (sats)"), ["24185763"]);
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
    assert_eq!(card_headline(&fields[0]), expected_withdrawal_headline());
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
    assert_eq!(card_headline(&fields[0]), expected_withdrawal_headline());
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
    assert_eq!(card_headline(&fields[0]), expected_withdrawal_headline());
}

/// The real deposit plus a second deposit of output 2 of the same Bitcoin
/// transaction, reusing the first deposit's txid, amount and recipient inputs.
fn batched_deposit_tx() -> TransactionData {
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
    tx
}

#[test]
fn batched_deposits_sharing_txid_amount_and_recipient_inputs_render() {
    let tx = batched_deposit_tx();
    for (command, vout) in [(DEPOSIT_COMMAND, "4"), (DEPOSIT_COMMAND + 3, "2")] {
        let fields = visualize(tx.clone(), command).unwrap();
        assert_eq!(card_headline(&fields[0]), expected_deposit_headline());
        assert_eq!(
            field_values(&fields[0], "Bitcoin Output"),
            [format!(
                "ae41124c332b7e50e9b47c4304af3fa6768f2386f27880eaff3b650baa1f3f0a:{vout}"
            )]
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

fn split_from_redeemed_coin_tx(withdrawal: FundsWithdrawalArg) -> TransactionData {
    let mut tx = top_up_withdrawal_tx();
    let split_amount = push_pure_input(&mut tx, bcs::to_bytes(&100_000_000u64).unwrap());
    let pt = programmable(&mut tx);
    pt.inputs[TOP_UP_FUNDS_WITHDRAWAL_INPUT] = CallArg::FundsWithdrawal(withdrawal);
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
    tx
}

#[test]
fn withdrawal_split_from_a_redeemed_coin_renders_the_split_amount() {
    let fields = visualize(
        split_from_redeemed_coin_tx(hbtc_from_sender(150_000_000)),
        3,
    )
    .unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(
            WITHDRAWAL_TITLE,
            "1",
            "tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk"
        )
    );
}

#[test]
fn withdrawal_split_from_a_coin_redeemed_from_the_sponsor_fails() {
    expect_error(
        visualize(
            split_from_redeemed_coin_tx(hbtc_from_sponsor(150_000_000)),
            3,
        ),
        "Withdrawing from the sponsor's address balance is not supported",
    );
}

#[test]
fn top_up_withdrawal_funded_from_the_sponsor_fails() {
    let mut tx = top_up_withdrawal_tx();
    programmable(&mut tx).inputs[TOP_UP_FUNDS_WITHDRAWAL_INPUT] =
        CallArg::FundsWithdrawal(hbtc_from_sponsor(150_000_000));

    expect_error(
        visualize(tx, TOP_UP_WITHDRAWAL_COMMAND),
        "Withdrawing from the sponsor's address balance is not supported",
    );
}

#[test]
fn address_balance_withdrawal_shows_the_reserved_amount() {
    let fields = visualize(redeem_withdrawal_tx(), REDEEM_WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(WITHDRAWAL_TITLE, "0.0003", "tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk")
    );
    assert_eq!(field_values(&fields[0], "Amount (sats)"), ["30000"]);
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
fn titles_name_both_networks_of_the_deployment() {
    let deployment = deployment_for(&ObjectID::from_hex_literal(HASHI_PACKAGE).unwrap()).unwrap();
    assert_eq!(withdrawal_title(deployment), WITHDRAWAL_TITLE);
    assert_eq!(deposit_title(deployment), DEPOSIT_TITLE);
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

fn hbtc_from_sponsor(amount: u64) -> FundsWithdrawalArg {
    FundsWithdrawalArg::balance_from_sponsor(
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
        card_headline(&fields[0]),
        headline(
            WITHDRAWAL_TITLE,
            "0.25",
            "tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk"
        )
    );
    assert_eq!(field_values(&fields[0], "Amount (sats)"), ["25000000"]);
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

/// Not a real deployment: stands in for a future mainnet entry in [`config::DEPLOYMENTS`].
const MAINNET_STAND_IN: HashiDeployment = HashiDeployment {
    package_id: "0xabababababababababababababababababababababababababababababababab",
    type_origin_id: "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
    sui_network: SuiNetwork::Mainnet,
    bitcoin_network: BitcoinNetwork::Mainnet,
};

fn hashi_can_handle(tx: TransactionData, command: usize) -> bool {
    let block_data = SuiTransactionBlockData::try_from_with_module_cache(
        tx,
        &SyncModuleCache::new(SuiModuleResolver),
    )
    .unwrap();
    let SuiTransactionBlockKind::ProgrammableTransaction(pt) = block_data.transaction() else {
        panic!("expected a programmable transaction");
    };
    let context = VisualizerContext::new(block_data.sender(), command, &pt.commands, &pt.inputs);
    HashiVisualizer.can_handle(&context)
}

#[test]
fn calls_to_a_configured_package_are_handled_and_others_are_not() {
    assert!(hashi_can_handle(deposit_tx(), DEPOSIT_COMMAND));
    assert!(hashi_can_handle(withdrawal_tx(), WITHDRAWAL_COMMAND));

    let mut unconfigured = withdrawal_tx();
    move_call(&mut unconfigured, WITHDRAWAL_COMMAND).package = ObjectID::from_single_byte(0x42);
    assert!(!hashi_can_handle(unconfigured, WITHDRAWAL_COMMAND));

    let mut unsupported_function = withdrawal_tx();
    move_call(&mut unsupported_function, WITHDRAWAL_COMMAND).function =
        "cancel_withdrawal".to_string();
    assert!(!hashi_can_handle(unsupported_function, WITHDRAWAL_COMMAND));
}

#[test]
fn every_deployment_is_matched_in_the_form_can_handle_receives() {
    let config = HASHI_CONFIG.get_or_init(Config::new);
    for deployment in config::DEPLOYMENTS {
        let package = ObjectID::from_hex_literal(deployment.package_id).unwrap();
        // `CommandVisualizer::can_handle` passes `to_hex_literal`, so a
        // `package_id` written in any other form would never match.
        assert_eq!(package.to_hex_literal(), deployment.package_id);
        assert!(config.can_handle(&package.to_hex_literal(), "deposit", "deposit"));
        assert!(config.can_handle(&package.to_hex_literal(), "withdraw", "request_withdrawal"));
        assert!(!config.can_handle(&package.to_hex_literal(), "withdraw", "deposit"));
        assert_eq!(
            deployment_for(&package).unwrap().package_id,
            deployment.package_id
        );
    }
}

#[test]
fn a_mainnet_deployment_needs_only_a_deployments_entry() {
    let config = Config::from_deployments(config::DEPLOYMENTS.iter().chain([&MAINNET_STAND_IN]));
    let mainnet = ObjectID::from_hex_literal(MAINNET_STAND_IN.package_id)
        .unwrap()
        .to_hex_literal();
    let testnet = ObjectID::from_hex_literal(HASHI_PACKAGE)
        .unwrap()
        .to_hex_literal();
    for package in [&mainnet, &testnet] {
        assert!(config.can_handle(package, "deposit", "deposit"));
        assert!(config.can_handle(package, "withdraw", "request_withdrawal"));
    }
    assert!(!HASHI_CONFIG
        .get_or_init(Config::new)
        .can_handle(&mainnet, "deposit", "deposit"));

    assert_eq!(
        deposit_title(&MAINNET_STAND_IN),
        "Bridge BTC from Bitcoin to Sui via Hashi"
    );
    assert_eq!(
        withdrawal_title(&MAINNET_STAND_IN),
        "Bridge hBTC from Sui to Bitcoin via Hashi"
    );
    assert_eq!(
        hbtc_coin_type(&MAINNET_STAND_IN).unwrap(),
        format!("{}::btc::BTC", MAINNET_STAND_IN.type_origin_id)
    );

    let withdrawal = Withdrawal {
        amount_sats: 150_000,
        bitcoin_address: encode_bitcoin_address(
            &bcs::to_bytes(&vec![0x75u8; 20]).unwrap(),
            MAINNET_STAND_IN.bitcoin_network,
        )
        .unwrap(),
    };
    assert!(withdrawal.bitcoin_address.starts_with("bc1q"));
    let preview = withdrawal_preview(&MAINNET_STAND_IN, &withdrawal).unwrap();
    assert_eq!(
        preview.summary.subtitle.as_deref(),
        Some(format!("0.0015 hBTC to {}", withdrawal.bitcoin_address).as_str())
    );
    // Key rows render in both the condensed and the expanded card.
    let field = preview_field(preview, "Hashi Withdrawal", Vec::new());
    assert_eq!(
        field_values(&field, "Destination Network"),
        ["Bitcoin", "Bitcoin"]
    );
}

#[test]
fn full_payload_includes_the_hashi_preview() {
    for (section, function, digest, label) in [
        ("deposit", "deposit", DEPOSIT_DIGEST, "Hashi Deposit"),
        (
            "withdraw",
            "request_withdrawal",
            REDEEM_WITHDRAWAL_DIGEST,
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

const WITHDRAWAL_KEY_ROWS: [&str; 4] = [
    "Amount",
    "Destination Network",
    "Recipient Address",
    "Bitcoin Miner Fee",
];
const DEPOSIT_KEY_ROWS: [&str; 3] = [
    "Amount (declared)",
    "Destination Network",
    "Recipient Address",
];
const WITHDRAWAL_DETAIL_ROWS: [&str; 5] = [
    "Signer",
    "Amount (sats)",
    "Source Network",
    "Bridge Object",
    "Bridge Package",
];
const DEPOSIT_DETAIL_ROWS: [&str; 6] = [
    "Signer",
    "Bitcoin Output",
    "Amount (sats)",
    "Source Network",
    "Bridge Object",
    "Bridge Package",
];

fn condensed_rows(field: &AnnotatedPayloadField) -> Vec<SignablePayloadField> {
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
        .map(|f| f.signable_payload_field.clone())
        .collect()
}

#[test]
fn single_actions_are_titled_with_the_bridge_intent_and_add_no_top_level_rows() {
    for (tx, title, subtitle, card) in [
        (
            withdrawal_tx(),
            WITHDRAWAL_TITLE,
            WITHDRAWAL_SUBTITLE,
            "Hashi Withdrawal",
        ),
        (
            deposit_tx(),
            DEPOSIT_TITLE,
            DEPOSIT_SUBTITLE,
            "Hashi Deposit",
        ),
    ] {
        let payload = payload_of(&tx);
        assert_eq!(payload.title, title);
        assert_eq!(payload.subtitle.as_deref(), Some(subtitle));
        assert_eq!(
            top_level_labels(&payload),
            ["Network", card, "Transaction Details"]
        );
    }
}

#[test]
fn cards_show_key_rows_collapsed_and_every_row_expanded() {
    let deposit = visualize(deposit_tx(), DEPOSIT_COMMAND).unwrap();
    let withdrawal = visualize(withdrawal_tx(), WITHDRAWAL_COMMAND).unwrap();

    assert_eq!(
        layout_labels(&deposit[0]),
        (
            DEPOSIT_KEY_ROWS.map(String::from).to_vec(),
            [&DEPOSIT_KEY_ROWS[..], &DEPOSIT_DETAIL_ROWS[..]]
                .concat()
                .into_iter()
                .map(String::from)
                .collect()
        )
    );
    assert_eq!(
        layout_labels(&withdrawal[0]),
        (
            WITHDRAWAL_KEY_ROWS.map(String::from).to_vec(),
            [&WITHDRAWAL_KEY_ROWS[..], &WITHDRAWAL_DETAIL_ROWS[..]]
                .concat()
                .into_iter()
                .map(String::from)
                .collect()
        )
    );
    assert_eq!(
        field_values(&deposit[0], "Source Network"),
        ["Bitcoin Signet"]
    );
    assert_eq!(
        field_values(&deposit[0], "Bitcoin Output"),
        [DEPOSIT_OUTPUT]
    );
    assert_eq!(
        field_values(&withdrawal[0], "Source Network"),
        ["Sui Testnet"]
    );
}

#[test]
fn card_takes_the_payload_title_and_has_no_subtitle() {
    for (tx, command) in [
        (deposit_tx(), DEPOSIT_COMMAND),
        (deposit_to(None), DEPOSIT_COMMAND),
        (withdrawal_tx(), WITHDRAWAL_COMMAND),
    ] {
        let payload = payload_of(&tx);
        let card = visualize(tx, command).unwrap();
        let SignablePayloadField::PreviewLayout { preview_layout, .. } =
            &card[0].signable_payload_field
        else {
            panic!("expected a preview layout");
        };
        assert_eq!(preview_layout.title.as_ref().unwrap().text, payload.title);
        assert!(preview_layout.subtitle.is_none());
    }
}

#[test]
fn bitcoin_recipient_is_plain_text_and_sui_recipient_is_an_address() {
    let withdrawal = condensed_rows(&visualize(withdrawal_tx(), WITHDRAWAL_COMMAND).unwrap()[0]);
    let deposit = condensed_rows(&visualize(deposit_tx(), DEPOSIT_COMMAND).unwrap()[0]);
    let recipient = |rows: &[SignablePayloadField]| {
        rows.iter()
            .find(|f| f.label() == "Recipient Address")
            .unwrap()
            .clone()
    };

    assert!(matches!(
        recipient(&withdrawal),
        SignablePayloadField::TextV2 { .. }
    ));
    assert!(matches!(
        recipient(&deposit),
        SignablePayloadField::AddressV2 { .. }
    ));
}

#[test]
fn deposit_recipient_is_named_by_whether_it_is_the_signer() {
    for (tx, name) in [
        (deposit_tx(), "Signer's address"),
        (deposit_to(Some([0x42; 32])), "Not the signer's address"),
    ] {
        let card = visualize(tx, DEPOSIT_COMMAND).unwrap();
        assert_eq!(
            address_name(&condensed_rows(&card[0]), "Recipient Address").as_deref(),
            Some(name)
        );
    }
}

#[test]
fn bridge_addresses_are_named_hashi() {
    for payload in [payload_of(&deposit_tx()), payload_of(&withdrawal_tx())] {
        assert_eq!(
            address_name(&payload.fields, "Bridge Package").as_deref(),
            Some("Hashi")
        );
        assert_eq!(
            address_name(&payload.fields, "Bridge Object").as_deref(),
            Some("Hashi bridge")
        );
    }
}

#[test]
fn every_preview_shape_passes_the_wallet_render_check() {
    for tx in [
        deposit_tx(),
        deposit_to(None),
        deposit_to(Some([0x42; 32])),
        batched_deposit_tx(),
        withdrawal_tx(),
        redeem_withdrawal_tx(),
        top_up_withdrawal_tx(),
    ] {
        payload_of(&tx)
            .validate_anchorage_wallet_renderable()
            .unwrap();
    }
}

#[test]
fn deposit_without_recipient_warns_in_the_payload_title() {
    let payload = payload_of(&deposit_to(None));

    assert_eq!(
        payload.title,
        "Warning: Hashi deposit from Bitcoin Signet credits no one"
    );
    assert_eq!(
        payload.subtitle.as_deref(),
        Some("0.00097291 BTC, no recipient")
    );
    assert_eq!(
        top_level_labels(&payload),
        ["Network", "Hashi Deposit", "Transaction Details"]
    );
}

#[test]
fn address_balance_and_top_up_withdrawals_get_the_bridge_intent() {
    for (tx, subtitle) in [
        (
            redeem_withdrawal_tx(),
            "0.0003 hBTC to tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk",
        ),
        (
            top_up_withdrawal_tx(),
            "1.43809724 hBTC to tb1q89kmd8ascuqm6fs5vam7psqwcyer4xazqlawvk",
        ),
    ] {
        let payload = payload_of(&tx);
        assert_eq!(payload.title, WITHDRAWAL_TITLE);
        assert_eq!(payload.subtitle.as_deref(), Some(subtitle));
    }
}

#[test]
fn batched_deposits_keep_the_generic_title_and_one_card_each() {
    let payload = payload_of(&batched_deposit_tx());

    assert_eq!(payload.title, "Programmable Transaction");
    assert_eq!(payload.subtitle, None);
    assert_eq!(
        top_level_labels(&payload),
        [
            "Network",
            "Hashi Deposit",
            "Hashi Deposit",
            "Transaction Details"
        ]
    );
}

#[test]
fn withdrawal_sending_change_to_another_address_keeps_the_generic_title() {
    let mut tx = top_up_withdrawal_tx();
    programmable(&mut tx).inputs[TOP_UP_CHANGE_RECIPIENT_INPUT] =
        CallArg::Pure(bcs::to_bytes(&[0x42u8; 32]).unwrap());

    let payload = payload_of(&tx);

    assert_eq!(payload.title, "Programmable Transaction");
    assert_eq!(
        top_level_labels(&payload),
        ["Network", "Hashi Withdrawal", "Transaction Details"]
    );
}

#[test]
fn withdrawal_alongside_an_unrelated_call_keeps_the_generic_title() {
    let mut tx = withdrawal_tx();
    let unrelated_input = push_pure_input(&mut tx, bcs::to_bytes(&7u64).unwrap());
    programmable(&mut tx)
        .commands
        .push(tamper_call("anything", vec![unrelated_input]));

    let payload = payload_of(&tx);

    assert_eq!(payload.title, "Programmable Transaction");
}

fn shared_coin_input(tx: &mut TransactionData) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(CallArg::Object(ObjectArg::SharedObject {
        id: ObjectID::from_single_byte(0x5c),
        initial_shared_version: SequenceNumber::from_u64(1),
        mutability: SharedObjectMutability::Mutable,
    }));
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn push_input(tx: &mut TransactionData, input: CallArg) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(input);
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn sui_from(amount: u64, sponsor: bool) -> FundsWithdrawalArg {
    let sui = sui_types::parse_sui_type_tag("0x2::sui::SUI").unwrap();
    if sponsor {
        FundsWithdrawalArg::balance_from_sponsor(amount, sui)
    } else {
        FundsWithdrawalArg::balance_from_sender(amount, sui)
    }
}

#[test]
fn withdrawal_merging_into_a_coin_the_signer_does_not_own_keeps_the_generic_title() {
    let mut into_shared = withdrawal_tx();
    let shared = shared_coin_input(&mut into_shared);
    for (mut tx, destination) in [(into_shared, shared), (withdrawal_tx(), Argument::GasCoin)] {
        programmable(&mut tx).commands[0] =
            Command::MergeCoins(destination, vec![Argument::Input(1)]);
        assert_eq!(payload_of(&tx).title, "Programmable Transaction");
    }
}

#[test]
fn withdrawal_merging_into_a_coin_redeemed_in_the_transaction_keeps_the_bridge_title() {
    let mut tx = top_up_withdrawal_tx();
    let pt = programmable(&mut tx);
    pt.commands[1] = Command::MergeCoins(result(0), vec![Argument::Input(TOP_UP_HBTC_COIN_INPUT)]);
    pt.commands[2] =
        Command::SplitCoins(result(0), vec![Argument::Input(TOP_UP_SPLIT_AMOUNT_INPUT)]);
    move_call(&mut tx, 4).arguments[0] = result(0);

    assert_eq!(payload_of(&tx).title, WITHDRAWAL_TITLE);
}

#[test]
fn deposit_alongside_a_split_of_the_gas_coin_keeps_the_generic_title() {
    let mut tx = deposit_tx();
    let sender = tx.sender();
    let to_sender = push_pure_input(&mut tx, bcs::to_bytes(&sender).unwrap());
    let amount = push_pure_input(&mut tx, bcs::to_bytes(&1u64).unwrap());
    let commands = &mut programmable(&mut tx).commands;
    commands.push(Command::SplitCoins(Argument::GasCoin, vec![amount]));
    commands.push(framework_call(
        SUI_FRAMEWORK,
        "coin",
        "send_funds",
        Some("0x2::sui::SUI"),
        vec![Argument::NestedResult(3, 0), to_sender],
    ));

    assert_eq!(payload_of(&tx).title, "Programmable Transaction");
}

#[test]
fn deposit_alongside_redeemed_funds_keeps_the_bridge_title_only_when_the_signer_pays() {
    for (sponsor, title) in [(false, DEPOSIT_TITLE), (true, "Programmable Transaction")] {
        let mut tx = deposit_tx();
        let sender = tx.sender();
        let to_sender = push_pure_input(&mut tx, bcs::to_bytes(&sender).unwrap());
        let withdrawal = push_input(&mut tx, CallArg::FundsWithdrawal(sui_from(10, sponsor)));
        let commands = &mut programmable(&mut tx).commands;
        commands.push(framework_call(
            SUI_FRAMEWORK,
            "balance",
            "redeem_funds",
            Some("0x2::sui::SUI"),
            vec![withdrawal],
        ));
        commands.push(framework_call(
            SUI_FRAMEWORK,
            "balance",
            "send_funds",
            Some("0x2::sui::SUI"),
            vec![result(3), to_sender],
        ));

        assert_eq!(payload_of(&tx).title, title, "sponsor: {sponsor}");
    }
}

#[test]
fn withdrawal_alongside_other_commands_or_lookalike_calls_keeps_the_generic_title() {
    let extras: [fn(Argument) -> Command; 3] = [
        |_| Command::MakeMoveVec(None, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
        |recipient| {
            framework_call(
                0x42,
                "coin",
                "into_balance",
                Some(&hbtc_type()),
                vec![recipient],
            )
        },
        |recipient| {
            framework_call(
                0x42,
                "coin",
                "send_funds",
                Some(&hbtc_type()),
                vec![Argument::Input(HBTC_COIN_INPUT), recipient],
            )
        },
    ];
    for extra in extras {
        let mut tx = withdrawal_tx();
        let sender = tx.sender();
        let recipient = push_pure_input(&mut tx, bcs::to_bytes(&sender).unwrap());
        programmable(&mut tx).commands.push(extra(recipient));
        assert_eq!(payload_of(&tx).title, "Programmable Transaction");
    }
}

#[test]
fn caller_supplied_title_disables_the_bridge_intent() {
    let payload = crate::transaction_string_to_visual_sign(
        &encoded_fixture_tx("withdraw", "request_withdrawal", REDEEM_WITHDRAWAL_DIGEST),
        VisualSignOptions {
            transaction_name: Some("Caller Title".to_string()),
            decode_transfers: true,
            ..VisualSignOptions::default()
        },
    )
    .unwrap();

    assert_eq!(payload.title, "Caller Title");
    assert_eq!(payload.subtitle, None);
    assert_eq!(
        top_level_labels(&payload),
        ["Network", "Hashi Withdrawal", "Transaction Details"]
    );
}

#[test]
fn format_btc_trims_trailing_zeros() {
    assert_eq!(format_btc(0), "0");
    assert_eq!(format_btc(546), "0.00000546");
    assert_eq!(format_btc(100_000_000), "1");
    assert_eq!(format_btc(150_000_000), "1.5");
    assert_eq!(format_btc(u64::MAX), "184467440737.09551615");
}
