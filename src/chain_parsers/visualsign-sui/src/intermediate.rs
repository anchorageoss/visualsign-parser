//! Sui intermediate output: a Borsh-serialized, structured decode of a Sui
//! programmable transaction, attached to the conversion result alongside the
//! human-readable `SignablePayload`.
//!
//! [`SUI_INTERMEDIATE_SCHEMA_VERSION`] is the first field so a shape change is
//! a single, reviewable signal that forces mirrored decoders to update.
//!
//! The output is built from the raw BCS `TransactionData`, not from the
//! JSON-RPC rendering the preview uses, so nothing is dropped (Publish and
//! Upgrade module bytes) or re-typed (pure inputs, whose RPC typing depends on
//! the order commands use them).
//!
//! Encoding rules:
//! - One [`SuiIntermediateCommand`] per PTB command, in transaction order.
//! - Addresses, object IDs and other hex values are `0x`-prefixed lowercase
//!   hex, including inside type arguments, which use Move's canonical
//!   full-length form. Numeric values are base-10 integer strings.
//! - `call_args_json` is a canonical JSON object (keys alphabetized at every
//!   nesting level) for every command. `MoveCall` lists its `arguments`
//!   positionally; native commands name theirs.
//! - Where a command fixes an argument's type (a `TransferObjects` recipient
//!   address, `SplitCoins` amounts), the argument is either the decoded literal
//!   (a string) or a `{"result":i}` / `{"nested_result":[i,j]}` reference.
//!   Any other argument is a tagged value: `{"pure":"0x<bcs>"}`,
//!   `{"object":{..}}`, `{"funds_withdrawal":{..}}`, `{"gas_coin":true}`,
//!   `{"result":i}` or `{"nested_result":[i,j]}`.
//! - Gas payment objects are canonical JSON object references:
//!   `{"digest":"0x..","id":"0x..","version":"N"}`.
//! - No intermediate output is emitted at all, rather than partial output, if
//!   any argument cannot be read (an out-of-range input, a pure value of the
//!   wrong length for its slot, or a non-literal, non-result value in a typed
//!   slot), or if the arguments' JSON would exceed
//!   [`MAX_CALL_ARGS_JSON_BYTES`] across the transaction.
//! - `expiration` is empty for no expiration, else a canonical JSON object.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use borsh::{BorshDeserialize, BorshSerialize};
use serde_json::{Map, Value, json};
use sui_sdk_types::hash::Hasher;
use sui_sdk_types::{
    Address, Argument, Command, FundsWithdrawal, Input, Mutability, ObjectReference, SharedInput,
    Transaction, TransactionExpiration, TransactionKind, TypeTag, WithdrawFrom,
};

use crate::types::{SuiAddress, type_tag_to_canonical_string};
use visualsign::canonical_json;
use visualsign::errors::VisualSignError;

/// Version of the `SuiIntermediateOutput` Borsh schema. Bump on ANY change to
/// the shape below or to the encoding rules in the module docs.
pub const SUI_INTERMEDIATE_SCHEMA_VERSION: u16 = 1;

/// Cap on the JSON emitted for command arguments across one transaction.
///
/// Every reference to an input re-emits that input's JSON, so a small
/// transaction that references a large input many times would otherwise
/// expand by orders of magnitude. A maximum-size Sui transaction (128 KiB)
/// emitted once as hex is about 256 KiB, so 1 MiB leaves room for any
/// legitimate transaction while bounding the work for a hostile one.
pub const MAX_CALL_ARGS_JSON_BYTES: usize = 1 << 20;

/// Top-level Sui intermediate output.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SuiIntermediateOutput {
    /// Always [`SUI_INTERMEDIATE_SCHEMA_VERSION`]. First field so decoders can
    /// gate on it before reading the rest.
    pub schema_version: u16,
    pub sender: String,
    pub commands: Vec<SuiIntermediateCommand>,
    pub gas: SuiGasSummary,
    pub expiration: String,
}

/// One PTB command.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SuiIntermediateCommand {
    /// One of `MoveCall`, `TransferObjects`, `SplitCoins`, `MergeCoins`,
    /// `Publish`, `MakeMoveVec`, `Upgrade`.
    pub command_kind: String,
    /// `MoveCall` only; empty for every other kind.
    pub package: String,
    /// `MoveCall` only; empty for every other kind.
    pub module_name: String,
    /// `MoveCall` only; empty for every other kind.
    pub function: String,
    /// `MoveCall` type arguments, or the element type of a `MakeMoveVec` that
    /// names one.
    pub type_arguments: Vec<String>,
    pub call_args_json: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct SuiGasSummary {
    pub budget: String,
    pub price: String,
    pub owner: String,
    /// One canonical JSON object reference per gas payment object.
    pub payment_objects: Vec<String>,
}

/// Build the intermediate output for a programmable transaction.
///
/// # Errors
/// Returns `VisualSignError::DecodeError` for transaction kinds other than a
/// programmable transaction, for any argument that cannot be read, if the
/// arguments' JSON exceeds [`MAX_CALL_ARGS_JSON_BYTES`], or if the JSON cannot
/// be serialized. Callers emit no intermediate output on error.
pub(crate) fn build_intermediate_output(
    transaction: &Transaction,
) -> Result<SuiIntermediateOutput, VisualSignError> {
    let TransactionKind::ProgrammableTransaction(ptb) = &transaction.kind else {
        return Err(decode_error(
            "Sui intermediate output only covers programmable transactions",
        ));
    };

    let mut budget = Budget::new(MAX_CALL_ARGS_JSON_BYTES);
    let gas_data = &transaction.gas_payment;
    Ok(SuiIntermediateOutput {
        schema_version: SUI_INTERMEDIATE_SCHEMA_VERSION,
        sender: SuiAddress::from(transaction.sender).to_string(),
        commands: ptb
            .commands
            .iter()
            .map(|command| build_command(command, &ptb.inputs, &mut budget))
            .collect::<Result<_, _>>()?,
        gas: SuiGasSummary {
            budget: gas_data.budget.to_string(),
            price: gas_data.price.to_string(),
            owner: SuiAddress::from(gas_data.owner).to_string(),
            payment_objects: gas_data
                .objects
                .iter()
                .map(|object_ref| to_canonical_string(&object_ref_json(object_ref)))
                .collect::<Result<_, _>>()?,
        },
        expiration: expiration_string(&transaction.expiration)?,
    })
}

/// Running total of argument JSON, checked as each argument is built so a
/// hostile transaction stops costing work once it is over the cap.
struct Budget {
    remaining: usize,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), VisualSignError> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(|| {
            decode_error(&format!(
                "Argument JSON exceeds {MAX_CALL_ARGS_JSON_BYTES} bytes"
            ))
        })?;
        Ok(())
    }

    /// Charge for an already-built value. Only used for values whose size does
    /// not scale with an input's length beyond a small constant factor.
    fn charge_value(&mut self, value: &Value) -> Result<(), VisualSignError> {
        self.charge(value.to_string().len())
    }
}

fn build_command(
    command: &Command,
    inputs: &[Input],
    budget: &mut Budget,
) -> Result<SuiIntermediateCommand, VisualSignError> {
    let (command_kind, type_arguments) = match command {
        Command::MoveCall(call) => {
            return Ok(SuiIntermediateCommand {
                command_kind: "MoveCall".to_string(),
                package: call.package.to_string(),
                module_name: call.module.to_string(),
                function: call.function.to_string(),
                type_arguments: call.type_arguments.iter().map(canonical_type).collect(),
                call_args_json: call_args_json(command, inputs, budget)?,
            });
        }
        Command::TransferObjects(..) => ("TransferObjects", vec![]),
        Command::SplitCoins(..) => ("SplitCoins", vec![]),
        Command::MergeCoins(..) => ("MergeCoins", vec![]),
        Command::Publish(..) => ("Publish", vec![]),
        Command::Upgrade(..) => ("Upgrade", vec![]),
        Command::MakeMoveVector(make) => (
            "MakeMoveVec",
            make.type_.iter().map(canonical_type).collect(),
        ),
        _ => return Err(decode_error("Unsupported command kind")),
    };
    Ok(SuiIntermediateCommand {
        command_kind: command_kind.to_string(),
        package: String::new(),
        module_name: String::new(),
        function: String::new(),
        type_arguments,
        call_args_json: call_args_json(command, inputs, budget)?,
    })
}

fn canonical_type(type_tag: &TypeTag) -> String {
    type_tag_to_canonical_string(type_tag)
}

/// Sui's package digest: Blake2b-256 over the sorted list of each module's
/// Blake2b-256 hash and each dependency ID, as
/// `MovePackage::compute_digest_for_modules_and_deps(.., hash_modules: true)`.
fn compute_package_digest(modules: &[Vec<u8>], dependencies: &[Address]) -> [u8; 32] {
    let module_digests: Vec<[u8; 32]> = modules
        .iter()
        .map(|module| Hasher::digest(module).into_inner())
        .collect();
    let mut components: Vec<&[u8]> = module_digests
        .iter()
        .map(<[u8; 32]>::as_slice)
        .chain(dependencies.iter().map(Address::as_bytes))
        .collect();
    components.sort_unstable();
    let mut hasher = Hasher::new();
    for component in components {
        hasher.update(component);
    }
    hasher.finalize().into_inner()
}

/// How a command's signature types an argument slot, which decides how a pure
/// input in that slot is decoded and which arguments the slot accepts.
#[derive(Clone, Copy)]
enum Slot {
    Address,
    U64,
    Untyped,
}

fn call_args_json(
    command: &Command,
    inputs: &[Input],
    budget: &mut Budget,
) -> Result<String, VisualSignError> {
    let ids = |ids: &[Address]| Value::Array(ids.iter().map(|id| id.to_string().into()).collect());
    let package_digest = |modules: &[Vec<u8>], dependencies: &[Address]| -> Value {
        // Sui's own package digest (module hashes and dependency IDs, sorted),
        // so two publishes of different bytecode never encode the same.
        let digest = compute_package_digest(modules, dependencies);
        format!("0x{}", hex::encode(digest)).into()
    };

    let entries: Vec<(&str, Value)> = match command {
        Command::MoveCall(call) => vec![(
            "arguments",
            arguments_json(&call.arguments, inputs, Slot::Untyped, budget)?,
        )],
        Command::TransferObjects(transfer) => vec![
            (
                "objects",
                arguments_json(&transfer.objects, inputs, Slot::Untyped, budget)?,
            ),
            (
                "recipient",
                argument_json(transfer.address, inputs, Slot::Address, budget)?,
            ),
        ],
        Command::SplitCoins(split) => vec![
            (
                "amounts",
                arguments_json(&split.amounts, inputs, Slot::U64, budget)?,
            ),
            (
                "coin",
                argument_json(split.coin, inputs, Slot::Untyped, budget)?,
            ),
        ],
        Command::MergeCoins(merge) => vec![
            (
                "destination",
                argument_json(merge.coin, inputs, Slot::Untyped, budget)?,
            ),
            (
                "sources",
                arguments_json(&merge.coins_to_merge, inputs, Slot::Untyped, budget)?,
            ),
        ],
        Command::Publish(publish) => vec![
            ("dependencies", ids(&publish.dependencies)),
            (
                "package_digest",
                package_digest(&publish.modules, &publish.dependencies),
            ),
        ],
        Command::Upgrade(upgrade) => vec![
            ("dependencies", ids(&upgrade.dependencies)),
            ("package", upgrade.package.to_string().into()),
            (
                "package_digest",
                package_digest(&upgrade.modules, &upgrade.dependencies),
            ),
            (
                "ticket",
                argument_json(upgrade.ticket, inputs, Slot::Untyped, budget)?,
            ),
        ],
        Command::MakeMoveVector(make) => vec![(
            "elements",
            arguments_json(&make.elements, inputs, Slot::Untyped, budget)?,
        )],
        _ => return Err(decode_error("Unsupported command kind")),
    };

    let map: Map<String, Value> = entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    to_canonical_string(&Value::Object(map))
}

fn arguments_json(
    arguments: &[Argument],
    inputs: &[Input],
    slot: Slot,
    budget: &mut Budget,
) -> Result<Value, VisualSignError> {
    arguments
        .iter()
        .map(|argument| argument_json(*argument, inputs, slot, budget))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn argument_json(
    argument: Argument,
    inputs: &[Input],
    slot: Slot,
    budget: &mut Budget,
) -> Result<Value, VisualSignError> {
    let value = match argument {
        Argument::Result(command) => json!({ "result": command }),
        Argument::NestedResult(command, index) => json!({ "nested_result": [command, index] }),
        Argument::Gas => match slot {
            Slot::Untyped => json!({ "gas_coin": true }),
            Slot::Address | Slot::U64 => {
                return Err(decode_error("Gas coin in an address or u64 slot"));
            }
        },
        Argument::Input(index) => {
            let input = inputs
                .get(usize::from(index))
                .ok_or_else(|| decode_error(&format!("Input {index} is out of range")))?;
            return input_json(input, slot, budget);
        }
    };
    budget.charge_value(&value)?;
    Ok(value)
}

fn input_json(input: &Input, slot: Slot, budget: &mut Budget) -> Result<Value, VisualSignError> {
    let value = match (input, slot) {
        (Input::Pure(bytes), Slot::Untyped) => {
            // Charged before the hex is allocated: `{"pure":"0x` + hex + `"}`.
            budget.charge(13 + 2 * bytes.len())?;
            return Ok(json!({ "pure": format!("0x{}", hex::encode(bytes)) }));
        }
        (Input::Pure(bytes), Slot::Address) => SuiAddress::from_bytes(bytes)
            .map(|address| Value::from(address.to_string()))
            .map_err(|_| decode_error("Address argument is not 32 bytes"))?,
        (Input::Pure(bytes), Slot::U64) => <[u8; 8]>::try_from(bytes.as_slice())
            .map(|le| Value::from(u64::from_le_bytes(le).to_string()))
            .map_err(|_| decode_error("u64 argument is not 8 bytes"))?,
        (Input::ImmutableOrOwned(object_ref), Slot::Untyped) => {
            json!({ "object": object_ref_with_kind("imm_or_owned", object_ref) })
        }
        (Input::Receiving(object_ref), Slot::Untyped) => {
            json!({ "object": object_ref_with_kind("receiving", object_ref) })
        }
        (Input::Shared(shared), Slot::Untyped) => json!({ "object": shared_object_json(shared) }),
        (Input::FundsWithdrawal(withdrawal), Slot::Untyped) => {
            json!({ "funds_withdrawal": funds_withdrawal_json(withdrawal)? })
        }
        (
            Input::ImmutableOrOwned(_)
            | Input::Receiving(_)
            | Input::Shared(_)
            | Input::FundsWithdrawal(_),
            Slot::Address | Slot::U64,
        ) => {
            return Err(decode_error(
                "Object or funds withdrawal in an address or u64 slot",
            ));
        }
        _ => return Err(decode_error("Unsupported input kind")),
    };
    budget.charge_value(&value)?;
    Ok(value)
}

fn object_ref_json(object_ref: &ObjectReference) -> Value {
    json!({
        "digest": format!("0x{}", hex::encode(object_ref.digest().inner())),
        "id": object_ref.object_id().to_string(),
        "version": object_ref.version().to_string(),
    })
}

fn object_ref_with_kind(kind: &str, object_ref: &ObjectReference) -> Value {
    let mut value = object_ref_json(object_ref);
    if let Value::Object(map) = &mut value {
        map.insert("kind".to_string(), kind.into());
    }
    value
}

fn shared_object_json(shared: &SharedInput) -> Value {
    json!({
        "id": shared.object_id().to_string(),
        "initial_shared_version": shared.version().to_string(),
        "kind": "shared",
        "mutability": match shared.mutability() {
            Mutability::Immutable => "immutable",
            Mutability::Mutable => "mutable",
            Mutability::NonExclusiveWrite => "non_exclusive_write",
        },
    })
}

/// Schema v1 covers a `MaxAmountU64` reservation of a `Balance<T>` withdrawn
/// from the sender or sponsor; anything newer drops the output.
fn funds_withdrawal_json(withdrawal: &FundsWithdrawal) -> Result<Value, VisualSignError> {
    let max_amount = withdrawal
        .amount()
        .ok_or_else(|| decode_error("Unsupported funds withdrawal reservation"))?;
    let withdraw_from = match withdrawal.source() {
        WithdrawFrom::Sender => "sender",
        WithdrawFrom::Sponsor => "sponsor",
        _ => return Err(decode_error("Unsupported funds withdrawal source")),
    };
    Ok(json!({
        "max_amount": max_amount.to_string(),
        "type": format!(
            "{}::balance::Balance<{}>",
            Address::TWO,
            type_tag_to_canonical_string(withdrawal.coin_type())
        ),
        "withdraw_from": withdraw_from,
    }))
}

fn expiration_string(expiration: &TransactionExpiration) -> Result<String, VisualSignError> {
    let optional = |value: Option<u64>| {
        value.map_or(Value::Null, |epoch_or_timestamp| {
            epoch_or_timestamp.to_string().into()
        })
    };
    let value = match expiration {
        TransactionExpiration::None => return Ok(String::new()),
        TransactionExpiration::Epoch(epoch) => json!({ "epoch": epoch.to_string() }),
        TransactionExpiration::ValidDuring {
            min_epoch,
            max_epoch,
            min_timestamp,
            max_timestamp,
            chain,
            nonce,
        } => json!({
            // Sui's short chain identifier: the first 4 bytes of the genesis
            // checkpoint digest, as hex.
            "chain": format!("0x{}", hex::encode(&chain.inner()[..4])),
            "max_epoch": optional(*max_epoch),
            "max_timestamp": optional(*max_timestamp),
            "min_epoch": optional(*min_epoch),
            "min_timestamp": optional(*min_timestamp),
            "nonce": nonce.to_string(),
        }),
        // `Validity` (allowed proposers) and anything newer is not in schema v1.
        _ => return Err(decode_error("Unsupported transaction expiration")),
    };
    to_canonical_string(&value)
}

fn to_canonical_string(value: &Value) -> Result<String, VisualSignError> {
    canonical_json::to_canonical_string(value)
        .map_err(|err| decode_error(&format!("Failed to serialize JSON: {err}")))
}

fn decode_error(message: &str) -> VisualSignError {
    VisualSignError::DecodeError(message.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::SuiVisualSignConverter;
    use crate::core::SuiTransactionWrapper;

    use sui_sdk_types::{
        Digest, Identifier, MakeMoveVector, MergeCoins, MoveCall, ProgrammableTransaction, Publish,
        SplitCoins, TransferObjects, Upgrade,
    };
    use visualsign::vsptrait::{
        ConversionResult, Transaction as _, VisualSignConverter, VisualSignOptions,
    };

    const HASHI_FIXTURE: &str = include_str!("integrations/hashi/aggregated_test_data.json");

    /// Plain PTB with native commands only (`SplitCoins` of the gas coin, then
    /// `TransferObjects` of the split coin).
    const PLAIN_PTB: &str = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";

    const DEPOSIT_DIGEST: &str = "4WGVP4rYLHxHCoA9jFjFFCoomNMxmKAVnSJsdGhMUSBF";
    /// Withdrawal funded by `balance::redeem_funds` of an address-balance withdrawal.
    const REDEEM_WITHDRAWAL_DIGEST: &str = "58XSeQkQffKNz1TJ4vJmNaeMvEm6N6LCN889YRi8yhhW";
    /// Withdrawal cancellation refunded with `TransferObjects`.
    const CANCEL_TRANSFER_DIGEST: &str = "4ahHNd8LuyYPJ8CucP7y5Quf5RQv8Di2LbFg7fd6TU3d";
    /// Three withdrawal cancellations, each refunded with `coin::send_funds`.
    const CANCEL_BATCH_DIGEST: &str = "2WCfwPwkgu2nDZDV8hJNZrSELNtyftgqzK1H1gCM8nsQ";
    const HASHI_PACKAGE: &str =
        "0x5e0b99c31f691742acea5de4ca1bf53c398d5639fed6503a51ffe8035dc68485";

    // Golden intermediate output, hex-encoded borsh. Any change to these bytes
    // is a change to the published output and must be deliberate.
    const PLAIN_PTB_GOLDEN: &str = "010042000000307864366539326530303265323663336166623230383830303163316235383230623634663262633335316266646135613233343261636266396635633163616332020000000a00000053706c6974436f696e73000000000000000000000000000000002d0000007b22616d6f756e7473223a5b2231303030225d2c22636f696e223a7b226761735f636f696e223a747275657d7d0f0000005472616e736665724f626a65637473000000000000000000000000000000006b0000007b226f626a65637473223a5b7b22726573756c74223a307d5d2c22726563697069656e74223a22307861316533616535353163326162653363366263656132326433663139333536613634373038366436626435366163306430396534656565643036323930623736227d070000003530303030303004000000313030304200000030786436653932653030326532366333616662323038383030316331623538323062363466326263333531626664613561323334326163626639663563316361633201000000af0000007b22646967657374223a22307837346363323138343030626231663066396262646633326537636139623238363232323565343061643066316334646438666262656166663166323362326436222c226964223a22307863623438356566383061306261663930373961363231633432633862306362363563653939383665316338316664313665653838303338663030346135616564222c2276657273696f6e223a22333439313739383931227d00000000";
    const DEPOSIT_GOLDEN: &str = "01004200000030786435613035616233616538373331323436666130663362623064393137323565346631336637373863383634323930633735353332313661353665326539303003000000080000004d6f766543616c6c42000000307835653062393963333166363931373432616365613564653463613162663533633339386435363339666564363530336135316666653830333564633638343835040000007574786f070000007574786f5f696400000000730000007b22617267756d656e7473223a5b7b2270757265223a22307830613366316661613062363533626666656138303738663238363233386637366136336661663034343337636234653935303765326233333463313234316165227d2c7b2270757265223a2230783034303030303030227d5d7d080000004d6f766543616c6c42000000307835653062393963333166363931373432616365613564653463613162663533633339386435363339666564363530336135316666653830333564633638343835040000007574786f040000007574786f000000008a0000007b22617267756d656e7473223a5b7b22726573756c74223a307d2c7b2270757265223a22307830623763303130303030303030303030227d2c7b2270757265223a223078303164356130356162336165383733313234366661306633626230643931373235653466313366373738633836343239306337353533323136613536653265393030227d5d7d080000004d6f766543616c6c42000000307835653062393963333166363931373432616365613564653463613162663533633339386435363339666564363530336135316666653830333564633638343835070000006465706f736974070000006465706f73697400000000540100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307833343265393038303433646236303930353538656437373636646639356266633336303938383162326561633765646235333433646435626233646464316262222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b22726573756c74223a317d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d5d7d0700000037353936313434040000003130303042000000307864356130356162336165383733313234366661306633626230643931373235653466313366373738633836343239306337353533323136613536653265393030000000007b0000007b22636861696e223a2230783463373861646163222c226d61785f65706f6368223a2231323534222c226d61785f74696d657374616d70223a6e756c6c2c226d696e5f65706f6368223a2231323533222c226d696e5f74696d657374616d70223a6e756c6c2c226e6f6e6365223a2233333739313430373733227d";
    const REDEEM_WITHDRAWAL_GOLDEN: &str = "01004200000030783564303230323363643830313930633862636230346136396135326662356432356333646230373636333233373031633032393061313466306533313037613802000000080000004d6f766543616c6c420000003078303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030320700000062616c616e63650c00000072656465656d5f66756e6473010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a425443000100007b22617267756d656e7473223a5b7b2266756e64735f7769746864726177616c223a7b226d61785f616d6f756e74223a223330303030222c2274797065223a223078303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030323a3a62616c616e63653a3a42616c616e63653c3078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a4254433e222c2277697468647261775f66726f6d223a2273656e646572227d7d5d7d080000004d6f766543616c6c4200000030783565306239396333316636393137343261636561356465346361316266353363333938643536333966656436353033613531666665383033356463363834383508000000776974686472617712000000726571756573745f7769746864726177616c00000000970100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307833343265393038303433646236303930353538656437373636646639356266633336303938383162326561633765646235333433646435626233646464316262222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d2c7b226e65737465645f726573756c74223a5b302c305d7d2c7b2270757265223a223078313433393664623639666230633730316264323631343637373765306330306563313332336139626132227d5d7d070000003931313537383404000000313030304200000030783564303230323363643830313930633862636230346136396135326662356432356333646230373636333233373031633032393061313466306533313037613801000000b00000007b22646967657374223a22307832336433306432313462633231306236336366653565353365373335613937333632353065663631633731653833643365346362653362303533353862333432222c226964223a22307830373165353461376333396135666433623461393932633864656532396531613933326438353937666562396365663834316437613632613632306639663033222c2276657273696f6e223a2231303534333034333339227d00000000";
    const CANCEL_TRANSFER_GOLDEN: &str = "01004200000030786438356136623435663435356336353832323464363634653735393039666238396536336232626531613134663436613238623564666431363135386533343403000000080000004d6f766543616c6c420000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438350800000077697468647261771100000063616e63656c5f7769746864726177616c00000000950100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307833343265393038303433646236303930353538656437373636646639356266633336303938383162326561633765646235333433646435626233646464316262222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b2270757265223a22307864373764313930343265366236646231636364613031393830396132313934303766633836373065613666626635653364613965363633616639633465373965227d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c00000066726f6d5f62616c616e6365010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a4254431c0000007b22617267756d656e7473223a5b7b22726573756c74223a307d5d7d0f0000005472616e736665724f626a6563747300000000000000000000000000000000760000007b226f626a65637473223a5b7b226e65737465645f726573756c74223a5b312c305d7d5d2c22726563697069656e74223a22307864383561366234356634353563363538323234643636346537353930396662383965363362326265316131346634366132386235646664313631353865333434227d070000003230303030303004000000313030304200000030786438356136623435663435356336353832323464363634653735393039666238396536336232626531613134663436613238623564666431363135386533343401000000b00000007b22646967657374223a22307839613031663065383834613736323064306161363635303035376264643839373839383461663063353735663863383561636630353333613030373238386533222c226964223a22307866326332626237303763373435346162653765663238383962373332326265306135383633326461386462353366373339646666323163626331613833303038222c2276657273696f6e223a2231303537373034343536227d00000000";
    const CANCEL_BATCH_GOLDEN: &str = "01004200000030783037363036636430356363363662353639366133383234313136353661346337393765333734386438383832316663366631383738356431363230613434343409000000080000004d6f766543616c6c420000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438350800000077697468647261771100000063616e63656c5f7769746864726177616c00000000950100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307833343265393038303433646236303930353538656437373636646639356266633336303938383162326561633765646235333433646435626233646464316262222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b2270757265223a22307835306462663265343531336663323333633066666334343932343065646466623130656239656661636638646163366534666365336534346564366663323434227d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c00000066726f6d5f62616c616e6365010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a4254431c0000007b22617267756d656e7473223a5b7b22726573756c74223a307d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0a00000073656e645f66756e6473010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a425443750000007b22617267756d656e7473223a5b7b226e65737465645f726573756c74223a5b312c305d7d2c7b2270757265223a22307830373630366364303563633636623536393661333832343131363536613463373937653337343864383838323166633666313837383564313632306134343434227d5d7d080000004d6f766543616c6c420000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438350800000077697468647261771100000063616e63656c5f7769746864726177616c00000000950100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307833343265393038303433646236303930353538656437373636646639356266633336303938383162326561633765646235333433646435626233646464316262222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b2270757265223a22307834343635623238663235663366393531376237366363323839633833633064373137303930653136373335373564316338383365316165353366636666666438227d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c00000066726f6d5f62616c616e6365010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a4254431c0000007b22617267756d656e7473223a5b7b22726573756c74223a337d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0a00000073656e645f66756e6473010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a425443750000007b22617267756d656e7473223a5b7b226e65737465645f726573756c74223a5b342c305d7d2c7b2270757265223a22307830373630366364303563633636623536393661333832343131363536613463373937653337343864383838323166633666313837383564313632306134343434227d5d7d080000004d6f766543616c6c420000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438350800000077697468647261771100000063616e63656c5f7769746864726177616c00000000950100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307833343265393038303433646236303930353538656437373636646639356266633336303938383162326561633765646235333433646435626233646464316262222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b2270757265223a22307836343965346463343439663238613461336663643166383639333466376534663633313138316362356561373032653863373334653966363364373236373965227d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c00000066726f6d5f62616c616e6365010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a4254431c0000007b22617267756d656e7473223a5b7b22726573756c74223a367d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0a00000073656e645f66756e6473010000004c0000003078356530623939633331663639313734326163656135646534636131626635336333393864353633396665643635303361353166666538303335646336383438353a3a6274633a3a425443750000007b22617267756d656e7473223a5b7b226e65737465645f726573756c74223a5b372c305d7d2c7b2270757265223a22307830373630366364303563633636623536393661333832343131363536613463373937653337343864383838323166633666313837383564313632306134343434227d5d7d070000003230303030303004000000313030304200000030783037363036636430356363363662353639366133383234313136353661346337393765333734386438383832316663366631383738356431363230613434343401000000b00000007b22646967657374223a22307838393465333864336130663430623064643736633833643361363734306332366261643334393965613564646261396435663639333532653165363965323862222c226964223a22307830663762663437313037366664336531313032386137356465626437626637373030303065343238383739396631633466353938313864626531383162373439222c2276657273696f6e223a2231303536353239383731227d00000000";

    fn hashi_b64(section: &str, function: &str, digest: &str) -> String {
        let fixture: serde_json::Value = serde_json::from_str(HASHI_FIXTURE).unwrap();
        fixture[section][function]["operations"][digest]["data"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn withdrawal_b64(digest: &str) -> String {
        hashi_b64("withdraw", "request_withdrawal", digest)
    }

    fn tx_from_b64(b64: &str) -> Transaction {
        SuiTransactionWrapper::from_string(b64)
            .unwrap()
            .inner()
            .clone()
    }

    fn convert(tx: Transaction, include_intermediate_output: bool) -> ConversionResult {
        let options = VisualSignOptions {
            include_intermediate_output,
            ..VisualSignOptions::default()
        };
        SuiVisualSignConverter
            .to_visual_sign_payload(SuiTransactionWrapper::new(tx), options)
            .unwrap()
    }

    /// Emitted bytes for a base64 transaction, through the converter.
    fn emitted_bytes(b64: &str) -> Vec<u8> {
        convert(tx_from_b64(b64), true)
            .intermediate_output
            .expect("intermediate output emitted")
    }

    fn output_for(tx: &Transaction) -> SuiIntermediateOutput {
        build_intermediate_output(tx).unwrap()
    }

    fn call_args(output: &SuiIntermediateOutput, command: usize) -> Value {
        serde_json::from_str(&output.commands[command].call_args_json).unwrap()
    }

    fn programmable(tx: &mut Transaction) -> &mut ProgrammableTransaction {
        match &mut tx.kind {
            TransactionKind::ProgrammableTransaction(programmable_transaction) => {
                programmable_transaction
            }
            _ => panic!("expected a programmable transaction"),
        }
    }

    /// The plain PTB with its inputs and commands replaced.
    fn ptb_with(inputs: Vec<Input>, commands: Vec<Command>) -> Transaction {
        let mut tx = tx_from_b64(PLAIN_PTB);
        let programmable_transaction = programmable(&mut tx);
        programmable_transaction.inputs = inputs;
        programmable_transaction.commands = commands;
        tx
    }

    fn pure<T: serde::Serialize>(value: &T) -> Input {
        Input::Pure(bcs::to_bytes(value).unwrap())
    }

    fn id(byte: u8) -> Address {
        Address::new([byte; 32])
    }

    fn owned_object(byte: u8) -> Input {
        Input::ImmutableOrOwned(ObjectReference::new(id(byte), 1, Digest::new([0; 32])))
    }

    fn shared(byte: u8, version: u64, mutability: Mutability) -> Input {
        Input::Shared(SharedInput::new(id(byte), version, mutability))
    }

    fn ident(name: &str) -> Identifier {
        Identifier::new(name).unwrap()
    }

    fn transfer(objects: Vec<Argument>, address: Argument) -> Command {
        Command::TransferObjects(TransferObjects { objects, address })
    }

    fn split(coin: Argument, amounts: Vec<Argument>) -> Command {
        Command::SplitCoins(SplitCoins { coin, amounts })
    }

    fn make_move_vec(type_: Option<TypeTag>, elements: Vec<Argument>) -> Command {
        Command::MakeMoveVector(MakeMoveVector { type_, elements })
    }

    fn owned_object_json(byte: u8) -> String {
        format!(
            r#"{{"object":{{"digest":"{}","id":"{}","kind":"imm_or_owned","version":"1"}}}}"#,
            hex_id(0),
            hex_id(byte)
        )
    }

    fn move_call(package: Address, arguments: Vec<Argument>) -> Command {
        Command::MoveCall(MoveCall {
            package,
            module: ident("pool"),
            function: ident("swap"),
            type_arguments: vec![],
            arguments,
        })
    }

    fn hex_id(byte: u8) -> String {
        format!("0x{}", hex::encode([byte; 32]))
    }

    fn assert_golden(b64: &str, golden: &str) {
        let bytes = emitted_bytes(b64);
        assert_eq!(hex::encode(&bytes), golden);
        // Deterministic: a second conversion yields identical bytes.
        assert_eq!(emitted_bytes(b64), bytes);
        // `from_slice` fails on trailing bytes, so this also proves the
        // decoder consumes the whole blob.
        let decoded: SuiIntermediateOutput = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded, output_for(&tx_from_b64(b64)));
        assert_eq!(decoded.schema_version, SUI_INTERMEDIATE_SCHEMA_VERSION);
    }

    #[test]
    fn plain_ptb_golden() {
        assert_golden(PLAIN_PTB, PLAIN_PTB_GOLDEN);
    }

    #[test]
    fn hashi_deposit_golden() {
        assert_golden(
            &hashi_b64("deposit", "deposit", DEPOSIT_DIGEST),
            DEPOSIT_GOLDEN,
        );
    }

    #[test]
    fn hashi_redeem_withdrawal_golden() {
        assert_golden(
            &withdrawal_b64(REDEEM_WITHDRAWAL_DIGEST),
            REDEEM_WITHDRAWAL_GOLDEN,
        );
    }

    #[test]
    fn hashi_cancel_transfer_golden() {
        assert_golden(
            &hashi_b64("withdraw", "cancel_withdrawal", CANCEL_TRANSFER_DIGEST),
            CANCEL_TRANSFER_GOLDEN,
        );
    }

    #[test]
    fn hashi_cancel_batch_golden() {
        assert_golden(
            &hashi_b64("withdraw", "cancel_withdrawal", CANCEL_BATCH_DIGEST),
            CANCEL_BATCH_GOLDEN,
        );
    }

    #[test]
    fn hashi_cancellations_list_every_call() {
        let output = output_for(&tx_from_b64(&hashi_b64(
            "withdraw",
            "cancel_withdrawal",
            CANCEL_BATCH_DIGEST,
        )));
        let calls: Vec<_> = output
            .commands
            .iter()
            .map(|command| {
                (
                    command.command_kind.as_str(),
                    command.module_name.as_str(),
                    command.function.as_str(),
                )
            })
            .collect();
        let cancel = [
            ("MoveCall", "withdraw", "cancel_withdrawal"),
            ("MoveCall", "coin", "from_balance"),
            ("MoveCall", "coin", "send_funds"),
        ];
        assert_eq!(calls, [cancel, cancel, cancel].concat());
        let btc = format!("{HASHI_PACKAGE}::btc::BTC");
        for command in output.commands.iter().skip(1).step_by(3) {
            assert_eq!(command.type_arguments, std::slice::from_ref(&btc));
        }
        // The request id is the second argument of each `cancel_withdrawal`.
        assert_eq!(
            call_args(&output, 6)["arguments"][1],
            json!({ "pure": "0x649e4dc449f28a4a3fcd1f86934f7e4f631181cb5ea702e8c734e9f63d72679e" })
        );
    }

    #[test]
    fn plain_ptb_fields() {
        let decoded: SuiIntermediateOutput = borsh::from_slice(&emitted_bytes(PLAIN_PTB)).unwrap();

        let sender = "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2";
        assert_eq!(decoded.sender, sender);
        let commands: Vec<(&str, &str)> = decoded
            .commands
            .iter()
            .map(|command| {
                (
                    command.command_kind.as_str(),
                    command.call_args_json.as_str(),
                )
            })
            .collect();
        assert_eq!(
            commands,
            [
                (
                    "SplitCoins",
                    r#"{"amounts":["1000"],"coin":{"gas_coin":true}}"#
                ),
                (
                    "TransferObjects",
                    r#"{"objects":[{"result":0}],"recipient":"0xa1e3ae551c2abe3c6bcea22d3f19356a647086d6bd56ac0d09e4eeed06290b76"}"#
                ),
            ]
        );
        let gas_object = tx_from_b64(PLAIN_PTB).gas_payment.objects[0].clone();
        let (id, version, digest) = (
            gas_object.object_id(),
            gas_object.version(),
            gas_object.digest(),
        );
        assert_eq!(
            id.to_string(),
            "0xcb485ef80a0baf9079a621c42c8b0cb65ce9986e1c81fd16ee88038f004a5aed"
        );
        assert_eq!(
            decoded.gas,
            SuiGasSummary {
                budget: "5000000".to_string(),
                price: "1000".to_string(),
                owner: sender.to_string(),
                payment_objects: vec![format!(
                    r#"{{"digest":"0x{}","id":"{id}","version":"{}"}}"#,
                    hex::encode(digest.inner()),
                    version
                )],
            }
        );
        assert_eq!(decoded.expiration, "");
    }

    #[test]
    fn hashi_deposit_fields() {
        let output = output_for(&tx_from_b64(&hashi_b64(
            "deposit",
            "deposit",
            DEPOSIT_DIGEST,
        )));

        let hashi = HASHI_PACKAGE;
        let calls: Vec<_> = output
            .commands
            .iter()
            .map(|command| {
                (
                    command.command_kind.as_str(),
                    command.package.as_str(),
                    command.module_name.as_str(),
                    command.function.as_str(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                ("MoveCall", hashi, "utxo", "utxo_id"),
                ("MoveCall", hashi, "utxo", "utxo"),
                ("MoveCall", hashi, "deposit", "deposit"),
            ]
        );

        // utxo_id(txid, vout), utxo(utxo_id, amount, path), deposit(hashi, utxo, clock)
        let argument_counts: Vec<usize> = (0..3)
            .map(|command| {
                call_args(&output, command)["arguments"]
                    .as_array()
                    .unwrap()
                    .len()
            })
            .collect();
        assert_eq!(argument_counts, [2, 3, 3]);
        let deposit = call_args(&output, 2);
        let bridge = &deposit["arguments"][0]["object"];
        assert_eq!(bridge["kind"], "shared");
        assert_eq!(
            bridge["id"],
            "0x342e908043db6090558ed7766df95bfc3609881b2eac7edb5343dd5bb3ddd1bb"
        );
        assert_eq!(deposit["arguments"][1], json!({ "result": 1 }));

        assert_eq!(
            output.sender,
            "0xd5a05ab3ae8731246fa0f3bb0d91725e4f13f778c864290c7553216a56e2e900"
        );
        assert_eq!(output.gas.budget, "7596144");
        assert_eq!(output.gas.price, "1000");
        // Paid from the address balance, so no gas coins, with a `ValidDuring`
        // expiration on testnet's short chain id.
        assert!(output.gas.payment_objects.is_empty());
        let expiration: Value = serde_json::from_str(&output.expiration).unwrap();
        assert_eq!(expiration["chain"], "0x4c78adac");
        assert_eq!(expiration["min_epoch"], "1253");
        assert_eq!(expiration["max_epoch"], "1254");
    }

    #[test]
    fn redeem_withdrawal_encodes_funds_withdrawal_and_full_type() {
        let output = output_for(&tx_from_b64(&withdrawal_b64(REDEEM_WITHDRAWAL_DIGEST)));
        let btc = format!("{HASHI_PACKAGE}::btc::BTC");

        let redeem = &output.commands[0];
        assert_eq!(redeem.package, format!("0x{}", "0".repeat(63) + "2"));
        assert_eq!(
            (redeem.module_name.as_str(), redeem.function.as_str()),
            ("balance", "redeem_funds")
        );
        assert_eq!(redeem.type_arguments, std::slice::from_ref(&btc));
        let withdrawal = &call_args(&output, 0)["arguments"][0]["funds_withdrawal"];
        assert_eq!(withdrawal["max_amount"], "30000");
        assert_eq!(withdrawal["withdraw_from"], "sender");
        assert_eq!(
            withdrawal["type"],
            format!("0x{}::balance::Balance<{btc}>", "0".repeat(63) + "2")
        );
    }

    #[test]
    fn redeemed_amount_matches_the_preview() {
        // Drift guard: the intermediate output and the preview are built from
        // different representations of the same transaction.
        let b64 = withdrawal_b64(REDEEM_WITHDRAWAL_DIGEST);
        let result = convert(tx_from_b64(&b64), true);
        let decoded: SuiIntermediateOutput =
            borsh::from_slice(result.intermediate_output.as_ref().unwrap()).unwrap();
        let args: Value = serde_json::from_str(&decoded.commands[0].call_args_json).unwrap();
        assert_eq!(
            args["arguments"][0]["funds_withdrawal"]["max_amount"],
            "30000"
        );
        let preview = serde_json::to_string(&result.payload).unwrap();
        assert!(preview.contains("30000"), "{preview}");
    }

    /// One command of every kind, over pure, object, gas-coin and result
    /// arguments.
    fn all_kinds_tx() -> Transaction {
        let recipient = SuiAddress::from_bytes([0xab; 32]).unwrap();
        let package = id(0xca);
        let sui: TypeTag = "0x2::sui::SUI".parse().unwrap();
        ptb_with(
            vec![
                pure(&1_000u64),
                pure(&recipient),
                owned_object(0x11),
                owned_object(0x22),
            ],
            vec![
                Command::MoveCall(MoveCall {
                    package,
                    module: ident("pool"),
                    function: ident("swap"),
                    type_arguments: vec![sui, TypeTag::U64],
                    arguments: vec![Argument::Input(2), Argument::Input(0)],
                }),
                transfer(
                    vec![Argument::NestedResult(0, 1), Argument::Input(2)],
                    Argument::Input(1),
                ),
                split(Argument::Gas, vec![Argument::Input(0), Argument::Result(0)]),
                Command::MergeCoins(MergeCoins {
                    coin: Argument::Input(2),
                    coins_to_merge: vec![Argument::Input(3)],
                }),
                Command::Publish(Publish {
                    modules: vec![vec![0xde, 0xad]],
                    dependencies: vec![id(0x02)],
                }),
                make_move_vec(Some(TypeTag::U64), vec![Argument::Input(0)]),
                make_move_vec(None, vec![Argument::Input(2)]),
                Command::Upgrade(Upgrade {
                    modules: vec![vec![0xbe, 0xef]],
                    dependencies: vec![id(0x01)],
                    package,
                    ticket: Argument::Result(0),
                }),
            ],
        )
    }

    #[test]
    fn every_command_kind_is_encoded() {
        let output = output_for(&all_kinds_tx());
        let decoded: SuiIntermediateOutput =
            borsh::from_slice(&borsh::to_vec(&output).unwrap()).unwrap();
        assert_eq!(decoded, output);

        let kinds: Vec<&str> = output
            .commands
            .iter()
            .map(|command| command.command_kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            [
                "MoveCall",
                "TransferObjects",
                "SplitCoins",
                "MergeCoins",
                "Publish",
                "MakeMoveVec",
                "MakeMoveVec",
                "Upgrade",
            ]
        );

        let move_call = &output.commands[0];
        assert_eq!(move_call.package, hex_id(0xca));
        assert_eq!(move_call.module_name, "pool");
        assert_eq!(move_call.function, "swap");
        assert_eq!(
            move_call.type_arguments,
            [
                format!("0x{}::sui::SUI", "0".repeat(63) + "2"),
                "u64".to_string()
            ]
        );

        for command in &output.commands[1..] {
            assert_eq!(command.package, "", "{command:?}");
            assert_eq!(command.module_name, "", "{command:?}");
            assert_eq!(command.function, "", "{command:?}");
        }
        for command in &output.commands {
            let args: Value = serde_json::from_str(&command.call_args_json).unwrap();
            assert!(args.is_object(), "{command:?}");
        }
        assert_eq!(output.commands[5].type_arguments, ["u64"]);
        for command in [1, 2, 3, 4, 6, 7] {
            assert!(output.commands[command].type_arguments.is_empty());
        }
    }

    #[test]
    fn call_args_are_canonical_json() {
        let output = output_for(&all_kinds_tx());
        let object_1 = owned_object_json(0x11);
        let object_2 = owned_object_json(0x22);
        let args: Vec<&str> = output
            .commands
            .iter()
            .map(|command| command.call_args_json.as_str())
            .collect();
        let package_digest = |command: usize| {
            call_args(&output, command)["package_digest"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(
            args,
            [
                format!(r#"{{"arguments":[{object_1},{{"pure":"0xe803000000000000"}}]}}"#),
                format!(
                    r#"{{"objects":[{{"nested_result":[0,1]}},{object_1}],"recipient":"{}"}}"#,
                    hex_id(0xab)
                ),
                r#"{"amounts":["1000",{"result":0}],"coin":{"gas_coin":true}}"#.to_string(),
                format!(r#"{{"destination":{object_1},"sources":[{object_2}]}}"#),
                format!(
                    r#"{{"dependencies":["{}"],"package_digest":"{}"}}"#,
                    hex_id(0x02),
                    package_digest(4)
                ),
                r#"{"elements":[{"pure":"0xe803000000000000"}]}"#.to_string(),
                format!(r#"{{"elements":[{object_1}]}}"#),
                format!(
                    r#"{{"dependencies":["{}"],"package":"{}","package_digest":"{}","ticket":{{"result":0}}}}"#,
                    hex_id(0x01),
                    hex_id(0xca),
                    package_digest(7)
                ),
            ]
        );
    }

    #[test]
    fn package_digest_covers_module_bytes() {
        let digest_of = |modules: Vec<Vec<u8>>| {
            let tx = ptb_with(
                vec![],
                vec![Command::Publish(Publish {
                    modules,
                    dependencies: vec![id(0x02)],
                })],
            );
            call_args(&output_for(&tx), 0)["package_digest"].clone()
        };
        let digest = digest_of(vec![vec![0xde, 0xad]]);
        assert_eq!(digest.as_str().unwrap().len(), 66);
        assert_eq!(digest, digest_of(vec![vec![0xde, 0xad]]));
        assert_ne!(digest, digest_of(vec![vec![0xbe, 0xef]]));
    }

    #[test]
    fn object_kinds_are_encoded() {
        let tx = ptb_with(
            vec![
                shared(0x33, 9, Mutability::Mutable),
                shared(0x33, 9, Mutability::Immutable),
                Input::Receiving(ObjectReference::new(id(0x33), 4, Digest::new([0x44; 32]))),
            ],
            vec![move_call(
                id(0xca),
                vec![Argument::Input(0), Argument::Input(1), Argument::Input(2)],
            )],
        );
        let shared_json = |mutability: &str| {
            json!({ "object": {
                "id": hex_id(0x33),
                "initial_shared_version": "9",
                "kind": "shared",
                "mutability": mutability,
            }})
        };
        assert_eq!(
            call_args(&output_for(&tx), 0)["arguments"],
            json!([
                shared_json("mutable"),
                shared_json("immutable"),
                { "object": {
                    "digest": hex_id(0x44),
                    "id": hex_id(0x33),
                    "kind": "receiving",
                    "version": "4",
                }},
            ])
        );
    }

    #[test]
    fn encoding_does_not_depend_on_command_order() {
        // An input used as a SplitCoins amount and as a MakeMoveVec element
        // encodes the same whether or not a MoveCall comes first.
        let make_move_vec = make_move_vec(None, vec![Argument::Input(0)]);
        let split = split(Argument::Gas, vec![Argument::Input(0)]);
        let first = ptb_with(
            vec![pure(&1_000u64)],
            vec![split.clone(), make_move_vec.clone()],
        );
        let second = ptb_with(
            vec![pure(&1_000u64)],
            vec![
                move_call(id(0xca), vec![Argument::Input(0)]),
                split,
                make_move_vec,
            ],
        );
        let first = output_for(&first);
        let second = output_for(&second);
        assert_eq!(first.commands[1], second.commands[2]);
        assert_eq!(
            first.commands[1].call_args_json,
            r#"{"elements":[{"pure":"0xe803000000000000"}]}"#
        );
        assert_eq!(first.commands[0], second.commands[1]);
    }

    #[test]
    fn any_unreadable_argument_drops_the_whole_output() {
        let unreadable = [
            // Recipient is an 8-byte pure value, not an address.
            transfer(vec![Argument::Input(1)], Argument::Input(0)),
            // Amount is a single byte, not a u64.
            split(Argument::Gas, vec![Argument::Input(2)]),
            // Input index out of range, here in a MoveCall.
            move_call(id(0xca), vec![Argument::Input(9)]),
        ];
        for command in unreadable {
            // A readable command first: no partial output survives.
            let tx = ptb_with(
                vec![pure(&1_000u64), owned_object(0x11), pure(&7u8)],
                vec![
                    split(Argument::Gas, vec![Argument::Input(0)]),
                    command.clone(),
                ],
            );
            assert!(
                build_intermediate_output(&tx).is_err(),
                "{command:?} should drop the output"
            );
        }
    }

    #[test]
    fn expiration_is_canonical_json() {
        assert_eq!(expiration_string(&TransactionExpiration::None).unwrap(), "");
        assert_eq!(
            expiration_string(&TransactionExpiration::Epoch(5)).unwrap(),
            r#"{"epoch":"5"}"#
        );
        let valid_during = TransactionExpiration::ValidDuring {
            min_epoch: Some(7),
            max_epoch: None,
            min_timestamp: None,
            max_timestamp: Some(9),
            chain: Digest::new([0x0f; 32]),
            nonce: 3,
        };
        assert_eq!(
            expiration_string(&valid_during).unwrap(),
            r#"{"chain":"0x0f0f0f0f","max_epoch":null,"max_timestamp":"9","min_epoch":"7","min_timestamp":null,"nonce":"3"}"#
        );
    }

    #[test]
    fn no_intermediate_output_unless_opted_in() {
        let result = convert(tx_from_b64(PLAIN_PTB), false);
        assert!(result.intermediate_output.is_none());
    }

    /// A large pure input referenced many times: `count` `MakeMoveVec`
    /// commands, each with `per_command` references to input 0.
    fn amplified_tx(input: Input, count: usize, per_command: usize) -> Transaction {
        let command = make_move_vec(None, vec![Argument::Input(0); per_command]);
        ptb_with(vec![input], vec![command; count])
    }

    #[test]
    fn amplified_references_drop_the_output() {
        // A 16 KiB pure input referenced 38,400 times across 75 commands
        // (Sui's per-command argument limit is 512). The transaction is about
        // 131 KB, but its unbounded argument JSON would exceed 1 GB.
        let tx = amplified_tx(Input::Pure(vec![0xab; 16 * 1024]), 75, 512);
        assert!(bcs::to_bytes(&tx).unwrap().len() < 132 * 1024);
        assert!(build_intermediate_output(&tx).is_err());
        assert!(convert(tx, true).intermediate_output.is_none());
    }

    #[test]
    fn amplified_object_references_drop_the_output() {
        let shared = shared(0x33, 9, Mutability::Mutable);
        assert!(build_intermediate_output(&amplified_tx(shared, 75, 512)).is_err());
    }

    #[test]
    fn argument_budget_is_cumulative() {
        // Each reference to a 16 KiB input emits 13 + 32,768 bytes, so 31 fit
        // under the 1 MiB cap and 32 do not, whether in one command or split
        // across several.
        let input = || Input::Pure(vec![0xab; 16 * 1024]);
        assert!(build_intermediate_output(&amplified_tx(input(), 1, 31)).is_ok());
        assert!(build_intermediate_output(&amplified_tx(input(), 1, 32)).is_err());
        assert!(build_intermediate_output(&amplified_tx(input(), 4, 8)).is_err());
    }

    #[test]
    fn typed_slots_accept_only_literals_and_results() {
        let inputs = || vec![pure(&1_000u64), owned_object(0x11), pure(&[0xab_u8; 32])];
        let rejected = [
            transfer(vec![Argument::Input(1)], Argument::Input(1)),
            transfer(vec![Argument::Input(1)], Argument::Gas),
            split(Argument::Gas, vec![Argument::Input(1)]),
            split(Argument::Gas, vec![Argument::Gas]),
        ];
        for command in rejected {
            let tx = ptb_with(inputs(), vec![command.clone()]);
            assert!(
                build_intermediate_output(&tx).is_err(),
                "{command:?} should drop the output"
            );
        }

        let tx = ptb_with(
            inputs(),
            vec![
                split(Argument::Gas, vec![Argument::NestedResult(0, 0)]),
                transfer(vec![Argument::Input(1)], Argument::Result(0)),
            ],
        );
        let output = output_for(&tx);
        assert_eq!(
            call_args(&output, 0)["amounts"],
            json!([{ "nested_result": [0, 0] }])
        );
        assert_eq!(call_args(&output, 1)["recipient"], json!({ "result": 0 }));
    }

    #[test]
    fn non_programmable_transaction_falls_back_without_intermediate_output() {
        let mut tx = tx_from_b64(PLAIN_PTB);
        tx.kind = TransactionKind::EndOfEpoch(vec![]);
        let result = convert(tx, true);
        assert!(result.intermediate_output.is_none());
        assert_eq!(result.payload.payload_type, "Sui");
    }
}
