use sui_json_rpc_types::SuiArgument;
use sui_json_rpc_types::SuiArgument::Input;

/// Get index from SUI arguments array (expects single argument)
pub fn get_index(sui_args: &[SuiArgument], index: Option<usize>) -> Option<u16> {
    let arg: &SuiArgument = match index {
        Some(i) => sui_args.get(i)?,
        None => sui_args.first()?,
    };

    parse_numeric_argument(arg)
}

/// Get specific value from `NestedResult` by argument index and nested index
pub fn get_nested_result_value(
    sui_args: &[SuiArgument],
    arg_index: usize,
    nested_index: usize,
) -> Option<u16> {
    let arg = sui_args.get(arg_index)?;

    match arg {
        SuiArgument::NestedResult(first, second) => [*first, *second].get(nested_index).copied(),
        _ => None,
    }
}

/// Parse numeric argument from SUI argument (Input or Result)
pub fn parse_numeric_argument(arg: &SuiArgument) -> Option<u16> {
    match arg {
        Input(index) => Some(*index),
        SuiArgument::Result(index) => Some(*index),
        _ => None,
    }
}

pub fn get_tx_type_arg<T>(type_args: &[String], index: usize) -> Option<T>
where
    T: std::str::FromStr,
{
    type_args.get(index).and_then(|arg| arg.parse().ok())
}
