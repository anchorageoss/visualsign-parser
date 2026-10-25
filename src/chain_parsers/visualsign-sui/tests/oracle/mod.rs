//! Dev-only migration oracle pinned to the upstream revision used before PR #576.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "legacy_intermediate.rs"]
mod legacy_intermediate;

use crate::types;
use crate::{SuiTransactionWrapper, SuiVisualSignConverter};
use base64::Engine;
use move_bytecode_utils::module_cache::SyncModuleCache;
use serde_json::{Value, json};
use sui_json_rpc_types::{SuiTransactionBlockDataAPI, SuiTransactionBlockKind};
use sui_sdk_types::{Address, Argument, Command, Input};
use sui_types::transaction::{SenderSignedData, TransactionData};
use types::{SuiCallArg, SuiObjectArg, SuiReservation, SuiWithdrawFrom, SuiWithdrawalTypeArg};
use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};

struct EmptyResolver;
impl move_core_types::resolver::ModuleResolver for EmptyResolver {
    type Error = String;
    fn get_module(
        &self,
        _: &move_core_types::language_storage::ModuleId,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }
}
fn old_decode(bytes: &[u8]) -> TransactionData {
    bcs::from_bytes::<SenderSignedData>(bytes).map_or_else(
        |_| bcs::from_bytes(bytes).expect("old TransactionData decode"),
        |signed| signed.transaction_data().clone(),
    )
}
fn normalize_input(input: &types::SuiCallArg) -> Value {
    match input {
        SuiCallArg::Pure(pure) => json!({"type":"pure",
            "valueType":pure.value_type().map(|t| t.to_string()), "value":pure.value().to_json_value()}),
        SuiCallArg::Object(object) => match object {
            SuiObjectArg::ImmOrOwnedObject {
                object_id,
                version,
                digest,
            } => json!({
                "type":"object", "objectType":"immOrOwnedObject", "objectId":object_id.to_string(),
                "version":version.value().to_string(), "digest":sui_sdk_types::Digest::new(*digest.inner()).to_string()}),
            SuiObjectArg::Receiving {
                object_id,
                version,
                digest,
            } => json!({
                "type":"object", "objectType":"receiving", "objectId":object_id.to_string(),
                "version":version.value().to_string(), "digest":sui_sdk_types::Digest::new(*digest.inner()).to_string()}),
            SuiObjectArg::SharedObject {
                object_id,
                initial_shared_version,
                mutable,
            } => json!({
                "type":"object", "objectType":"sharedObject", "objectId":object_id.to_string(),
                "initialSharedVersion":initial_shared_version.value().to_string(), "mutable":mutable}),
        },
        SuiCallArg::FundsWithdrawal(withdrawal) => {
            let SuiReservation::MaxAmountU64(amount) = withdrawal.reservation;
            let SuiWithdrawalTypeArg::Balance(type_tag) = &withdrawal.type_arg;
            let from = match withdrawal.withdraw_from {
                SuiWithdrawFrom::Sender => "sender",
                SuiWithdrawFrom::Sponsor => "sponsor",
            };
            json!({"type":"fundsWithdrawal", "reservation":{"maxAmountU64":amount.to_string()},
                "typeArg":{"balance":types::type_tag_to_string(type_tag)}, "withdrawFrom":from})
        }
    }
}
fn assert_parity(label: &str, wrapper: SuiTransactionWrapper, old: &TransactionData) {
    let transaction = wrapper.inner();
    assert_eq!(
        bcs::to_bytes(transaction).unwrap(),
        bcs::to_bytes(old).unwrap(),
        "raw BCS: {label}"
    );
    let new_view = types::SuiTransactionBlockData::try_from(transaction).unwrap();
    let old_view = sui_json_rpc_types::SuiTransactionBlockData::try_from_with_module_cache(
        old.clone(),
        &SyncModuleCache::new(EmptyResolver),
    )
    .unwrap();
    assert_eq!(
        new_view.sender().to_string(),
        old_view.sender().to_string(),
        "sender: {label}"
    );
    assert_eq!(
        new_view.gas_data().owner.to_string(),
        old_view.gas_data().owner.to_string(),
        "gas owner: {label}"
    );
    assert_eq!(
        new_view.gas_data().price,
        old_view.gas_data().price,
        "gas price: {label}"
    );
    assert_eq!(
        new_view.gas_data().budget,
        old_view.gas_data().budget,
        "gas budget: {label}"
    );
    let types::SuiTransactionBlockKind::ProgrammableTransaction(new_ptb) = new_view.transaction()
    else {
        panic!("fixture must be programmable: {label}");
    };
    let SuiTransactionBlockKind::ProgrammableTransaction(old_ptb) = old_view.transaction() else {
        panic!("old fixture must be programmable: {label}");
    };
    let old_inputs: Vec<Value> = old_ptb
        .inputs
        .iter()
        .map(|v| serde_json::to_value(v).unwrap())
        .collect();
    let new_inputs: Vec<Value> = new_ptb.inputs.iter().map(normalize_input).collect();
    assert_eq!(
        new_inputs, old_inputs,
        "input values, pure typing, object metadata: {label}"
    );
    assert_eq!(
        new_ptb
            .commands
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        old_ptb
            .commands
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "commands: {label}"
    );
    let old_output = legacy_intermediate::build_intermediate_output(old);
    let old_intermediate = if matches!(
        transaction.kind,
        sui_sdk_types::TransactionKind::ProgrammableSystemTransaction(_)
    ) {
        assert!(
            old_output.is_err(),
            "system transaction intentionally has no intermediate: {label}"
        );
        None
    } else {
        Some(
            borsh::to_vec(&old_output.expect("supported PTB must produce old intermediate output"))
                .unwrap(),
        )
    };
    let result = SuiVisualSignConverter
        .to_visual_sign_payload(
            wrapper,
            VisualSignOptions {
                include_intermediate_output: true,
                decode_transfers: false,
                transaction_name: None,
                metadata: None,
                developer_config: None,
            },
        )
        .unwrap();
    assert_eq!(
        result.intermediate_output, old_intermediate,
        "Borsh intermediate: {label}"
    );
}
fn walk_fixtures(value: &Value, path: &str, fixtures: &mut Vec<(String, String)>) {
    if let Some(object) = value.as_object() {
        if let Some(data) = object.get("data").and_then(Value::as_str) {
            fixtures.push((path.to_owned(), data.to_owned()));
        }
        for (key, child) in object {
            walk_fixtures(child, &format!("{path}/{key}"), fixtures);
        }
    }
}
#[test]
fn sixty_existing_fixture_cases_match_the_pre_migration_oracle() {
    let mut fixtures = Vec::new();
    for (name, data) in [
        (
            "cetus",
            include_str!("../../src/presets/cetus/aggregated_test_data.json"),
        ),
        (
            "suilend",
            include_str!("../../src/presets/suilend/aggregated_test_data.json"),
        ),
        (
            "momentum",
            include_str!("../../src/presets/momentum/aggregated_test_data.json"),
        ),
        (
            "hashi",
            include_str!("../../src/integrations/hashi/aggregated_test_data.json"),
        ),
    ] {
        walk_fixtures(&serde_json::from_str(data).unwrap(), name, &mut fixtures);
    }
    let decoder: Vec<String> = serde_json::from_str(include_str!("decode_fixtures.json")).unwrap();
    fixtures.extend(
        decoder
            .into_iter()
            .enumerate()
            .map(|(i, data)| (format!("decoder/{i}"), data)),
    );
    // 58 named protocol cases (some share a transaction), plus 2 decoder cases.
    // Pin coverage so deleting an existing fixture cannot silently reduce it.
    assert_eq!(fixtures.len(), 60);
    for (label, data) in fixtures {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        assert_parity(
            &label,
            SuiTransactionWrapper::from_string(&data).unwrap(),
            &old_decode(&bytes),
        );
    }
}
#[test]
fn native_typing_order_and_package_digest_match_upstream() {
    let fixtures: Vec<String> = serde_json::from_str(include_str!("decode_fixtures.json")).unwrap();
    let mut transaction = SuiTransactionWrapper::from_string(&fixtures[0])
        .unwrap()
        .inner()
        .clone();
    let sui_sdk_types::TransactionKind::ProgrammableTransaction(ptb) = &mut transaction.kind else {
        panic!("PTB")
    };
    ptb.inputs = vec![
        Input::Pure(42u64.to_le_bytes().to_vec()),
        Input::Pure(vec![0xab; 32]),
    ];
    let split = Command::SplitCoins(sui_sdk_types::SplitCoins {
        coin: Argument::Gas,
        amounts: vec![Argument::Input(0)],
    });
    let transfer = Command::TransferObjects(sui_sdk_types::TransferObjects {
        objects: vec![Argument::Result(0)],
        address: Argument::Input(1),
    });
    let call = Command::MoveCall(sui_sdk_types::MoveCall {
        package: Address::TWO,
        module: "test".parse().unwrap(),
        function: "call".parse().unwrap(),
        type_arguments: vec!["0x2::sui::SUI".parse().unwrap()],
        arguments: vec![Argument::Input(0)],
    });
    let publish = Command::Publish(sui_sdk_types::Publish {
        modules: vec![vec![3, 1, 4], vec![1, 5, 9]],
        dependencies: vec![Address::TWO, Address::new([1; 32])],
    });
    let upgrade = Command::Upgrade(sui_sdk_types::Upgrade {
        modules: vec![vec![2, 7, 1], vec![8, 2, 8]],
        dependencies: vec![Address::new([1; 32]), Address::TWO],
        package: Address::TWO,
        ticket: Argument::Result(0),
    });
    for commands in [
        vec![split.clone(), transfer.clone(), call.clone()],
        vec![call.clone(), split.clone(), transfer.clone()],
        vec![publish.clone()],
        vec![upgrade],
        vec![transfer, split, publish, call],
    ] {
        let sui_sdk_types::TransactionKind::ProgrammableTransaction(ptb) = &mut transaction.kind
        else {
            panic!("PTB")
        };
        ptb.commands = commands;
        let bytes = bcs::to_bytes(&transaction).unwrap();
        assert_parity(
            "synthetic native order/package",
            SuiTransactionWrapper::new(transaction.clone()),
            &old_decode(&bytes),
        );
    }
}

#[test]
fn programmable_system_transfer_matches_upstream_normalization() {
    let fixtures: Vec<String> = serde_json::from_str(include_str!("decode_fixtures.json")).unwrap();
    let mut transaction = SuiTransactionWrapper::from_string(&fixtures[0])
        .unwrap()
        .inner()
        .clone();
    let sui_sdk_types::TransactionKind::ProgrammableTransaction(ptb) = transaction.kind else {
        panic!("native transfer fixture must be programmable")
    };
    transaction.kind = sui_sdk_types::TransactionKind::ProgrammableSystemTransaction(ptb);
    let bytes = bcs::to_bytes(&transaction).unwrap();
    assert_parity(
        "programmable system native transfer",
        SuiTransactionWrapper::new(transaction),
        &old_decode(&bytes),
    );
}

#[test]
fn newer_sdk_variants_are_rejected_by_the_pinned_upstream_decoder() {
    let fixtures: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../fixtures/unsupported-sdk-transactions.json"
    ))
    .unwrap();
    assert_eq!(fixtures.len(), 3);
    for (name, data) in fixtures {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&data)
            .unwrap();
        assert!(
            bcs::from_bytes::<sui_sdk_types::Transaction>(&bytes).is_ok(),
            "valid SDK fixture: {name}"
        );
        assert!(
            bcs::from_bytes::<TransactionData>(&bytes).is_err(),
            "unsupported by old TransactionData: {name}"
        );
        assert!(
            bcs::from_bytes::<SenderSignedData>(&bytes).is_err(),
            "unsupported by old SenderSignedData: {name}"
        );
        assert!(
            crate::transaction_string_to_visual_sign(&data, VisualSignOptions::default()).is_err(),
            "public converter fails closed: {name}"
        );
    }
}

#[test]
fn fluid_fixture_cases_match_the_pre_migration_type_oracle() {
    let mut fixtures = Vec::new();
    for (name, source) in [
        (
            "fluid",
            include_str!("../../src/integrations/fluid/aggregated_test_data.json"),
        ),
        (
            "fluid-previous",
            include_str!("../../src/integrations/fluid/previous_deployment_test_data.json"),
        ),
    ] {
        let data: Value = serde_json::from_str(source).unwrap();
        walk_fixtures(&data, name, &mut fixtures);
    }
    assert_eq!(fixtures.len(), 14);
    for (label, encoded) in fixtures {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .unwrap();
        assert_parity(
            &label,
            SuiTransactionWrapper::from_string(&encoded).unwrap(),
            &old_decode(&bytes),
        );
    }
}

#[test]
fn deeply_nested_type_tags_are_rejected_by_the_sdk_decoder() {
    let fixtures: Vec<String> = serde_json::from_str(include_str!("decode_fixtures.json")).unwrap();
    let mut transaction = SuiTransactionWrapper::from_string(&fixtures[0])
        .unwrap()
        .inner()
        .clone();
    let sui_sdk_types::TransactionKind::ProgrammableTransaction(ptb) = &mut transaction.kind else {
        panic!("PTB")
    };
    ptb.inputs = vec![Input::Pure(42u64.to_le_bytes().to_vec())];
    ptb.commands = vec![Command::MoveCall(sui_sdk_types::MoveCall {
        package: Address::TWO,
        module: "test".parse().unwrap(),
        function: "call".parse().unwrap(),
        type_arguments: vec!["u64".parse().unwrap()],
        arguments: vec![Argument::Input(0)],
    })];
    let bytes = bcs::to_bytes(&transaction).unwrap();
    // BCS TypeTag: vector = 6, u64 = 2. Wrap the single `u64` argument in 128 vectors.
    let marker = [&[4u8][..], b"call", &[1, 2]].concat();
    let at = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    let tag_at = at + 1 + 4 + 1;
    let mut nested = bytes[..tag_at].to_vec();
    nested.extend(std::iter::repeat_n(6u8, 128));
    nested.extend_from_slice(&bytes[tag_at..]);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&nested);

    assert!(
        bcs::from_bytes::<TransactionData>(&nested).is_ok(),
        "historical decoder accepts 128 nested vectors"
    );
    assert!(
        bcs::from_bytes::<sui_sdk_types::Transaction>(&nested).is_err(),
        "SDK caps type-tag nesting at 128 levels"
    );
    assert!(
        crate::transaction_string_to_visual_sign(&encoded, VisualSignOptions::default()).is_err(),
        "public converter fails closed"
    );
}
