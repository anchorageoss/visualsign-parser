use sui_json_rpc_types::{SuiArgument, SuiCallArg};
use sui_types::base_types::ObjectID;
use visualsign::errors::VisualSignError;

/// Gets the index from the Sui arguments array (expects a single argument).
///
/// Only `SuiArgument::Input(N)` is accepted, since the returned `u16` is used
/// downstream to index the PTB inputs vector. `SuiArgument::Result(N)` refers
/// to the output of a previous command and must not be silently coerced to an
/// inputs-vector index (see PRS-227).
pub fn get_index(sui_args: &[SuiArgument], index: Option<usize>) -> Result<u16, VisualSignError> {
    let arg: &SuiArgument = match index {
        Some(i) => sui_args
            .get(i)
            .ok_or(VisualSignError::MissingData("Index out of bounds".into()))?,
        None => sui_args
            .first()
            .ok_or(VisualSignError::MissingData("No arguments provided".into()))?,
    };

    parse_numeric_argument(*arg)
}

/// Gets a specific value from `NestedResult` by argument index and nested index
pub fn get_nested_result_value(
    sui_args: &[SuiArgument],
    arg_index: usize,
    nested_index: usize,
) -> Result<u16, VisualSignError> {
    let arg = sui_args.get(arg_index).ok_or(VisualSignError::MissingData(
        "Index out of bounds for nested result".into(),
    ))?;

    match arg {
        SuiArgument::NestedResult(first, second) => [*first, *second]
            .get(nested_index)
            .copied()
            .ok_or(VisualSignError::MissingData(
                "Nested index out of bounds".into(),
            )),
        _ => Err(VisualSignError::DecodeError(
            "Expected `NestedResult`".into(),
        )),
    }
}

/// Parses a numeric `Input(N)` index from a Sui argument.
///
/// `SuiArgument::Result(N)` is rejected because `Result(N)` references the
/// output of a prior command, not a slot in the PTB inputs vector. Conflating
/// the two lets a crafted PTB display an attacker-chosen inputs entry while
/// on-chain execution uses the (unrelated) result value (PRS-227).
///
/// Callers that legitimately want the command index inside `Result(N)` (for
/// indexing the commands vector, not the inputs vector) must use
/// [`parse_result_command_index`] instead.
pub fn parse_numeric_argument(arg: SuiArgument) -> Result<u16, VisualSignError> {
    match arg {
        SuiArgument::Input(index) => Ok(index),
        SuiArgument::Result(_) => Err(VisualSignError::DecodeError(
            "Cannot dereference `SuiArgument::Result(N)` as an input index: it references the \
             output of a prior command, not the inputs vector"
                .into(),
        )),
        _ => Err(VisualSignError::DecodeError(
            "Parsing numeric argument from Sui argument (expected `Input`)".into(),
        )),
    }
}

/// Parses the command index out of a `SuiArgument::Result(N)`.
///
/// The returned `u16` indexes the PTB commands vector (the producer of the
/// referenced result), never the inputs vector. Errors for any other variant
/// so callers cannot accidentally substitute an input index (PRS-227).
pub fn parse_result_command_index(arg: SuiArgument) -> Result<u16, VisualSignError> {
    match arg {
        SuiArgument::Result(command_index) => Ok(command_index),
        _ => Err(VisualSignError::DecodeError(
            "Expected `SuiArgument::Result(N)` for command-index lookup".into(),
        )),
    }
}

pub fn get_tx_type_arg<T>(type_args: &[String], index: usize) -> Result<T, VisualSignError>
where
    T: std::str::FromStr,
{
    type_args
        .get(index)
        .and_then(|arg| arg.parse().ok())
        .ok_or(VisualSignError::MissingData(
            "Index out of bounds for transaction type argument".into(),
        ))
}

pub fn get_object_value(
    sui_args: &[SuiArgument],
    sui_inputs: &[SuiCallArg],
    arg_index: usize,
) -> Result<ObjectID, VisualSignError> {
    let input = sui_inputs
        .get(get_index(sui_args, Some(arg_index))? as usize)
        .ok_or(VisualSignError::MissingData("Command not found".into()))?;

    match input.object() {
        Some(obj) => Ok(*obj),
        _ => Err(VisualSignError::MissingData("Object not found".into())),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use sui_json_rpc_types::SuiObjectArg;
    use sui_types::base_types::{ObjectID, SequenceNumber};

    fn shared_object_input(object_id: ObjectID) -> SuiCallArg {
        SuiCallArg::Object(SuiObjectArg::SharedObject {
            object_id,
            initial_shared_version: SequenceNumber::new(),
            mutable: true,
        })
    }

    #[test]
    fn parse_numeric_argument_accepts_input() {
        let parsed = parse_numeric_argument(SuiArgument::Input(7)).unwrap();
        assert_eq!(parsed, 7);
    }

    /// Regression for PRS-227: `Result(N)` must never be silently coerced to
    /// an inputs-vector index. `Input(N)` and `Result(N)` are semantically
    /// distinct and downstream lookups treat the returned index as an inputs
    /// slot.
    #[test]
    fn parse_numeric_argument_rejects_result_distinct_from_input() {
        // `Input(N)` succeeds and yields `N`.
        let from_input = parse_numeric_argument(SuiArgument::Input(0)).unwrap();
        assert_eq!(from_input, 0);

        // `Result(N)` with the same numeric payload must fail loudly rather
        // than coercing to the same value, so callers cannot accidentally
        // index the inputs vector with a command-output reference.
        let from_result = parse_numeric_argument(SuiArgument::Result(0));
        match from_result {
            Err(VisualSignError::DecodeError(msg)) => {
                assert!(
                    msg.contains("Result"),
                    "Expected error to mention `Result`, got: {msg}"
                );
            }
            other => panic!("Expected DecodeError for `Result(N)`, got: {other:?}"),
        }
    }

    #[test]
    fn parse_numeric_argument_rejects_gas_coin() {
        assert!(matches!(
            parse_numeric_argument(SuiArgument::GasCoin),
            Err(VisualSignError::DecodeError(_))
        ));
    }

    #[test]
    fn parse_numeric_argument_rejects_nested_result() {
        assert!(matches!(
            parse_numeric_argument(SuiArgument::NestedResult(0, 1)),
            Err(VisualSignError::DecodeError(_))
        ));
    }

    /// Regression for PRS-227: `get_object_value` must not return the
    /// inputs-vector entry when the argument is `Result(N)`. Previously the
    /// `Result(N)` branch was conflated with `Input(N)`, letting an attacker
    /// craft a PTB where the displayed pool address was `sui_inputs[N]`
    /// (attacker-chosen) while on-chain execution used the unrelated output
    /// of a prior command.
    #[test]
    fn get_object_value_distinguishes_result_from_input() {
        let benign_object = ObjectID::random();
        let inputs = vec![shared_object_input(benign_object)];

        // `Input(0)` resolves to `sui_inputs[0]` -- the benign object.
        let resolved =
            get_object_value(&[SuiArgument::Input(0)], &inputs, 0).expect("Input(0) must resolve");
        assert_eq!(resolved, benign_object);

        // `Result(0)` MUST NOT resolve to `sui_inputs[0]`. It refers to the
        // output of command 0, not an input slot, and the visualizer cannot
        // dereference it.
        let result_lookup = get_object_value(&[SuiArgument::Result(0)], &inputs, 0);
        assert!(
            matches!(result_lookup, Err(VisualSignError::DecodeError(_))),
            "Result(0) must error rather than returning the inputs[0] object; got: {result_lookup:?}"
        );
    }
}
