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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::SuiVisualSignConverter;
    use crate::core::SuiTransactionWrapper;

    use move_core_types::language_storage::TypeTag;
    use sui_types::base_types::SequenceNumber;
    use sui_types::digests::{ChainIdentifier, CheckpointDigest, ObjectDigest};
    use sui_types::transaction::{ProgrammableMoveCall, ProgrammableTransaction};
    use visualsign::vsptrait::{
        ConversionResult, Transaction, VisualSignConverter, VisualSignOptions,
    };

    const HASHI_FIXTURE: &str = include_str!("integrations/hashi/aggregated_test_data.json");

    /// Plain PTB with native commands only (`SplitCoins` of the gas coin, then
    /// `TransferObjects` of the split coin).
    const PLAIN_PTB: &str = "AQAAAAAAAgAI6AMAAAAAAAAAIKHjrlUcKr48a86iLT8ZNWpkcIbWvVasDQnk7u0GKQt2AgIAAQEAAAEBAgAAAQEA1ukuAC4mw6+yCIABwbWCC2TyvDUb/aWiNCrL+fXBysIBy0he+AoLr5B5piHELIsMtlzpmG4cgf0W7ogDjwBKWu3zD9AUAAAAACB0zCGEALsfD5u98y58qbKGIiXkCtDxxN2Pu+r/HyOy1tbpLgAuJsOvsgiAAcG1ggtk8rw1G/2lojQqy/n1wcrC6AMAAAAAAABAS0wAAAAAAAABYQBMegviWYFsLskcYMnTIhZRxiZkET3j2RqtgG1g7f1/EuPjfCHfTvgDqVys+AA6jLWojR35eW4HoOh8qURdshkADNDs6YjOg+HDmdMLe0zMuMDJKqzwIYg08CT6mXiLc2Y=";

    const DEPOSIT_DIGEST: &str = "9oBYx5DMJCLBj6LALo3shpwUJSp6dNvZic6c2FpaF7jo";
    /// Withdrawal funded by `SplitCoins` of an hBTC coin into `coin::into_balance`.
    const SPLIT_WITHDRAWAL_DIGEST: &str = "4BESv9z6m2PRHUXg4uniDXQpeMtKsk9vxc7h2jgogVga";
    /// Withdrawal funded by `balance::redeem_funds` of an address-balance withdrawal.
    const REDEEM_WITHDRAWAL_DIGEST: &str = "E7fvzD239zzTLnhTVsuXVKPKKkFUKSRvdR43oJ8Je8cD";
    /// Withdrawal funded by an hBTC coin topped up from the address balance.
    const TOP_UP_WITHDRAWAL_DIGEST: &str = "8cxs733Zm4LYaVnJBMUptjvr3CXxp2ETwBGpfxnR7kcf";

    // Golden intermediate output, hex-encoded borsh. Any change to these bytes
    // is a change to the published output and must be deliberate.
    const PLAIN_PTB_GOLDEN: &str = "010042000000307864366539326530303265323663336166623230383830303163316235383230623634663262633335316266646135613233343261636266396635633163616332020000000a00000053706c6974436f696e73000000000000000000000000000000002d0000007b22616d6f756e7473223a5b2231303030225d2c22636f696e223a7b226761735f636f696e223a747275657d7d0f0000005472616e736665724f626a65637473000000000000000000000000000000006b0000007b226f626a65637473223a5b7b22726573756c74223a307d5d2c22726563697069656e74223a22307861316533616535353163326162653363366263656132326433663139333536613634373038366436626435366163306430396534656565643036323930623736227d070000003530303030303004000000313030304200000030786436653932653030326532366333616662323038383030316331623538323062363466326263333531626664613561323334326163626639663563316361633201000000af0000007b22646967657374223a22307837346363323138343030626231663066396262646633326537636139623238363232323565343061643066316334646438666262656166663166323362326436222c226964223a22307863623438356566383061306261663930373961363231633432633862306362363563653939383665316338316664313665653838303338663030346135616564222c2276657273696f6e223a22333439313739383931227d00000000";
    const DEPOSIT_GOLDEN: &str = "01004200000030783937343639373963313232653264366664616237666130396662373233346635613330343365616233666132666530613539623038653563303935353363633303000000080000004d6f766543616c6c42000000307838663765666437343338393766646534386363333562363230336364373263376164343234386630656230326139616433373865346132643339636332633765040000007574786f070000007574786f5f696400000000730000007b22617267756d656e7473223a5b7b2270757265223a22307834373236666234633162376261663335353431666531396566336561666338396365663930356135333737643264636430343165343831373934333764396539227d2c7b2270757265223a2230783031303030303030227d5d7d080000004d6f766543616c6c42000000307838663765666437343338393766646534386363333562363230336364373263376164343234386630656230326139616433373865346132643339636332633765040000007574786f040000007574786f000000008a0000007b22617267756d656e7473223a5b7b22726573756c74223a307d2c7b2270757265223a22307866623232656230303030303030303030227d2c7b2270757265223a223078303139373436393739633132326532643666646162376661303966623732333466356133303433656162336661326665306135396230386535633039353533636333227d5d7d080000004d6f766543616c6c42000000307838663765666437343338393766646534386363333562363230336364373263376164343234386630656230326139616433373865346132643339636332633765070000006465706f736974070000006465706f736974000000005c0100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307832326330636536366365303964663264633838613331626433323064343137376237363635313862396238383031303336386366626463643732343532386638222c22696e697469616c5f7368617265645f76657273696f6e223a22383035343734323331222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b22726573756c74223a317d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d5d7d070000003735333938333604000000313030304200000030783937343639373963313232653264366664616237666130396662373233346635613330343365616233666132666530613539623038653563303935353363633301000000af0000007b22646967657374223a22307834323864333739623232323535663039623861613630336361313238373461633634323565356663646333396235386361636232343963656365323666333566222c226964223a22307862653262376339396564363465633135613834333461306132616234323863343536386162326536633365646562313762336539636534343634353963623435222c2276657273696f6e223a22393838313738323235227d00000000";
    const SPLIT_WITHDRAWAL_GOLDEN: &str = "010042000000307838383233306336336565363337373439393162376430346661346331643664323639663634353163363962313338343833383333373537333530393363353936040000000a0000004d65726765436f696e7300000000000000000000000000000000bd0100007b2264657374696e6174696f6e223a7b226f626a656374223a7b22646967657374223a22307862376330363139373931306232336235313865366431326261656363643737303936383730303031366236386234323063666265393465323566633064626662222c226964223a22307830343463623761303665646234376362616261613631663632356236383230353637396463643466663464336263383937613137356636306561633762653534222c226b696e64223a22696d6d5f6f725f6f776e6564222c2276657273696f6e223a22393834353835323636227d7d2c22736f7572636573223a5b7b226f626a656374223a7b22646967657374223a22307861333062373130313030303030303030623930343030303061636163616361636163616361636163616361636163616361636163616361636163616361636163222c226964223a22307865343363636264636434653064336334396332393163666538626637393339643037633737303761373731333932393930333964623630643862663635373766222c226b696e64223a22696d6d5f6f725f6f776e6564222c2276657273696f6e223a22393334383838353735227d7d5d7d0a00000053706c6974436f696e7300000000000000000000000000000000f00000007b22616d6f756e7473223a5b223234313835373633225d2c22636f696e223a7b226f626a656374223a7b22646967657374223a22307862376330363139373931306232336235313865366431326261656363643737303936383730303031366236386234323063666265393465323566633064626662222c226964223a22307830343463623761303665646234376362616261613631663632356236383230353637396463643466663464336263383937613137356636306561633762653534222c226b696e64223a22696d6d5f6f725f6f776e6564222c2276657273696f6e223a22393834353835323636227d7d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c000000696e746f5f62616c616e6365010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a425443270000007b22617267756d656e7473223a5b7b226e65737465645f726573756c74223a5b312c305d7d5d7d080000004d6f766543616c6c4200000030783866376566643734333839376664653438636333356236323033636437326337616434323438663065623032613961643337386534613264333963633263376508000000776974686472617712000000726571756573745f7769746864726177616c00000000ac0100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307832326330636536366365303964663264633838613331626433323064343137376237363635313862396238383031303336386366626463643732343532386638222c22696e697469616c5f7368617265645f76657273696f6e223a22383035343734323331222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d2c7b22726573756c74223a327d2c7b2270757265223a223078323061373930643535366238366132613239356539653332623534666562626635633239383365393466666434373632613963303039613531306439663965653035227d5d7d08000000313131353933343804000000313030304200000030783838323330633633656536333737343939316237643034666134633164366432363966363435316336396231333834383338333337353733353039336335393601000000af0000007b22646967657374223a22307831336262323965396335646434343234303066383566663938333234393036666637623734396635323165633035653536366635333032613734643363326635222c226964223a22307866623939383532383634316238323134393266626535636534336630656262373034343635353662643861396361326332343764386562346133383131396335222c2276657273696f6e223a22393936303134343631227d00000000";
    const REDEEM_WITHDRAWAL_GOLDEN: &str = "01004200000030786630643337343764623635356666636537383361633434383235663332323835646531323933366639333139346339613930343333353531336264336135626402000000080000004d6f766543616c6c420000003078303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030320700000062616c616e63650c00000072656465656d5f66756e6473010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a425443030100007b22617267756d656e7473223a5b7b2266756e64735f7769746864726177616c223a7b226d61785f616d6f756e74223a223130303030303030222c2274797065223a223078303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030323a3a62616c616e63653a3a42616c616e63653c3078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a4254433e222c2277697468647261775f66726f6d223a2273656e646572227d7d5d7d080000004d6f766543616c6c4200000030783866376566643734333839376664653438636333356236323033636437326337616434323438663065623032613961643337386534613264333963633263376508000000776974686472617712000000726571756573745f7769746864726177616c000000009f0100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307832326330636536366365303964663264633838613331626433323064343137376237363635313862396238383031303336386366626463643732343532386638222c22696e697469616c5f7368617265645f76657273696f6e223a22383035343734323331222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d2c7b226e65737465645f726573756c74223a5b302c305d7d2c7b2270757265223a223078313462616663656438613438343162653539363565323737393366616138373239303265353735366464227d5d7d08000000313030353730373604000000313030304200000030786630643337343764623635356666636537383361633434383235663332323835646531323933366639333139346339613930343333353531336264336135626401000000af0000007b22646967657374223a22307832383535306636383162363637373737353337343137643934613933356666393163373464333736336264343335336438313331366433386134633562636236222c226964223a22307866316332626437333139313764313335653331353536393133333032376564333434666262623833386634376637386430303632353461653963653735393133222c2276657273696f6e223a22393835333031383132227d00000000";
    const TOP_UP_WITHDRAWAL_GOLDEN: &str = "01004200000030783166633338663736626663363733396463396563323738643935623164613466373631623663383535316636396538343739356438333835626236656535633406000000080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c00000072656465656d5f66756e6473010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a425443040100007b22617267756d656e7473223a5b7b2266756e64735f7769746864726177616c223a7b226d61785f616d6f756e74223a22313338303137353934222c2274797065223a223078303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030323a3a62616c616e63653a3a42616c616e63653c3078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a4254433e222c2277697468647261775f66726f6d223a2273656e646572227d7d5d7d0a0000004d65726765436f696e7300000000000000000000000000000000f90000007b2264657374696e6174696f6e223a7b226f626a656374223a7b22646967657374223a22307861613430386662366565353263663931393737353961653831323936633836363063316161343337333861346337326263666266363566313031663532643161222c226964223a22307834306538356664636432663530336534373031613762666437356165353861356163326130313336373066376134336365653737616235323537303363373839222c226b696e64223a22696d6d5f6f725f6f776e6564222c2276657273696f6e223a22393933353739333235227d7d2c22736f7572636573223a5b7b22726573756c74223a307d5d7d0a00000053706c6974436f696e7300000000000000000000000000000000f10000007b22616d6f756e7473223a5b22313433383039373234225d2c22636f696e223a7b226f626a656374223a7b22646967657374223a22307861613430386662366565353263663931393737353961653831323936633836363063316161343337333861346337326263666266363566313031663532643161222c226964223a22307834306538356664636432663530336534373031613762666437356165353861356163326130313336373066376134336365653737616235323537303363373839222c226b696e64223a22696d6d5f6f725f6f776e6564222c2276657273696f6e223a22393933353739333235227d7d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c000000696e746f5f62616c616e6365010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a425443270000007b22617267756d656e7473223a5b7b226e65737465645f726573756c74223a5b322c305d7d5d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0a00000073656e645f66756e6473010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a4254432e0100007b22617267756d656e7473223a5b7b226f626a656374223a7b22646967657374223a22307861613430386662366565353263663931393737353961653831323936633836363063316161343337333861346337326263666266363566313031663532643161222c226964223a22307834306538356664636432663530336534373031613762666437356165353861356163326130313336373066376134336365653737616235323537303363373839222c226b696e64223a22696d6d5f6f725f6f776e6564222c2276657273696f6e223a22393933353739333235227d7d2c7b2270757265223a22307831666333386637366266633637333964633965633237386439356231646134663736316236633835353166363965383437393564383338356262366565356334227d5d7d080000004d6f766543616c6c4200000030783866376566643734333839376664653438636333356236323033636437326337616434323438663065623032613961643337386534613264333963633263376508000000776974686472617712000000726571756573745f7769746864726177616c00000000b70100007b22617267756d656e7473223a5b7b226f626a656374223a7b226964223a22307832326330636536366365303964663264633838613331626433323064343137376237363635313862396238383031303336386366626463643732343532386638222c22696e697469616c5f7368617265645f76657273696f6e223a22383035343734323331222c226b696e64223a22736861726564222c226d75746162696c697479223a226d757461626c65227d7d2c7b226f626a656374223a7b226964223a22307830303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303036222c22696e697469616c5f7368617265645f76657273696f6e223a2231222c226b696e64223a22736861726564222c226d75746162696c697479223a22696d6d757461626c65227d7d2c7b226e65737465645f726573756c74223a5b332c305d7d2c7b2270757265223a223078323061653137666261653635623163306366643861666535383233636331623433316263376439333139363631623337313665643431333863333331333566333536227d5d7d0700000038383634313438040000003130303042000000307831666333386637366266633637333964633965633237386439356231646134663736316236633835353166363965383437393564383338356262366565356334000000007b0000007b22636861696e223a2230783463373861646163222c226d61785f65706f6368223a2231323130222c226d61785f74696d657374616d70223a6e756c6c2c226d696e5f65706f6368223a2231323039222c226d696e5f74696d657374616d70223a6e756c6c2c226e6f6e6365223a2233303539373930323631227d";

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

    fn tx_from_b64(b64: &str) -> TransactionData {
        SuiTransactionWrapper::from_string(b64)
            .unwrap()
            .inner()
            .clone()
    }

    fn convert(tx: TransactionData, include_intermediate_output: bool) -> ConversionResult {
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

    fn output_for(tx: &TransactionData) -> SuiIntermediateOutput {
        build_intermediate_output(tx).unwrap()
    }

    fn call_args(output: &SuiIntermediateOutput, command: usize) -> Value {
        serde_json::from_str(&output.commands[command].call_args_json).unwrap()
    }

    fn programmable(tx: &mut TransactionData) -> &mut ProgrammableTransaction {
        match tx.kind_mut() {
            TransactionKind::ProgrammableTransaction(programmable_transaction) => {
                programmable_transaction
            }
            _ => panic!("expected a programmable transaction"),
        }
    }

    /// The plain PTB with its inputs and commands replaced.
    fn ptb_with(inputs: Vec<CallArg>, commands: Vec<Command>) -> TransactionData {
        let mut tx = tx_from_b64(PLAIN_PTB);
        let programmable_transaction = programmable(&mut tx);
        programmable_transaction.inputs = inputs;
        programmable_transaction.commands = commands;
        tx
    }

    fn pure<T: serde::Serialize>(value: &T) -> CallArg {
        CallArg::Pure(bcs::to_bytes(value).unwrap())
    }

    fn owned_object(byte: u8) -> CallArg {
        CallArg::Object(ObjectArg::ImmOrOwnedObject((
            ObjectID::new([byte; 32]),
            SequenceNumber::from_u64(1),
            ObjectDigest::new([0; 32]),
        )))
    }

    fn owned_object_json(byte: u8) -> String {
        format!(
            r#"{{"object":{{"digest":"{}","id":"{}","kind":"imm_or_owned","version":"1"}}}}"#,
            hex_id(0),
            hex_id(byte)
        )
    }

    fn move_call(package: ObjectID, arguments: Vec<Argument>) -> Command {
        Command::MoveCall(Box::new(ProgrammableMoveCall {
            package,
            module: "pool".to_string(),
            function: "swap".to_string(),
            type_arguments: vec![],
            arguments,
        }))
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
    fn hashi_split_withdrawal_golden() {
        assert_golden(
            &withdrawal_b64(SPLIT_WITHDRAWAL_DIGEST),
            SPLIT_WITHDRAWAL_GOLDEN,
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
    fn hashi_top_up_withdrawal_golden() {
        assert_golden(
            &withdrawal_b64(TOP_UP_WITHDRAWAL_DIGEST),
            TOP_UP_WITHDRAWAL_GOLDEN,
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
        let (id, version, digest) = tx_from_b64(PLAIN_PTB).gas_data().payment[0];
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
                    version.value()
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

        let hashi = "0x8f7efd743897fde48cc35b6203cd72c7ad4248f0eb02a9ad378e4a2d39cc2c7e";
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
            "0x22c0ce66ce09df2dc88a31bd320d4177b766518b9b88010368cfbdcd724528f8"
        );
        assert_eq!(deposit["arguments"][1], json!({ "result": 1 }));

        assert_eq!(
            output.sender,
            "0x9746979c122e2d6fdab7fa09fb7234f5a3043eab3fa2fe0a59b08e5c09553cc3"
        );
        assert_eq!(output.gas.budget, "7539836");
        assert_eq!(output.gas.price, "1000");
        assert_eq!(output.expiration, "");
    }

    #[test]
    fn redeem_withdrawal_encodes_funds_withdrawal_and_full_type() {
        let output = output_for(&tx_from_b64(&withdrawal_b64(REDEEM_WITHDRAWAL_DIGEST)));
        let btc = "0xfcea10cadbb553c4874201584abf68771592678952efd957b2e82c010c7f4360::btc::BTC";

        let redeem = &output.commands[0];
        assert_eq!(redeem.package, format!("0x{}", "0".repeat(63) + "2"));
        assert_eq!(
            (redeem.module_name.as_str(), redeem.function.as_str()),
            ("balance", "redeem_funds")
        );
        assert_eq!(redeem.type_arguments, [btc]);
        let withdrawal = &call_args(&output, 0)["arguments"][0]["funds_withdrawal"];
        assert_eq!(withdrawal["max_amount"], "10000000");
        assert_eq!(withdrawal["withdraw_from"], "sender");
        assert_eq!(
            withdrawal["type"],
            format!("0x{}::balance::Balance<{btc}>", "0".repeat(63) + "2")
        );
    }

    #[test]
    fn top_up_withdrawal_uses_short_chain_id() {
        let output = output_for(&tx_from_b64(&withdrawal_b64(TOP_UP_WITHDRAWAL_DIGEST)));
        let expiration: Value = serde_json::from_str(&output.expiration).unwrap();
        assert_eq!(expiration["chain"], "0x4c78adac");
        assert_eq!(expiration["min_epoch"], "1209");
        assert_eq!(expiration["max_epoch"], "1210");
    }

    #[test]
    fn split_amount_matches_the_preview() {
        // Drift guard: the intermediate output and the preview are built from
        // different representations of the same transaction.
        let b64 = withdrawal_b64(SPLIT_WITHDRAWAL_DIGEST);
        let result = convert(tx_from_b64(&b64), true);
        let decoded: SuiIntermediateOutput =
            borsh::from_slice(result.intermediate_output.as_ref().unwrap()).unwrap();
        assert_eq!(decoded.commands[1].command_kind, "SplitCoins");
        assert_eq!(
            serde_json::from_str::<Value>(&decoded.commands[1].call_args_json).unwrap()["amounts"],
            json!(["24185763"])
        );
        let preview = serde_json::to_string(&result.payload).unwrap();
        assert!(preview.contains("24185763"), "{preview}");
    }

    /// One command of every kind, over pure, object, gas-coin and result
    /// arguments.
    fn all_kinds_tx() -> TransactionData {
        let recipient = SuiAddress::from_bytes([0xab; 32]).unwrap();
        let package = ObjectID::new([0xca; 32]);
        let sui: TypeTag = "0x2::sui::SUI".parse().unwrap();
        ptb_with(
            vec![
                pure(&1_000u64),
                pure(&recipient),
                owned_object(0x11),
                owned_object(0x22),
            ],
            vec![
                Command::MoveCall(Box::new(ProgrammableMoveCall {
                    package,
                    module: "pool".to_string(),
                    function: "swap".to_string(),
                    type_arguments: vec![TypeInput::from(sui), TypeInput::from(TypeTag::U64)],
                    arguments: vec![Argument::Input(2), Argument::Input(0)],
                })),
                Command::TransferObjects(
                    vec![Argument::NestedResult(0, 1), Argument::Input(2)],
                    Argument::Input(1),
                ),
                Command::SplitCoins(
                    Argument::GasCoin,
                    vec![Argument::Input(0), Argument::Result(0)],
                ),
                Command::MergeCoins(Argument::Input(2), vec![Argument::Input(3)]),
                Command::Publish(vec![vec![0xde, 0xad]], vec![ObjectID::new([0x02; 32])]),
                Command::MakeMoveVec(
                    Some(TypeInput::from(TypeTag::U64)),
                    vec![Argument::Input(0)],
                ),
                Command::MakeMoveVec(None, vec![Argument::Input(2)]),
                Command::Upgrade(
                    vec![vec![0xbe, 0xef]],
                    vec![ObjectID::new([0x01; 32])],
                    package,
                    Argument::Result(0),
                ),
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
                vec![Command::Publish(modules, vec![ObjectID::new([0x02; 32])])],
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
        let id = ObjectID::new([0x33; 32]);
        let shared = |mutability| {
            CallArg::Object(ObjectArg::SharedObject {
                id,
                initial_shared_version: SequenceNumber::from_u64(9),
                mutability,
            })
        };
        let tx = ptb_with(
            vec![
                shared(SharedObjectMutability::Mutable),
                shared(SharedObjectMutability::Immutable),
                CallArg::Object(ObjectArg::Receiving((
                    id,
                    SequenceNumber::from_u64(4),
                    ObjectDigest::new([0x44; 32]),
                ))),
            ],
            vec![move_call(
                ObjectID::new([0xca; 32]),
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
        let make_move_vec = Command::MakeMoveVec(None, vec![Argument::Input(0)]);
        let split = Command::SplitCoins(Argument::GasCoin, vec![Argument::Input(0)]);
        let first = ptb_with(
            vec![pure(&1_000u64)],
            vec![split.clone(), make_move_vec.clone()],
        );
        let second = ptb_with(
            vec![pure(&1_000u64)],
            vec![
                move_call(ObjectID::new([0xca; 32]), vec![Argument::Input(0)]),
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
            Command::TransferObjects(vec![Argument::Input(1)], Argument::Input(0)),
            // Amount is a single byte, not a u64.
            Command::SplitCoins(Argument::GasCoin, vec![Argument::Input(2)]),
            // Input index out of range, here in a MoveCall.
            move_call(ObjectID::new([0xca; 32]), vec![Argument::Input(9)]),
        ];
        for command in unreadable {
            // A readable command first: no partial output survives.
            let tx = ptb_with(
                vec![pure(&1_000u64), owned_object(0x11), pure(&7u8)],
                vec![
                    Command::SplitCoins(Argument::GasCoin, vec![Argument::Input(0)]),
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
            chain: ChainIdentifier::from(CheckpointDigest::new([0x0f; 32])),
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
    fn amplified_tx(input: CallArg, count: usize, per_command: usize) -> TransactionData {
        let command = Command::MakeMoveVec(None, vec![Argument::Input(0); per_command]);
        ptb_with(vec![input], vec![command; count])
    }

    #[test]
    fn amplified_references_drop_the_output() {
        // A 16 KiB pure input referenced 38,400 times across 75 commands
        // (Sui's per-command argument limit is 512). The transaction is about
        // 131 KB, but its unbounded argument JSON would exceed 1 GB.
        let tx = amplified_tx(CallArg::Pure(vec![0xab; 16 * 1024]), 75, 512);
        assert!(bcs::to_bytes(&tx).unwrap().len() < 132 * 1024);
        assert!(build_intermediate_output(&tx).is_err());
        assert!(convert(tx, true).intermediate_output.is_none());
    }

    #[test]
    fn amplified_object_references_drop_the_output() {
        let shared = CallArg::Object(ObjectArg::SharedObject {
            id: ObjectID::new([0x33; 32]),
            initial_shared_version: SequenceNumber::from_u64(9),
            mutability: SharedObjectMutability::Mutable,
        });
        assert!(build_intermediate_output(&amplified_tx(shared, 75, 512)).is_err());
    }

    #[test]
    fn argument_budget_is_cumulative() {
        // Each reference to a 16 KiB input emits 13 + 32,768 bytes, so 31 fit
        // under the 1 MiB cap and 32 do not, whether in one command or split
        // across several.
        let input = || CallArg::Pure(vec![0xab; 16 * 1024]);
        assert!(build_intermediate_output(&amplified_tx(input(), 1, 31)).is_ok());
        assert!(build_intermediate_output(&amplified_tx(input(), 1, 32)).is_err());
        assert!(build_intermediate_output(&amplified_tx(input(), 4, 8)).is_err());
    }

    #[test]
    fn typed_slots_accept_only_literals_and_results() {
        let inputs = || vec![pure(&1_000u64), owned_object(0x11), pure(&[0xab_u8; 32])];
        let rejected = [
            Command::TransferObjects(vec![Argument::Input(1)], Argument::Input(1)),
            Command::TransferObjects(vec![Argument::Input(1)], Argument::GasCoin),
            Command::SplitCoins(Argument::GasCoin, vec![Argument::Input(1)]),
            Command::SplitCoins(Argument::GasCoin, vec![Argument::GasCoin]),
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
                Command::SplitCoins(Argument::GasCoin, vec![Argument::NestedResult(0, 0)]),
                Command::TransferObjects(vec![Argument::Input(1)], Argument::Result(0)),
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
        *tx.kind_mut() = TransactionKind::EndOfEpochTransaction(vec![]);
        let result = convert(tx, true);
        assert!(result.intermediate_output.is_none());
        assert_eq!(result.payload.payload_type, "Sui");
    }
}
