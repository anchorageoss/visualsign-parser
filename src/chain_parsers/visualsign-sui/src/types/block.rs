//! The transaction view the presets read, built from `sui_sdk_types::Transaction`.
//!
//! The shapes and names follow `sui_json_rpc_types` (`SuiCallArg`, `SuiCommand`,
//! ...), which the presets were written against, and the conversion reproduces
//! `SuiTransactionBlockData::try_from_with_module_cache` with an empty module
//! cache. In particular, `Pure` inputs are typed only where a native command
//! fixes the type, and only for commands before the first `MoveCall`; see
//! [`resolve_input_types`].

use std::fmt::{self, Display, Formatter};

use sui_sdk_types::{
    Argument, Command, EndOfEpochTransactionKind, Input, Mutability, ProgrammableTransaction,
    Transaction, TransactionExpiration, TransactionKind, TypeTag, WithdrawFrom,
};

use super::base::{ObjectDigest, ObjectID, SequenceNumber, SuiAddress};
use super::display::type_tag_to_string;
use super::json::SuiJsonValue;

/// Error building the view from a decoded transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionError(String);

impl Display for ConversionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConversionError {}

/// A decoded transaction: sender, gas, and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiTransactionBlockData {
    transaction: SuiTransactionBlockKind,
    sender: SuiAddress,
    gas_data: SuiGasData,
}

impl SuiTransactionBlockData {
    #[must_use]
    pub fn transaction(&self) -> &SuiTransactionBlockKind {
        &self.transaction
    }

    #[must_use]
    pub fn sender(&self) -> &SuiAddress {
        &self.sender
    }

    #[must_use]
    pub fn gas_data(&self) -> &SuiGasData {
        &self.gas_data
    }
}

impl TryFrom<&Transaction> for SuiTransactionBlockData {
    type Error = ConversionError;

    fn try_from(transaction: &Transaction) -> Result<Self, Self::Error> {
        // The preview cannot describe newer expiry constraints. Keep the
        // pre-migration acceptance boundary instead of silently omitting them.
        match &transaction.expiration {
            TransactionExpiration::None
            | TransactionExpiration::Epoch(_)
            | TransactionExpiration::ValidDuring { .. } => {}
            _ => {
                return Err(ConversionError(
                    "Unsupported transaction expiration".to_string(),
                ));
            }
        }
        Ok(Self {
            transaction: SuiTransactionBlockKind::try_from(&transaction.kind)?,
            sender: transaction.sender.into(),
            gas_data: SuiGasData {
                owner: transaction.gas_payment.owner.into(),
                price: transaction.gas_payment.price,
                budget: transaction.gas_payment.budget,
            },
        })
    }
}

/// Gas owner, price and budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiGasData {
    pub owner: SuiAddress,
    pub price: u64,
    pub budget: u64,
}

/// The transaction kind. Only programmable transactions carry their contents;
/// the presets read nothing from the system kinds beyond which one it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiTransactionBlockKind {
    ProgrammableTransaction(SuiProgrammableTransactionBlock),
    ChangeEpoch,
    Genesis,
    ConsensusCommitPrologue,
    AuthenticatorStateUpdate,
    RandomnessStateUpdate,
    EndOfEpochTransaction,
    ConsensusCommitPrologueV2,
    ConsensusCommitPrologueV3,
    ConsensusCommitPrologueV4,
}

impl TryFrom<&TransactionKind> for SuiTransactionBlockKind {
    type Error = ConversionError;

    fn try_from(kind: &TransactionKind) -> Result<Self, Self::Error> {
        Ok(match kind {
            TransactionKind::ProgrammableTransaction(ptb)
            | TransactionKind::ProgrammableSystemTransaction(ptb) => {
                Self::ProgrammableTransaction(SuiProgrammableTransactionBlock::try_from(ptb)?)
            }
            TransactionKind::ChangeEpoch(_) => Self::ChangeEpoch,
            TransactionKind::Genesis(_) => Self::Genesis,
            TransactionKind::ConsensusCommitPrologue(_) => Self::ConsensusCommitPrologue,
            TransactionKind::AuthenticatorStateUpdate(_) => Self::AuthenticatorStateUpdate,
            TransactionKind::RandomnessStateUpdate(_) => Self::RandomnessStateUpdate,
            TransactionKind::EndOfEpoch(kinds) => {
                for kind in kinds {
                    match kind {
                        EndOfEpochTransactionKind::ChangeEpoch(_)
                        | EndOfEpochTransactionKind::AuthenticatorStateCreate
                        | EndOfEpochTransactionKind::AuthenticatorStateExpire(_)
                        | EndOfEpochTransactionKind::RandomnessStateCreate
                        | EndOfEpochTransactionKind::DenyListStateCreate
                        | EndOfEpochTransactionKind::BridgeStateCreate { .. }
                        | EndOfEpochTransactionKind::BridgeCommitteeInit { .. }
                        | EndOfEpochTransactionKind::StoreExecutionTimeObservations(_)
                        | EndOfEpochTransactionKind::AccumulatorRootCreate
                        | EndOfEpochTransactionKind::CoinRegistryCreate
                        | EndOfEpochTransactionKind::DisplayRegistryCreate
                        | EndOfEpochTransactionKind::AddressAliasStateCreate
                        | EndOfEpochTransactionKind::WriteAccumulatorStorageCost { .. } => {}
                        _ => {
                            return Err(ConversionError(
                                "Unsupported end-of-epoch kind".to_string(),
                            ));
                        }
                    }
                }
                Self::EndOfEpochTransaction
            }
            TransactionKind::ConsensusCommitPrologueV2(_) => Self::ConsensusCommitPrologueV2,
            TransactionKind::ConsensusCommitPrologueV3(_) => Self::ConsensusCommitPrologueV3,
            TransactionKind::ConsensusCommitPrologueV4(_) => Self::ConsensusCommitPrologueV4,
            _ => return Err(ConversionError("Unsupported transaction kind".to_string())),
        })
    }
}

/// Inputs and commands of a programmable transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiProgrammableTransactionBlock {
    pub inputs: Vec<SuiCallArg>,
    pub commands: Vec<SuiCommand>,
}

impl TryFrom<&ProgrammableTransaction> for SuiProgrammableTransactionBlock {
    type Error = ConversionError;

    fn try_from(ptb: &ProgrammableTransaction) -> Result<Self, Self::Error> {
        let input_types = resolve_input_types(ptb);
        Ok(Self {
            inputs: ptb
                .inputs
                .iter()
                .zip(input_types)
                .map(|(input, layout)| SuiCallArg::try_from_input(input, layout))
                .collect::<Result<_, _>>()?,
            commands: ptb
                .commands
                .iter()
                .map(SuiCommand::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

/// The type a native command fixes for a `Pure` input it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PureLayout {
    U64,
    Address,
}

/// Types `Pure` inputs the way the JSON-RPC conversion does without package
/// data: `SplitCoins` amounts are `u64` and a `TransferObjects` recipient is an
/// `address`, a later use overriding an earlier one, and the walk stops at the
/// first `MoveCall` (whose signature is unknown). Everything else stays untyped.
pub(crate) fn resolve_input_types(ptb: &ProgrammableTransaction) -> Vec<Option<PureLayout>> {
    let mut types = vec![None; ptb.inputs.len()];
    let mut set = |argument: &Argument, layout: PureLayout| {
        if let Argument::Input(i) = argument
            && let Some(slot) = types.get_mut(usize::from(*i))
        {
            *slot = Some(layout);
        }
    };
    for command in &ptb.commands {
        match command {
            Command::MoveCall(_) => break,
            Command::SplitCoins(split) => {
                for amount in &split.amounts {
                    set(amount, PureLayout::U64);
                }
            }
            Command::TransferObjects(transfer) => set(&transfer.address, PureLayout::Address),
            _ => {}
        }
    }
    types
}

/// A transaction input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiCallArg {
    Object(SuiObjectArg),
    Pure(SuiPureValue),
    FundsWithdrawal(SuiFundsWithdrawalArg),
}

impl SuiCallArg {
    fn try_from_input(input: &Input, layout: Option<PureLayout>) -> Result<Self, ConversionError> {
        Ok(match input {
            Input::Pure(bytes) => Self::Pure(SuiPureValue::from_bcs_bytes(layout, bytes)),
            Input::ImmutableOrOwned(object) => Self::Object(SuiObjectArg::ImmOrOwnedObject {
                object_id: (*object.object_id()).into(),
                version: SequenceNumber::from_u64(object.version()),
                digest: (*object.digest()).into(),
            }),
            Input::Shared(shared) => Self::Object(SuiObjectArg::SharedObject {
                object_id: shared.object_id().into(),
                initial_shared_version: SequenceNumber::from_u64(shared.version()),
                mutable: shared.mutability() == Mutability::Mutable,
            }),
            Input::Receiving(object) => Self::Object(SuiObjectArg::Receiving {
                object_id: (*object.object_id()).into(),
                version: SequenceNumber::from_u64(object.version()),
                digest: (*object.digest()).into(),
            }),
            Input::FundsWithdrawal(withdrawal) => Self::FundsWithdrawal(SuiFundsWithdrawalArg {
                reservation: SuiReservation::MaxAmountU64(withdrawal.amount().ok_or_else(
                    || ConversionError("Unsupported funds withdrawal reservation".to_string()),
                )?),
                type_arg: SuiWithdrawalTypeArg::Balance(withdrawal.coin_type().clone()),
                withdraw_from: match withdrawal.source() {
                    WithdrawFrom::Sender => SuiWithdrawFrom::Sender,
                    WithdrawFrom::Sponsor => SuiWithdrawFrom::Sponsor,
                    _ => {
                        return Err(ConversionError(
                            "Unsupported funds withdrawal source".to_string(),
                        ));
                    }
                },
            }),
            _ => return Err(ConversionError("Unsupported input kind".to_string())),
        })
    }

    #[must_use]
    pub fn pure(&self) -> Option<&SuiJsonValue> {
        match self {
            Self::Pure(value) => Some(&value.value),
            _ => None,
        }
    }

    #[must_use]
    pub fn object(&self) -> Option<&ObjectID> {
        match self {
            Self::Object(
                SuiObjectArg::SharedObject { object_id, .. }
                | SuiObjectArg::ImmOrOwnedObject { object_id, .. }
                | SuiObjectArg::Receiving { object_id, .. },
            ) => Some(object_id),
            _ => None,
        }
    }
}

/// A `Pure` input, rendered as JSON and typed where a native command fixes its
/// type (see [`resolve_input_types`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiPureValue {
    value_type: Option<TypeTag>,
    value: SuiJsonValue,
}

impl SuiPureValue {
    #[cfg(test)]
    #[must_use]
    pub fn new(value_type: Option<TypeTag>, value: SuiJsonValue) -> Self {
        Self { value_type, value }
    }

    /// Mirrors `SuiJsonValue::from_bcs_bytes`: a `u64` is a decimal string, an
    /// `address` a `0x` hex string, and anything untyped (or bytes of the wrong
    /// length for their type) is a JSON array of the raw bytes. The type is
    /// kept even when the bytes do not fit it.
    fn from_bcs_bytes(layout: Option<PureLayout>, bytes: &[u8]) -> Self {
        let raw = || serde_json::Value::from(bytes.to_vec());
        let (value_type, value) = match layout {
            None => (None, raw()),
            Some(PureLayout::U64) => (
                Some(TypeTag::U64),
                <[u8; 8]>::try_from(bytes)
                    .map_or_else(|_| raw(), |le| u64::from_le_bytes(le).to_string().into()),
            ),
            Some(PureLayout::Address) => (
                Some(TypeTag::Address),
                SuiAddress::from_bytes(bytes)
                    .map_or_else(|_| raw(), |address| address.to_string().into()),
            ),
        };
        Self {
            value_type,
            value: SuiJsonValue::new(value),
        }
    }

    #[must_use]
    pub fn value(&self) -> SuiJsonValue {
        self.value.clone()
    }

    #[must_use]
    pub fn value_type(&self) -> Option<TypeTag> {
        self.value_type.clone()
    }
}

/// An object input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiObjectArg {
    ImmOrOwnedObject {
        object_id: ObjectID,
        version: SequenceNumber,
        digest: ObjectDigest,
    },
    SharedObject {
        object_id: ObjectID,
        initial_shared_version: SequenceNumber,
        mutable: bool,
    },
    Receiving {
        object_id: ObjectID,
        version: SequenceNumber,
        digest: ObjectDigest,
    },
}

/// A reservation to withdraw from an address balance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiFundsWithdrawalArg {
    pub reservation: SuiReservation,
    pub type_arg: SuiWithdrawalTypeArg,
    pub withdraw_from: SuiWithdrawFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiReservation {
    MaxAmountU64(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiWithdrawalTypeArg {
    Balance(TypeTag),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiWithdrawFrom {
    Sender,
    Sponsor,
}

/// A PTB command. Package bytes of `Publish` and `Upgrade` are dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiCommand {
    MoveCall(Box<SuiProgrammableMoveCall>),
    TransferObjects(Vec<SuiArgument>, SuiArgument),
    SplitCoins(SuiArgument, Vec<SuiArgument>),
    MergeCoins(SuiArgument, Vec<SuiArgument>),
    Publish(Vec<ObjectID>),
    Upgrade(Vec<ObjectID>, ObjectID, SuiArgument),
    MakeMoveVec(Option<String>, Vec<SuiArgument>),
}

fn arguments(arguments: &[Argument]) -> Vec<SuiArgument> {
    arguments.iter().map(SuiArgument::from).collect()
}

fn ids(ids: &[sui_sdk_types::Address]) -> Vec<ObjectID> {
    ids.iter().copied().map(ObjectID::from).collect()
}

impl TryFrom<&Command> for SuiCommand {
    type Error = ConversionError;

    fn try_from(command: &Command) -> Result<Self, Self::Error> {
        Ok(match command {
            Command::MoveCall(call) => Self::MoveCall(Box::new(SuiProgrammableMoveCall {
                package: call.package.into(),
                module: call.module.to_string(),
                function: call.function.to_string(),
                type_arguments: call.type_arguments.iter().map(type_tag_to_string).collect(),
                arguments: arguments(&call.arguments),
            })),
            Command::TransferObjects(transfer) => Self::TransferObjects(
                arguments(&transfer.objects),
                SuiArgument::from(&transfer.address),
            ),
            Command::SplitCoins(split) => {
                Self::SplitCoins(SuiArgument::from(&split.coin), arguments(&split.amounts))
            }
            Command::MergeCoins(merge) => Self::MergeCoins(
                SuiArgument::from(&merge.coin),
                arguments(&merge.coins_to_merge),
            ),
            Command::Publish(publish) => Self::Publish(ids(&publish.dependencies)),
            Command::Upgrade(upgrade) => Self::Upgrade(
                ids(&upgrade.dependencies),
                upgrade.package.into(),
                SuiArgument::from(&upgrade.ticket),
            ),
            Command::MakeMoveVector(make) => Self::MakeMoveVec(
                make.type_.as_ref().map(type_tag_to_string),
                arguments(&make.elements),
            ),
            _ => return Err(ConversionError("Unsupported command".to_string())),
        })
    }
}

fn write_sep<T: Display>(
    f: &mut Formatter<'_>,
    items: impl IntoIterator<Item = T>,
    sep: &str,
) -> fmt::Result {
    let mut items = items.into_iter().peekable();
    while let Some(item) = items.next() {
        write!(f, "{item}")?;
        if items.peek().is_some() {
            write!(f, "{sep}")?;
        }
    }
    Ok(())
}

impl Display for SuiCommand {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MoveCall(call) => write!(f, "MoveCall({call})"),
            Self::MakeMoveVec(type_tag, elements) => {
                write!(f, "MakeMoveVec(")?;
                if let Some(type_tag) = type_tag {
                    write!(f, "Some{type_tag}")?;
                } else {
                    write!(f, "None")?;
                }
                write!(f, ",[")?;
                write_sep(f, elements, ",")?;
                write!(f, "])")
            }
            Self::TransferObjects(objects, address) => {
                write!(f, "TransferObjects([")?;
                write_sep(f, objects, ",")?;
                write!(f, "],{address})")
            }
            Self::SplitCoins(coin, amounts) => {
                write!(f, "SplitCoins({coin},")?;
                write_sep(f, amounts, ",")?;
                write!(f, ")")
            }
            Self::MergeCoins(target, coins) => {
                write!(f, "MergeCoins({target},")?;
                write_sep(f, coins, ",")?;
                write!(f, ")")
            }
            Self::Publish(dependencies) => {
                write!(f, "Publish(<modules>,")?;
                write_sep(f, dependencies, ",")?;
                write!(f, ")")
            }
            Self::Upgrade(dependencies, package, ticket) => {
                write!(f, "Upgrade(<modules>, {ticket},")?;
                write_sep(f, dependencies, ",")?;
                write!(f, ", {package})")
            }
        }
    }
}

/// A Move call. Type arguments are in Move's short form (`0x2::sui::SUI`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiProgrammableMoveCall {
    pub package: ObjectID,
    pub module: String,
    pub function: String,
    pub type_arguments: Vec<String>,
    pub arguments: Vec<SuiArgument>,
}

impl Display for SuiProgrammableMoveCall {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}::{}", self.package, self.module, self.function)?;
        if !self.type_arguments.is_empty() {
            write!(f, "<")?;
            write_sep(f, &self.type_arguments, ",")?;
            write!(f, ">")?;
        }
        write!(f, "(")?;
        write_sep(f, &self.arguments, ",")?;
        write!(f, ")")
    }
}

/// A command argument.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SuiArgument {
    GasCoin,
    Input(u16),
    Result(u16),
    NestedResult(u16, u16),
}

impl From<&Argument> for SuiArgument {
    fn from(argument: &Argument) -> Self {
        match *argument {
            Argument::Gas => Self::GasCoin,
            Argument::Input(i) => Self::Input(i),
            Argument::Result(i) => Self::Result(i),
            Argument::NestedResult(i, j) => Self::NestedResult(i, j),
        }
    }
}

impl Display for SuiArgument {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::GasCoin => write!(f, "GasCoin"),
            Self::Input(i) => write!(f, "Input({i})"),
            Self::Result(i) => write!(f, "Result({i})"),
            Self::NestedResult(i, j) => write!(f, "NestedResult({i},{j})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sui_sdk_types::{MoveCall, SplitCoins, TransferObjects};

    fn split(amounts: Vec<Argument>) -> Command {
        Command::SplitCoins(SplitCoins {
            coin: Argument::Gas,
            amounts,
        })
    }

    fn transfer(address: Argument) -> Command {
        Command::TransferObjects(TransferObjects {
            objects: vec![Argument::Result(0)],
            address,
        })
    }

    fn move_call() -> Command {
        Command::MoveCall(MoveCall {
            package: sui_sdk_types::Address::TWO,
            module: "m".parse().unwrap(),
            function: "f".parse().unwrap(),
            type_arguments: vec![],
            arguments: vec![Argument::Input(0)],
        })
    }

    fn ptb(commands: Vec<Command>) -> ProgrammableTransaction {
        ProgrammableTransaction {
            inputs: vec![
                Input::Pure(1000u64.to_le_bytes().to_vec()),
                Input::Pure(vec![0xab; 32]),
            ],
            commands,
        }
    }

    #[test]
    fn programmable_system_transaction_uses_programmable_view() {
        let normal = TransactionKind::ProgrammableTransaction(ptb(vec![
            split(vec![Argument::Input(0)]),
            transfer(Argument::Input(1)),
        ]));
        let system = TransactionKind::ProgrammableSystemTransaction(ptb(vec![
            split(vec![Argument::Input(0)]),
            transfer(Argument::Input(1)),
        ]));
        assert_eq!(
            SuiTransactionBlockKind::try_from(&system).unwrap(),
            SuiTransactionBlockKind::try_from(&normal).unwrap(),
            "system PTBs must keep the same commands, typed inputs and transfer view"
        );
    }

    #[test]
    fn programmable_system_transfer_renders_the_same_preview_fields() {
        let fixtures: Vec<String> =
            serde_json::from_str(include_str!("../../tests/oracle/decode_fixtures.json")).unwrap();
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &fixtures[0])
                .unwrap();
        let transaction: Transaction = bcs::from_bytes(&bytes).unwrap();
        let TransactionKind::ProgrammableTransaction(ptb) = transaction.kind.clone() else {
            panic!("native transfer fixture must be programmable");
        };
        let mut system = transaction.clone();
        system.kind = TransactionKind::ProgrammableSystemTransaction(ptb);
        for decode_transfers in [false, true] {
            let options = visualsign::vsptrait::VisualSignOptions {
                decode_transfers,
                ..Default::default()
            };
            let mut normal_preview =
                crate::transaction_to_visual_sign(transaction.clone(), options.clone()).unwrap();
            let mut system_preview =
                crate::transaction_to_visual_sign(system.clone(), options).unwrap();
            // Transaction Details contains the deliberately different raw kind
            // byte. Every displayed command/transfer field must still match.
            assert_eq!(
                normal_preview.fields.last().unwrap().label(),
                "Transaction Details"
            );
            assert_eq!(
                system_preview.fields.last().unwrap().label(),
                "Transaction Details"
            );
            normal_preview.fields.pop();
            system_preview.fields.pop();
            assert_eq!(
                serde_json::to_value(system_preview).unwrap(),
                serde_json::to_value(normal_preview).unwrap(),
            );
        }
    }

    fn assert_unsupported_fixture(name: &str) {
        let fixtures: std::collections::BTreeMap<String, String> = serde_json::from_str(
            include_str!("../../tests/fixtures/unsupported-sdk-transactions.json"),
        )
        .unwrap();
        let encoded = &fixtures[name];
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).unwrap();
        let transaction: Transaction = bcs::from_bytes(&bytes).unwrap();
        assert!(
            SuiTransactionBlockData::try_from(&transaction).is_err(),
            "{name}"
        );
        for decode_transfers in [false, true] {
            let result = crate::transaction_string_to_visual_sign(
                encoded,
                visualsign::vsptrait::VisualSignOptions {
                    decode_transfers,
                    include_intermediate_output: true,
                    ..Default::default()
                },
            );
            assert!(
                result.is_err(),
                "{name}: no preview or intermediate output is safe yet"
            );
        }
    }

    #[test]
    fn validity_expiration_is_rejected() {
        assert_unsupported_fixture("validity");
    }

    #[test]
    fn sender_allowance_withdrawal_is_rejected() {
        assert_unsupported_fixture("sender_allowance");
    }

    #[test]
    fn forwarding_address_registry_create_is_rejected() {
        assert_unsupported_fixture("forwarding_address_registry_create");
    }

    #[test]
    fn native_commands_type_pure_inputs() {
        let types = resolve_input_types(&ptb(vec![
            split(vec![Argument::Input(0)]),
            transfer(Argument::Input(1)),
        ]));
        assert_eq!(
            types,
            vec![Some(PureLayout::U64), Some(PureLayout::Address)]
        );
    }

    /// Without package data the conversion cannot type a `MoveCall`'s
    /// arguments, and it stops typing at the first one.
    #[test]
    fn typing_stops_at_first_move_call() {
        let types = resolve_input_types(&ptb(vec![
            split(vec![Argument::Input(0)]),
            move_call(),
            transfer(Argument::Input(1)),
        ]));
        assert_eq!(types, vec![Some(PureLayout::U64), None]);

        let types = resolve_input_types(&ptb(vec![move_call(), split(vec![Argument::Input(0)])]));
        assert_eq!(types, vec![None, None]);
    }

    #[test]
    fn later_use_overrides_earlier_type() {
        let types = resolve_input_types(&ptb(vec![
            split(vec![Argument::Input(1)]),
            transfer(Argument::Input(1)),
        ]));
        assert_eq!(types, vec![None, Some(PureLayout::Address)]);
    }

    #[test]
    fn pure_values_render_like_sui_json() {
        let value = SuiPureValue::from_bcs_bytes(Some(PureLayout::U64), &1000u64.to_le_bytes());
        assert_eq!(value.value_type(), Some(TypeTag::U64));
        assert_eq!(value.value().to_json_value(), serde_json::json!("1000"));

        // Bytes that do not fit the type fall back to the raw array but keep the type.
        let value = SuiPureValue::from_bcs_bytes(Some(PureLayout::U64), &[1, 2, 3]);
        assert_eq!(value.value_type(), Some(TypeTag::U64));
        assert_eq!(value.value().to_json_value(), serde_json::json!([1, 2, 3]));

        let value = SuiPureValue::from_bcs_bytes(Some(PureLayout::Address), &[0xab; 32]);
        assert_eq!(
            value.value().to_json_value(),
            serde_json::json!(format!("0x{}", "ab".repeat(32)))
        );

        let value = SuiPureValue::from_bcs_bytes(None, &[7, 8]);
        assert_eq!(value.value_type(), None);
        assert_eq!(value.value().to_json_value(), serde_json::json!([7, 8]));
    }
}
