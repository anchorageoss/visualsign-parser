//! Sui intermediate output: a Borsh-serialized, structured decode of a Sui
//! programmable transaction, attached to the conversion result alongside the
//! human-readable `SignablePayload`.
//!
//! [`SUI_INTERMEDIATE_SCHEMA_VERSION`] is the first field so a shape change is
//! a single, reviewable signal that forces mirrored decoders to update.
//!
//! Encoding rules:
//! - One [`SuiIntermediateCommand`] per PTB command, in transaction order.
//! - Addresses and object IDs are `0x`-prefixed lowercase hex; numeric values
//!   are base-10 integer strings.
//! - `call_args_json` is empty when the arguments are not decoded (every
//!   `MoveCall`, since the parser has no on-chain ABI to type its inputs), and
//!   otherwise a canonical JSON object: keys alphabetized at every nesting
//!   level, independent of `serde_json`'s `preserve_order` build feature.
//! - An argument that is not a literal input is emitted as a tagged
//!   reference (`{"gas_coin":true}`, `{"result":i}`, `{"nested_result":[i,j]}`,
//!   `{"object":"0x.."}`); a pure input whose type the command does not fix is
//!   emitted as its raw BCS bytes (`{"pure":"0x.."}`). If any argument of a
//!   command cannot be read, that command's `call_args_json` is left empty
//!   rather than partially filled.
//! - `expiration` is empty for no expiration, else a canonical JSON object.

use std::collections::BTreeMap;
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use move_core_types::language_storage::TypeTag;
use serde_json::{Map, Value, json};
use sui_json_rpc_types::{
    SuiArgument, SuiCallArg, SuiCommand, SuiObjectArg, SuiPureValue, SuiTransactionBlockData,
    SuiTransactionBlockDataAPI, SuiTransactionBlockKind,
};
use sui_types::base_types::{ObjectID, SuiAddress};
use sui_types::transaction::{TransactionData, TransactionDataAPI, TransactionExpiration};
use visualsign::errors::VisualSignError;

use crate::utils::{decode_number, pure_bcs_bytes};

/// Version of the `SuiIntermediateOutput` Borsh schema. Bump on ANY change to
/// the shape below or to the encoding rules in the module docs.
pub const SUI_INTERMEDIATE_SCHEMA_VERSION: u16 = 1;

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
    pub payment_object_ids: Vec<String>,
}

/// Build the intermediate output for a programmable transaction.
///
/// # Errors
/// Returns `VisualSignError::DecodeError` for transaction kinds other than a
/// programmable transaction, which have no PTB commands to describe, or if the
/// expiration cannot be serialized.
pub(crate) fn build_intermediate_output(
    transaction: &TransactionData,
    block_data: &SuiTransactionBlockData,
) -> Result<SuiIntermediateOutput, VisualSignError> {
    let SuiTransactionBlockKind::ProgrammableTransaction(ptb) = block_data.transaction() else {
        return Err(VisualSignError::DecodeError(
            "Sui intermediate output only covers programmable transactions".to_string(),
        ));
    };

    let gas_data = block_data.gas_data();
    Ok(SuiIntermediateOutput {
        schema_version: SUI_INTERMEDIATE_SCHEMA_VERSION,
        sender: block_data.sender().to_string(),
        commands: ptb
            .commands
            .iter()
            .map(|command| build_command(command, &ptb.inputs))
            .collect(),
        gas: SuiGasSummary {
            budget: gas_data.budget.to_string(),
            price: gas_data.price.to_string(),
            owner: gas_data.owner.to_string(),
            payment_object_ids: gas_data
                .payment
                .iter()
                .map(|object| object.object_id.to_string())
                .collect(),
        },
        expiration: expiration_string(transaction.expiration())?,
    })
}

fn build_command(command: &SuiCommand, inputs: &[SuiCallArg]) -> SuiIntermediateCommand {
    let native = |kind: &str, type_arguments: Vec<String>| SuiIntermediateCommand {
        command_kind: kind.to_string(),
        package: String::new(),
        module_name: String::new(),
        function: String::new(),
        type_arguments,
        call_args_json: native_call_args(command, inputs)
            .and_then(|args| canonical_json(&Value::Object(args)).ok())
            .unwrap_or_default(),
    };

    match command {
        SuiCommand::MoveCall(call) => SuiIntermediateCommand {
            command_kind: "MoveCall".to_string(),
            package: call.package.to_string(),
            module_name: call.module.clone(),
            function: call.function.clone(),
            type_arguments: call.type_arguments.clone(),
            call_args_json: String::new(),
        },
        SuiCommand::TransferObjects(..) => native("TransferObjects", vec![]),
        SuiCommand::SplitCoins(..) => native("SplitCoins", vec![]),
        SuiCommand::MergeCoins(..) => native("MergeCoins", vec![]),
        SuiCommand::Publish(..) => native("Publish", vec![]),
        SuiCommand::Upgrade(..) => native("Upgrade", vec![]),
        SuiCommand::MakeMoveVec(element_type, _) => {
            native("MakeMoveVec", element_type.iter().cloned().collect())
        }
    }
}

/// How a command's signature types an argument slot, which decides how a pure
/// input in that slot is decoded.
#[derive(Clone, Copy)]
enum Slot {
    Address,
    U64,
    Untyped,
}

/// Decoded arguments of a native command, or `None` if any of them cannot be
/// read. `MoveCall` is never decoded here.
fn native_call_args(command: &SuiCommand, inputs: &[SuiCallArg]) -> Option<Map<String, Value>> {
    let args = |slot, arguments: &[SuiArgument]| -> Option<Value> {
        arguments
            .iter()
            .map(|argument| argument_json(*argument, inputs, slot))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array)
    };
    let ids = |ids: &[ObjectID]| Value::Array(ids.iter().map(|id| id.to_string().into()).collect());

    let entries: Vec<(&str, Value)> = match command {
        SuiCommand::MoveCall(_) => return None,
        SuiCommand::TransferObjects(objects, recipient) => vec![
            ("objects", args(Slot::Untyped, objects)?),
            (
                "recipient",
                argument_json(*recipient, inputs, Slot::Address)?,
            ),
        ],
        SuiCommand::SplitCoins(coin, amounts) => vec![
            ("amounts", args(Slot::U64, amounts)?),
            ("coin", argument_json(*coin, inputs, Slot::Untyped)?),
        ],
        SuiCommand::MergeCoins(destination, sources) => vec![
            (
                "destination",
                argument_json(*destination, inputs, Slot::Untyped)?,
            ),
            ("sources", args(Slot::Untyped, sources)?),
        ],
        SuiCommand::Publish(dependencies) => vec![("dependencies", ids(dependencies))],
        SuiCommand::Upgrade(dependencies, package, ticket) => vec![
            ("dependencies", ids(dependencies)),
            ("package", package.to_string().into()),
            ("ticket", argument_json(*ticket, inputs, Slot::Untyped)?),
        ],
        SuiCommand::MakeMoveVec(_, elements) => vec![("elements", args(Slot::Untyped, elements)?)],
    };

    Some(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn argument_json(argument: SuiArgument, inputs: &[SuiCallArg], slot: Slot) -> Option<Value> {
    match argument {
        SuiArgument::GasCoin => Some(json!({ "gas_coin": true })),
        SuiArgument::Result(command) => Some(json!({ "result": command })),
        SuiArgument::NestedResult(command, index) => {
            Some(json!({ "nested_result": [command, index] }))
        }
        SuiArgument::Input(index) => input_json(inputs.get(usize::from(index))?, slot),
    }
}

fn input_json(input: &SuiCallArg, slot: Slot) -> Option<Value> {
    match input {
        SuiCallArg::Object(object) => {
            let id = match object {
                SuiObjectArg::ImmOrOwnedObject { object_id, .. }
                | SuiObjectArg::SharedObject { object_id, .. }
                | SuiObjectArg::Receiving { object_id, .. } => object_id,
            };
            Some(json!({ "object": id.to_string() }))
        }
        SuiCallArg::FundsWithdrawal(_) => None,
        SuiCallArg::Pure(value) => match slot {
            Slot::Address => Some(pure_address(input, value)?.to_string().into()),
            Slot::U64 => Some(decode_number::<u64>(input).ok()?.to_string().into()),
            Slot::Untyped => {
                let bytes = pure_bcs_bytes(input).ok()?;
                Some(json!({ "pure": format!("0x{}", hex::encode(bytes)) }))
            }
        },
    }
}

/// A pure address input, either untyped BCS bytes or typed as `address` (the
/// RPC conversion types a `TransferObjects` recipient this way).
fn pure_address(input: &SuiCallArg, value: &SuiPureValue) -> Option<SuiAddress> {
    match value.value_type() {
        None => SuiAddress::from_bytes(pure_bcs_bytes(input).ok()?).ok(),
        Some(TypeTag::Address) => match value.value().to_json_value() {
            Value::String(address) => SuiAddress::from_str(&address).ok(),
            _ => None,
        },
        Some(_) => None,
    }
}

fn expiration_string(expiration: &TransactionExpiration) -> Result<String, VisualSignError> {
    let optional = |value: Option<u64>| value.map_or(Value::Null, |v| v.to_string().into());
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
            "chain": format!("0x{}", hex::encode(chain.as_bytes())),
            "max_epoch": optional(*max_epoch),
            "max_timestamp": optional(*max_timestamp),
            "min_epoch": optional(*min_epoch),
            "min_timestamp": optional(*min_timestamp),
            "nonce": nonce.to_string(),
        }),
    };
    canonical_json(&value)
}

/// Serialize with every object's keys in sorted order.
///
/// `serde_json` is built with `preserve_order` in this workspace, which makes
/// `serde_json::Map` keep insertion order, so the tree is re-keyed in sorted
/// order at every level before serializing.
fn canonical_json(value: &Value) -> Result<String, VisualSignError> {
    serde_json::to_string(&canonicalize(value))
        .map_err(|e| VisualSignError::DecodeError(format!("Failed to serialize JSON: {e}")))
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonicalize(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::SuiVisualSignConverter;
    use crate::core::{SuiModuleResolver, SuiTransactionWrapper};

    use move_bytecode_utils::module_cache::SyncModuleCache;
    use sui_types::base_types::SequenceNumber;
    use sui_types::digests::{ChainIdentifier, CheckpointDigest, ObjectDigest};
    use sui_types::transaction::{
        Argument, CallArg, Command, ObjectArg, ProgrammableMoveCall, ProgrammableTransaction,
        TransactionKind,
    };
    use sui_types::type_input::TypeInput;
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

    // Golden intermediate output, hex-encoded borsh. Regenerate only on a
    // deliberate schema change, together with a schema version bump.
    const PLAIN_PTB_GOLDEN: &str = "010042000000307864366539326530303265323663336166623230383830303163316235383230623634663262633335316266646135613233343261636266396635633163616332020000000a00000053706c6974436f696e73000000000000000000000000000000002d0000007b22616d6f756e7473223a5b2231303030225d2c22636f696e223a7b226761735f636f696e223a747275657d7d0f0000005472616e736665724f626a65637473000000000000000000000000000000006b0000007b226f626a65637473223a5b7b22726573756c74223a307d5d2c22726563697069656e74223a22307861316533616535353163326162653363366263656132326433663139333536613634373038366436626435366163306430396534656565643036323930623736227d0700000035303030303030040000003130303042000000307864366539326530303265323663336166623230383830303163316235383230623634663262633335316266646135613233343261636266396635633163616332010000004200000030786362343835656638306130626166393037396136323163343263386230636236356365393938366531633831666431366565383830333866303034613561656400000000";
    const DEPOSIT_GOLDEN: &str = "01004200000030783937343639373963313232653264366664616237666130396662373233346635613330343365616233666132666530613539623038653563303935353363633303000000080000004d6f766543616c6c42000000307838663765666437343338393766646534386363333562363230336364373263376164343234386630656230326139616433373865346132643339636332633765040000007574786f070000007574786f5f69640000000000000000080000004d6f766543616c6c42000000307838663765666437343338393766646534386363333562363230336364373263376164343234386630656230326139616433373865346132643339636332633765040000007574786f040000007574786f0000000000000000080000004d6f766543616c6c42000000307838663765666437343338393766646534386363333562363230336364373263376164343234386630656230326139616433373865346132643339636332633765070000006465706f736974070000006465706f73697400000000000000000700000037353339383336040000003130303042000000307839373436393739633132326532643666646162376661303966623732333466356133303433656162336661326665306135396230386535633039353533636333010000004200000030786265326237633939656436346563313561383433346130613261623432386334353638616232653663336564656231376233653963653434363435396362343500000000";
    const SPLIT_WITHDRAWAL_GOLDEN: &str = "010042000000307838383233306336336565363337373439393162376430346661346331643664323639663634353163363962313338343833383333373537333530393363353936040000000a0000004d65726765436f696e7300000000000000000000000000000000bb0000007b2264657374696e6174696f6e223a7b226f626a656374223a22307830343463623761303665646234376362616261613631663632356236383230353637396463643466663464336263383937613137356636306561633762653534227d2c22736f7572636573223a5b7b226f626a656374223a22307865343363636264636434653064336334396332393163666538626637393339643037633737303761373731333932393930333964623630643862663635373766227d5d7d0a00000053706c6974436f696e73000000000000000000000000000000006f0000007b22616d6f756e7473223a5b223234313835373633225d2c22636f696e223a7b226f626a656374223a22307830343463623761303665646234376362616261613631663632356236383230353637396463643466663464336263383937613137356636306561633762653534227d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c000000696e746f5f62616c616e6365010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a42544300000000080000004d6f766543616c6c4200000030783866376566643734333839376664653438636333356236323033636437326337616434323438663065623032613961643337386534613264333963633263376508000000776974686472617712000000726571756573745f7769746864726177616c0000000000000000080000003131313539333438040000003130303042000000307838383233306336336565363337373439393162376430346661346331643664323639663634353163363962313338343833383333373537333530393363353936010000004200000030786662393938353238363431623832313439326662653563653433663065626237303434363535366264386139636132633234376438656234613338313139633500000000";
    const REDEEM_WITHDRAWAL_GOLDEN: &str = "01004200000030786630643337343764623635356666636537383361633434383235663332323835646531323933366639333139346339613930343333353531336264336135626402000000080000004d6f766543616c6c420000003078303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030320700000062616c616e63650c00000072656465656d5f66756e6473010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a42544300000000080000004d6f766543616c6c4200000030783866376566643734333839376664653438636333356236323033636437326337616434323438663065623032613961643337386534613264333963633263376508000000776974686472617712000000726571756573745f7769746864726177616c0000000000000000080000003130303537303736040000003130303042000000307866306433373437646236353566666365373833616334343832356633323238356465313239333666393331393463396139303433333535313362643361356264010000004200000030786631633262643733313931376431333565333135353639313333303237656433343466626262383338663437663738643030363235346165396365373539313300000000";
    const TOP_UP_WITHDRAWAL_GOLDEN: &str = "01004200000030783166633338663736626663363733396463396563323738643935623164613466373631623663383535316636396538343739356438333835626236656535633406000000080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c00000072656465656d5f66756e6473010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a425443000000000a0000004d65726765436f696e7300000000000000000000000000000000780000007b2264657374696e6174696f6e223a7b226f626a656374223a22307834306538356664636432663530336534373031613762666437356165353861356163326130313336373066376134336365653737616235323537303363373839227d2c22736f7572636573223a5b7b22726573756c74223a307d5d7d0a00000053706c6974436f696e7300000000000000000000000000000000700000007b22616d6f756e7473223a5b22313433383039373234225d2c22636f696e223a7b226f626a656374223a22307834306538356664636432663530336534373031613762666437356165353861356163326130313336373066376134336365653737616235323537303363373839227d7d080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0c000000696e746f5f62616c616e6365010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a42544300000000080000004d6f766543616c6c4200000030783030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303204000000636f696e0a00000073656e645f66756e6473010000004c0000003078666365613130636164626235353363343837343230313538346162663638373731353932363738393532656664393537623265383263303130633766343336303a3a6274633a3a42544300000000080000004d6f766543616c6c4200000030783866376566643734333839376664653438636333356236323033636437326337616434323438663065623032613961643337386534613264333963633263376508000000776974686472617712000000726571756573745f7769746864726177616c0000000000000000070000003838363431343804000000313030304200000030783166633338663736626663363733396463396563323738643935623164613466373631623663383535316636396538343739356438333835626236656535633400000000b30000007b22636861696e223a22307834633738616461636632613266356164383066323765643764353461613639643361373866316361363766646566396563663537353466356238626237376230222c226d61785f65706f6368223a2231323130222c226d61785f74696d657374616d70223a6e756c6c2c226d696e5f65706f6368223a2231323039222c226d696e5f74696d657374616d70223a6e756c6c2c226e6f6e6365223a2233303539373930323631227d";

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

    /// Built output for a transaction, bypassing the payload visualizers.
    fn output_for(tx: &TransactionData) -> SuiIntermediateOutput {
        let block_data = SuiTransactionBlockData::try_from_with_module_cache(
            tx.clone(),
            &SyncModuleCache::new(SuiModuleResolver),
        )
        .unwrap();
        build_intermediate_output(tx, &block_data).unwrap()
    }

    fn programmable(tx: &mut TransactionData) -> &mut ProgrammableTransaction {
        match tx.kind_mut() {
            TransactionKind::ProgrammableTransaction(pt) => pt,
            _ => panic!("expected a programmable transaction"),
        }
    }

    /// The plain PTB with its inputs and commands replaced.
    fn ptb_with(inputs: Vec<CallArg>, commands: Vec<Command>) -> TransactionData {
        let mut tx = tx_from_b64(PLAIN_PTB);
        let pt = programmable(&mut tx);
        pt.inputs = inputs;
        pt.commands = commands;
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
    fn plain_ptb_round_trips() {
        let expected = output_for(&tx_from_b64(PLAIN_PTB));
        let decoded: SuiIntermediateOutput = borsh::from_slice(&emitted_bytes(PLAIN_PTB)).unwrap();
        assert_eq!(decoded, expected);

        let sender = "0xd6e92e002e26c3afb2088001c1b5820b64f2bc351bfda5a2342acbf9f5c1cac2";
        assert_eq!(decoded.sender, sender);
        let commands: Vec<(&str, &str)> = decoded
            .commands
            .iter()
            .map(|c| (c.command_kind.as_str(), c.call_args_json.as_str()))
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
        assert_eq!(
            decoded.gas,
            SuiGasSummary {
                budget: "5000000".to_string(),
                price: "1000".to_string(),
                owner: sender.to_string(),
                payment_object_ids: vec![
                    "0xcb485ef80a0baf9079a621c42c8b0cb65ce9986e1c81fd16ee88038f004a5aed"
                        .to_string()
                ],
            }
        );
        assert_eq!(decoded.expiration, "");
    }

    #[test]
    fn hashi_deposit_round_trips() {
        let b64 = hashi_b64("deposit", "deposit", DEPOSIT_DIGEST);
        let decoded: SuiIntermediateOutput = borsh::from_slice(&emitted_bytes(&b64)).unwrap();
        assert_eq!(decoded, output_for(&tx_from_b64(&b64)));

        let hashi = "0x8f7efd743897fde48cc35b6203cd72c7ad4248f0eb02a9ad378e4a2d39cc2c7e";
        let calls: Vec<_> = decoded
            .commands
            .iter()
            .map(|c| {
                (
                    c.command_kind.as_str(),
                    c.package.as_str(),
                    c.module_name.as_str(),
                    c.function.as_str(),
                    c.call_args_json.as_str(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                ("MoveCall", hashi, "utxo", "utxo_id", ""),
                ("MoveCall", hashi, "utxo", "utxo", ""),
                ("MoveCall", hashi, "deposit", "deposit", ""),
            ]
        );
        assert_eq!(
            decoded.sender,
            "0x9746979c122e2d6fdab7fa09fb7234f5a3043eab3fa2fe0a59b08e5c09553cc3"
        );
        assert_eq!(decoded.gas.budget, "7539836");
        assert_eq!(decoded.gas.price, "1000");
        assert_eq!(decoded.expiration, "");
    }

    /// One command of every kind, over pure, object, gas-coin and result
    /// arguments.
    fn all_kinds_tx() -> TransactionData {
        let recipient = SuiAddress::from_bytes([0xab; 32]).unwrap();
        let package = ObjectID::new([0xca; 32]);
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
                    module: "pool".parse().unwrap(),
                    function: "swap".parse().unwrap(),
                    type_arguments: vec![TypeInput::from(TypeTag::U64)],
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
            .map(|c| c.command_kind.as_str())
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
        assert_eq!(move_call.type_arguments, ["u64"]);
        assert_eq!(move_call.call_args_json, "");

        for command in &output.commands[1..] {
            assert_eq!(command.package, "", "{command:?}");
            assert_eq!(command.module_name, "", "{command:?}");
            assert_eq!(command.function, "", "{command:?}");
            let args: Value = serde_json::from_str(&command.call_args_json).unwrap();
            assert!(args.is_object(), "{command:?}");
        }
    }

    #[test]
    fn native_call_args_are_canonical_json() {
        let output = output_for(&all_kinds_tx());
        let args: Vec<&str> = output.commands[1..]
            .iter()
            .map(|c| c.call_args_json.as_str())
            .collect();
        let object_1 = hex_id(0x11);
        let object_2 = hex_id(0x22);
        assert_eq!(
            args,
            [
                format!(
                    r#"{{"objects":[{{"nested_result":[0,1]}},{{"object":"{object_1}"}}],"recipient":"{}"}}"#,
                    hex_id(0xab)
                ),
                r#"{"amounts":["1000",{"result":0}],"coin":{"gas_coin":true}}"#.to_string(),
                format!(
                    r#"{{"destination":{{"object":"{object_1}"}},"sources":[{{"object":"{object_2}"}}]}}"#
                ),
                format!(r#"{{"dependencies":["{}"]}}"#, hex_id(0x02)),
                r#"{"elements":[{"pure":"0xe803000000000000"}]}"#.to_string(),
                format!(r#"{{"elements":[{{"object":"{object_1}"}}]}}"#),
                format!(
                    r#"{{"dependencies":["{}"],"package":"{}","ticket":{{"result":0}}}}"#,
                    hex_id(0x01),
                    hex_id(0xca)
                ),
            ]
        );

        assert_eq!(output.commands[5].type_arguments, ["u64"]);
        assert!(output.commands[6].type_arguments.is_empty());
        for command in [1, 2, 3, 4, 7] {
            assert!(output.commands[command].type_arguments.is_empty());
        }
    }

    #[test]
    fn unreadable_native_arguments_leave_call_args_empty() {
        let tx = ptb_with(
            vec![pure(&1_000u64), owned_object(0x11), pure(&7u8)],
            vec![
                // Recipient is an 8-byte pure value, not an address.
                Command::TransferObjects(vec![Argument::Input(1)], Argument::Input(0)),
                // Amount is a single byte, not a u64.
                Command::SplitCoins(Argument::GasCoin, vec![Argument::Input(2)]),
                // Input index out of range.
                Command::MergeCoins(Argument::Input(1), vec![Argument::Input(9)]),
            ],
        );
        let output = output_for(&tx);
        assert_eq!(output.commands.len(), 3);
        for command in &output.commands {
            assert_eq!(command.call_args_json, "", "{command:?}");
        }
    }

    #[test]
    fn funds_withdrawal_in_native_command_leaves_call_args_empty() {
        // Input 3 of this withdrawal is an address-balance funds withdrawal.
        let mut tx = tx_from_b64(&withdrawal_b64(REDEEM_WITHDRAWAL_DIGEST));
        programmable(&mut tx).commands = vec![Command::MergeCoins(
            Argument::GasCoin,
            vec![Argument::Input(3)],
        )];
        let output = output_for(&tx);
        assert_eq!(output.commands[0].command_kind, "MergeCoins");
        assert_eq!(output.commands[0].call_args_json, "");
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
            format!(
                r#"{{"chain":"{}","max_epoch":null,"max_timestamp":"9","min_epoch":"7","min_timestamp":null,"nonce":"3"}}"#,
                hex_id(0x0f)
            )
        );
    }

    #[test]
    fn canonical_json_sorts_nested_keys() {
        let mut inner = Map::new();
        inner.insert("z".to_string(), json!(1));
        inner.insert("a".to_string(), json!([{"y": 2, "b": 3}]));
        let mut outer = Map::new();
        outer.insert("m".to_string(), Value::Object(inner));
        outer.insert("c".to_string(), json!("x"));
        assert_eq!(
            canonical_json(&Value::Object(outer)).unwrap(),
            r#"{"c":"x","m":{"a":[{"b":3,"y":2}],"z":1}}"#
        );
    }

    #[test]
    fn no_intermediate_output_unless_opted_in() {
        let result = convert(tx_from_b64(PLAIN_PTB), false);
        assert!(result.intermediate_output.is_none());
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
