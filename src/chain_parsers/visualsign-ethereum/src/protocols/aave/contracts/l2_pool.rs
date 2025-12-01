use alloy_primitives::Address;
use alloy_sol_types::{SolCall as _, sol};
use chrono::{TimeZone, Utc};
use visualsign::{
    AnnotatedPayloadField, SignablePayloadField, SignablePayloadFieldCommon,
    SignablePayloadFieldListLayout, SignablePayloadFieldPreviewLayout, SignablePayloadFieldTextV2,
};

use crate::registry::ContractRegistry;

// Aave v3 L2Pool interface - Layer 2 optimized version
//
// L2Pool uses packed bytes32 parameters to save gas costs on L2 networks.
// Deployed on: Arbitrum, Optimism, Polygon
//
// Encoding format:
// - Bits 0-15: Asset ID (uint16) - reserve ID from Aave protocol
// - Bits 16-143: Amount (uint128) - transaction amount
// - Bits 144-151: Interest Rate Mode (uint8) - only for borrow/repay (2 = Variable)
//
// Source: https://github.com/aave-dao/aave-v3-origin/blob/main/src/contracts/interfaces/IL2Pool.sol
sol! {
    interface IL2Pool {
        function supply(bytes32 args) external;
        function withdraw(bytes32 args) external returns (uint256);
        function borrow(bytes32 args) external;
        function repay(bytes32 args) external returns (uint256);
        function repayWithATokens(bytes32 args) external returns (uint256);
        function liquidationCall(bytes32 args1, bytes32 args2) external;
        function setUserUseReserveAsCollateral(bytes32 args) external;

        function supplyWithPermit(bytes32 args, bytes32 r, bytes32 s) external;
        function repayWithPermit(bytes32 args, bytes32 r, bytes32 s) external returns (uint256);
    }
}

pub struct L2PoolVisualizer {}

impl L2PoolVisualizer {
    pub fn new() -> Self {
        Self {}
    }

    /// Visualizes Aave v3 L2Pool operations
    ///
    /// # Arguments
    /// * `input` - The calldata bytes
    /// * `chain_id` - The chain ID for registry lookups
    /// * `registry` - Optional registry for resolving token symbols
    pub fn visualize_l2pool_operation(
        &self,
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        if input.len() < 4 {
            return None;
        }

        let selector = &input[0..4];

        match selector {
            _ if selector == IL2Pool::supplyCall::SELECTOR => {
                Self::decode_l2_supply(input, chain_id, registry)
            }
            _ if selector == IL2Pool::withdrawCall::SELECTOR => {
                Self::decode_l2_withdraw(input, chain_id, registry)
            }
            _ if selector == IL2Pool::borrowCall::SELECTOR => {
                Self::decode_l2_borrow(input, chain_id, registry)
            }
            _ if selector == IL2Pool::repayCall::SELECTOR => {
                Self::decode_l2_repay(input, chain_id, registry)
            }
            _ if selector == IL2Pool::repayWithATokensCall::SELECTOR => {
                Self::decode_l2_repay_with_atokens(input, chain_id, registry)
            }
            _ if selector == IL2Pool::liquidationCallCall::SELECTOR => {
                Self::decode_l2_liquidation_call(input, chain_id, registry)
            }
            _ if selector == IL2Pool::setUserUseReserveAsCollateralCall::SELECTOR => {
                Self::decode_l2_set_user_use_reserve_as_collateral(input, chain_id, registry)
            }
            _ if selector == IL2Pool::supplyWithPermitCall::SELECTOR => {
                Self::decode_l2_supply_with_permit(input, chain_id, registry)
            }
            _ if selector == IL2Pool::repayWithPermitCall::SELECTOR => {
                Self::decode_l2_repay_with_permit(input, chain_id, registry)
            }
            _ => None,
        }
    }

    fn split_bytes32(args: [u8; 32]) -> (u128, u128) {
        let mut lower_bytes = [0u8; 16];
        lower_bytes.copy_from_slice(&args[16..32]);
        let lower = u128::from_be_bytes(lower_bytes);

        let mut upper_bytes = [0u8; 16];
        upper_bytes.copy_from_slice(&args[0..16]);
        let upper = u128::from_be_bytes(upper_bytes);

        (lower, upper)
    }

    fn decode_supply_params(args: [u8; 32]) -> (u16, u128, u16) {
        let (lower, upper) = Self::split_bytes32(args);

        let asset_id = (lower & 0xFFFF) as u16;
        let amount = (lower >> 16) as u128;
        let referral_code = ((upper >> 16) & 0xFFFF) as u16;

        (asset_id, amount, referral_code)
    }

    fn decode_withdraw_params(args: [u8; 32]) -> (u16, u128) {
        let (lower, _upper) = Self::split_bytes32(args);

        let asset_id = (lower & 0xFFFF) as u16;
        let amount = (lower >> 16) as u128;

        (asset_id, amount)
    }

    fn decode_borrow_params(args: [u8; 32]) -> (u16, u128, u8, u16) {
        let (lower, upper) = Self::split_bytes32(args);

        let asset_id = (lower & 0xFFFF) as u16;
        let amount = (lower >> 16) as u128;
        let interest_rate_mode = ((upper >> 16) & 0xFF) as u8;
        let referral_code = ((upper >> 24) & 0xFFFF) as u16;

        (asset_id, amount, interest_rate_mode, referral_code)
    }

    fn decode_repay_params(args: [u8; 32]) -> (u16, u128, u8) {
        let (lower, upper) = Self::split_bytes32(args);

        let asset_id = (lower & 0xFFFF) as u16;
        let amount = (lower >> 16) as u128;
        let interest_rate_mode = ((upper >> 16) & 0xFF) as u8;

        (asset_id, amount, interest_rate_mode)
    }

    fn decode_set_user_use_reserve_as_collateral_params(args: [u8; 32]) -> (u16, bool) {
        let (lower, _upper) = Self::split_bytes32(args);

        let asset_id = (lower & 0xFFFF) as u16;
        let use_as_collateral = ((lower >> 16) & 0x1) == 1;

        (asset_id, use_as_collateral)
    }

    fn extract_supply_permit_deadline(args: [u8; 32]) -> u32 {
        let mut deadline_bytes = [0u8; 4];
        deadline_bytes.copy_from_slice(&args[8..12]);
        u32::from_be_bytes(deadline_bytes)
    }

    fn extract_repay_permit_deadline(args: [u8; 32]) -> u32 {
        let mut deadline_bytes = [0u8; 4];
        deadline_bytes.copy_from_slice(&args[9..13]);
        u32::from_be_bytes(deadline_bytes)
    }

    fn decode_liquidation_call_params(
        args1: [u8; 32],
        args2: [u8; 32],
    ) -> (u16, u16, Address, u128, bool) {
        let (lower1, _upper1) = Self::split_bytes32(args1);
        let (lower2, upper2) = Self::split_bytes32(args2);

        let collateral_asset_id = (lower1 & 0xFFFF) as u16;
        let debt_asset_id = ((lower1 >> 16) & 0xFFFF) as u16;

        let mut user_addr_bytes = [0u8; 20];
        user_addr_bytes.copy_from_slice(&args1[8..28]);
        let user = Address::from_slice(&user_addr_bytes);

        let debt_to_cover = lower2;
        let receive_atoken = ((upper2 >> 0) & 0x1) == 1;

        (
            collateral_asset_id,
            debt_asset_id,
            user,
            debt_to_cover,
            receive_atoken,
        )
    }

    /// Maps L2Pool asset IDs to contract addresses for supported chains
    fn get_asset_from_id(
        asset_id: u16,
        chain_id: u64,
        _registry: Option<&ContractRegistry>,
    ) -> Option<Address> {
        if chain_id == 42161 {
            return match asset_id {
                0 => Some("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1".parse().ok()?), // WETH
                1 => Some("0x2f2a2543B76A4166549F7aaB2e75Bef0aefC5B0f".parse().ok()?), // WBTC
                2 => Some("0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9".parse().ok()?), // USDT
                3 => Some("0xFF970A61A04b1cA14834A43f5dE4533eBDDB5CC8".parse().ok()?), // USDC.e (bridged)
                4 => Some("0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1".parse().ok()?), // DAI
                5 => Some("0xf97f4df75117a78c1A5a0DBb814Af92458539FB4".parse().ok()?), // LINK
                6 => Some("0xFa7F8980b0f1E64A2062791cc3b0871572f1F7f0".parse().ok()?), // UNI
                7 => Some("0x912CE59144191C1204E64559FE8253a0e49E6548".parse().ok()?), // ARB
                8 => Some("0x3082CC23568eA640225c2467653dB90e9250AaA0".parse().ok()?), // RDNT
                9 => Some("0x6694340fc020c5E6B96567843da2df01b2CE1eb6".parse().ok()?), // STG
                10 => Some("0x17FC002b466eEc40DaE837Fc4bE5c67993ddBd6F".parse().ok()?), // FRAX
                11 => Some("0xd22a58f79e9481D1a88e00c343885A588b34b68B".parse().ok()?), // EURS
                12 => Some("0xaf88d065e77c8cC2239327C5EDb3A432268e5831".parse().ok()?), // USDC (native)
                13 => Some("0x93b346b6BC2548dA6A1E7d98E9a421B42541425b".parse().ok()?), // LUSD
                14 => Some("0x5979D7b546E38E414F7E9822514be443A4800529".parse().ok()?), // wstETH
                15 => Some("0x35751007a407ca6FEFfE80b3cB397736D2cf4dbe".parse().ok()?), // weETH
                16 => Some("0x1a7e4e63778B4f12a199C062f3eFdD288afCBce8".parse().ok()?), // agEUR
                17 => Some("0xaf88d065e77c8cC2239327C5EDb3A432268e5831".parse().ok()?), // MAI
                _ => None,
            };
        }

        if chain_id == 10 {
            return match asset_id {
                0 => Some("0x4200000000000000000000000000000000000006".parse().ok()?), // WETH
                1 => Some("0x68f180fcCe6836688e9084f035309E29Bf0A2095".parse().ok()?), // WBTC
                2 => Some("0x94b008aA00579c1307B0EF2c499aD98a8ce58e58".parse().ok()?), // USDT
                3 => Some("0x7F5c764cBc14f9669B88837ca1490cCa17c31607".parse().ok()?), // USDC.e (bridged)
                4 => Some("0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1".parse().ok()?), // DAI
                5 => Some("0x350a791Bfc2C21F9Ed5d10980Dad2e2638ffa7f6".parse().ok()?), // LINK
                6 => Some("0x6fd9d7AD17242c41f7131d257212c54A0e816691".parse().ok()?), // UNI
                7 => Some("0x76FB31fb4af56892A25e32cFC43De717950c9278".parse().ok()?), // AAVE
                8 => Some("0x9Bcef72be871e61ED4fBbc7630889beE758eb81D".parse().ok()?), // rETH
                9 => Some("0x1F32b1c2345538c0c6f582fCB022739c4A194Ebb".parse().ok()?), // wstETH
                10 => Some("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85".parse().ok()?), // USDC (native)
                11 => Some("0x8700dAec35aF8Ff88c16BdF0418774CB3D7599B4".parse().ok()?), // SNX
                _ => None,
            };
        }

        if chain_id == 137 {
            return match asset_id {
                0 => Some("0x7ceB23fD6bC0adD59E62ac25578270cFf1b9f619".parse().ok()?), // WETH
                1 => Some("0x1BFD67037B42Cf73acF2047067bd4F2C47D9BfD6".parse().ok()?), // WBTC
                2 => Some("0xc2132D05D31c914a87C6611C10748AEb04B58e8F".parse().ok()?), // USDT
                3 => Some("0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".parse().ok()?), // USDC.e (bridged)
                4 => Some("0x8f3Cf7ad23Cd3CaDbD9735AFf958023239c6A063".parse().ok()?), // DAI
                5 => Some("0x53E0bca35eC356BD5ddDFebbD1Fc0fD03FaBad39".parse().ok()?), // LINK
                6 => Some("0xb33EaAd8d922B1083446DC23f610c2567fB5180f".parse().ok()?), // UNI
                7 => Some("0xD6DF932A45C0f255f85145f286eA0b292B21C90B".parse().ok()?), // AAVE
                8 => Some("0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270".parse().ok()?), // WMATIC
                9 => Some("0x385Eeac5cB85A38A9a07A70c73e0a3271CfB54A7".parse().ok()?), // GHST
                10 => Some("0x172370d5Cd63279eFa6d502DAB29171933a610AF".parse().ok()?), // CRV
                11 => Some("0x0b3F868E0BE5597D5DB7fEB59E1CADBb0fdDa50a".parse().ok()?), // SUSHI
                12 => Some("0x3A58a54C066FdC0f2D55FC9C89F0415C92eBf3C4".parse().ok()?), // stMATIC
                13 => Some("0x03b54A6e9a984069379fae1a4fC4dBAE93B3bCCD".parse().ok()?), // wstETH
                14 => Some("0xfa68FB4628DFF1028CFEc22b4162FCcd0d45efb6".parse().ok()?), // MaticX
                15 => Some("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359".parse().ok()?), // USDC (native)
                _ => None,
            };
        }

        None
    }

    /// Decodes L2Pool supply operation
    fn decode_l2_supply(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::supplyCall::abi_decode(input).ok()?;

        let (asset_id, amount, _referral_code) = Self::decode_supply_params(call.args.0);
        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let summary = format!("Supply {} {}", amount_str, token_symbol);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Supply".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Supply".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool withdraw operation
    fn decode_l2_withdraw(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::withdrawCall::abi_decode(input).ok()?;

        let (asset_id, amount) = Self::decode_withdraw_params(call.args.0);
        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let is_max = amount == u128::MAX;
        let amount_display = if is_max {
            "Maximum available".to_string()
        } else {
            amount_str.clone()
        };

        let summary = format!("Withdraw {} {}", amount_display, token_symbol);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_display, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Maximum available (type(uint128).max)")
                        } else {
                            format!("{} {} (raw: {})", amount_str, token_symbol, amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Withdraw".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Withdraw".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool borrow operation
    fn decode_l2_borrow(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::borrowCall::abi_decode(input).ok()?;

        let (asset_id, amount, interest_rate_mode, _referral_code) =
            Self::decode_borrow_params(call.args.0);

        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let rate_mode = match interest_rate_mode {
            2 => "Variable",
            1 => "Stable (Deprecated)",
            mode => {
                return Some(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Unknown rate mode: {}", mode),
                        label: "Error".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("Invalid interest rate mode: {}", mode),
                    },
                });
            }
        };

        let summary = format!(
            "Borrow {} {} at {} rate",
            amount_str, token_symbol, rate_mode
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, interest_rate_mode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Borrow".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Borrow".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool repay operation
    fn decode_l2_repay(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::repayCall::abi_decode(input).ok()?;

        let (asset_id, amount, interest_rate_mode) = Self::decode_repay_params(call.args.0);
        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let is_max = amount == u128::MAX;
        let amount_display = if is_max {
            "Full debt".to_string()
        } else {
            amount_str.clone()
        };

        let rate_mode = match interest_rate_mode {
            2 => "Variable",
            1 => "Stable (Deprecated)",
            mode => {
                return Some(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Unknown rate mode: {}", mode),
                        label: "Error".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("Invalid interest rate mode: {}", mode),
                    },
                });
            }
        };

        let summary = format!(
            "Repay {} {} at {} rate",
            amount_display, token_symbol, rate_mode
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_display, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full debt (type(uint128).max)")
                        } else {
                            format!("{} {} (raw: {})", amount_str, token_symbol, amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, interest_rate_mode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Repay".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Repay".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool repayWithATokens operation
    fn decode_l2_repay_with_atokens(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::repayWithATokensCall::abi_decode(input).ok()?;

        let (asset_id, amount, interest_rate_mode) = Self::decode_repay_params(call.args.0);
        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let is_max = amount == u128::MAX;
        let amount_display = if is_max {
            "Full balance".to_string()
        } else {
            amount_str.clone()
        };

        let rate_mode = match interest_rate_mode {
            2 => "Variable",
            1 => "Stable (Deprecated)",
            mode => {
                return Some(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Unknown rate mode: {}", mode),
                        label: "Error".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("Invalid interest rate mode: {}", mode),
                    },
                });
            }
        };

        let summary = format!(
            "Repay {} {} using aTokens at {} rate",
            amount_display, token_symbol, rate_mode
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_display, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full balance (type(uint128).max)")
                        } else {
                            format!("{} {} (raw: {})", amount_str, token_symbol, amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, interest_rate_mode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Using aTokens".to_string(),
                        label: "Repayment Method".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Repaying with aTokens (no transfer required)".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Repay with aTokens".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Repay with aTokens".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool liquidation call operation
    fn decode_l2_liquidation_call(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::liquidationCallCall::abi_decode(input).ok()?;

        let (collateral_asset_id, debt_asset_id, user, debt_to_cover, receive_atoken) =
            Self::decode_liquidation_call_params(call.args1.0, call.args2.0);

        let collateral_address = Self::get_asset_from_id(collateral_asset_id, chain_id, registry);
        let debt_address = Self::get_asset_from_id(debt_asset_id, chain_id, registry);

        let (debt_amount_str, debt_symbol) = if let Some(addr) = debt_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, debt_to_cover))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (debt_to_cover.to_string(), symbol.clone()));

            formatted
        } else {
            (
                debt_to_cover.to_string(),
                format!("Asset#{}", debt_asset_id),
            )
        };

        let collateral_symbol = if let Some(addr) = collateral_address {
            registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr))
        } else {
            format!("Asset#{}", collateral_asset_id)
        };

        let is_max = debt_to_cover == u128::MAX;
        let amount_display = if is_max {
            "Full debt".to_string()
        } else {
            debt_amount_str.clone()
        };

        let summary = format!(
            "Liquidate {} {} debt, seize {} collateral{}",
            amount_display,
            debt_symbol,
            collateral_symbol,
            if receive_atoken {
                " (receive aTokens)"
            } else {
                ""
            }
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{:?}", user),
                        label: "User".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{:?}", user),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", debt_asset_id),
                        label: "Debt Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", debt_symbol, debt_asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_display, debt_symbol),
                        label: "Debt to Cover".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full debt (type(uint128).max)")
                        } else {
                            format!(
                                "{} {} (raw: {})",
                                debt_amount_str, debt_symbol, debt_to_cover
                            )
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", collateral_asset_id),
                        label: "Collateral Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", collateral_symbol, collateral_asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: if receive_atoken {
                            "Receive aTokens"
                        } else {
                            "Receive underlying"
                        }
                        .to_string(),
                        label: "Liquidation Bonus".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if receive_atoken {
                            "Receive aTokens (no transfer)"
                        } else {
                            "Receive underlying asset"
                        }
                        .to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Liquidation".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Liquidation Call".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool setUserUseReserveAsCollateral operation
    fn decode_l2_set_user_use_reserve_as_collateral(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::setUserUseReserveAsCollateralCall::abi_decode(input).ok()?;

        let (asset_id, use_as_collateral) =
            Self::decode_set_user_use_reserve_as_collateral_params(call.args.0);

        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let token_symbol = if let Some(addr) = asset_address {
            registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr))
        } else {
            format!("Asset#{}", asset_id)
        };

        let action = if use_as_collateral {
            "Enable"
        } else {
            "Disable"
        };

        let summary = format!("{} {} as collateral", action, token_symbol);

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: action.to_string(),
                        label: "Action".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if use_as_collateral {
                            "Enable as collateral (allows borrowing)"
                        } else {
                            "Disable as collateral (reduces borrowing power)"
                        }
                        .to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Collateral Setting".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Set Collateral".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool supplyWithPermit operation
    fn decode_l2_supply_with_permit(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::supplyWithPermitCall::abi_decode(input).ok()?;

        let (asset_id, amount, _referral_code) = Self::decode_supply_params(call.args.0);
        let deadline = Self::extract_supply_permit_deadline(call.args.0);

        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let deadline_str = if deadline == u32::MAX {
            "never".to_string()
        } else {
            let dt = Utc.timestamp_opt(deadline as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

        let summary = format!(
            "Supply {} {} with permit (expires: {})",
            amount_str, token_symbol, deadline_str
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_str, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} {} (raw: {})", amount_str, token_symbol, amount),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "ERC-2612 Permit".to_string(),
                        label: "Authorization".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Using gasless ERC-2612 permit signature".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Supply with Permit".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Supply with Permit".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }

    /// Decodes L2Pool repayWithPermit operation
    fn decode_l2_repay_with_permit(
        input: &[u8],
        chain_id: u64,
        registry: Option<&ContractRegistry>,
    ) -> Option<SignablePayloadField> {
        let call = IL2Pool::repayWithPermitCall::abi_decode(input).ok()?;

        let (asset_id, amount, interest_rate_mode) = Self::decode_repay_params(call.args.0);
        let deadline = Self::extract_repay_permit_deadline(call.args.0);

        let asset_address = Self::get_asset_from_id(asset_id, chain_id, registry);

        let (amount_str, token_symbol) = if let Some(addr) = asset_address {
            let symbol = registry
                .and_then(|r| r.get_token_symbol(chain_id, addr))
                .unwrap_or_else(|| format!("{:?}", addr));

            let formatted = registry
                .and_then(|r| r.format_token_amount(chain_id, addr, amount))
                .map(|(amt, sym)| (amt, sym))
                .unwrap_or_else(|| (amount.to_string(), symbol.clone()));

            formatted
        } else {
            (amount.to_string(), format!("Asset#{}", asset_id))
        };

        let is_max = amount == u128::MAX;
        let amount_display = if is_max {
            "Full balance".to_string()
        } else {
            amount_str.clone()
        };

        let rate_mode = match interest_rate_mode {
            2 => "Variable",
            1 => "Stable (Deprecated)",
            mode => {
                return Some(SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Unknown rate mode: {}", mode),
                        label: "Error".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("Invalid interest rate mode: {}", mode),
                    },
                });
            }
        };

        let deadline_str = if deadline == u32::MAX {
            "never".to_string()
        } else {
            let dt = Utc.timestamp_opt(deadline as i64, 0).unwrap();
            dt.format("%Y-%m-%d %H:%M UTC").to_string()
        };

        let summary = format!(
            "Repay {} {} at {} rate with permit (expires: {})",
            amount_display, token_symbol, rate_mode, deadline_str
        );

        let fields = vec![
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("Asset ID: {}", asset_id),
                        label: "Asset".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} (ID: {})", token_symbol, asset_id),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: format!("{} {}", amount_display, token_symbol),
                        label: "Amount".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: if is_max {
                            format!("Full balance (type(uint128).max)")
                        } else {
                            format!("{} {} (raw: {})", amount_str, token_symbol, amount)
                        },
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: rate_mode.to_string(),
                        label: "Interest Rate Mode".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: format!("{} ({})", rate_mode, interest_rate_mode),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
            AnnotatedPayloadField {
                signable_payload_field: SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "ERC-2612 Permit".to_string(),
                        label: "Authorization".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Using gasless ERC-2612 permit signature".to_string(),
                    },
                },
                static_annotation: None,
                dynamic_annotation: None,
            },
        ];

        Some(SignablePayloadField::PreviewLayout {
            common: SignablePayloadFieldCommon {
                fallback_text: summary.clone(),
                label: "Aave L2 Repay with Permit".to_string(),
            },
            preview_layout: SignablePayloadFieldPreviewLayout {
                title: Some(SignablePayloadFieldTextV2 {
                    text: "Aave v3 L2 Repay with Permit".to_string(),
                }),
                subtitle: Some(SignablePayloadFieldTextV2 { text: summary }),
                condensed: None,
                expanded: Some(SignablePayloadFieldListLayout { fields }),
            },
        })
    }
}
