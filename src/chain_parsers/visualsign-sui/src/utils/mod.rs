mod address;
mod coin;
mod numeric;
mod package;
mod tx_args;

pub use address::truncate_address;
pub use coin::{CoinObject, SuiCoin};
pub use numeric::decode_number;
pub use package::SuiPackage;
pub use tx_args::{get_index, get_nested_result_value, get_tx_type_arg, parse_numeric_argument};

#[cfg(test)]
pub mod test_utils {
    use crate::transaction_string_to_visual_sign;
    use visualsign::SignablePayload;
    use visualsign::vsptrait::VisualSignOptions;

    pub fn payload_from_b64(data: &str) -> SignablePayload {
        transaction_string_to_visual_sign(
            data,
            VisualSignOptions {
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .expect("Failed to visualize tx commands")
    }

    pub fn assert_has_field(payload: &SignablePayload, label: &str) {
        payload
            .fields
            .iter()
            .find(|f| f.label() == label)
            .unwrap_or_else(|| panic!("Should have a {label} field"));
    }
}
