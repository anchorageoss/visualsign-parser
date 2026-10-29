use super::*;

use super::config::SuiNetwork;
use crate::types::{SuiTransactionBlockData, SuiTransactionBlockKind};
use crate::utils::run_aggregated_fixture;

use base64::Engine;
use sui_sdk_types::{
    Address, Argument, Command, Digest, FundsWithdrawal, Identifier, Input, MakeMoveVector,
    MergeCoins, MoveCall, Mutability, ObjectReference, ProgrammableTransaction, SharedInput,
    SplitCoins, Transaction, TransactionKind, TransferObjects, WithdrawFrom,
};
use visualsign::SignablePayload;
use visualsign::test_utils::check_signable_payload_field;
use visualsign::vsptrait::VisualSignOptions;

const FIXTURE: &str = include_str!("aggregated_test_data.json");

// Constructors over `sui_sdk_types` in the shape the tests below were written in.

fn package_id(byte: u8) -> Address {
    Address::new(ObjectID::from_single_byte(byte).into_bytes())
}

fn package_literal(literal: &str) -> Address {
    Address::new(ObjectID::from_hex_literal(literal).unwrap().into_bytes())
}

fn ident(name: &str) -> Identifier {
    Identifier::new(name).unwrap()
}

fn parse_type_tag(tag: &str) -> Result<TypeTag, sui_sdk_types::TypeParseError> {
    tag.parse()
}

fn command_move_call(
    package: Address,
    module: Identifier,
    function: Identifier,
    type_arguments: Vec<TypeTag>,
    arguments: Vec<Argument>,
) -> Command {
    Command::MoveCall(MoveCall {
        package,
        module,
        function,
        type_arguments,
        arguments,
    })
}

fn merge_coins(coin: Argument, coins_to_merge: Vec<Argument>) -> Command {
    Command::MergeCoins(MergeCoins {
        coin,
        coins_to_merge,
    })
}

fn split_coins(coin: Argument, amounts: Vec<Argument>) -> Command {
    Command::SplitCoins(SplitCoins { coin, amounts })
}

fn transfer_objects(objects: Vec<Argument>, address: Argument) -> Command {
    Command::TransferObjects(TransferObjects { objects, address })
}

fn make_move_vec(type_: Option<TypeTag>, elements: Vec<Argument>) -> Command {
    Command::MakeMoveVector(MakeMoveVector { type_, elements })
}

fn balance_from_sender(amount: u64, coin_type: TypeTag) -> FundsWithdrawal {
    FundsWithdrawal::new(amount, coin_type, WithdrawFrom::Sender)
}

fn balance_from_sponsor(amount: u64, coin_type: TypeTag) -> FundsWithdrawal {
    FundsWithdrawal::new(amount, coin_type, WithdrawFrom::Sponsor)
}

fn block_data(tx: &Transaction) -> SuiTransactionBlockData {
    SuiTransactionBlockData::try_from(tx).unwrap()
}
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

// Real cancellations: 0 cancel_withdrawal(I0 hashi, I1 request id, I2 clock),
// 1 coin::from_balance<BTC>(R0), 2 refund of R1 to I3.
const CANCEL_COMMAND: usize = 0;
const FROM_BALANCE_COMMAND: usize = 1;
const REFUND_COMMAND: usize = 2;
const REQUEST_ID_INPUT: usize = 1;
const REFUND_RECIPIENT_INPUT: u16 = 3;
/// Refund sent with `TransferObjects`.
const CANCEL_TRANSFER_DIGEST: &str = "4ahHNd8LuyYPJ8CucP7y5Quf5RQv8Di2LbFg7fd6TU3d";
/// Refund sent with `coin::send_funds`.
const CANCEL_SEND_FUNDS_DIGEST: &str = "2W7pwd312DpS5mornpMj7pB8Ha6yQ33Wmx1dT2GkwpJN";
/// Three cancellations, each refunded with `coin::send_funds`.
const CANCEL_BATCH_DIGEST: &str = "2WCfwPwkgu2nDZDV8hJNZrSELNtyftgqzK1H1gCM8nsQ";
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

fn fixture_tx(section: &str, function: &str, digest: &str) -> Transaction {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded_fixture_tx(section, function, digest))
        .unwrap();
    bcs::from_bytes(&bytes).unwrap()
}

fn deposit_tx() -> Transaction {
    fixture_tx("deposit", "deposit", DEPOSIT_DIGEST)
}

// Amounts and coins of the coin-funded withdrawals built below. Every other
// input (the `Hashi` object, the clock, the Bitcoin address, the sender and gas)
// is the real address-balance withdrawal's.
const SPLIT_AMOUNT_SATS: u64 = 24_185_763;
const TOP_UP_AMOUNT_SATS: u64 = 143_809_724;
const TOP_UP_REDEEMED_SATS: u64 = 138_017_594;

fn owned_coin(byte: u8) -> Input {
    Input::ImmutableOrOwned(ObjectReference::new(
        package_id(byte),
        1,
        Digest::new([byte; 32]),
    ))
}

fn real_request_withdrawal(tx: &Transaction) -> MoveCall {
    let TransactionKind::ProgrammableTransaction(pt) = &tx.kind else {
        panic!("expected a programmable transaction");
    };
    let Command::MoveCall(call) = &pt.commands[REDEEM_WITHDRAWAL_COMMAND] else {
        panic!("expected request_withdrawal at command {REDEEM_WITHDRAWAL_COMMAND}");
    };
    call.clone()
}

/// The real withdrawal's inputs are `I0 hashi, I1 clock, I2 address,
/// I3 funds withdrawal`.
fn real_withdrawal_inputs(tx: &Transaction) -> (Input, Input, Input, Input) {
    let TransactionKind::ProgrammableTransaction(pt) = &tx.kind else {
        panic!("expected a programmable transaction");
    };
    (
        pt.inputs[0].clone(),
        pt.inputs[1].clone(),
        pt.inputs[2].clone(),
        pt.inputs[usize::from(FUNDS_WITHDRAWAL_INPUT)].clone(),
    )
}

fn withdrawal_tx() -> Transaction {
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
        Input::Pure(bcs::to_bytes(&SPLIT_AMOUNT_SATS).unwrap()),
        hashi,
        clock,
        address,
    ];
    pt.commands = vec![
        merge_coins(
            Argument::Input(HBTC_COIN_INPUT),
            vec![Argument::Input(1)],
        ),
        split_coins(
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
        Command::MoveCall(request),
    ];
    tx
}

fn redeem_withdrawal_tx() -> Transaction {
    fixture_tx("withdraw", "request_withdrawal", REDEEM_WITHDRAWAL_DIGEST)
}

fn top_up_withdrawal_tx() -> Transaction {
    let mut tx = redeem_withdrawal_tx();
    let (hashi, clock, address, _) = real_withdrawal_inputs(&tx);
    let sender = tx.sender;
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
        Input::FundsWithdrawal(hbtc_from_sender(TOP_UP_REDEEMED_SATS)),
        Input::Pure(bcs::to_bytes(&TOP_UP_AMOUNT_SATS).unwrap()),
        Input::Pure(bcs::to_bytes(&sender).unwrap()),
    ];
    pt.commands = vec![
        framework_call(
            SUI_FRAMEWORK,
            "coin",
            "redeem_funds",
            Some(&hbtc_type()),
            vec![Argument::Input(4)],
        ),
        merge_coins(
            Argument::Input(TOP_UP_HBTC_COIN_INPUT),
            vec![result(0)],
        ),
        split_coins(
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
        Command::MoveCall(request),
    ];
    tx
}

fn programmable(tx: &mut Transaction) -> &mut ProgrammableTransaction {
    match &mut tx.kind {
        TransactionKind::ProgrammableTransaction(pt) => pt,
        _ => panic!("expected a programmable transaction"),
    }
}

fn move_call(tx: &mut Transaction, index: usize) -> &mut MoveCall {
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
    command_move_call(
        package_id(package),
        module.parse().unwrap(),
        function.parse().unwrap(),
        type_arg
            .map(|tag| vec![parse_type_tag(tag).unwrap()])
            .unwrap_or_default(),
        args,
    )
}

fn push_pure_input(tx: &mut Transaction, bytes: Vec<u8>) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(Input::Pure(bytes));
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn tamper_call(function: &str, args: Vec<Argument>) -> Command {
    command_move_call(
        package_id(0x42),
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
fn insert_command(tx: &mut Transaction, index: usize, command: Command) {
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
            Command::TransferObjects(transfer) => {
                transfer.objects.iter_mut().for_each(shift);
                shift(&mut transfer.address);
            }
            Command::SplitCoins(split) => {
                shift(&mut split.coin);
                split.amounts.iter_mut().for_each(shift);
            }
            Command::MergeCoins(merge) => {
                shift(&mut merge.coin);
                merge.coins_to_merge.iter_mut().for_each(shift);
            }
            Command::MakeMoveVector(make) => make.elements.iter_mut().for_each(shift),
            Command::Upgrade(upgrade) => shift(&mut upgrade.ticket),
            _ => {}
        }
    }
    pt.commands.insert(index, command);
}

// Takes the transaction by value so callers can hand over the one they built.
#[allow(clippy::needless_pass_by_value)]
fn visualize(
    tx: Transaction,
    command_index: usize,
) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let block_data = block_data(&tx);
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

fn card_title(field: &AnnotatedPayloadField) -> String {
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = &field.signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    preview_layout.title.as_ref().unwrap().text.clone()
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

fn payload_of(tx: &Transaction) -> SignablePayload {
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

fn deposit_to(recipient: Option<[u8; 32]>) -> Transaction {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[DERIVATION_PATH_INPUT] =
        Input::Pure(bcs::to_bytes(&recipient).unwrap());
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
        Input::Pure(bcs::to_bytes(&program).unwrap());
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
        Input::Pure(bcs::to_bytes(&vec![0x75u8; 20]).unwrap());

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
        Input::Pure(bcs::to_bytes(&vec![0u8; 25]).unwrap());

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Bitcoin address must be a 20-byte P2WPKH or 32-byte P2TR program, got 25 bytes",
    );
}

#[test]
fn deposit_with_a_txid_that_is_not_32_bytes_fails() {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[usize::from(TXID_INPUT)] = Input::Pure(vec![0u8; 31]);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Bitcoin txid must be 32 bytes, got 31",
    );
}

#[test]
fn deposit_with_an_invalid_derivation_path_encoding_fails() {
    let mut tx = deposit_tx();
    programmable(&mut tx).inputs[DERIVATION_PATH_INPUT] = Input::Pure(vec![2]);

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
    move_call(&mut tx, UTXO_COMMAND).package = package_id(0x42);

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Expected argument to come from Hashi `utxo::utxo`",
    );
}

#[test]
fn deposit_whose_utxo_id_comes_from_a_foreign_package_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, UTXO_ID_COMMAND).package = package_id(0x42);

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
        merge_coins(split_coin, vec![Argument::Input(HBTC_COIN_INPUT)]),
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
        split_coins(split_coin, vec![resplit_amount]),
    );
    insert_command(
        &mut tx,
        INTO_BALANCE_COMMAND + 1,
        merge_coins(
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
        make_move_vec(None, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
    );
    insert_command(
        &mut tx,
        1,
        split_coins(other_coin, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
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
fn batched_deposit_tx() -> Transaction {
    let mut tx = deposit_tx();
    let second_vout = push_pure_input(&mut tx, bcs::to_bytes(&2u32).unwrap());
    let hashi = package_literal(HASHI_PACKAGE);
    let deposit_call = move_call(&mut tx, DEPOSIT_COMMAND).clone();
    let hashi_input = deposit_call.arguments[0];
    let clock_input = deposit_call.arguments[2];
    let commands = &mut programmable(&mut tx).commands;
    commands.push(command_move_call(
        hashi,
        "utxo".parse().unwrap(),
        "utxo_id".parse().unwrap(),
        vec![],
        vec![Argument::Input(TXID_INPUT), second_vout],
    ));
    commands.push(command_move_call(
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
    commands.push(command_move_call(
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
        command_move_call(
            package_id(0x42),
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
        Input::Pure(bcs::to_bytes(&7u32).unwrap());

    expect_error(
        visualize(tx, DEPOSIT_COMMAND),
        "Invalid u64 value: could not convert slice to array",
    );
}

#[test]
fn deposit_with_a_txid_that_is_not_an_input_fails() {
    let mut tx = deposit_tx();
    move_call(&mut tx, UTXO_ID_COMMAND).arguments[0] = Argument::Gas;

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
        merge_coins(
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
    move_call(&mut tx, INTO_BALANCE_COMMAND).package = package_id(0x42);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn withdrawal_balance_from_a_foreign_module_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).module = ident("balance");

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "Withdrawal balance is not produced by 0x2::coin::into_balance or 0x2::balance::redeem_funds",
    );
}

#[test]
fn withdrawal_funded_by_the_gas_coin_fails() {
    let mut tx = withdrawal_tx();
    move_call(&mut tx, INTO_BALANCE_COMMAND).arguments[0] = Argument::Gas;

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "The gas coin cannot fund an hBTC withdrawal",
    );
}

#[test]
fn withdrawal_split_from_the_gas_coin_fails() {
    let mut tx = withdrawal_tx();
    let Command::SplitCoins(split) = &mut programmable(&mut tx).commands[SPLIT_COMMAND] else {
        panic!("expected SplitCoins at command {SPLIT_COMMAND}");
    };
    split.coin = Argument::Gas;

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
        split_coins(Argument::Gas, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]);
    let Command::SplitCoins(split) = &mut pt.commands[SPLIT_COMMAND] else {
        panic!("expected SplitCoins at command {SPLIT_COMMAND}");
    };
    split.coin = Argument::NestedResult(0, 0);

    expect_error(
        visualize(tx, WITHDRAWAL_COMMAND),
        "The gas coin cannot fund an hBTC withdrawal",
    );
}

fn split_from_redeemed_coin_tx(withdrawal: FundsWithdrawal) -> Transaction {
    let mut tx = top_up_withdrawal_tx();
    let split_amount = push_pure_input(&mut tx, bcs::to_bytes(&100_000_000u64).unwrap());
    let pt = programmable(&mut tx);
    pt.inputs[TOP_UP_FUNDS_WITHDRAWAL_INPUT] = Input::FundsWithdrawal(withdrawal);
    let redeemed = result(0);
    pt.commands = vec![
        pt.commands[0].clone(),
        split_coins(redeemed, vec![split_amount]),
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
        Input::FundsWithdrawal(hbtc_from_sponsor(150_000_000));

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
        Input::FundsWithdrawal(balance_from_sender(
            10_000_000,
            parse_type_tag("0x2::sui::SUI").unwrap(),
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
        Input::FundsWithdrawal(balance_from_sponsor(
            10_000_000,
            parse_type_tag(&hbtc_type()).unwrap(),
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
        parse_type_tag("0x2::sui::SUI")
            .unwrap();

    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        &format!("Withdrawal balance is not a Balance<{HBTC_TYPE_ORIGIN}::btc::BTC>"),
    );
}

#[test]
fn withdrawal_balance_from_an_unsupported_framework_call_with_a_foreign_type_fails() {
    let mut tx = withdrawal_tx();
    let call = move_call(&mut tx, INTO_BALANCE_COMMAND);
    call.function = ident("into_balance_lossy");
    call.type_arguments[0] = parse_type_tag("0x2::sui::SUI")
        .unwrap();

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
    move_call(&mut tx, REDEEM_COMMAND).package = package_id(0x42);

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
    move_call(&mut tx, INTO_BALANCE_COMMAND).function = ident("into_balance_lossy");

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

fn redeemed_coin_withdrawal_tx(withdrawal: FundsWithdrawal) -> Transaction {
    let mut tx = top_up_withdrawal_tx();
    let pt = programmable(&mut tx);
    pt.inputs[TOP_UP_FUNDS_WITHDRAWAL_INPUT] = Input::FundsWithdrawal(withdrawal);
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

fn hbtc_from_sender(amount: u64) -> FundsWithdrawal {
    balance_from_sender(
        amount,
        parse_type_tag(&hbtc_type()).unwrap(),
    )
}

fn hbtc_from_sponsor(amount: u64) -> FundsWithdrawal {
    balance_from_sponsor(
        amount,
        parse_type_tag(&hbtc_type()).unwrap(),
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
    let tx = redeemed_coin_withdrawal_tx(balance_from_sender(
        25_000_000,
        parse_type_tag("0x2::sui::SUI").unwrap(),
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
        split_coins(result(0), vec![Argument::Input(TOP_UP_SPLIT_AMOUNT_INPUT)]),
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
        merge_coins(result(0), vec![Argument::Input(TOP_UP_HBTC_COIN_INPUT)]),
    );

    expect_error(
        visualize(tx, REDEEMED_COIN_WITHDRAWAL_COMMAND + 1),
        "`withdrawal coin` is passed to command 1, which may modify it before command 2 consumes it",
    );
}

#[test]
fn withdrawal_of_a_redeemed_coin_from_the_sponsor_fails() {
    let tx = redeemed_coin_withdrawal_tx(balance_from_sponsor(
        25_000_000,
        parse_type_tag(&hbtc_type()).unwrap(),
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
        parse_type_tag("0x2::sui::SUI")
            .unwrap();

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

const MAINNET_PACKAGE: &str = "0xd30fb3f8914efdbeb7b8e85fae1d65dc584b44a1281f66737b92bb9f03a59d06";
const MAINNET_HASHI_OBJECT: &str =
    "0x8d110642d9b5133bd661cda4873edf135ab6b91ff6918601015880874b950e5d";
const TESTNET_HASHI_OBJECT: &str =
    "0x342e908043db6090558ed7766df95bfc3609881b2eac7edb5343dd5bb3ddd1bb";

/// The same transaction against the mainnet deployment. Mainnet runs the same
/// code as testnet, so only the package, the `Hashi` object and the `BTC` type
/// differ.
fn on_mainnet(mut tx: Transaction) -> Transaction {
    let testnet = package_literal(HASHI_PACKAGE);
    let mainnet = package_literal(MAINNET_PACKAGE);
    let to_mainnet_type = |tag: &TypeTag| {
        parse_type_tag(
            &crate::types::type_tag_to_canonical_string(tag)
                .replace(&testnet.to_string()[2..], &mainnet.to_string()[2..]),
        )
        .unwrap()
    };
    let pt = programmable(&mut tx);
    for input in &mut pt.inputs {
        match input {
            Input::Shared(shared)
                if shared.object_id() == package_literal(TESTNET_HASHI_OBJECT) =>
            {
                *shared = SharedInput::new(
                    package_literal(MAINNET_HASHI_OBJECT),
                    shared.version(),
                    shared.mutability(),
                );
            }
            Input::FundsWithdrawal(withdrawal) => {
                *withdrawal = FundsWithdrawal::new(
                    withdrawal.amount().unwrap(),
                    to_mainnet_type(withdrawal.coin_type()),
                    withdrawal.source(),
                );
            }
            _ => {}
        }
    }
    for command in &mut pt.commands {
        if let Command::MoveCall(call) = command {
            if call.package == testnet {
                call.package = mainnet;
            }
            for type_argument in &mut call.type_arguments {
                *type_argument = to_mainnet_type(type_argument);
            }
        }
    }
    tx
}

#[allow(clippy::needless_pass_by_value)]
fn hashi_can_handle(tx: Transaction, command: usize) -> bool {
    let block_data = block_data(&tx);
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
    move_call(&mut unconfigured, WITHDRAWAL_COMMAND).package = package_id(0x42);
    assert!(!hashi_can_handle(unconfigured, WITHDRAWAL_COMMAND));

    let mut unsupported_function = withdrawal_tx();
    move_call(&mut unsupported_function, WITHDRAWAL_COMMAND).function =
        ident("approve_request");
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
fn mainnet_deposit_renders_the_hashi_preview() {
    let fields = visualize(on_mainnet(deposit_tx()), DEPOSIT_COMMAND).unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(
            "Bridge BTC from Bitcoin to Sui via Hashi",
            "0.00097291",
            "0xd5a05ab3ae8731246fa0f3bb0d91725e4f13f778c864290c7553216a56e2e900"
        )
    );
    assert_eq!(field_values(&fields[0], "Bridge Package"), [MAINNET_PACKAGE]);
    assert_eq!(field_values(&fields[0], "Bridge Object"), [MAINNET_HASHI_OBJECT]);
}

#[test]
fn mainnet_address_balance_withdrawal_renders_the_hashi_preview() {
    let fields = visualize(on_mainnet(redeem_withdrawal_tx()), REDEEM_WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(
            "Bridge hBTC from Sui to Bitcoin via Hashi",
            "0.0003",
            "bc1q89kmd8ascuqm6fs5vam7psqwcyer4xaz2exah9"
        )
    );
    assert_eq!(field_values(&fields[0], "Bridge Package"), [MAINNET_PACKAGE]);
}

#[test]
fn mainnet_coin_funded_withdrawal_renders_the_hashi_preview() {
    let fields = visualize(on_mainnet(withdrawal_tx()), WITHDRAWAL_COMMAND).unwrap();
    assert_eq!(
        card_headline(&fields[0]),
        headline(
            "Bridge hBTC from Sui to Bitcoin via Hashi",
            "0.24185763",
            "bc1q89kmd8ascuqm6fs5vam7psqwcyer4xaz2exah9"
        )
    );
}

#[test]
fn testnet_hbtc_withdrawn_to_the_mainnet_package_fails() {
    // The mainnet call with the testnet `BTC` type: the networks never mix.
    let mut tx = on_mainnet(redeem_withdrawal_tx());
    programmable(&mut tx).inputs[usize::from(FUNDS_WITHDRAWAL_INPUT)] =
        Input::FundsWithdrawal(hbtc_from_sender(30_000));
    expect_error(
        visualize(tx, REDEEM_WITHDRAWAL_COMMAND),
        &format!("Funds withdrawal is not denominated in {MAINNET_PACKAGE}::btc::BTC"),
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
        Input::Pure(bcs::to_bytes(&[0x42u8; 32]).unwrap());

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

fn shared_coin_input(tx: &mut Transaction) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(Input::Shared(SharedInput::new(
        package_id(0x5c),
        1,
        Mutability::Mutable,
    )));
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn push_input(tx: &mut Transaction, input: Input) -> Argument {
    let inputs = &mut programmable(tx).inputs;
    inputs.push(input);
    Argument::Input(u16::try_from(inputs.len() - 1).unwrap())
}

fn sui_from(amount: u64, sponsor: bool) -> FundsWithdrawal {
    let sui = parse_type_tag("0x2::sui::SUI").unwrap();
    if sponsor {
        balance_from_sponsor(amount, sui)
    } else {
        balance_from_sender(amount, sui)
    }
}

#[test]
fn withdrawal_merging_into_a_coin_the_signer_does_not_own_keeps_the_generic_title() {
    let mut into_shared = withdrawal_tx();
    let shared = shared_coin_input(&mut into_shared);
    for (mut tx, destination) in [(into_shared, shared), (withdrawal_tx(), Argument::Gas)] {
        programmable(&mut tx).commands[0] =
            merge_coins(destination, vec![Argument::Input(1)]);
        assert_eq!(payload_of(&tx).title, "Programmable Transaction");
    }
}

#[test]
fn withdrawal_merging_into_a_coin_redeemed_in_the_transaction_keeps_the_bridge_title() {
    let mut tx = top_up_withdrawal_tx();
    let pt = programmable(&mut tx);
    pt.commands[1] = merge_coins(result(0), vec![Argument::Input(TOP_UP_HBTC_COIN_INPUT)]);
    pt.commands[2] =
        split_coins(result(0), vec![Argument::Input(TOP_UP_SPLIT_AMOUNT_INPUT)]);
    move_call(&mut tx, 4).arguments[0] = result(0);

    assert_eq!(payload_of(&tx).title, WITHDRAWAL_TITLE);
}

#[test]
fn deposit_alongside_a_split_of_the_gas_coin_keeps_the_generic_title() {
    let mut tx = deposit_tx();
    let sender = tx.sender;
    let to_sender = push_pure_input(&mut tx, bcs::to_bytes(&sender).unwrap());
    let amount = push_pure_input(&mut tx, bcs::to_bytes(&1u64).unwrap());
    let commands = &mut programmable(&mut tx).commands;
    commands.push(split_coins(Argument::Gas, vec![amount]));
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
        let sender = tx.sender;
        let to_sender = push_pure_input(&mut tx, bcs::to_bytes(&sender).unwrap());
        let withdrawal = push_input(&mut tx, Input::FundsWithdrawal(sui_from(10, sponsor)));
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
        |_| make_move_vec(None, vec![Argument::Input(SPLIT_AMOUNT_INPUT)]),
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
        let sender = tx.sender;
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

fn cancel_tx(digest: &str) -> Transaction {
    fixture_tx("withdraw", "cancel_withdrawal", digest)
}

const CANCEL_TITLE: &str = "Cancel hBTC withdrawal on Sui Testnet via Hashi";

#[test]
fn cancellation_refunded_with_send_funds_is_titled_with_the_refund() {
    let payload = payload_of(&cancel_tx(CANCEL_SEND_FUNDS_DIGEST));
    assert_eq!(payload.title, CANCEL_TITLE);
    assert_eq!(
        payload.subtitle.as_deref(),
        Some("Refund hBTC to 0xbb5082e0b96b03414b8c5b20df82469a12522c60321393fd61d0efd0e785eeee")
    );
}

#[test]
fn cancellation_refunded_with_transfer_objects_also_renders_the_transfer() {
    // The generic transfer preview renders the `TransferObjects` refund as a
    // second card, so no single summary titles the payload.
    let payload = payload_of(&cancel_tx(CANCEL_TRANSFER_DIGEST));
    let labels: Vec<_> = payload.fields.iter().map(SignablePayloadField::label).collect();
    assert_eq!(
        labels,
        [
            "Network",
            "Transfer Command",
            "Hashi Withdrawal Cancellation",
            "Transaction Details"
        ]
    );
    assert_eq!(payload.title, "Programmable Transaction");
}

#[test]
fn cancellation_names_the_request_and_a_refund_to_the_signer() {
    let fields = visualize(cancel_tx(CANCEL_TRANSFER_DIGEST), CANCEL_COMMAND).unwrap();
    assert_eq!(
        field_values(&fields[0], "Withdrawal Request"),
        ["0xd77d19042e6b6db1ccda019809a219407fc8670ea6fbf5e3da9e663af9c4e79e"; 2]
    );
    let SignablePayloadField::PreviewLayout { preview_layout, .. } =
        &fields[0].signable_payload_field
    else {
        panic!("expected a preview layout");
    };
    let recipient = preview_layout.condensed.as_ref().unwrap().fields[1]
        .signable_payload_field
        .clone();
    let SignablePayloadField::AddressV2 { address_v2, .. } = recipient else {
        panic!("expected an address row, got {recipient:?}");
    };
    assert_eq!(address_v2.name, "Signer's address");
}

#[test]
fn every_cancellation_in_a_batch_renders_its_own_card() {
    let tx = cancel_tx(CANCEL_BATCH_DIGEST);
    let signer = "0x07606cd05cc66b5696a382411656a4c797e3748d88821fc6f18785d1620a4444";
    for (command, request) in [
        (
            0,
            "0x50dbf2e4513fc233c0ffc449240eddfb10eb9efacf8dac6e4fce3e44ed6fc244",
        ),
        (
            3,
            "0x4465b28f25f3f9517b76cc289c83c0d717090e1673575d1c883e1ae53fcfffd8",
        ),
        (
            6,
            "0x649e4dc449f28a4a3fcd1f86934f7e4f631181cb5ea702e8c734e9f63d72679e",
        ),
    ] {
        let fields = visualize(tx.clone(), command).unwrap();
        assert_eq!(
            card_title(&fields[0]),
            CANCEL_TITLE,
            "command {command}"
        );
        assert_eq!(
            field_values(&fields[0], "Withdrawal Request"),
            [request; 2],
            "command {command}"
        );
        assert_eq!(
            field_values(&fields[0], "Refund Recipient"),
            [signer; 2],
            "command {command}"
        );
    }
    // Several actions: no single bridge summary for the whole transaction.
    assert_eq!(payload_of(&tx).title, "Programmable Transaction");
}

#[test]
fn mainnet_cancellation_renders_the_hashi_preview() {
    let fields = visualize(on_mainnet(cancel_tx(CANCEL_TRANSFER_DIGEST)), CANCEL_COMMAND).unwrap();
    assert_eq!(card_title(&fields[0]), "Cancel hBTC withdrawal on Sui via Hashi");
    assert_eq!(field_values(&fields[0], "Bridge Package"), [MAINNET_PACKAGE]);
    assert_eq!(field_values(&fields[0], "Bridge Object"), [MAINNET_HASHI_OBJECT]);
    assert_eq!(field_values(&fields[0], "Network"), ["Sui"; 2]);
}

const UNSUPPORTED_REFUND: &str = "Cancelled withdrawal is not refunded through 0x2::coin::from_balance and then 0x2::coin::send_funds or TransferObjects";

#[test]
fn cancellation_refunded_by_another_route_fails() {
    // The coin goes to a call the preview does not know.
    let mut tx = cancel_tx(CANCEL_SEND_FUNDS_DIGEST);
    programmable(&mut tx).commands[REFUND_COMMAND] = tamper_call(
        "send_funds",
        vec![result(FROM_BALANCE_COMMAND), Argument::Input(REFUND_RECIPIENT_INPUT)],
    );
    expect_error(visualize(tx, CANCEL_COMMAND), UNSUPPORTED_REFUND);

    // The transfer also moves another object, which the preview would not show.
    let mut tx = cancel_tx(CANCEL_TRANSFER_DIGEST);
    programmable(&mut tx).commands[REFUND_COMMAND] = transfer_objects(
        vec![result(FROM_BALANCE_COMMAND), Argument::Gas],
        Argument::Input(REFUND_RECIPIENT_INPUT),
    );
    expect_error(visualize(tx, CANCEL_COMMAND), UNSUPPORTED_REFUND);

    // The refund is converted to a coin of another type.
    let mut tx = cancel_tx(CANCEL_SEND_FUNDS_DIGEST);
    move_call(&mut tx, FROM_BALANCE_COMMAND).type_arguments =
        vec![parse_type_tag("0x2::sui::SUI").unwrap()];
    expect_error(visualize(tx, CANCEL_COMMAND), UNSUPPORTED_REFUND);
}

#[test]
fn cancellation_whose_refund_is_used_twice_fails() {
    let mut tx = cancel_tx(CANCEL_SEND_FUNDS_DIGEST);
    let recipient = Argument::Input(REFUND_RECIPIENT_INPUT);
    programmable(&mut tx)
        .commands
        .push(transfer_objects(vec![result(FROM_BALANCE_COMMAND)], recipient));
    expect_error(
        visualize(tx, CANCEL_COMMAND),
        "The refund coin from command 1 must be used by exactly one later command",
    );
}

#[test]
fn cancellation_recipient_modified_before_the_refund_fails() {
    let mut tx = cancel_tx(CANCEL_SEND_FUNDS_DIGEST);
    insert_command(
        &mut tx,
        CANCEL_COMMAND,
        tamper_call("touch", vec![Argument::Input(REFUND_RECIPIENT_INPUT)]),
    );
    expect_error(
        visualize(tx, CANCEL_COMMAND + 1),
        "`recipient` is passed to command 0, which may modify it before command 3 consumes it",
    );
}

#[test]
fn cancellation_request_id_modified_before_the_cancel_fails() {
    let mut tx = cancel_tx(CANCEL_SEND_FUNDS_DIGEST);
    let request = Argument::Input(u16::try_from(REQUEST_ID_INPUT).unwrap());
    insert_command(&mut tx, CANCEL_COMMAND, tamper_call("touch", vec![request]));
    expect_error(
        visualize(tx, CANCEL_COMMAND + 1),
        "`request_id` is passed to command 0, which may modify it before command 1 consumes it",
    );
}

#[test]
fn cancellations_refunded_to_one_shared_recipient_input_render() {
    // `coin::send_funds` takes the recipient by value, so refunding several
    // cancellations to the same input cannot change it between them.
    let mut tx = cancel_tx(CANCEL_BATCH_DIGEST);
    let shared = Argument::Input(REFUND_RECIPIENT_INPUT);
    for refund in [5, 8] {
        move_call(&mut tx, refund).arguments[1] = shared;
    }
    let signer = "0x07606cd05cc66b5696a382411656a4c797e3748d88821fc6f18785d1620a4444";
    for command in [0, 3, 6] {
        let fields = visualize(tx.clone(), command).unwrap();
        assert_eq!(
            field_values(&fields[0], "Refund Recipient"),
            [signer; 2],
            "command {command}"
        );
    }
}

#[test]
fn cancellations_transferred_to_one_shared_recipient_input_render() {
    // The `TransferObjects` recipient is taken by value as well.
    let mut tx = cancel_tx(CANCEL_TRANSFER_DIGEST);
    let second_request = push_pure_input(&mut tx, bcs::to_bytes(&[0x42u8; 32]).unwrap());
    let mut second_cancel = move_call(&mut tx, CANCEL_COMMAND).clone();
    second_cancel.arguments[1] = second_request;
    let mut second_from_balance = move_call(&mut tx, FROM_BALANCE_COMMAND).clone();
    second_from_balance.arguments[0] = result(REFUND_COMMAND + 1);
    let commands = &mut programmable(&mut tx).commands;
    commands.push(Command::MoveCall(second_cancel));
    commands.push(Command::MoveCall(second_from_balance));
    commands.push(transfer_objects(
        vec![result(REFUND_COMMAND + 2)],
        Argument::Input(REFUND_RECIPIENT_INPUT),
    ));

    let signer = "0xd85a6b45f455c658224d664e75909fb89e63b2be1a14f46a28b5dfd16158e344";
    for (command, request) in [
        (
            CANCEL_COMMAND,
            "0xd77d19042e6b6db1ccda019809a219407fc8670ea6fbf5e3da9e663af9c4e79e".to_string(),
        ),
        (REFUND_COMMAND + 1, format!("0x{}", hex::encode([0x42u8; 32]))),
    ] {
        let fields = visualize(tx.clone(), command).unwrap();
        assert_eq!(
            field_values(&fields[0], "Withdrawal Request"),
            [request.as_str(); 2],
            "command {command}"
        );
        assert_eq!(
            field_values(&fields[0], "Refund Recipient"),
            [signer; 2],
            "command {command}"
        );
    }
}

