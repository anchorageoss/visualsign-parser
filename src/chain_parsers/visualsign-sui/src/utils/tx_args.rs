use sui_json_rpc_types::SuiArgument;
use sui_json_rpc_types::SuiArgument::Input;
use visualsign::errors::VisualSignError;

/// Gets the index from the Sui arguments array (expects a single argument)
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

/// Parses a numeric argument from a Sui argument (`Input` or `Result`)
pub fn parse_numeric_argument(arg: SuiArgument) -> Result<u16, VisualSignError> {
    match arg {
        SuiArgument::Result(index) | Input(index) => Ok(index),
        _ => Err(VisualSignError::DecodeError(
            "Parsing numeric argument from Sui argument (expected `Input` or `Result`)".into(),
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
