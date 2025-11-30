use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::{SolCall as _, SolValue as _, sol};
use chrono::TimeZone;
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::context::VisualizerContext;
use crate::contracts::core::erc20::IERC20;
use crate::fmt::is_unlimited_u256;
use crate::protocols::morpho::config::Bundler3Contract;
use crate::registry::{ContractRegistry, ContractType};

// Morpho Bundler3 interface definitions
//
// Official Documentation:
// - Technical Reference: https://docs.morpho.org/contracts/bundler
// - Contract Source: https://github.com/morpho-org/bundler3
// - GeneralAdapter1 Operations: https://github.com/morpho-org/bundler3/blob/3b22daf606bdef4f119f168c74496f87a90ac8e5/src/adapters/GeneralAdapter1.sol#L373
//
// The Bundler3 contract allows batching multiple operations into a single transaction.
// Key operations:
// - permit2TransferFrom: Transfer via Permit2 standard
// - erc20TransferFrom: Direct ERC20 transfer with pre-approved allowance
// - erc4626Deposit: Deposit into ERC-4626 vault
sol! {
    /// @notice Struct containing all the data needed to make a call.
    struct Call {
        address to;
        bytes data;
        uint256 value;
        bool skipRevert;
        bytes32 callbackHash;
    }

    interface IBundler3 {
        /// @notice Executes multiple calls in sequence
        function multicall(Call[] calldata) external payable;
    }

    // Standard ERC-2612 permit interface (not bundler-specific)
    interface IERC2612 {
        /// @notice ERC-2612 permit function
        function permit(
            address owner,
            address spender,
            uint256 value,
            uint256 deadline,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external;
    }

    // Bundler-specific permit operations
    interface IBundlerPermit {
        /// @notice ERC-2612 permit function
        function permit(
            address owner,
            address spender,
            uint256 value,
            uint256 deadline,
            uint8 v,
            bytes32 r,
            bytes32 s
        ) external;
    }

    // Morpho GeneralAdapter1 operations (type-safe parameter decoding)
    // Reference: https://github.com/morpho-org/bundler3/blob/3b22daf606bdef4f119f168c74496f87a90ac8e5/src/adapters/GeneralAdapter1.sol

    interface IGeneralAdapter1 {
        /// @notice Direct ERC20 transfer with pre-approved allowance
        function erc20TransferFrom(
            address token,
            address receiver,
            uint256 amount
        ) external;

        /// @notice Deposit into ERC-4626 vault
        function erc4626Deposit(
            address vault,
            uint256 assets,
            uint256 minShares,
            address receiver
        ) external;
    }

    /// @notice Direct ERC20 transfer with pre-approved allowance
    struct Erc20TransferFromParams {
        address token;
        address receiver;
        uint256 amount;
    }

    /// @notice Deposit into ERC-4626 vault
    struct Erc4626DepositParams {
        address vault;
        uint256 assets;
        uint256 minShares;
        address receiver;
    }
}

/// Visualizer for Morpho Bundler3 contract
pub struct BundlerVisualizer {}

impl BundlerVisualizer {
    /// Visualizes Morpho Bundler3 multicall operations
    ///
    /// # Arguments
    /// * `input` - The calldata bytes
    /// * `chain_id` - The chain ID for registry lookups
    /// * `registry` - Optional registry for resolving token symbols
    pub fn visualize_multicall(
        &self,
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        // Check multicall selector (IBundler3::multicall)
        let selector = &input[0..4];
        if selector != IBundler3::multicallCall::SELECTOR {
            return None;
        }

        // Try decoding the multicall
        let call = match IBundler3::multicallCall::abi_decode(input) {
            Ok(c) => c,
            Err(_) => return None,
        };

        let calls = &call.0;
        let mut detail_fields = Vec::new();

        for morpho_call in calls.iter() {
            // Decode the nested call data
            let nested_field = Self::decode_nested_call(
                &morpho_call.to,
                &morpho_call.data,
                &morpho_call.value,
                chain_id,
                registry,
            );

            detail_fields.push(nested_field);
        }

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("Morpho Bundler: {} operations", calls.len()),
                label: "Morpho Bundler".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Morpho Bundler Multicall".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 {
                    text: format!("{} operation(s)", calls.len()),
                }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout {
                    fields: detail_fields
                        .into_iter()
                        .map(|f| AnnotatedPayloadField {
                            signable_payload_field: f,
                            static_annotation: None,
                            dynamic_annotation: None,
                        })
                        .collect(),
                }),
            },
        })
    }

    /// Decodes a nested call within the multicall
    fn decode_nested_call(
        to: &Address,
        data: &Bytes,
        _value: &U256,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        if data.len() < 4 {
            return Self::unknown_call_field(to, data);
        }

        let selector = &data[0..4];

        // Match known Morpho Bundler3 operation selectors (type-safe from sol! macros)
        match selector {
            // Standard ERC-2612 permit(address,address,uint256,uint256,uint8,bytes32,bytes32)
            s if s == IERC2612::permitCall::SELECTOR => {
                Self::decode_permit(data, to, chain_id, registry)
            }
            // IBundlerPermit::permit(address,address,uint256,uint256,uint8,bytes32,bytes32)
            s if s == IBundlerPermit::permitCall::SELECTOR => {
                Self::decode_permit(data, to, chain_id, registry)
            }
            // IERC20::transferFrom(address,address,uint256) - standard ERC20
            s if s == IERC20::transferFromCall::SELECTOR => {
                Self::decode_erc20_transfer_from(&data[4..], chain_id, registry)
            }
            // IGeneralAdapter1::erc20TransferFrom(address,address,uint256)
            s if s == IGeneralAdapter1::erc20TransferFromCall::SELECTOR => {
                Self::decode_morpho_transfer_from(&data[4..], chain_id, registry)
            }
            // IGeneralAdapter1::erc4626Deposit(address,uint256,uint256,address)
            s if s == IGeneralAdapter1::erc4626DepositCall::SELECTOR => {
                Self::decode_erc4626_deposit(&data[4..], chain_id, registry)
            }
            _ => Self::unknown_call_field(to, data),
        }
    }

    /// Decodes ERC-2612 permit operation
    /// This handles both standard ERC-2612 and custom Morpho Bundler3 permit calls.
    fn decode_permit(
        bytes: &[u8],
        token_address: &Address,
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Since both permit types have identical signatures, use IERC2612 for decoding
        let call = match IERC2612::permitCall::abi_decode(bytes) {
            Ok(c) => c,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Permit: Invalid data".to_string(),
                        label: "Permit".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode permit parameters".to_string(),
                    },
                };
            }
        };

        let owner = call.owner;
        let spender = call.spender;
        let value = call.value;
        let deadline = call.deadline;

        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, *token_address))
            .unwrap_or_else(|| format!("{:?}", token_address));

        let value_u128: u128 = value.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, *token_address, value_u128))
            .unwrap_or_else(|| (value.to_string(), token_symbol.clone()));

        // Check if value is unlimited (U256::MAX for permit)
        let display_amount = if is_unlimited_u256(value) {
            "Unlimited".to_string()
        } else {
            amount_str.clone()
        };

        let deadline_str = if is_unlimited_u256(deadline) {
            "No expiry".to_string()
        } else {
            let deadline_u64: u64 = deadline.to_string().parse().unwrap_or(0);
            let dt = chrono::Utc.timestamp_opt(deadline_u64 as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

        let summary = format!(
            "Permit {} {} to {:?} (expires: {})",
            display_amount, token_symbol, spender, deadline_str
        );

        // Create detailed parameter fields for debugging
        let param_fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", token_address),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, token_address),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", owner),
                        label: "Owner".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", owner),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", spender),
                        label: "Spender".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", spender),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: value.to_string(),
                        label: "Value".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_unlimited_u256(value) {
                            format!("{} (unlimited)", value)
                        } else {
                            format!("{} {} (raw: {})", amount_str, token_symbol, value)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: deadline.to_string(),
                        label: "Deadline".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", deadline, deadline_str),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Permit".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "ERC-2612 Permit".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout {
                    fields: param_fields,
                }),
            },
        }
    }

    /// Decodes erc20TransferFrom operation using shared core IERC20 interface
    /// This delegates to the core ERC-20 implementation for type-safe decoding
    fn decode_erc20_transfer_from(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Use the shared IERC20 interface for decoding
        let call = match IERC20::transferFromCall::abi_decode(bytes) {
            Ok(c) => c,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("ERC20 Transfer From: 0x{}", hex::encode(bytes)),
                        label: "ERC20 Transfer From".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let token_address = call.from; // from is the token address in the encoding
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_address))
            .unwrap_or_else(|| format!("{:?}", token_address));

        let amount_u128: u128 = call.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_address, amount_u128))
            .unwrap_or_else(|| (call.amount.to_string(), token_symbol.clone()));

        let summary = format!(
            "Transfer {} {} from {:?}",
            amount_str, token_symbol, call.from
        );

        // Create detailed parameter fields for debugging
        let param_fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", token_address),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, token_address),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", call.from),
                        label: "From".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", call.from),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: call.amount.to_string(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, call.amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Transfer From".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "ERC20 Transfer From".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout {
                    fields: param_fields,
                }),
            },
        }
    }

    /// Decodes Morpho-specific ERC20 transfer operation
    /// From GeneralAdapter1: erc20TransferFrom(address token, address receiver, uint256 amount)
    /// Reference: https://github.com/morpho-org/bundler3/blob/3b22daf606bdef4f119f168c74496f87a90ac8e5/src/adapters/GeneralAdapter1.sol#L373
    fn decode_morpho_transfer_from(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        // Decode using type-safe Erc20TransferFromParams (token, receiver, amount)
        let params = match Erc20TransferFromParams::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Morpho Transfer: 0x{}", hex::encode(bytes)),
                        label: "Morpho Transfer".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        let token_address = params.token;
        let token_symbol = registry
            .and_then(|r| r.get_token_symbol(chain_id, token_address))
            .unwrap_or_else(|| format!("{:?}", token_address));

        let amount_u128: u128 = params.amount.to_string().parse().unwrap_or(0);
        let (amount_str, _) = registry
            .and_then(|r| r.format_token_amount(chain_id, token_address, amount_u128))
            .unwrap_or_else(|| (params.amount.to_string(), token_symbol.clone()));

        let summary = format!(
            "Transfer {} {} to {:?}",
            amount_str, token_symbol, params.receiver
        );

        // Create detailed parameter fields
        let param_fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", token_address),
                        label: "Token".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({:?})", token_symbol, token_address),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.receiver),
                        label: "Receiver".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.receiver),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: params.amount.to_string(),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, params.amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Morpho Transfer".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "ERC20 Transfer".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout {
                    fields: param_fields,
                }),
            },
        }
    }

    /// Decodes erc4626Deposit operation
    fn decode_erc4626_deposit(
        bytes: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> SignablePayloadField {
        let params = match Erc4626DepositParams::abi_decode(bytes) {
            Ok(p) => p,
            Err(_) => {
                return SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("ERC4626 Deposit: 0x{}", hex::encode(bytes)),
                        label: "ERC4626 Deposit".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Failed to decode parameters".to_string(),
                    },
                };
            }
        };

        // Try to get vault info from registry
        let vault_symbol = registry.and_then(|r| r.get_token_symbol(chain_id, params.vault));

        let assets_u128: u128 = params.assets.to_string().parse().unwrap_or(0);
        let min_shares_u128: u128 = params.minShares.to_string().parse().unwrap_or(0);

        // Format the deposit summary
        let vault_display = vault_symbol
            .as_ref()
            .map(|s| format!("{} vault", s))
            .unwrap_or_else(|| format!("vault {:?}", params.vault));

        let summary = format!(
            "Deposit {} assets into {} (min {} shares) for {:?}",
            assets_u128, vault_display, min_shares_u128, params.receiver
        );

        // Format vault display for expanded view
        let vault_text = if let Some(symbol) = &vault_symbol {
            format!("{} ({:?})", symbol, params.vault)
        } else {
            format!("{:?}", params.vault)
        };

        // Create detailed parameter fields for debugging
        let param_fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.vault),
                        label: "Vault".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 { text: vault_text },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: params.assets.to_string(),
                        label: "Assets".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: params.assets.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: params.minShares.to_string(),
                        label: "Min Shares".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: params.minShares.to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", params.receiver),
                        label: "Receiver".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", params.receiver),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Vault Deposit".to_string(),
            },
            preview_layout: visualsign::SignablePayloadFieldPreviewLayout {
                title: Some(visualsign::SignablePayloadFieldTextV2 {
                    text: "ERC4626 Vault Deposit".to_string(),
                }),
                subtitle: Some(visualsign::SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(visualsign::SignablePayloadFieldListLayout {
                    fields: param_fields,
                }),
            },
        }
    }

    /// Creates a field for unknown calls
    fn unknown_call_field(to: &Address, data: &Bytes) -> SignablePayloadField {
        let selector = if data.len() >= 4 {
            format!("0x{}", hex::encode(&data[0..4]))
        } else {
            "Unknown".to_string()
        };

        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("Call to {:?}", to),
                label: "Unknown Call".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("To: {:?}, Selector: {}", to, selector),
            },
        }
    }
}

/// ContractVisualizer implementation for Morpho Bundler3
pub struct BundlerContractVisualizer {
    inner: BundlerVisualizer,
}

impl BundlerContractVisualizer {
    pub fn new() -> Self {
        Self {
            inner: BundlerVisualizer {},
        }
    }
}

impl Default for BundlerContractVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::visualizer::ContractVisualizer for BundlerContractVisualizer {
    fn contract_type(&self) -> &str {
        Bundler3Contract::short_type_id()
    }

    fn visualize(
        &self,
        context: &VisualizerContext,
    ) -> Result<Option<Vec<AnnotatedPayloadField>>, visualsign::vsptrait::VisualSignError> {
        let (contract_registry, _visualizer_reg) = ContractRegistry::with_default_protocols();

        if let Some(field) = self.inner.visualize_multicall(
            &context.calldata,
            context.chain_id,
            Some(&contract_registry),
        ) {
            let annotated = AnnotatedPayloadField {
                signable_payload_field: field,
                static_annotation: None,
                dynamic_annotation: None,
            };

            Ok(Some(vec![annotated]))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visualize_multicall_real_transaction() {
        // Real Morpho transaction calldata with 3 operations
        let input_hex = "374f435d00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000002200000000000000000000000000000000000000000000000000000000000000360000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb4800000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000e4d505accf000000000000000000000000078473fc814d2581c0e9b06efb2443ea503421cb0000000000000000000000004a6c312ec70e8747a587ee860a0353cd42be0ae000000000000000000000000000000000000000000000000000000000000f42400000000000000000000000000000000000000000000000000000000068f67d97000000000000000000000000000000000000000000000000000000000000001b5c10d948b0e33626f5f196df389c9f8b95c85a66065bc16c5a23a5ba9dde396941a237ed342773264d7a1694bcce90bf5538ae75eab39edd0ebcb1077442df9f000000000000000000000000000000000000000000000000000000000000000000000000000000004a6c312ec70e8747a587ee860a0353cd42be0ae000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000064d96ca0b9000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb480000000000000000000000004a6c312ec70e8747a587ee860a0353cd42be0ae000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000000000000000000000000004a6c312ec70e8747a587ee860a0353cd42be0ae000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000846ef5eeae000000000000000000000000beef01735c132ada46aa9aa4c54623caa92a64cb00000000000000000000000000000000000000000000000000000000000f42400000000000000000000000000000000000000000000000000003ece3bf77e9a9000000000000000000000000078473fc814d2581c0e9b06efb2443ea503421cb0000000000000000000000000000000000000000000000000000000068f661a72222da44";
        let input = hex::decode(input_hex).unwrap();

        let (registry, _) = ContractRegistry::with_default_protocols();
        let result = BundlerVisualizer {}.visualize_multicall(&input, 1, Some(&registry));

        assert!(
            result.is_some(),
            "Should successfully decode Morpho multicall"
        );

        let field = result.unwrap();
        if let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = field
        {
            assert!(
                common.fallback_text.contains("3 operations"),
                "Expected 3 operations, got: {}",
                common.fallback_text
            );

            assert!(
                preview_layout.expanded.is_some(),
                "Expected expanded section"
            );

            if let Some(list_layout) = preview_layout.expanded {
                assert_eq!(list_layout.fields.len(), 3, "Expected 3 decoded operations");
            }
        } else {
            panic!("Expected PreviewLayout");
        }
    }

    #[test]
    fn test_decode_permit() {
        // Create proper ERC-2612 permit calldata with function selector
        let mut calldata = Vec::new();

        // Add ERC-2612 permit function selector: permit(address,address,uint256,uint256,uint8,bytes32,bytes32)
        // Selector: 0xd505accf
        calldata.extend_from_slice(&[0xd5, 0x05, 0xac, 0xcf]);

        // ABI encode the parameters
        let owner =
            Address::from_slice(&hex::decode("078473fc814d2581c0e9b06efb2443ea503421cb").unwrap());
        let spender =
            Address::from_slice(&hex::decode("4a6c312ec70e8747a587ee860a0353cd42be0ae0").unwrap());
        let value = U256::from(1000000u64); // 1 USDC (6 decimals)
        let deadline = U256::from(1758288535u64); // Future timestamp
        let v = 27u8;
        let r = [1u8; 32]; // Dummy signature
        let s = [2u8; 32]; // Dummy signature

        // Encode parameters (each is 32 bytes in ABI encoding)
        calldata.extend_from_slice(&[0u8; 12]); // padding for address
        calldata.extend_from_slice(owner.as_slice()); // owner
        calldata.extend_from_slice(&[0u8; 12]); // padding for address  
        calldata.extend_from_slice(spender.as_slice()); // spender
        calldata.extend_from_slice(&value.to_be_bytes::<32>()); // value
        calldata.extend_from_slice(&deadline.to_be_bytes::<32>()); // deadline
        calldata.extend_from_slice(&[0u8; 31]); // padding for uint8
        calldata.push(v); // v
        calldata.extend_from_slice(&r); // r
        calldata.extend_from_slice(&s); // s

        let token_address: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
            .parse()
            .unwrap(); // USDC

        let (registry, _) = ContractRegistry::with_default_protocols();
        let result =
            BundlerVisualizer::decode_permit(&calldata, &token_address, 1, Some(&registry));

        match result {
            SignablePayloadField::PreviewLayout {
                common,
                preview_layout,
            } => {
                assert_eq!(common.label, "Permit");
                assert!(common.fallback_text.contains("USDC"));

                // Verify expanded view has parameters
                assert!(preview_layout.expanded.is_some());
                if let Some(expanded) = preview_layout.expanded {
                    assert_eq!(expanded.fields.len(), 5, "Should have 5 parameter fields");
                }
            }
            other => panic!("Expected PreviewLayout field, got: {:?}", other),
        }
    }
}
