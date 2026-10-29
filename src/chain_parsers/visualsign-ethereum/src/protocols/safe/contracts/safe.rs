use super::super::config::SafeWallet;
use crate::context::VisualizerContext;
use crate::fmt::format_ether;
use crate::registry::ContractType;
use crate::visualizer::ContractVisualizer;
use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolCall, sol};
use visualsign::AnnotatedPayloadField;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_number_field, create_preview_layout,
    create_text_field,
};
use visualsign::vsptrait::VisualSignError;

// Define Safe wallet operations using sol! macro for type-safe ABI decoding
// The macro automatically generates SolCall trait implementations with SELECTOR constants
sol! {
    interface IGnosisSafe {
        function execTransaction(
            address to,
            uint256 value,
            bytes calldata data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address refundReceiver,
            bytes calldata signatures
        ) external payable returns (bool success);

        function addOwnerWithThreshold(address owner, uint256 _threshold) external;
        function removeOwner(address prevOwner, address owner, uint256 _threshold) external;
        function swapOwner(address prevOwner, address oldOwner, address newOwner) external;
        function changeThreshold(uint256 _threshold) external;
    }
}


// Safe wallet visualizer that uses auto-visualization
pub struct SafeWalletVisualizer;

impl SafeWalletVisualizer {
    pub fn new() -> Self {
        SafeWalletVisualizer
    }

    fn decode_add_owner(&self, params_bytes: &[u8]) -> Option<AnnotatedPayloadField> {
        let call = IGnosisSafe::addOwnerWithThresholdCall::abi_decode(params_bytes).ok()?;
        let fallback = format!(
            "Add owner {:?} with threshold {}",
            call.owner, call._threshold
        );

        let fields = vec![
            create_address_field(
                "New Owner",
                &format!("{:?}", call.owner),
                None,
                None,
                None,
                None,
            )
            .ok()?,
            create_number_field(
                "New Threshold",
                &call._threshold.to_string(),
                "signatures required",
            )
            .ok()?,
        ];

        Some(create_preview_layout("Safe: Add Owner", fallback, fields))
    }

    fn decode_remove_owner(&self, params_bytes: &[u8]) -> Option<AnnotatedPayloadField> {
        let call = IGnosisSafe::removeOwnerCall::abi_decode(params_bytes).ok()?;
        let fallback = format!(
            "Remove owner {:?} with new threshold {}",
            call.owner, call._threshold
        );

        let fields = vec![
            create_address_field(
                "Previous Owner",
                &format!("{:?}", call.prevOwner),
                None,
                Some("Required for linked list ordering"),
                None,
                None,
            )
            .ok()?,
            create_address_field(
                "Owner to Remove",
                &format!("{:?}", call.owner),
                None,
                None,
                None,
                Some("Will be removed"),
            )
            .ok()?,
            create_number_field(
                "New Threshold",
                &call._threshold.to_string(),
                "signatures required",
            )
            .ok()?,
        ];

        Some(create_preview_layout(
            "Safe: Remove Owner",
            fallback,
            fields,
        ))
    }

    fn decode_swap_owner(&self, params_bytes: &[u8]) -> Option<AnnotatedPayloadField> {
        let call = IGnosisSafe::swapOwnerCall::abi_decode(params_bytes).ok()?;
        let fallback = format!("Swap owner {:?} with {:?}", call.oldOwner, call.newOwner);

        let fields = vec![
            create_address_field(
                "Previous Owner",
                &format!("{:?}", call.prevOwner),
                None,
                Some("Required for linked list ordering"),
                None,
                None,
            )
            .ok()?,
            create_address_field(
                "Old Owner",
                &format!("{:?}", call.oldOwner),
                None,
                None,
                None,
                Some("Will be removed"),
            )
            .ok()?,
            create_address_field(
                "New Owner",
                &format!("{:?}", call.newOwner),
                None,
                None,
                None,
                Some("Will be added"),
            )
            .ok()?,
        ];

        Some(create_preview_layout("Safe: Swap Owner", fallback, fields))
    }

    fn decode_change_threshold(&self, params_bytes: &[u8]) -> Option<AnnotatedPayloadField> {
        let call = IGnosisSafe::changeThresholdCall::abi_decode(params_bytes).ok()?;
        let fallback = format!("Change threshold to {}", call._threshold);

        let mut fields = vec![
            create_number_field(
                "New Threshold",
                &call._threshold.to_string(),
                "signatures required",
            )
            .ok()?,
        ];

        if call._threshold == U256::from(1) {
            fields.push(
                create_text_field(
                    "Warning",
                    "Setting threshold to 1 allows single signature control",
                )
                .ok()?,
            );
        }

        Some(create_preview_layout(
            "Safe: Change Threshold",
            fallback,
            fields,
        ))
    }

    fn decode_exec_transaction(&self, params_bytes: &[u8]) -> Option<AnnotatedPayloadField> {
        let call = IGnosisSafe::execTransactionCall::abi_decode(params_bytes).ok()?;

        let mut fields = vec![
            create_address_field("Target", &format!("{:?}", call.to), None, None, None, None)
                .ok()?,
        ];

        if call.value > U256::ZERO {
            let value_str = format_ether(call.value);
            fields.push(create_amount_field("Value", &value_str, "ETH").ok()?);
        }

        let operation_text = match call.operation {
            0 => "Call",
            1 => "DelegateCall",
            _ => "Unknown",
        };
        fields.push(create_text_field("Operation Type", operation_text).ok()?);

        if call.gasToken != Address::ZERO {
            fields.push(create_text_field("Gas Token", &format!("{:?}", call.gasToken)).ok()?);
        }

        if !call.signatures.is_empty() {
            let signature_count = call.signatures.len() / 65;
            fields.push(
                create_number_field("Signatures", &signature_count.to_string(), "provided").ok()?,
            );
        }

        if !call.data.is_empty() {
            fields.push(create_text_field("Data", &format!("{} bytes", call.data.len())).ok()?);
        }

        let fallback = if call.value > U256::ZERO && call.data.is_empty() {
            format!(
                "Send {} ETH to {:?}",
                format_ether(call.value),
                call.to
            )
        } else if call.value > U256::ZERO {
            format!(
                "Execute transaction with {} ETH to {:?}",
                format_ether(call.value),
                call.to
            )
        } else {
            format!("Execute transaction to {:?}", call.to)
        };

        Some(create_preview_layout(
            "Safe: Execute Transaction",
            fallback,
            fields,
        ))
    }
}

// Function selectors are automatically generated by the sol! macro
// They're derived as the first 4 bytes of keccak256(function_signature)
// We extract them from the SolCall trait implementations

impl ContractVisualizer for SafeWalletVisualizer {
    fn contract_type(&self) -> &str {
        SafeWallet::short_type_id()
    }

    fn visualize(
        &self,
        context: &VisualizerContext,
    ) -> Result<Option<Vec<AnnotatedPayloadField>>, VisualSignError> {
        if context.calldata.len() < 4 {
            return Ok(None);
        }

        let selector: [u8; 4] = match context.calldata[0..4].try_into() {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let params_bytes = &context.calldata[4..];

        // Match against function selectors from sol! macro generated SolCall implementations
        let field = match selector {
            IGnosisSafe::addOwnerWithThresholdCall::SELECTOR => self.decode_add_owner(params_bytes),
            IGnosisSafe::removeOwnerCall::SELECTOR => self.decode_remove_owner(params_bytes),
            IGnosisSafe::swapOwnerCall::SELECTOR => self.decode_swap_owner(params_bytes),
            IGnosisSafe::changeThresholdCall::SELECTOR => {
                self.decode_change_threshold(params_bytes)
            }
            IGnosisSafe::execTransactionCall::SELECTOR => {
                self.decode_exec_transaction(params_bytes)
            }
            _ => None,
        };

        Ok(field.map(|f| vec![f]))
    }
}
