//! Amounts of coins made by `SplitCoins`.
//!
//! A split coin's amount can be read from its `SplitCoins` command, but that
//! is the coin's balance only until another command touches it. A
//! `MergeCoins` into it, a further `SplitCoins` from it, or a Move call taking
//! it (`0x2::coin::join`, or any `&mut Coin` parameter) changes the balance:
//! with a whole coin merged into a 1 MIST split, the split still reads 1 MIST.
//! So the split amount is the amount a command receives only if no earlier
//! command uses the coin; otherwise the amount is unknown. Later uses do not
//! change what the command received (a `&mut` consumer such as Suilend's
//! `repay` may be followed by a transfer of the remainder).

use crate::types::{SuiArgument, SuiCallArg, SuiCommand};
use crate::utils::{decode_number, parse_numeric_argument};
use visualsign::errors::VisualSignError;

/// Every argument a command reads.
pub fn command_arguments(command: &SuiCommand) -> Vec<SuiArgument> {
    match command {
        SuiCommand::MoveCall(call) => call.arguments.clone(),
        SuiCommand::TransferObjects(objects, recipient) => {
            objects.iter().copied().chain([*recipient]).collect()
        }
        SuiCommand::SplitCoins(coin, amounts) | SuiCommand::MergeCoins(coin, amounts) => {
            std::iter::once(*coin)
                .chain(amounts.iter().copied())
                .collect()
        }
        SuiCommand::Upgrade(_, _, ticket) => vec![*ticket],
        SuiCommand::MakeMoveVec(_, elements) => elements.clone(),
        SuiCommand::Publish(_) => Vec::new(),
    }
}

/// `Result(n)` and `NestedResult(n, 0)` name the same single-output value.
pub fn result_slot(argument: SuiArgument) -> Option<(u16, u16)> {
    match argument {
        SuiArgument::Result(command) => Some((command, 0)),
        SuiArgument::NestedResult(command, index) => Some((command, index)),
        _ => None,
    }
}

/// The amount of a coin made by `SplitCoins` when `consumer` receives it.
///
/// `Ok(None)` when the amount is not in the transaction: `coin` is not a split
/// coin (a whole input coin, the gas coin, another command's output, or
/// `Result(n)` of a split into several coins, which names no single coin), or
/// a command before `consumer` uses it, or `consumer` names it twice.
pub fn split_coin_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    coin: SuiArgument,
    consumer: usize,
) -> Result<Option<u64>, VisualSignError> {
    let Some(slot) = result_slot(coin) else {
        return Ok(None);
    };
    let Some(SuiCommand::SplitCoins(_, amounts)) = commands.get(usize::from(slot.0)) else {
        return Ok(None);
    };
    if matches!(coin, SuiArgument::Result(_)) && amounts.len() != 1 {
        return Ok(None);
    }
    let uses = |command: &SuiCommand| {
        command_arguments(command)
            .into_iter()
            .filter(|argument| result_slot(*argument) == Some(slot))
            .count()
    };
    let earlier = commands
        .iter()
        .enumerate()
        .take(consumer)
        .skip(usize::from(slot.0) + 1)
        .any(|(_, command)| uses(command) > 0);
    if earlier || commands.get(consumer).map(uses) != Some(1) {
        return Ok(None);
    }
    let amount = amounts
        .get(usize::from(slot.1))
        .ok_or(VisualSignError::MissingData(
            "Split coin index out of range".into(),
        ))?;
    let amount = inputs
        .get(usize::from(parse_numeric_argument(*amount)?))
        .ok_or(VisualSignError::MissingData(
            "Amount argument not found".into(),
        ))?;
    Ok(Some(decode_number::<u64>(amount)?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::{SuiJsonValue, SuiProgrammableMoveCall, SuiPureValue};

    const GAS: SuiArgument = SuiArgument::GasCoin;
    const RECIPIENT: SuiArgument = SuiArgument::Input(1);
    const OTHER_COIN: SuiArgument = SuiArgument::Input(2);

    fn pure(bytes: &[u8]) -> SuiCallArg {
        SuiCallArg::Pure(SuiPureValue::new(
            None,
            SuiJsonValue::new(serde_json::Value::from(bytes.to_vec())),
        ))
    }

    fn inputs() -> Vec<SuiCallArg> {
        vec![
            pure(&1_000u64.to_le_bytes()),
            pure(&[0x77; 32]),
            pure(&[]), // stands in for an owned coin; never decoded
            pure(&400u64.to_le_bytes()),
        ]
    }

    fn split(coin: SuiArgument, amounts: &[u16]) -> SuiCommand {
        SuiCommand::SplitCoins(
            coin,
            amounts.iter().map(|i| SuiArgument::Input(*i)).collect(),
        )
    }

    fn transfer(objects: &[SuiArgument]) -> SuiCommand {
        SuiCommand::TransferObjects(objects.to_vec(), RECIPIENT)
    }

    fn coin_join(target: SuiArgument, other: SuiArgument) -> SuiCommand {
        SuiCommand::MoveCall(Box::new(SuiProgrammableMoveCall {
            package: crate::types::ObjectID::from_single_byte(2),
            module: "coin".into(),
            function: "join".into(),
            type_arguments: vec!["0x2::sui::SUI".into()],
            arguments: vec![target, other],
        }))
    }

    fn amount(commands: &[SuiCommand], coin: SuiArgument, consumer: usize) -> Option<u64> {
        split_coin_amount(commands, &inputs(), coin, consumer).unwrap()
    }

    fn assert_unknown(commands: &[SuiCommand], coin: SuiArgument, consumer: usize) {
        assert_eq!(amount(commands, coin, consumer), None);
    }

    #[test]
    fn single_use_split_coin_reads_its_amount() {
        let commands = [split(GAS, &[0]), transfer(&[SuiArgument::Result(0)])];
        assert_eq!(amount(&commands, SuiArgument::Result(0), 1), Some(1_000));
    }

    #[test]
    fn nested_result_reads_its_own_amount() {
        let commands = [
            split(GAS, &[0, 3]),
            transfer(&[SuiArgument::NestedResult(0, 1)]),
            transfer(&[SuiArgument::NestedResult(0, 0)]),
        ];
        assert_eq!(
            amount(&commands, SuiArgument::NestedResult(0, 1), 1),
            Some(400)
        );
        assert_eq!(
            amount(&commands, SuiArgument::NestedResult(0, 0), 2),
            Some(1_000)
        );
    }

    /// A `&mut` consumer (Suilend `repay`) followed by a transfer of the
    /// remainder: the consumer received the split amount; the remainder is
    /// unknown.
    #[test]
    fn use_after_consumer_keeps_its_amount() {
        let commands = [
            split(GAS, &[0]),
            coin_join(SuiArgument::Result(0), OTHER_COIN),
            transfer(&[SuiArgument::Result(0)]),
        ];
        assert_eq!(amount(&commands, SuiArgument::Result(0), 1), Some(1_000));
        assert_unknown(&commands, SuiArgument::Result(0), 2);
    }

    #[test]
    fn coin_named_twice_by_consumer_has_no_amount() {
        let commands = [
            split(GAS, &[0]),
            transfer(&[SuiArgument::Result(0), SuiArgument::NestedResult(0, 0)]),
        ];
        assert_unknown(&commands, SuiArgument::Result(0), 1);
    }

    #[test]
    fn coin_not_from_a_split_has_no_amount() {
        let commands = [transfer(&[OTHER_COIN])];
        assert_eq!(amount(&commands, OTHER_COIN, 0), None);
        assert_eq!(amount(&commands, GAS, 0), None);
        let several = [split(GAS, &[0, 3]), transfer(&[SuiArgument::Result(0)])];
        assert_eq!(amount(&several, SuiArgument::Result(0), 1), None);
    }

    #[test]
    fn merge_into_split_coin_has_no_amount() {
        let commands = [
            split(GAS, &[0]),
            SuiCommand::MergeCoins(SuiArgument::Result(0), vec![OTHER_COIN]),
            transfer(&[SuiArgument::Result(0)]),
        ];
        assert_unknown(&commands, SuiArgument::Result(0), 2);
    }

    #[test]
    fn merge_through_nested_result_has_no_amount() {
        let commands = [
            split(GAS, &[0]),
            SuiCommand::MergeCoins(SuiArgument::NestedResult(0, 0), vec![OTHER_COIN]),
            transfer(&[SuiArgument::Result(0)]),
        ];
        assert_unknown(&commands, SuiArgument::Result(0), 2);
    }

    #[test]
    fn join_by_move_call_has_no_amount() {
        let commands = [
            split(GAS, &[0]),
            coin_join(SuiArgument::Result(0), OTHER_COIN),
            transfer(&[SuiArgument::Result(0)]),
        ];
        assert_unknown(&commands, SuiArgument::Result(0), 2);
    }

    #[test]
    fn further_split_from_split_coin_has_no_amount() {
        let commands = [
            split(GAS, &[0]),
            split(SuiArgument::Result(0), &[3]),
            transfer(&[SuiArgument::Result(0), SuiArgument::Result(1)]),
        ];
        assert_unknown(&commands, SuiArgument::Result(0), 2);
    }

    #[test]
    fn merged_away_split_coin_has_no_amount() {
        let commands = [
            split(GAS, &[0]),
            SuiCommand::MergeCoins(GAS, vec![SuiArgument::Result(0)]),
            transfer(&[SuiArgument::Result(0)]),
        ];
        assert_unknown(&commands, SuiArgument::Result(0), 2);
    }
}

/// Through the converter: a coin changed before it moves renders no amount
/// instead of its split amount.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod converter_tests {
    use crate::{SuiTransactionWrapper, SuiVisualSignConverter};
    use sui_sdk_types::{
        Address, Argument, Command, Digest, Identifier, Input, MergeCoins, MoveCall, Mutability,
        ObjectReference, SharedInput, SplitCoins, Transaction, TransactionKind, TransferObjects,
    };
    use visualsign::errors::VisualSignError;
    use visualsign::vsptrait::{Transaction as _, VisualSignConverter, VisualSignOptions};

    /// A mainnet transfer (`coin_transfer` tests), used for its sender and gas.
    const BASE: &str = "AQAAAAAAAwEAVCf5McvToD8qhL+h2/xg7M2l287m7+8IGIpLQ/6cxBYgqxwkAAAAACDT4HJIX5m7UeyrSJQAHz+p5ZniCwngoTE8GX8E6Vu8HgAI5AYAAAAAAAAAIPEcpBpzFvUgalVqqqnn/Y6mrsto2zVvr1FpVbQvZUfiAgIBAAABAQEAAQECAAABAgCCWsIch38qw9SMwyrvCbO4KfA+TwtC/MZ6NYYnVJq0nAFzUrDKacSVDVVrzYCDnNWtV6Of8JseRtaWdzmHWx/eACCrHCQAAAAAIHZmDcOF5ICx52aJBITeT+GXuGbiP1LOdMK9ewrTvoU3glrCHId/KsPUjMMq7wmzuCnwPk8LQvzGejWGJ1SatJz0AQAAAAAAAOi2MgAAAAAAAAFhALPjB1b3CwKNTPZTHUWogbc9Wz5fgXzVTh1I0dhWVAPGoWxP8HzKFAKr7pZSF/eF1ls/V+m8by7W62K4GbDHLAbJHKJuw6P/F6xoTvR/p7PpYvz2kjD0Z+S3PwARYTCtiw==";

    fn convert(inputs: Vec<Input>, commands: Vec<Command>) -> Result<String, VisualSignError> {
        let mut tx: Transaction = SuiTransactionWrapper::from_string(BASE)
            .unwrap()
            .inner()
            .clone();
        let TransactionKind::ProgrammableTransaction(ptb) = &mut tx.kind else {
            panic!("expected a programmable transaction");
        };
        ptb.inputs = inputs;
        ptb.commands = commands;
        let options = VisualSignOptions {
            decode_transfers: true,
            ..VisualSignOptions::default()
        };
        SuiVisualSignConverter
            .to_visual_sign_payload(SuiTransactionWrapper::new(tx), options)
            .map(|r| serde_json::to_string(&r.payload).unwrap())
    }

    fn pure_u64(v: u64) -> Input {
        Input::Pure(v.to_le_bytes().to_vec())
    }
    fn address_input() -> Input {
        Input::Pure(vec![0x77; 32])
    }
    fn owned_coin() -> Input {
        Input::ImmutableOrOwned(ObjectReference::new(
            Address::new([0x42; 32]),
            1,
            Digest::new([0; 32]),
        ))
    }
    fn split_gas(amount: u16) -> Command {
        Command::SplitCoins(SplitCoins {
            coin: Argument::Gas,
            amounts: vec![Argument::Input(amount)],
        })
    }
    fn merge_into_split(other: u16) -> Command {
        Command::MergeCoins(MergeCoins {
            coin: Argument::Result(0),
            coins_to_merge: vec![Argument::Input(other)],
        })
    }

    fn assert_no_split_amount(json: &str, unknown: &str) {
        assert!(!json.contains("1 MIST"), "split amount shown: {json}");
        assert!(json.contains(unknown), "{json}");
    }

    fn transfer_split() -> Command {
        Command::TransferObjects(TransferObjects {
            objects: vec![Argument::Result(0)],
            address: Argument::Input(1),
        })
    }

    #[test]
    fn transfer_of_single_use_split_coin_shows_amount() {
        let json = convert(
            vec![pure_u64(1_000), address_input()],
            vec![split_gas(0), transfer_split()],
        )
        .unwrap();
        assert!(json.contains("Transfer: 1000 MIST"), "{json}");
    }

    /// Split 1 MIST, merge a whole owned coin into it, transfer it: the
    /// transfer used to render as "Transfer: 1 MIST".
    #[test]
    fn transfer_of_merged_split_coin_shows_no_amount() {
        let json = convert(
            vec![pure_u64(1), address_input(), owned_coin()],
            vec![split_gas(0), merge_into_split(2), transfer_split()],
        )
        .unwrap();
        assert_no_split_amount(&json, "N/A MIST");
    }

    fn add_stake() -> Command {
        Command::MoveCall(MoveCall {
            package: Address::from_static("0x3"),
            module: Identifier::new("sui_system").unwrap(),
            function: Identifier::new("request_add_stake").unwrap(),
            type_arguments: vec![],
            arguments: vec![Argument::Input(3), Argument::Result(0), Argument::Input(1)],
        })
    }

    fn system_state() -> Input {
        Input::Shared(SharedInput::new(
            Address::from_static("0x5"),
            1,
            Mutability::Mutable,
        ))
    }

    #[test]
    fn stake_of_single_use_split_coin_shows_amount() {
        let json = convert(
            vec![
                pure_u64(1_000),
                address_input(),
                owned_coin(),
                system_state(),
            ],
            vec![split_gas(0), add_stake()],
        )
        .unwrap();
        assert!(json.contains("Stake: 1000 MIST"), "{json}");
    }

    /// The stake used to render as "Stake: 1 MIST".
    #[test]
    fn stake_of_merged_split_coin_shows_no_amount() {
        let json = convert(
            vec![pure_u64(1), address_input(), owned_coin(), system_state()],
            vec![split_gas(0), merge_into_split(2), add_stake()],
        )
        .unwrap();
        assert_no_split_amount(&json, "Stake Command");
    }
}
