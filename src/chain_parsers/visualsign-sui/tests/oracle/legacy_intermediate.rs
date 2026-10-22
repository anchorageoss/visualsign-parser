// Frozen pre-migration intermediate builder from 91eef67d9f6f6e6f22273880aa32ad9738a9f694.
// Keep this independent of the SDK implementation; schema changes require review.
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
use sui_types::base_types::{ObjectID, ObjectRef, SuiAddress};
use sui_types::move_package::MovePackage;
use sui_types::transaction::{
    Argument, CallArg, Command, FundsWithdrawalArg, ObjectArg, Reservation, SharedObjectMutability,
    TransactionData, TransactionDataAPI, TransactionExpiration, TransactionKind, WithdrawFrom,
};
use sui_types::type_input::TypeInput;
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
    transaction: &TransactionData,
) -> Result<SuiIntermediateOutput, VisualSignError> {
    let TransactionKind::ProgrammableTransaction(ptb) = transaction.kind() else {
        return Err(decode_error(
            "Sui intermediate output only covers programmable transactions",
        ));
    };

    let mut budget = Budget::new(MAX_CALL_ARGS_JSON_BYTES);
    let gas_data = transaction.gas_data();
    Ok(SuiIntermediateOutput {
        schema_version: SUI_INTERMEDIATE_SCHEMA_VERSION,
        sender: transaction.sender().to_string(),
        commands: ptb
            .commands
            .iter()
            .map(|command| build_command(command, &ptb.inputs, &mut budget))
            .collect::<Result<_, _>>()?,
        gas: SuiGasSummary {
            budget: gas_data.budget.to_string(),
            price: gas_data.price.to_string(),
            owner: gas_data.owner.to_string(),
            payment_objects: gas_data
                .payment
                .iter()
                .map(|object_ref| to_canonical_string(&object_ref_json(object_ref)))
                .collect::<Result<_, _>>()?,
        },
        expiration: expiration_string(transaction.expiration())?,
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
    inputs: &[CallArg],
    budget: &mut Budget,
) -> Result<SuiIntermediateCommand, VisualSignError> {
    let (command_kind, type_arguments) = match command {
        Command::MoveCall(call) => {
            return Ok(SuiIntermediateCommand {
                command_kind: "MoveCall".to_string(),
                package: call.package.to_string(),
                module_name: call.module.clone(),
                function: call.function.clone(),
                type_arguments: call.type_arguments.iter().map(canonical_type).collect(),
                call_args_json: call_args_json(command, inputs, budget)?,
            });
        }
        Command::TransferObjects(..) => ("TransferObjects", vec![]),
        Command::SplitCoins(..) => ("SplitCoins", vec![]),
        Command::MergeCoins(..) => ("MergeCoins", vec![]),
        Command::Publish(..) => ("Publish", vec![]),
        Command::Upgrade(..) => ("Upgrade", vec![]),
        Command::MakeMoveVec(element_type, _) => (
            "MakeMoveVec",
            element_type.iter().map(canonical_type).collect(),
        ),
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

fn canonical_type(type_input: &TypeInput) -> String {
    type_input.to_canonical_string(true)
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
    inputs: &[CallArg],
    budget: &mut Budget,
) -> Result<String, VisualSignError> {
    let ids = |ids: &[ObjectID]| Value::Array(ids.iter().map(|id| id.to_string().into()).collect());
    let package_digest = |modules: &[Vec<u8>], dependencies: &[ObjectID]| -> Value {
        // Sui's own package digest (module hashes and dependency IDs, sorted),
        // so two publishes of different bytecode never encode the same.
        let digest = MovePackage::compute_digest_for_modules_and_deps(modules, dependencies, true);
        format!("0x{}", hex::encode(digest)).into()
    };

    let entries: Vec<(&str, Value)> = match command {
        Command::MoveCall(call) => vec![(
            "arguments",
            arguments_json(&call.arguments, inputs, Slot::Untyped, budget)?,
        )],
        Command::TransferObjects(objects, recipient) => vec![
            (
                "objects",
                arguments_json(objects, inputs, Slot::Untyped, budget)?,
            ),
            (
                "recipient",
                argument_json(*recipient, inputs, Slot::Address, budget)?,
            ),
        ],
        Command::SplitCoins(coin, amounts) => vec![
            (
                "amounts",
                arguments_json(amounts, inputs, Slot::U64, budget)?,
            ),
            ("coin", argument_json(*coin, inputs, Slot::Untyped, budget)?),
        ],
        Command::MergeCoins(destination, sources) => vec![
            (
                "destination",
                argument_json(*destination, inputs, Slot::Untyped, budget)?,
            ),
            (
                "sources",
                arguments_json(sources, inputs, Slot::Untyped, budget)?,
            ),
        ],
        Command::Publish(modules, dependencies) => vec![
            ("dependencies", ids(dependencies)),
            ("package_digest", package_digest(modules, dependencies)),
        ],
        Command::Upgrade(modules, dependencies, package, ticket) => vec![
            ("dependencies", ids(dependencies)),
            ("package", package.to_string().into()),
            ("package_digest", package_digest(modules, dependencies)),
            (
                "ticket",
                argument_json(*ticket, inputs, Slot::Untyped, budget)?,
            ),
        ],
        Command::MakeMoveVec(_, elements) => vec![(
            "elements",
            arguments_json(elements, inputs, Slot::Untyped, budget)?,
        )],
    };

    let map: Map<String, Value> = entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    to_canonical_string(&Value::Object(map))
}

fn arguments_json(
    arguments: &[Argument],
    inputs: &[CallArg],
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
    inputs: &[CallArg],
    slot: Slot,
    budget: &mut Budget,
) -> Result<Value, VisualSignError> {
    let value = match argument {
        Argument::Result(command) => json!({ "result": command }),
        Argument::NestedResult(command, index) => json!({ "nested_result": [command, index] }),
        Argument::GasCoin => match slot {
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

fn input_json(input: &CallArg, slot: Slot, budget: &mut Budget) -> Result<Value, VisualSignError> {
    let value = match (input, slot) {
        (CallArg::Pure(bytes), Slot::Untyped) => {
            // Charged before the hex is allocated: `{"pure":"0x` + hex + `"}`.
            budget.charge(13 + 2 * bytes.len())?;
            return Ok(json!({ "pure": format!("0x{}", hex::encode(bytes)) }));
        }
        (CallArg::Pure(bytes), Slot::Address) => SuiAddress::from_bytes(bytes)
            .map(|address| Value::from(address.to_string()))
            .map_err(|_| decode_error("Address argument is not 32 bytes"))?,
        (CallArg::Pure(bytes), Slot::U64) => <[u8; 8]>::try_from(bytes.as_slice())
            .map(|le| Value::from(u64::from_le_bytes(le).to_string()))
            .map_err(|_| decode_error("u64 argument is not 8 bytes"))?,
        (CallArg::Object(object), Slot::Untyped) => json!({ "object": object_json(object) }),
        (CallArg::FundsWithdrawal(withdrawal), Slot::Untyped) => {
            json!({ "funds_withdrawal": funds_withdrawal_json(withdrawal) })
        }
        (CallArg::Object(_) | CallArg::FundsWithdrawal(_), Slot::Address | Slot::U64) => {
            return Err(decode_error(
                "Object or funds withdrawal in an address or u64 slot",
            ));
        }
    };
    budget.charge_value(&value)?;
    Ok(value)
}

fn object_ref_json((id, version, digest): &ObjectRef) -> Value {
    json!({
        "digest": format!("0x{}", hex::encode(digest.inner())),
        "id": id.to_string(),
        "version": version.value().to_string(),
    })
}

fn object_json(object: &ObjectArg) -> Value {
    let by_ref = |kind: &str, object_ref: &ObjectRef| {
        let mut value = object_ref_json(object_ref);
        if let Value::Object(map) = &mut value {
            map.insert("kind".to_string(), kind.into());
        }
        value
    };
    match object {
        ObjectArg::ImmOrOwnedObject(object_ref) => by_ref("imm_or_owned", object_ref),
        ObjectArg::Receiving(object_ref) => by_ref("receiving", object_ref),
        ObjectArg::SharedObject {
            id,
            initial_shared_version,
            mutability,
        } => json!({
            "id": id.to_string(),
            "initial_shared_version": initial_shared_version.value().to_string(),
            "kind": "shared",
            "mutability": match mutability {
                SharedObjectMutability::Immutable => "immutable",
                SharedObjectMutability::Mutable => "mutable",
                SharedObjectMutability::NonExclusiveWrite => "non_exclusive_write",
            },
        }),
    }
}

fn funds_withdrawal_json(withdrawal: &FundsWithdrawalArg) -> Value {
    let Reservation::MaxAmountU64(max_amount) = withdrawal.reservation;
    json!({
        "max_amount": max_amount.to_string(),
        "type": withdrawal.type_arg.to_type_tag().to_canonical_string(true),
        "withdraw_from": match withdrawal.withdraw_from {
            WithdrawFrom::Sender => "sender",
            WithdrawFrom::Sponsor => "sponsor",
        },
    })
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
            "chain": format!("0x{chain}"),
            "max_epoch": optional(*max_epoch),
            "max_timestamp": optional(*max_timestamp),
            "min_epoch": optional(*min_epoch),
            "min_timestamp": optional(*min_timestamp),
            "nonce": nonce.to_string(),
        }),
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
