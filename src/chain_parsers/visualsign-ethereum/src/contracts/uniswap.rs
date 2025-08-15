use alloy_sol_types::{SolCall as _, sol};
use chrono::{TimeZone, Utc};
use num_enum::TryFromPrimitive;
use visualsign::{SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldTextV2};

// From: https://github.com/Uniswap/universal-router/blob/main/contracts/interfaces/IUniversalRouter.sol
sol! {
    interface IUniversalRouter {
        /// @notice Executes encoded commands along with provided inputs. Reverts if deadline has expired.
        /// @param commands A set of concatenated commands, each 1 byte in length
        /// @param inputs An array of byte strings containing abi encoded inputs for each command
        /// @param deadline The deadline by which the transaction must be executed
        function execute(bytes calldata commands, bytes[] calldata inputs, uint256 deadline) external payable;
    }
}

// From: https://github.com/Uniswap/universal-router/blob/main/contracts/libraries/Commands.sol
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum Command {
    V3SwapExactIn = 0x00,
    V3SwapExactOut = 0x01,
    Permit2TransferFrom = 0x02,
    Permit2PermitBatch = 0x03,
    Sweep = 0x04,
    Transfer = 0x05,
    PayPortion = 0x06,

    V2SwapExactIn = 0x08,
    V2SwapExactOut = 0x09,
    Permit2Permit = 0x0a,
    WrapEth = 0x0b,
    UnwrapWeth = 0x0c,
    Permit2TransferFromBatch = 0x0d,
    BalanceCheckErc20 = 0x0e,

    V4Swap = 0x10,
    V3PositionManagerPermit = 0x11,
    V3PositionManagerCall = 0x12,
    V4InitializePool = 0x13,
    V4PositionManagerCall = 0x14,

    ExecuteSubPlan = 0x21,
}

fn map_commands(raw: &[u8]) -> Option<Vec<Command>> {
    let mut out = Vec::with_capacity(raw.len());
    for &b in raw {
        out.push(Command::try_from(b).unwrap());
    }
    Some(out)
}

fn make_field(
    commands: &[u8],
    deadline: Option<&str>,
    mapped: &Vec<Command>,
) -> SignablePayloadField {
    let (fallback, text) = if let Some(dl) = deadline {
        (
            format!(
                "Universal Router Execute: {} commands ({:?}), deadline {}",
                commands.len(),
                mapped,
                dl
            ),
            format!("Commands: {:?}\nDeadline: {}", mapped, dl),
        )
    } else {
        (
            format!(
                "Universal Router Execute: {} commands ({:?})",
                commands.len(),
                mapped
            ),
            format!("Commands: {:?}", mapped),
        )
    };
    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: fallback,
            label: "Universal Router".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text },
    }
}

pub fn parse_universal_router_execute(input: &[u8]) -> Vec<SignablePayloadField> {
    let mut fields = Vec::new();
    if input.len() < 4 {
        return fields;
    }
    if let Ok(call) = IUniversalRouter::executeCall::abi_decode(input) {
        let deadline_val: i64 = match call.deadline.try_into() {
            Ok(val) => val,
            Err(_) => return fields,
        };
        let deadline = if deadline_val > 0 {
            Utc.timestamp_opt(deadline_val, 0)
                .single()
                .map(|dt| dt.to_string())
        } else {
            None
        };
        let commands = call.commands.0;
        if let Some(mapped) = map_commands(&commands) {
            fields.push(make_field(&commands, deadline.as_deref(), &mapped));
        }
    }
    fields
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Uint;

    #[test]
    fn test_parse_universal_router_execute_invalid_selector() {
        // Wrong selector, but parse_universal_router_execute just returns empty Vec
        let input_data = vec![0x00, 0x00, 0x00, 0x00];
        let fields = parse_universal_router_execute(&input_data);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_parse_universal_router_execute_too_short() {
        // Less than 4 bytes
        let input_data = vec![0x35, 0x93, 0x56];
        let fields = parse_universal_router_execute(&input_data);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_parse_universal_router_execute_field_with_deadline() {
        let commands: Vec<u8> = vec![
            Command::V4Swap as u8,
            Command::Transfer as u8,
            Command::Permit2Permit as u8,
        ];
        let inputs: Vec<Vec<u8>> = vec![];
        let deadline = Uint::<256, 4>::from(1234567890);
        let call = IUniversalRouter::executeCall {
            commands: commands.clone().into(),
            inputs: inputs.iter().map(|v| v.clone().into()).collect(),
            deadline,
        };

        let input_data = call.abi_encode();
        let fields = parse_universal_router_execute(&input_data);
        assert_eq!(fields.len(), 1);
        if let SignablePayloadField::TextV2 { common, text_v2 } = &fields[0] {
            assert_eq!(
                "Universal Router Execute: 3 commands ([V4Swap, Transfer, Permit2Permit]), deadline 2009-02-13 23:31:30 UTC",
                common.fallback_text
            );
            assert_eq!(
                "Commands: [V4Swap, Transfer, Permit2Permit]\nDeadline: 2009-02-13 23:31:30 UTC",
                text_v2.text
            );
        } else {
            panic!("Expected TextV2 field");
        }
    }

    #[test]
    fn test_parse_universal_router_execute_field_without_deadline() {
        let commands: Vec<u8> = vec![Command::V3SwapExactIn as u8, Command::Transfer as u8];
        let inputs: Vec<Vec<u8>> = vec![];
        let call = IUniversalRouter::executeCall {
            commands: commands.clone().into(),
            inputs: inputs.iter().map(|v| v.clone().into()).collect(),
            deadline: Uint::<256, 4>::from(0),
        };
        let input_data = call.abi_encode();
        let fields = parse_universal_router_execute(&input_data);
        assert_eq!(fields.len(), 1);
        if let SignablePayloadField::TextV2 { common, text_v2 } = &fields[0] {
            assert_eq!(
                "Universal Router Execute: 2 commands ([V3SwapExactIn, Transfer])",
                common.fallback_text
            );
            assert_eq!("Commands: [V3SwapExactIn, Transfer]", text_v2.text);
        } else {
            panic!("Expected TextV2 field");
        }
    }
}
