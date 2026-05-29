// TODO(#231): Remove these exemptions once Ethereum violations are fixed in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::sync::Arc;

use crate::fmt::{format_ether, format_gwei};
use crate::registry::ContractType;
use crate::visualizer::CalldataVisualizer;
use alloy_consensus::{Transaction as _, TxEnvelope, TxType, TypedTransaction};
use alloy_rlp::{Buf, Decodable};
use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use visualsign::{
    SignablePayload, SignablePayloadField, SignablePayloadFieldAddressV2,
    SignablePayloadFieldAmountV2, SignablePayloadFieldCommon, SignablePayloadFieldTextV2,
    encodings::SupportedEncodings,
    registry::LayeredRegistry,
    vsptrait::{
        DeveloperConfig, Transaction, TransactionParseError, VisualSignConverter,
        VisualSignConverterFromString, VisualSignError, VisualSignOptions,
    },
};

pub mod abi_decoder;
pub mod abi_metadata;
pub mod abi_registry;
pub mod context;
pub mod contracts;
pub mod embedded_abis;
pub(crate) mod eth_json;
pub mod fmt;
pub mod networks;
pub mod protocols;
pub mod registry;
pub mod token_metadata;
pub mod visualizer;

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum EthereumParserError {
    #[error("Unexpected trailing data: {0}")]
    UnexpectedTrailingData(String),
    #[error("Unexpected transaction type: {0}")]
    UnexpectedTransactionType(String),
    #[error("Unsupported transaction type: {0}")]
    UnsupportedTransactionType(String),
    #[error("Failed to decode transaction: {0}")]
    FailedToDecodeTransaction(String),
    #[error("Failed to parse JSON transaction: {0}")]
    FailedToParseJsonTransaction(String),
}

// Helper function to extract gas price from different transaction types
fn extract_gas_price(transaction: &TypedTransaction) -> u128 {
    match transaction {
        TypedTransaction::Legacy(tx) => tx.gas_price,
        TypedTransaction::Eip2930(tx) => tx.gas_price,
        TypedTransaction::Eip1559(tx) => tx.max_fee_per_gas,
        TypedTransaction::Eip4844(tx) => match tx {
            alloy_consensus::TxEip4844Variant::TxEip4844(inner_tx) => inner_tx.max_fee_per_gas,
            alloy_consensus::TxEip4844Variant::TxEip4844WithSidecar(sidecar_tx) => {
                sidecar_tx.tx.max_fee_per_gas
            }
        },
        TypedTransaction::Eip7702(tx) => tx.max_fee_per_gas,
    }
}

// Helper function to extract priority fee from transaction types that support it
fn extract_priority_fee(transaction: &TypedTransaction) -> Option<u128> {
    match transaction {
        TypedTransaction::Eip1559(tx) => Some(tx.max_priority_fee_per_gas),
        TypedTransaction::Eip4844(tx) => match tx {
            alloy_consensus::TxEip4844Variant::TxEip4844(inner_tx) => {
                Some(inner_tx.max_priority_fee_per_gas)
            }
            alloy_consensus::TxEip4844Variant::TxEip4844WithSidecar(sidecar_tx) => {
                Some(sidecar_tx.tx.max_priority_fee_per_gas)
            }
        },
        TypedTransaction::Eip7702(tx) => Some(tx.max_priority_fee_per_gas),
        TypedTransaction::Legacy(_) | TypedTransaction::Eip2930(_) => None,
    }
}

// Helper function to create priority fee field
fn create_priority_fee_field(max_priority_fee_per_gas: u128) -> SignablePayloadField {
    let priority_fee_text = format!("{} gwei", format_gwei(max_priority_fee_per_gas));
    SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: priority_fee_text.clone(),
            label: "Max Priority Fee Per Gas".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: priority_fee_text,
        },
    }
}

/// Wrapper around Alloy's transaction type that implements the Transaction trait
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EthereumTransactionWrapper {
    transaction: TypedTransaction,
}

impl Transaction for EthereumTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        // Default: don't allow signed transactions (production API behavior)
        Self::from_string_with_options(data, None)
    }
    fn transaction_type(&self) -> String {
        "Ethereum".to_string()
    }
}

impl EthereumTransactionWrapper {
    pub fn new(transaction: TypedTransaction) -> Self {
        Self { transaction }
    }
    pub fn inner(&self) -> &TypedTransaction {
        &self.transaction
    }

    /// Parse transaction from string with developer options.
    /// When developer_config.allow_signed_transactions is true, signed transactions
    /// will be accepted and the unsigned portion extracted for visualization.
    pub fn from_string_with_options(
        data: &str,
        developer_config: Option<&DeveloperConfig>,
    ) -> Result<Self, TransactionParseError> {
        // JSON path: detect and route early.
        // developer_config is intentionally not consulted here — JSON input is always
        // an unsigned transaction structure (no signature fields), so the
        // allow_signed_transactions flag does not apply.
        if eth_json::is_json_input(data) {
            let transaction = eth_json::decode_json_transaction(data)
                .map_err(|e| TransactionParseError::InvalidFormat(e.to_string()))?;
            return Ok(Self { transaction });
        }

        // Existing RLP path
        let format = if data.starts_with("0x") {
            SupportedEncodings::Hex
        } else {
            visualsign::encodings::SupportedEncodings::detect(data)
        };
        let allow_signed = developer_config
            .map(|c| c.allow_signed_transactions)
            .unwrap_or(false);
        let transaction = decode_transaction(data, format, allow_signed)
            .map_err(|e| TransactionParseError::DecodeError(e.to_string()))?;
        Ok(Self { transaction })
    }
}

/// Converter that knows how to format Ethereum transactions for VisualSign.
///
/// Uses `Arc<ContractRegistry>` for efficient sharing of the global registry across requests.
/// Per-request wallet metadata is layered on top using `LayeredRegistry`, which checks
/// the request layer first before falling back to the global registry.
pub struct EthereumVisualSignConverter {
    registry: Arc<registry::ContractRegistry>,
    visualizer_registry: visualizer::EthereumVisualizerRegistry,
}

impl EthereumVisualSignConverter {
    /// Creates a new converter with a custom registry wrapped in Arc.
    pub fn with_registry(registry: Arc<registry::ContractRegistry>) -> Self {
        Self {
            registry,
            visualizer_registry: visualizer::EthereumVisualizerRegistryBuilder::new().build(),
        }
    }

    /// Creates a new converter with a default registry including all known protocols.
    pub fn new() -> Self {
        let (contract_registry, visualizer_builder) =
            registry::ContractRegistry::with_default_protocols();
        Self {
            registry: Arc::new(contract_registry),
            visualizer_registry: visualizer_builder.build(),
        }
    }

    /// Creates a layered registry for the current request.
    ///
    /// The global registry is shared via Arc (O(1) clone). If wallet metadata contains
    /// token information, it's loaded into a request-scoped registry that takes precedence
    /// during lookups. The request registry is dropped after the request completes.
    fn create_layered_registry(
        &self,
        _options: &VisualSignOptions,
    ) -> LayeredRegistry<registry::ContractRegistry> {
        // TODO: When wallet-provided ChainMetadata includes token metadata (not just ABIs),
        // create a request registry and use LayeredRegistry::with_request:
        //
        // if let Some(ref chain_metadata) = options.metadata {
        //     if let Some(chain_metadata::Metadata::Ethereum(eth_metadata)) = &chain_metadata.metadata {
        //         let mut request_registry = registry::ContractRegistry::new();
        //         // Load wallet tokens into request_registry
        //         // request_registry.load_wallet_tokens(&eth_metadata.tokens)?;
        //         return LayeredRegistry::with_request(Arc::clone(&self.registry), request_registry);
        //     }
        // }

        // No wallet metadata, use global registry only
        LayeredRegistry::new(Arc::clone(&self.registry))
    }

    /// Shared conversion logic used by both trait impls.
    ///
    /// ABIs are resolved automatically from `options.metadata.abi_mappings`.
    fn convert_transaction_inner(
        &self,
        transaction: TypedTransaction,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let layered_registry = self.create_layered_registry(&options);

        match transaction.tx_type() {
            TxType::Legacy | TxType::Eip1559 => {}
            unsupported => {
                return Err(VisualSignError::DecodeError(format!(
                    "Unsupported transaction type: {unsupported}"
                )));
            }
        }

        // Resolve chain_id: metadata > transaction > default (1 for legacy).
        let chain_id = resolve_chain_id(&transaction, &options)?;
        let metadata_abi = extract_metadata_abi(&options, chain_id);

        convert_to_visual_sign_payload(
            transaction,
            options,
            chain_id,
            &layered_registry,
            &self.visualizer_registry,
            metadata_abi.as_ref(),
        )
    }
}

impl Default for EthereumVisualSignConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualSignConverter<EthereumTransactionWrapper> for EthereumVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction_wrapper: EthereumTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        self.convert_transaction_inner(transaction_wrapper.inner().clone(), options)
    }
}

impl VisualSignConverterFromString<EthereumTransactionWrapper> for EthereumVisualSignConverter {
    fn to_visual_sign_payload_from_string(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let wrapper = EthereumTransactionWrapper::from_string_with_options(
            transaction_data,
            options.developer_config.as_ref(),
        )
        .map_err(VisualSignError::ParseError)?;
        // Use the validated variant so any wallet-supplied or caller-supplied
        // string that reaches a rendered field is screened for non-ASCII,
        // unicode escapes, and non-printable characters. Matches the default
        // impl every other chain converter inherits, and preserves the
        // invariant that every payload returned by `to_visual_sign_payload_from_string`
        // is charset-validated. Callers that go through `to_visual_sign_payload`
        // or `transaction_to_visual_sign` still bypass this check by design.
        self.to_validated_visual_sign_payload(wrapper, options)
    }
}

/// Extract unsigned transaction from a signed transaction envelope.
/// Only for developer tools - production API should only accept unsigned transactions.
fn extract_unsigned_from_envelope(
    envelope: TxEnvelope,
) -> Result<TypedTransaction, EthereumParserError> {
    match envelope {
        TxEnvelope::Legacy(signed) => Ok(TypedTransaction::Legacy(signed.tx().clone())),
        TxEnvelope::Eip1559(signed) => Ok(TypedTransaction::Eip1559(signed.tx().clone())),
        TxEnvelope::Eip2930(_) => Err(EthereumParserError::UnsupportedTransactionType(
            "eip-2930".to_string(),
        )),
        TxEnvelope::Eip4844(_) => Err(EthereumParserError::UnsupportedTransactionType(
            "eip-4844".to_string(),
        )),
        TxEnvelope::Eip7702(_) => Err(EthereumParserError::UnsupportedTransactionType(
            "eip-7702".to_string(),
        )),
    }
}

/// Decode transaction bytes, optionally allowing signed transactions (developer mode only).
fn decode_transaction_bytes(
    buf: &[u8],
    allow_signed: bool,
) -> Result<TypedTransaction, EthereumParserError> {
    // First try unsigned decoding
    match decode_unsigned_transaction_bytes(buf) {
        Ok(tx) => Ok(tx),
        Err(unsigned_err) => {
            // If developer mode enabled, try signed transaction decoding
            if allow_signed {
                match TxEnvelope::decode(&mut &buf[..]) {
                    Ok(envelope) => {
                        log::info!(
                            "Detected signed transaction, extracting unsigned portion for visualization"
                        );
                        extract_unsigned_from_envelope(envelope)
                    }
                    Err(_) => Err(unsigned_err),
                }
            } else {
                Err(unsigned_err)
            }
        }
    }
}

/// Decode unsigned transaction bytes only (standard API path).
fn decode_unsigned_transaction_bytes(
    mut buf: &[u8],
) -> Result<TypedTransaction, EthereumParserError> {
    let tx = if buf.is_empty() {
        Err(EthereumParserError::FailedToDecodeTransaction(
            "Input too short".to_string(),
        ))
    } else if buf[0] == 0 || (buf[0] > 0x7f && buf[0] < 0xc0) {
        Err(EthereumParserError::FailedToDecodeTransaction(format!(
            "Unexpected type flag {}.",
            buf[0]
        )))
    } else if buf[0] <= 0x7f {
        let ty: TxType = match buf[0].try_into() {
            Ok(t) => t,
            Err(e) => {
                return Err(EthereumParserError::FailedToDecodeTransaction(
                    e.to_string(),
                ));
            }
        };
        buf.advance(1); // Skip type byte
        match ty {
            TxType::Eip1559 => Ok(TypedTransaction::Eip1559(
                alloy_consensus::TxEip1559::decode(&mut buf)
                    .map_err(|e| EthereumParserError::FailedToDecodeTransaction(e.to_string()))?,
            )),
            TxType::Eip2930 => Err(EthereumParserError::UnsupportedTransactionType(
                "eip-2930".to_string(),
            )),
            TxType::Eip4844 => Err(EthereumParserError::UnsupportedTransactionType(
                "eip-4844".to_string(),
            )),
            TxType::Eip7702 => Err(EthereumParserError::UnsupportedTransactionType(
                "eip-7702".to_string(),
            )),
            TxType::Legacy => Err(EthereumParserError::UnexpectedTransactionType(
                "legacy".to_string(), // This shouldn't happen
            )),
        }
    } else {
        Ok(TypedTransaction::Legacy(
            alloy_consensus::TxLegacy::decode(&mut buf)
                .map_err(|e| EthereumParserError::FailedToDecodeTransaction(e.to_string()))?,
        ))
    };
    if tx.is_ok() && !buf.is_empty() {
        return Err(EthereumParserError::UnexpectedTrailingData(hex::encode(
            buf,
        )));
    }
    tx
}

fn decode_transaction(
    raw_transaction: &str,
    encodings: SupportedEncodings,
    allow_signed: bool,
) -> Result<TypedTransaction, EthereumParserError> {
    let bytes = match encodings {
        SupportedEncodings::Hex => {
            let clean_hex = raw_transaction
                .strip_prefix("0x")
                .unwrap_or(raw_transaction);
            hex::decode(clean_hex).map_err(|e| {
                EthereumParserError::FailedToDecodeTransaction(format!("Failed to decode hex: {e}"))
            })?
        }
        SupportedEncodings::Base64 => b64.decode(raw_transaction).map_err(|e| {
            EthereumParserError::FailedToDecodeTransaction(format!("Failed to decode base64: {e}"))
        })?,
    };
    decode_transaction_bytes(&bytes, allow_signed)
}

/// Resolve chain ID. The chain_id encoded in the transaction bytes is
/// authoritative: if `chain_metadata.network_id` is also provided and resolves
/// to a different chain, we refuse to render so the payload can never bind a
/// "Network: X" label to bytes that execute on chain Y.
///
/// Order of precedence:
/// 1. If the transaction carries a chain_id, that wins. Metadata may agree
///    (no-op) but a mismatch is a hard error.
/// 2. Otherwise (pre-EIP-155 legacy txs that omit chain_id), fall back to
///    metadata if supplied.
/// 3. Otherwise, default to 1 for legacy txs (historical behavior).
/// 4. Otherwise, return an error.
fn resolve_chain_id(
    transaction: &TypedTransaction,
    options: &VisualSignOptions,
) -> Result<u64, VisualSignError> {
    let metadata_chain_id = networks::extract_chain_id_from_metadata(options.metadata.as_ref());

    match (transaction.chain_id(), metadata_chain_id) {
        (Some(tx_chain_id), Some(meta_chain_id)) if tx_chain_id != meta_chain_id => {
            Err(VisualSignError::ValidationError(format!(
                "chain_id mismatch: transaction bytes declare chain_id {tx_chain_id} but chain_metadata.network_id resolves to chain_id {meta_chain_id}. The transaction bytes are authoritative; refusing to produce a payload."
            )))
        }
        (Some(tx_chain_id), _) => Ok(tx_chain_id),
        (None, Some(meta_chain_id)) => Ok(meta_chain_id),
        (None, None) if matches!(transaction, TypedTransaction::Legacy(_)) => Ok(1),
        (None, None) => Err(VisualSignError::DecodeError(
            "Unable to determine chain_id: no metadata provided and transaction does not contain chain_id".to_string()
        )),
    }
}

/// Extract ABI from wallet-provided metadata with graceful degradation.
fn extract_metadata_abi(
    options: &VisualSignOptions,
    chain_id: u64,
) -> Option<abi_registry::AbiRegistry> {
    abi_metadata::try_extract_from_chain_metadata(options.metadata.as_ref(), chain_id)
}

/// Known-token short-circuit: if the destination is a token registered in the
/// compiled-in `ContractRegistry` (e.g. USDC, USDT, WETH), route to the safe
/// built-in ERC20/ERC721 visualizer and lock out any caller-supplied ABI.
/// This prevents a malicious dApp from supplying an ABI for a canonical token
/// address with attacker-chosen parameter labels and spoofing the signing UI
/// (the selector still has to match `transfer(address,uint256)` for the
/// dispatcher to bind it, but parameter names are unconstrained).
///
/// We deliberately consult only the global (compiled-in) layer here: a
/// request-scoped registry is itself caller-supplied and cannot be trusted to
/// decide the override.
///
/// The lookup keys off `transaction.chain_id()` and never the resolved
/// `chain_id`. `resolve_chain_id` gives metadata priority and metadata is
/// caller-controlled in this threat model, so feeding it into a security
/// decision would let an attacker mismatch `network_id` and dodge the
/// canonical-token lookup. For pre-EIP-155 legacy txs that don't carry a chain
/// id of their own we fall back to an address-only any-chain lookup rather
/// than the metadata-derived chain id: any canonical-token address on any
/// chain still wins over a caller-supplied ABI.
///
/// Returns `Some(vec![field])` (always non-empty) if the destination is a
/// registered canonical token. The field is either the decoded result or a
/// raw-hex fallback when the built-in visualizer can't decode the selector
/// (ERC721 stub returns `None` for all inputs today; ERC1155 has no built-in
/// visualizer yet). Returning `Some` even with a raw-hex field is what gives
/// callers a clean "if `Some`, you're done" contract: the downstream
/// caller-ABI path and ERC20 `decode_transfers` fallback are both gated on
/// `input_fields.is_empty()`, so populating any field locks them out. This
/// matters because `approve(address,uint256)` and `transfer(address,uint256)`
/// share selectors across ERC standards; without the lock-out a known ERC721
/// or ERC1155 call would be mis-rendered as an ERC20 op.
///
/// Returns `None` only when the destination is not a registered canonical
/// token.
fn try_known_token_dispatch(
    layered_registry: &LayeredRegistry<registry::ContractRegistry>,
    chain_id: Option<registry::ChainId>,
    to_address: alloy_primitives::Address,
    input: &[u8],
) -> Option<Vec<SignablePayloadField>> {
    let erc_standard = layered_registry
        .global()
        .get_token_erc_standard(chain_id, to_address)?;
    let decoded = match erc_standard {
        token_metadata::ErcStandard::Erc20 => {
            (contracts::core::ERC20Visualizer {}).visualize_tx_commands(input)
        }
        // `ERC721Visualizer::visualize_tx_commands` currently returns `None`
        // for all inputs (no built-in ERC721 decoder yet). A follow-up can add
        // minimal transferFrom/safeTransferFrom decoding for UX.
        token_metadata::ErcStandard::Erc721 => {
            (contracts::core::ERC721Visualizer {}).visualize_tx_commands(input)
        }
        // No built-in ERC1155 visualizer yet.
        token_metadata::ErcStandard::Erc1155 => None,
    };
    let field =
        decoded.unwrap_or_else(|| contracts::core::FallbackVisualizer::new().visualize_hex(input));
    Some(vec![field])
}

/// Decode calldata using a caller-supplied ABI registry, resolving proxy
/// destinations to their implementation ABI.
///
/// For a `Proxy` destination, the calldata is decoded against the linked
/// implementation's ABI (with an `Implementation` address field prepended so the
/// signer sees where decoding came from). If the implementation ABI is missing or
/// can't decode the selector, it falls back to the proxy's own ABI. For
/// implementation/unspecified destinations this decodes against the ABI mapped to
/// the address, exactly as before.
///
/// The caller gates this on `input_fields.is_empty()` after the known-token
/// short-circuit, so canonical tokens are never reached here.
fn visualize_with_abi_registry(
    abi_reg: &abi_registry::AbiRegistry,
    chain_id: u64,
    to: alloy_primitives::Address,
    input: &[u8],
) -> Vec<SignablePayloadField> {
    use contracts::core::DynamicAbiVisualizer;

    let decode = |abi| DynamicAbiVisualizer::new(abi).visualize_calldata(input, chain_id, None);

    // For proxy destinations prefer the linked implementation ABI: the calldata
    // selector belongs to the implementation, not the proxy. Three paths:
    //   1. Impl ABI present and decodes selector → [implementation_address_field, decoded_field]
    //   2. Impl ABI present but selector not found → [unresolved_implementation_field]
    //      + best-effort proxy-own-ABI decode (or raw hex if that also misses)
    //   3. No impl ABI linked → fall through to proxy's own ABI (non-proxy path below)
    if abi_reg.get_abi_kind(chain_id, to) == Some(abi_registry::AbiKind::Proxy) {
        if let Some((impl_addr, impl_abi)) = abi_reg.get_implementation_abi(chain_id, to) {
            if let Some(field) = decode(impl_abi) {
                return vec![implementation_address_field(impl_addr), field];
            }
            // impl ABI present but selector not found — still surface the implementation
            // address (as unresolved) and attempt proxy's own ABI as the decode fallback.
            // If neither ABI matches, append raw hex so the signer always sees the
            // calldata bytes even when the function is unrecognized.
            let mut fields = vec![unresolved_implementation_field(impl_addr)];
            if let Some(field) = abi_reg.get_abi_for_address(chain_id, to).and_then(decode) {
                fields.push(field);
            } else {
                fields.push(contracts::core::FallbackVisualizer::new().visualize_hex(input));
            }
            return fields;
        }
    }

    // Non-proxy destinations and proxy fallback without a linked implementation address.
    abi_reg
        .get_abi_for_address(chain_id, to)
        .and_then(decode)
        .into_iter()
        .collect()
}

/// Builds an informational `Implementation` address field shown when a proxy call
/// was decoded against its implementation ABI.
fn implementation_address_field(implementation: alloy_primitives::Address) -> SignablePayloadField {
    SignablePayloadField::AddressV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: implementation.to_string(),
            label: "Implementation".to_string(),
        },
        address_v2: SignablePayloadFieldAddressV2 {
            address: implementation.to_string(),
            name: "Implementation".to_string(),
            asset_label: String::new(),
            memo: None,
            badge_text: Some("Proxy implementation".to_string()),
        },
    }
}

/// Builds an `Implementation` address field for the case where the implementation
/// ABI was found but could not decode the call selector.
fn unresolved_implementation_field(
    implementation: alloy_primitives::Address,
) -> SignablePayloadField {
    SignablePayloadField::AddressV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: implementation.to_string(),
            label: "Implementation".to_string(),
        },
        address_v2: SignablePayloadFieldAddressV2 {
            address: implementation.to_string(),
            name: "Implementation".to_string(),
            asset_label: String::new(),
            memo: None,
            badge_text: Some("Proxy implementation (unresolved)".to_string()),
        },
    }
}

fn convert_to_visual_sign_payload(
    transaction: TypedTransaction,
    options: VisualSignOptions,
    chain_id: u64,
    layered_registry: &LayeredRegistry<registry::ContractRegistry>,
    visualizer_registry: &visualizer::EthereumVisualizerRegistry,
    abi_registry: Option<&abi_registry::AbiRegistry>,
) -> Result<SignablePayload, VisualSignError> {
    let network_name = networks::get_network_name(Some(chain_id));
    let fee_symbol = networks::get_fee_paying_asset_symbol(chain_id);

    let mut fields = vec![SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: network_name.clone(),
            label: "Network".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 { text: network_name },
    }];
    if let Some(to) = transaction.to() {
        // Flag proxy destinations so the signer can see the call goes through a
        // proxy. The kind is caller-supplied (unauthenticated) metadata, so this
        // is purely informational.
        let badge_text = match abi_registry {
            Some(reg) if reg.get_abi_kind(chain_id, to) == Some(abi_registry::AbiKind::Proxy) => {
                Some("Proxy".to_string())
            }
            _ => None,
        };
        fields.push(SignablePayloadField::AddressV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: to.to_string(),
                label: "To".to_string(),
            },
            address_v2: SignablePayloadFieldAddressV2 {
                address: to.to_string(),
                name: "To".to_string(),
                asset_label: fee_symbol.unwrap_or_default().to_string(),
                memo: None,
                badge_text,
            },
        });
    }
    let value = format_ether(transaction.value());
    fields.extend([
        SignablePayloadField::AmountV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: fee_symbol.map_or_else(|| value.clone(), |s| format!("{value} {s}")),
                label: "Value".to_string(),
            },
            amount_v2: SignablePayloadFieldAmountV2 {
                amount: value,
                abbreviation: fee_symbol.map(|s| s.to_string()),
            },
        },
        SignablePayloadField::TextV2 {
            common: SignablePayloadFieldCommon {
                fallback_text: format!("{}", transaction.gas_limit()),
                label: "Gas Limit".to_string(),
            },
            text_v2: SignablePayloadFieldTextV2 {
                text: format!("{}", transaction.gas_limit()),
            },
        },
    ]);

    // Handle gas pricing based on transaction type
    let gas_price_text = format!("{} gwei", format_gwei(extract_gas_price(&transaction)));

    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: gas_price_text.clone(),
            label: "Gas Price".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: gas_price_text,
        },
    });

    // Add priority fee for EIP-1559, EIP-4844, and EIP-7702 transactions
    if let Some(priority_fee) = extract_priority_fee(&transaction) {
        fields.push(create_priority_fee_field(priority_fee));
    }

    fields.push(SignablePayloadField::TextV2 {
        common: SignablePayloadFieldCommon {
            fallback_text: format!("{}", transaction.nonce()),
            label: "Nonce".to_string(),
        },
        text_v2: SignablePayloadFieldTextV2 {
            text: format!("{}", transaction.nonce()),
        },
    });

    // Add contract call data if present
    let input = transaction.input();
    if !input.is_empty() {
        let mut input_fields: Vec<SignablePayloadField> = Vec::new();

        // Try to visualize using the registered visualizers
        let chain_id_val = chain_id;
        if let Some(to_address) = transaction.to() {
            if let Some(contract_type) =
                layered_registry.lookup(|r| r.get_contract_type(chain_id_val, to_address))
            {
                if visualizer_registry.get(&contract_type).is_some() {
                    // Check if this is a Universal Router contract and visualize it
                    if contract_type
                        == crate::protocols::uniswap::config::UniswapUniversalRouter::short_type_id(
                        )
                    {
                        if let Some(field) = (protocols::uniswap::UniversalRouterVisualizer {})
                            .visualize_tx_commands(
                                input,
                                chain_id_val,
                                Some(layered_registry.global()),
                            )
                        {
                            input_fields.push(field);
                        }
                    }
                    // Check if this is a Permit2 contract and visualize it
                    else if contract_type
                        == crate::protocols::uniswap::config::Permit2Contract::short_type_id()
                    {
                        if let Some(field) = (protocols::uniswap::Permit2Visualizer)
                            .visualize_tx_commands(
                                input,
                                chain_id_val,
                                Some(layered_registry.global()),
                            )
                        {
                            input_fields.push(field);
                        }
                    }
                }
            }
        }

        // Known-token short-circuit: see `try_known_token_dispatch` for the
        // full rationale (global-only consultation, `transaction.chain_id()`
        // vs `resolve_chain_id`, pre-EIP-155 any-chain fallback, selector
        // collisions across ERC standards). The helper guarantees a non-empty
        // `Some` when it fires, so the `extend` flips `input_fields.is_empty()`
        // to false and the caller-ABI path and ERC20 `decode_transfers`
        // fallback below both skip on their existing `is_empty` gates.
        if input_fields.is_empty() {
            if let Some(to_address) = transaction.to() {
                if let Some(known_fields) = try_known_token_dispatch(
                    layered_registry,
                    transaction.chain_id(),
                    to_address,
                    input,
                ) {
                    input_fields.extend(known_fields);
                }
            }
        }

        // Try dynamic ABI visualization if available. Skipped for known tokens
        // (the short-circuit above already populated `input_fields`) so
        // caller-supplied ABIs cannot override the safe built-in decoders for
        // canonical tokens. For proxy destinations, decode against the linked
        // implementation ABI rather than assuming the proxy address is the
        // implementation; this stays strictly after the known-token short-circuit,
        // so a caller-supplied "proxy" entry can never redirect a canonical token.
        if input_fields.is_empty() {
            if let (Some(to_address), Some(abi_reg)) = (transaction.to(), abi_registry) {
                input_fields.extend(visualize_with_abi_registry(
                    abi_reg, chain_id, to_address, input,
                ));
            }
        }

        // Fallback: Try ERC20 if decode_transfers is enabled. Skipped for
        // known tokens because the short-circuit above already populated
        // `input_fields`: `approve(address,uint256)` and
        // `transfer(address,uint256)` share selectors across ERC20/ERC721, so
        // without this guard a known ERC721 (or ERC1155) call would be
        // mis-rendered as an ERC20 op once its own visualizer returns `None`,
        // undermining the "canonical-token short-circuit wins over any other
        // decoder" property.
        if input_fields.is_empty() && options.decode_transfers {
            if let Some(field) = (contracts::core::ERC20Visualizer {}).visualize_tx_commands(input)
            {
                input_fields.push(field);
            }
        }
        if input_fields.is_empty() {
            input_fields.push(contracts::core::FallbackVisualizer::new().visualize_hex(input));
        }

        fields.append(&mut input_fields);
    }

    let title = options
        .transaction_name
        .unwrap_or_else(|| "Ethereum Transaction".to_string());
    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields,
        "EthereumTx".to_string(),
    ))
}

// Public API functions for ease of use
pub fn transaction_to_visual_sign(
    transaction: TypedTransaction,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let wrapper = EthereumTransactionWrapper::new(transaction);
    let converter = EthereumVisualSignConverter::new();
    converter.to_visual_sign_payload(wrapper, options)
}

pub fn transaction_string_to_visual_sign(
    transaction_data: &str,
    options: VisualSignOptions,
) -> Result<SignablePayload, VisualSignError> {
    let converter = EthereumVisualSignConverter::new();
    converter.to_visual_sign_payload_from_string(transaction_data, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::core::erc20::IERC20;
    use crate::registry::ContractRegistry;
    use crate::token_metadata::{ErcStandard, TokenMetadata};
    use alloy_consensus::{SignableTransaction, TxLegacy, TypedTransaction};
    use alloy_primitives::{Address, Bytes, ChainId, U256};
    use alloy_sol_types::SolCall;
    use alloy_primitives::keccak256;
    use generated::parser::{Abi, AbiType, ChainMetadata, EthereumMetadata, chain_metadata};
    use visualsign::{SignablePayloadFieldAddressV2, SignablePayloadFieldAmountV2};

    fn unsigned_to_hex(tx: &TypedTransaction) -> String {
        let mut encoded = Vec::new();
        tx.encode_for_signing(&mut encoded);
        format!("0x{}", hex::encode(&encoded))
    }

    #[test]
    fn test_transaction_to_visual_sign_basic() {
        // Create a dummy Ethereum transaction
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 42,
            gas_price: 20_000_000_000u128, // 20 gwei
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(
                "0x000000000000000000000000000000000000dead"
                    .parse()
                    .unwrap(),
            ),
            value: U256::from(1000000000000000000u64), // 1 ETH
            input: Bytes::new(),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        let expected_payload = SignablePayload::new(
            0,
            "Ethereum Transaction".to_string(),
            None,
            vec![
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Ethereum Mainnet".to_string(),
                        label: "Network".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "Ethereum Mainnet".to_string(),
                    },
                },
                SignablePayloadField::AddressV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "0x000000000000000000000000000000000000dEaD".to_string(),
                        label: "To".to_string(),
                    },
                    address_v2: SignablePayloadFieldAddressV2 {
                        address: "0x000000000000000000000000000000000000dEaD".to_string(),
                        name: "To".to_string(),
                        asset_label: "ETH".to_string(),
                        memo: None,
                        badge_text: None,
                    },
                },
                SignablePayloadField::AmountV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "1 ETH".to_string(),
                        label: "Value".to_string(),
                    },
                    amount_v2: SignablePayloadFieldAmountV2 {
                        amount: "1".to_string(),
                        abbreviation: Some("ETH".to_string()),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "21000".to_string(),
                        label: "Gas Limit".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "21000".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "20 gwei".to_string(),
                        label: "Gas Price".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "20 gwei".to_string(),
                    },
                },
                SignablePayloadField::TextV2 {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "42".to_string(),
                        label: "Nonce".to_string(),
                    },
                    text_v2: SignablePayloadFieldTextV2 {
                        text: "42".to_string(),
                    },
                },
            ],
            "EthereumTx".to_string(),
        );

        // Compare individual fields since SignablePayload doesn't implement PartialEq
        assert_eq!(expected_payload.title, payload.title);
        assert_eq!(expected_payload.version, payload.version);
        assert_eq!(expected_payload.subtitle, payload.subtitle);
        assert_eq!(expected_payload.fields.len(), payload.fields.len());
        assert_eq!(expected_payload.payload_type, payload.payload_type);

        for (expected_field, actual_field) in
            expected_payload.fields.iter().zip(payload.fields.iter())
        {
            assert_eq!(expected_field.label(), actual_field.label());
            match (expected_field, actual_field) {
                (
                    SignablePayloadField::TextV2 {
                        text_v2: expected_text,
                        ..
                    },
                    SignablePayloadField::TextV2 {
                        text_v2: actual_text,
                        ..
                    },
                ) => {
                    assert_eq!(expected_text.text, actual_text.text);
                }
                (
                    SignablePayloadField::AddressV2 {
                        common: expected_common,
                        address_v2: expected_addr,
                        ..
                    },
                    SignablePayloadField::AddressV2 {
                        common: actual_common,
                        address_v2: actual_addr,
                        ..
                    },
                ) => {
                    assert_eq!(expected_common.fallback_text, actual_common.fallback_text);
                    assert_eq!(expected_addr.address, actual_addr.address);
                    assert_eq!(expected_addr.asset_label, actual_addr.asset_label);
                    assert_eq!(expected_addr.name, actual_addr.name);
                    assert_eq!(expected_addr.memo, actual_addr.memo);
                    assert_eq!(expected_addr.badge_text, actual_addr.badge_text);
                }
                (
                    SignablePayloadField::AmountV2 {
                        amount_v2: expected_amt,
                        common: expected_common,
                        ..
                    },
                    SignablePayloadField::AmountV2 {
                        amount_v2: actual_amt,
                        common: actual_common,
                        ..
                    },
                ) => {
                    assert_eq!(expected_amt.amount, actual_amt.amount);
                    assert_eq!(expected_amt.abbreviation, actual_amt.abbreviation);
                    assert_eq!(expected_common.fallback_text, actual_common.fallback_text);
                }
                _ => {
                    panic!(
                        "Field type mismatch for label '{}': expected {:?}, got {:?}",
                        expected_field.label(),
                        std::mem::discriminant(expected_field),
                        std::mem::discriminant(actual_field)
                    );
                }
            }
        }
    }

    #[test]
    fn test_transaction_with_input_data() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 1,
            gas_price: 1_000_000_000u128,
            gas_limit: 50000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::from(vec![0x12, 0x34, 0x56, 0x78]),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        // Check that input data field is present (FallbackVisualizer)
        assert!(payload.fields.iter().any(|f| f.label() == "Input Data"));
        let input_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Input Data")
            .unwrap();
        if let SignablePayloadField::TextV2 { text_v2, .. } = input_field {
            assert_eq!(text_v2.text, "0x12345678");
        }
    }

    /// Regression: caller-supplied ABIs keyed to a known token address
    /// (e.g. USDC) must not override the safe built-in ERC20/ERC721 decoder.
    ///
    /// An attacker who can inject `chain_metadata.abi_mappings` could otherwise
    /// supply an ABI entry whose function name + input types match selector
    /// `0xa9059cbb` (transfer) but with attacker-chosen parameter labels, then
    /// spoof the wallet UI with those labels. (The function name itself is
    /// fixed by the selector match; only the parameter names are
    /// attacker-controlled here.) The fix in lib.rs prefers the built-in
    /// visualizer for any address present in the compiled-in
    /// ContractRegistry's token registry.
    #[test]
    fn test_known_token_ignores_caller_supplied_abi_for_transfer() {
        let usdc_address: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let victim: Address = "0x000000000000000000000000000000000000beef"
            .parse()
            .unwrap();
        let amount = U256::from(1_000_000_000u64);

        // Build a registry with USDC registered as a known ERC20.
        let mut registry = ContractRegistry::new();
        registry
            .register_token(
                1,
                TokenMetadata {
                    symbol: "USDC".to_string(),
                    name: "USD Coin".to_string(),
                    erc_standard: ErcStandard::Erc20,
                    contract_address: usdc_address.to_string(),
                    decimals: 6,
                },
            )
            .unwrap();
        let converter = EthereumVisualSignConverter::with_registry(Arc::new(registry));

        // Real `transfer(victim, 1_000_000_000)` calldata to USDC.
        let call = IERC20::transferCall { to: victim, amount };
        let calldata = Bytes::from(IERC20::transferCall::abi_encode(&call));

        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(usdc_address),
            value: U256::ZERO,
            input: calldata,
        });

        // Caller-supplied ABI: same selector as `transfer(address,uint256)`
        // (which is required for the AbiRegistry dispatcher to match it), but
        // with attacker-chosen parameter names that misrepresent what the user
        // is signing. Without the fix, the UI would render labels controlled
        // by the dApp ("backup_wallet", "safety_deposit") instead of the
        // canonical "Recipient"/"Amount" from the built-in ERC20 visualizer.
        //
        // Note: alloy's `Function::selector()` is derived from name + input
        // types, so the function name MUST stay `transfer`. Parameter NAMES,
        // however, do not affect the selector and are fully attacker-controlled.
        // This is sufficient to spoof the signing prompt.
        let malicious_abi = r#"[
            {
                "type": "function",
                "name": "transfer",
                "inputs": [
                    {"name": "backup_wallet", "type": "address"},
                    {"name": "safety_deposit", "type": "uint256"}
                ],
                "outputs": [],
                "stateMutability": "nonpayable"
            }
        ]"#;
        // Build the fixture as a BTreeMap (crate determinism rule) and let
        // it `.collect()` into the proto field's HashMap at the call site.
        let abi_mappings: std::collections::BTreeMap<String, Abi> = std::iter::once((
            usdc_address.to_string(),
            Abi {
                value: malicious_abi.to_string(),
                signature: None,
                ..Default::default()
            },
        ))
        .collect();

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("ETHEREUM_MAINNET".to_string()),
                    abi_mappings: abi_mappings.into_iter().collect(),
                })),
            }),
            developer_config: None,
        };

        let wrapper = EthereumTransactionWrapper::new(tx);
        let payload = converter.to_visual_sign_payload(wrapper, options).unwrap();

        // The built-in ERC20 visualizer emits a PreviewLayout titled
        // "ERC20 Transfer" with a "Recipient" address field and an "Amount"
        // field. Without the fix, the malicious ABI would have produced
        // attacker-chosen labels ("backup_wallet"/"safety_deposit") on the
        // same selector, spoofing the signing prompt.
        let preview = payload
            .fields
            .iter()
            .find(|f| matches!(f, SignablePayloadField::PreviewLayout { .. }))
            .expect("expected a PreviewLayout field from the built-in ERC20 visualizer");

        let SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        } = preview
        else {
            panic!("matched discriminant changed");
        };
        assert_eq!(common.label, "ERC20 Transfer");
        assert_eq!(
            preview_layout
                .title
                .as_ref()
                .map(|t| t.text.as_str())
                .unwrap_or(""),
            "ERC20 Transfer",
        );

        let expanded = preview_layout
            .expanded
            .as_ref()
            .expect("expected expanded ListLayout for transfer details");
        let labels: Vec<&str> = expanded
            .fields
            .iter()
            .map(|f| f.signable_payload_field.label().as_str())
            .collect();
        assert!(
            labels.contains(&"Recipient"),
            "expected built-in 'Recipient' label, got {labels:?}",
        );
        assert!(
            labels.contains(&"Amount"),
            "expected built-in 'Amount' label, got {labels:?}",
        );

        // And, defensively, the attacker-chosen parameter names must not
        // appear anywhere in the rendered payload (labels, titles, subtitles,
        // fallback_text, or any other serialized text). Serializing the whole
        // payload to JSON and scanning the resulting string is the simplest
        // way to assert this property without enumerating every text-bearing
        // field on every SignablePayloadField variant.
        let rendered = serde_json::to_string(&payload).unwrap();
        assert!(
            !rendered.contains("backup_wallet"),
            "caller ABI param name must not appear anywhere in the rendered payload: {rendered}",
        );
        assert!(
            !rendered.contains("safety_deposit"),
            "caller ABI param name must not appear anywhere in the rendered payload: {rendered}",
        );
    }

    /// When the tx carries an explicit chain_id and the caller-supplied metadata
    /// claims a different chain, the mismatch is a hard rejection. This prevents
    /// a malicious dApp from supplying `network_id = POLYGON` against a mainnet
    /// tx to sneak in a caller-supplied ABI: the converter refuses to produce a
    /// payload at all, so no ABI (malicious or otherwise) gets bound.
    ///
    /// This test supersedes the earlier "known-token short-circuit uses tx chain_id"
    /// variant. The security property (attacker cannot bind a malicious ABI to a
    /// canonical token via a mismatched network_id) is still upheld -- the reject
    /// path is strictly stronger than a proceed-with-tx-chain_id path.
    #[test]
    fn test_known_token_short_circuit_uses_tx_chain_id_not_metadata() {
        let usdc_address: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let victim: Address = "0x000000000000000000000000000000000000beef"
            .parse()
            .unwrap();
        let amount = U256::from(1_000_000_000u64);

        // USDC registered on mainnet (chain id 1) only.
        let mut registry = ContractRegistry::new();
        registry
            .register_token(
                1,
                TokenMetadata {
                    symbol: "USDC".to_string(),
                    name: "USD Coin".to_string(),
                    erc_standard: ErcStandard::Erc20,
                    contract_address: usdc_address.to_string(),
                    decimals: 6,
                },
            )
            .unwrap();
        let converter = EthereumVisualSignConverter::with_registry(Arc::new(registry));

        // Real `transfer(victim, 1_000_000_000)` against mainnet USDC.
        let call = IERC20::transferCall { to: victim, amount };
        let calldata = Bytes::from(IERC20::transferCall::abi_encode(&call));
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(usdc_address),
            value: U256::ZERO,
            input: calldata,
        });

        let malicious_abi = r#"[
            {
                "type": "function",
                "name": "transfer",
                "inputs": [
                    {"name": "backup_wallet", "type": "address"},
                    {"name": "safety_deposit", "type": "uint256"}
                ],
                "outputs": [],
                "stateMutability": "nonpayable"
            }
        ]"#;
        let abi_mappings: std::collections::BTreeMap<String, Abi> = std::iter::once((
            usdc_address.to_string(),
            Abi {
                value: malicious_abi.to_string(),
                signature: None,
                ..Default::default()
            },
        ))
        .collect();

        // Attacker mismatches metadata to Polygon (137). The converter must refuse
        // outright: tx chain_id (1) is authoritative, and any mismatch with
        // metadata is a hard error so no ABI -- malicious or otherwise -- gets
        // bound to the payload.
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("POLYGON_MAINNET".to_string()),
                    abi_mappings: abi_mappings.into_iter().collect(),
                })),
            }),
            developer_config: None,
        };

        let wrapper = EthereumTransactionWrapper::new(tx);
        let err = converter
            .to_visual_sign_payload(wrapper, options)
            .unwrap_err();
        assert!(
            matches!(err, VisualSignError::ValidationError(_)),
            "mismatched tx/metadata chain_id must be rejected: {err:?}",
        );
        let VisualSignError::ValidationError(msg) = err else {
            unreachable!()
        };
        assert!(
            msg.contains("chain_id mismatch"),
            "error must mention chain_id mismatch, got: {msg}",
        );
        assert!(
            msg.contains("chain_id 1 "),
            "error must reference tx-declared chain_id 1, got: {msg}",
        );
        assert!(
            msg.contains("chain_id 137"),
            "error must reference metadata chain_id 137, got: {msg}",
        );
    }

    /// Pre-EIP-155 legacy transactions don't carry a chain id of their own, so
    /// the canonical-token short-circuit must not fall back to the resolved
    /// (metadata-derived) chain id. Otherwise an attacker can supply a legacy
    /// tx to USDC's mainnet address with `network_id` pointing at a chain
    /// where USDC isn't registered, the global lookup misses, and a
    /// caller-supplied ABI gets to bind to a canonical token. We expect the
    /// dispatcher to do an address-only any-chain lookup in that case.
    #[test]
    fn test_known_token_short_circuit_handles_chain_id_less_legacy_tx() {
        let usdc_address: Address = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .parse()
            .unwrap();
        let victim: Address = "0x000000000000000000000000000000000000beef"
            .parse()
            .unwrap();
        let amount = U256::from(1_000_000_000u64);

        // USDC registered on mainnet (chain id 1) only.
        let mut registry = ContractRegistry::new();
        registry
            .register_token(
                1,
                TokenMetadata {
                    symbol: "USDC".to_string(),
                    name: "USD Coin".to_string(),
                    erc_standard: ErcStandard::Erc20,
                    contract_address: usdc_address.to_string(),
                    decimals: 6,
                },
            )
            .unwrap();
        let converter = EthereumVisualSignConverter::with_registry(Arc::new(registry));

        // Pre-EIP-155 legacy tx: `chain_id == None`. Calldata is a real
        // `transfer(victim, 1_000_000_000)` against the USDC mainnet address.
        let call = IERC20::transferCall { to: victim, amount };
        let calldata = Bytes::from(IERC20::transferCall::abi_encode(&call));
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: None,
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(usdc_address),
            value: U256::ZERO,
            input: calldata,
        });

        let malicious_abi = r#"[
            {
                "type": "function",
                "name": "transfer",
                "inputs": [
                    {"name": "backup_wallet", "type": "address"},
                    {"name": "safety_deposit", "type": "uint256"}
                ],
                "outputs": [],
                "stateMutability": "nonpayable"
            }
        ]"#;
        let abi_mappings: std::collections::BTreeMap<String, Abi> = std::iter::once((
            usdc_address.to_string(),
            Abi {
                value: malicious_abi.to_string(),
                signature: None,
                ..Default::default()
            },
        ))
        .collect();

        // Attacker points `network_id` at Polygon (no USDC entry in our local
        // registry) so any chain-id-based lookup that trusts metadata misses.
        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("POLYGON_MAINNET".to_string()),
                    abi_mappings: abi_mappings.into_iter().collect(),
                })),
            }),
            developer_config: None,
        };

        let wrapper = EthereumTransactionWrapper::new(tx);
        let payload = converter.to_visual_sign_payload(wrapper, options).unwrap();

        let preview = payload
            .fields
            .iter()
            .find(|f| matches!(f, SignablePayloadField::PreviewLayout { .. }))
            .expect("expected a PreviewLayout from the built-in ERC20 visualizer");
        let SignablePayloadField::PreviewLayout { common, .. } = preview else {
            panic!("matched discriminant changed");
        };
        assert_eq!(
            common.label, "ERC20 Transfer",
            "chain-id-less legacy tx to a canonical token must still hit the built-in decoder",
        );

        // Mirror the substring scan used earlier in this file: serialize the
        // whole payload to JSON so we cover every text-bearing field, not just
        // those `Debug` happens to print.
        let rendered = serde_json::to_string(&payload).unwrap();
        assert!(
            !rendered.contains("backup_wallet") && !rendered.contains("safety_deposit"),
            "caller ABI param name must not appear anywhere in the rendered payload: {rendered}",
        );
    }

    /// A known ERC721 token called with the ERC20/ERC721 shared
    /// `approve(address,uint256)` selector must not be mis-rendered as an ERC20
    /// approval. The dispatch order is:
    ///
    ///   1. `try_known_token_dispatch` fires (ERC721 visualizer returns None,
    ///      helper substitutes a raw-hex field so its `Some` is non-empty).
    ///   2. Caller-ABI path skipped (`input_fields.is_empty()` is now false).
    ///   3. ERC20 `decode_transfers` fallback also skipped for the same reason,
    ///      or it would decode `approve` as an ERC20 op.
    ///   4. Raw-hex (from the helper) is the rendered result.
    ///
    /// Without the helper's non-empty `Some` contract this regresses to "ERC20
    /// Approve" output, which is the spoofing surface this fix closes.
    #[test]
    fn test_known_erc721_token_skips_erc20_fallback_on_shared_selector() {
        // Address-only fixture, the actual contract standard is decided by the
        // registry entry below.
        let nft_address: Address = "0xb0b0000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let spender: Address = "0x000000000000000000000000000000000000beef"
            .parse()
            .unwrap();
        let token_id = U256::from(42u64);

        // Register the address as an ERC721 token on mainnet.
        let mut registry = ContractRegistry::new();
        registry
            .register_token(
                1,
                TokenMetadata {
                    symbol: "NFT".to_string(),
                    name: "Test NFT".to_string(),
                    erc_standard: ErcStandard::Erc721,
                    contract_address: nft_address.to_string(),
                    decimals: 0,
                },
            )
            .unwrap();
        let converter = EthereumVisualSignConverter::with_registry(Arc::new(registry));

        // ERC721 `approve(spender, tokenId)` shares its 4-byte selector with
        // ERC20 `approve(address,uint256)`.
        let call = IERC20::approveCall {
            spender,
            amount: token_id,
        };
        let calldata = Bytes::from(IERC20::approveCall::abi_encode(&call));
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(nft_address),
            value: U256::ZERO,
            input: calldata,
        });

        let options = VisualSignOptions {
            decode_transfers: true,
            transaction_name: None,
            metadata: None,
            developer_config: None,
        };

        let wrapper = EthereumTransactionWrapper::new(tx);
        let payload = converter.to_visual_sign_payload(wrapper, options).unwrap();

        // No PreviewLayout should be emitted: ERC721 has no decoder, the
        // ERC20 fallback is skipped for known non-ERC20 tokens, and the
        // raw-hex fallback produces a RawData field, not a PreviewLayout.
        let preview = payload
            .fields
            .iter()
            .find(|f| matches!(f, SignablePayloadField::PreviewLayout { .. }));
        assert!(
            preview.is_none(),
            "known ERC721 + ERC20-shared selector must not produce a PreviewLayout: {payload:?}",
        );

        // And, defensively, no ERC20-decoder text should appear anywhere in
        // the serialized payload. Use serde_json so we scan every text-bearing
        // field, not just what `Debug` prints. The ERC20 approve visualizer
        // emits the label "ERC20 Approve" and a "Spender" field, both of
        // which are the regression signals.
        let rendered = serde_json::to_string(&payload).unwrap();
        assert!(
            !rendered.contains("ERC20 Approve") && !rendered.contains("Spender"),
            "known ERC721 must not be mis-rendered as an ERC20 approval: {rendered}",
        );
    }

    #[test]
    fn test_transaction_with_custom_title() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: Some("Custom Transaction Title".to_string()),
            metadata: None,
            developer_config: None,
        };
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        assert_eq!(payload.title, "Custom Transaction Title");
    }

    #[test]
    fn test_transaction_wrapper_from_string() {
        // Test with empty string
        assert_eq!(
            EthereumTransactionWrapper::from_string(""),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Input too short".to_string()
            )),
        );
        // Test with invalid hex data
        assert_eq!(
            EthereumTransactionWrapper::from_string("invalid_hex_data"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Failed to decode base64: Invalid symbol 95, offset 7.".to_string()
            )),
        );
        // Test with malformed hex (odd length)
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x123"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Failed to decode hex: Odd number of digits"
                    .to_string()
            )),
        );
        // Test with valid hex prefix but invalid RLP data
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x1234567890abcdef"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Unexpected type flag. Got 18.".to_string()
            )),
        );
        // Test with valid base64 but invalid RLP data
        assert_eq!(
            EthereumTransactionWrapper::from_string("aGVsbG8gd29ybGQ="),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Unexpected type flag. Got 104.".to_string()
            )),
        );
        // Test with unknown transaction type
        assert_eq!(
            EthereumTransactionWrapper::from_string(
                "0x05f86401808504a817c800825208940000000000000000000000000000000000000000880de0b6b3a764000080c0"
            ),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Unexpected type flag. Got 5.".to_string()
            )),
        );
        // Test with corrupted typed transaction (invalid RLP after type byte)
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x02ff"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: input too short".to_string()
            )),
        );
        // Test with valid transaction type but insufficient data
        assert_eq!(
            EthereumTransactionWrapper::from_string("0x02"),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: input too short".to_string()
            )),
        );
        // Test with whitespace in input (should fail due to invalid format)
        assert_eq!(
            EthereumTransactionWrapper::from_string(" 0x1234 "),
            Err(TransactionParseError::DecodeError(
                "Failed to decode transaction: Failed to decode base64: Invalid symbol 32, offset 0.".to_string()
            )),
        );
        // Test with legacy transaction
        let legacy_tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 20_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&legacy_tx)),
            Ok(EthereumTransactionWrapper::new(legacy_tx.clone())),
        );
        // Test with EIP-1559 transaction
        let eip1559_tx = TypedTransaction::Eip1559(alloy_consensus::TxEip1559 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 30_000_000_000u128,
            max_priority_fee_per_gas: 2_000_000_000u128,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip1559_tx)),
            Ok(EthereumTransactionWrapper::new(eip1559_tx.clone())),
        );
        // Test with EIP-2930 transaction (unsupported)
        let eip2930_tx = TypedTransaction::Eip2930(alloy_consensus::TxEip2930 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            gas_price: 20_000_000_000u128,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip2930_tx)),
            Err(TransactionParseError::DecodeError(
                "Unsupported transaction type: eip-2930".to_string()
            ))
        );
        // Test with EIP-4844 transaction (unsupported)
        let eip4844_tx = TypedTransaction::Eip4844(alloy_consensus::TxEip4844Variant::TxEip4844(
            alloy_consensus::TxEip4844 {
                chain_id: ChainId::from(1u64),
                nonce: 1,
                gas_limit: 21000,
                max_fee_per_gas: 30_000_000_000u128,
                max_priority_fee_per_gas: 2_000_000_000u128,
                to: Address::ZERO,
                value: U256::from(1000000000000000000u64),
                access_list: Default::default(),
                input: Bytes::new(),
                blob_versioned_hashes: Default::default(),
                max_fee_per_blob_gas: 10_000_000_000u128,
            },
        ));
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip4844_tx)),
            Err(TransactionParseError::DecodeError(
                "Unsupported transaction type: eip-4844".to_string()
            ))
        );
        // Test with EIP-7702 transaction (unsupported)
        let eip7702_tx = TypedTransaction::Eip7702(alloy_consensus::TxEip7702 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 30_000_000_000u128,
            max_priority_fee_per_gas: 2_000_000_000u128,
            to: Address::ZERO,
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
            authorization_list: Default::default(),
        });
        assert_eq!(
            EthereumTransactionWrapper::from_string(&unsigned_to_hex(&eip7702_tx)),
            Err(TransactionParseError::DecodeError(
                "Unsupported transaction type: eip-7702".to_string()
            ))
        );
    }

    #[test]
    fn test_transaction_wrapper_type() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let wrapper = EthereumTransactionWrapper::new(tx);
        assert_eq!(wrapper.transaction_type(), "Ethereum");
    }

    #[test]
    fn test_zero_value_transaction() {
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        let value_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Value")
            .unwrap();
        if let SignablePayloadField::AmountV2 { amount_v2, common } = value_field {
            assert!(amount_v2.amount.contains("0"));
            assert_eq!(amount_v2.abbreviation.as_deref(), Some("ETH"));
            assert_eq!(common.fallback_text, "0 ETH");
        } else {
            panic!("Expected AmountV2 for Value field");
        }
    }

    #[test]
    fn test_non_eth_chain_fee_symbol() {
        // Polygon (chain 137) should use "POL" as the fee-paying asset symbol
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(137u64)),
            nonce: 0,
            gas_price: 30_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64), // 1 POL
            input: Bytes::new(),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        let to_field = payload.fields.iter().find(|f| f.label() == "To").unwrap();
        if let SignablePayloadField::AddressV2 { address_v2, .. } = to_field {
            assert_eq!(address_v2.asset_label, "POL");
        } else {
            panic!("Expected AddressV2 for To field");
        }

        let value_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Value")
            .unwrap();
        if let SignablePayloadField::AmountV2 { amount_v2, common } = value_field {
            assert_eq!(amount_v2.amount, "1");
            assert_eq!(amount_v2.abbreviation.as_deref(), Some("POL"));
            assert_eq!(common.fallback_text, "1 POL");
        } else {
            panic!("Expected AmountV2 for Value field");
        }
    }

    #[test]
    fn test_unknown_chain_fee_symbol() {
        // Unknown chain (999999) should have no fee-paying asset symbol:
        // empty asset_label, no abbreviation, and value-only fallback text.
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(999999u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 21000,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            input: Bytes::new(),
        });

        let options = VisualSignOptions::default();
        let payload = transaction_to_visual_sign(tx, options).unwrap();

        let to_field = payload.fields.iter().find(|f| f.label() == "To").unwrap();
        if let SignablePayloadField::AddressV2 { address_v2, .. } = to_field {
            assert_eq!(address_v2.asset_label, "");
        } else {
            panic!("Expected AddressV2 for To field");
        }

        let value_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Value")
            .unwrap();
        if let SignablePayloadField::AmountV2 { amount_v2, common } = value_field {
            assert_eq!(amount_v2.amount, "1");
            assert_eq!(amount_v2.abbreviation, None);
            assert_eq!(common.fallback_text, "1");
        } else {
            panic!("Expected AmountV2 for Value field");
        }
    }

    #[test]
    fn test_transaction_to_visual_sign_public_api() {
        // Test the public API function
        let tx = TypedTransaction::Eip1559(alloy_consensus::TxEip1559 {
            chain_id: ChainId::from(1u64),
            nonce: 1,
            gas_limit: 21000,
            max_fee_per_gas: 30_000_000_000u128,
            max_priority_fee_per_gas: 2_000_000_000u128,
            to: alloy_primitives::TxKind::Call(Address::ZERO),
            value: U256::from(1000000000000000000u64),
            access_list: Default::default(),
            input: Bytes::new(),
        });
        assert_eq!(
            transaction_string_to_visual_sign(
                &unsigned_to_hex(&tx),
                VisualSignOptions {
                    decode_transfers: true,
                    transaction_name: Some("Test Transaction".to_string()),
                    metadata: None,
                    developer_config: None,
                }
            ),
            Ok(SignablePayload::new(
                0,
                "Test Transaction".to_string(),
                None,
                vec![
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "Ethereum Mainnet".to_string(),
                            label: "Network".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "Ethereum Mainnet".to_string(),
                        },
                    },
                    SignablePayloadField::AddressV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "0x0000000000000000000000000000000000000000".to_string(),
                            label: "To".to_string(),
                        },
                        address_v2: SignablePayloadFieldAddressV2 {
                            address: "0x0000000000000000000000000000000000000000".to_string(),
                            name: "To".to_string(),
                            asset_label: "ETH".to_string(),
                            memo: None,
                            badge_text: None,
                        },
                    },
                    SignablePayloadField::AmountV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "1 ETH".to_string(),
                            label: "Value".to_string(),
                        },
                        amount_v2: SignablePayloadFieldAmountV2 {
                            amount: "1".to_string(),
                            abbreviation: Some("ETH".to_string()),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "21000".to_string(),
                            label: "Gas Limit".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "21000".to_string(),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "30 gwei".to_string(),
                            label: "Gas Price".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "30 gwei".to_string(),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "2 gwei".to_string(),
                            label: "Max Priority Fee Per Gas".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "2 gwei".to_string(),
                        },
                    },
                    SignablePayloadField::TextV2 {
                        common: SignablePayloadFieldCommon {
                            fallback_text: "1".to_string(),
                            label: "Nonce".to_string(),
                        },
                        text_v2: SignablePayloadFieldTextV2 {
                            text: "1".to_string(),
                        },
                    },
                ],
                "EthereumTx".to_string()
            ))
        );
    }

    #[test]
    fn test_signed_transaction_rejected_by_default() {
        // Signed EIP-1559 transaction from Etherscan (has signature r, s, v at the end)
        let signed_tx = "0x02f9043801588477359400847a1e5be18303e05c943fc91a3afd70395cd496c647d5a6cc9d4b2b7fad870b5e620f480000b903c43593564c000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000692a575a00000000000000000000000000000000000000000000000000000000000000040b080604000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000028000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000b5e620f48000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000b5e620f480000000000000000000000000000000000000000000000000000000002109cfe602600000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2000000000000000000000000255494b830bd4fe7220b3ec4842cba75600b6c800000000000000000000000000000000000000000000000000000000000000060000000000000000000000000255494b830bd4fe7220b3ec4842cba75600b6c80000000000000000000000000000000fee13a103a10d593b9ae06b3e05f2e7e1c00000000000000000000000000000000000000000000000000000000000000190000000000000000000000000000000000000000000000000000000000000060000000000000000000000000255494b830bd4fe7220b3ec4842cba75600b6c80000000000000000000000000d27f4bbd67bd4ad1674c9c2c5a75ca8c3e389f3b0000000000000000000000000000000000000000000000000000020f4aae6130c080a0e25b5930432fd92177b2f62f7edbd4c029cee52fc196bc91f2071b7a2ac565f6a05e67015b7153d1330fe7f975d3ab6d0ab6b606ef1e40f685b110dfbb62d4439d";

        // Should fail with default options (developer_config: None)
        let result = EthereumTransactionWrapper::from_string(signed_tx);
        assert!(
            result.is_err(),
            "Signed transaction should be rejected by default"
        );
    }

    #[test]
    fn test_signed_transaction_accepted_with_developer_config() {
        // Same signed EIP-1559 transaction
        let signed_tx = "0x02f9043801588477359400847a1e5be18303e05c943fc91a3afd70395cd496c647d5a6cc9d4b2b7fad870b5e620f480000b903c43593564c000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000692a575a00000000000000000000000000000000000000000000000000000000000000040b080604000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000028000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000b5e620f48000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000b5e620f480000000000000000000000000000000000000000000000000000000002109cfe602600000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2000000000000000000000000255494b830bd4fe7220b3ec4842cba75600b6c800000000000000000000000000000000000000000000000000000000000000060000000000000000000000000255494b830bd4fe7220b3ec4842cba75600b6c80000000000000000000000000000000fee13a103a10d593b9ae06b3e05f2e7e1c00000000000000000000000000000000000000000000000000000000000000190000000000000000000000000000000000000000000000000000000000000060000000000000000000000000255494b830bd4fe7220b3ec4842cba75600b6c80000000000000000000000000d27f4bbd67bd4ad1674c9c2c5a75ca8c3e389f3b0000000000000000000000000000000000000000000000000000020f4aae6130c080a0e25b5930432fd92177b2f62f7edbd4c029cee52fc196bc91f2071b7a2ac565f6a05e67015b7153d1330fe7f975d3ab6d0ab6b606ef1e40f685b110dfbb62d4439d";

        // Should succeed with developer_config.allow_signed_transactions = true
        let developer_config = DeveloperConfig {
            allow_signed_transactions: true,
        };
        let result = EthereumTransactionWrapper::from_string_with_options(
            signed_tx,
            Some(&developer_config),
        );
        assert!(
            result.is_ok(),
            "Signed transaction should be accepted with allow_signed_transactions: true"
        );

        let wrapper = result.unwrap();
        let tx = wrapper.inner();

        // Verify we extracted the correct unsigned transaction fields
        assert_eq!(tx.chain_id(), Some(1)); // Mainnet
        assert_eq!(tx.nonce(), 88);
    }

    #[test]
    fn test_proxy_with_unregistered_impl_produces_no_implementation_field() {
        // Proxy is registered and points to an implementation address, but
        // no ABI is registered for that implementation address. The registry
        // returns None from get_implementation_abi, so the code falls through
        // to the proxy's own ABI (here empty). End-to-end: no Implementation
        // field should be emitted.
        let proxy: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let impl_addr: Address = "0x0000000000000000000000000000000000000002"
            .parse()
            .unwrap();

        let abi_mappings: std::collections::BTreeMap<String, Abi> =
            [(
                proxy.to_string(),
                Abi {
                    value: "[]".to_string(),
                    abi_type: Some(AbiType::Proxy as i32),
                    implementation_address: Some(impl_addr.to_string()),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect();

        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(proxy),
            value: U256::ZERO,
            input: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]),
        });
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("ETHEREUM_MAINNET".to_string()),
                    abi_mappings,
                })),
            }),
            developer_config: None,
        };

        let converter = EthereumVisualSignConverter::new();
        let payload = converter
            .to_visual_sign_payload(EthereumTransactionWrapper::new(tx), options)
            .unwrap();

        assert!(
            !payload.fields.iter().any(|f| f.label() == "Implementation"),
            "no Implementation field expected when impl ABI is unregistered",
        );
    }

    #[test]
    fn test_proxy_impl_abi_selector_mismatch_emits_unresolved_implementation() {
        // Proxy registered with a real implementation ABI. The calldata selector
        // does not match any function in that ABI. After the fallback the
        // Implementation field must still be emitted — marked as unresolved —
        // so the signer can see where the call would delegate.
        let proxy: Address = "0x0000000000000000000000000000000000000010"
            .parse()
            .unwrap();
        let impl_addr: Address = "0x0000000000000000000000000000000000000011"
            .parse()
            .unwrap();

        let impl_abi = r#"[{"type":"function","name":"transfer",
            "inputs":[{"name":"to","type":"address"},{"name":"amount","type":"uint256"}],
            "outputs":[{"name":"","type":"bool"}],"stateMutability":"nonpayable"}]"#;

        let abi_mappings: std::collections::BTreeMap<String, Abi> = [
            (
                proxy.to_string(),
                Abi {
                    value: "[]".to_string(),
                    abi_type: Some(AbiType::Proxy as i32),
                    implementation_address: Some(impl_addr.to_string()),
                    ..Default::default()
                },
            ),
            (
                impl_addr.to_string(),
                Abi {
                    value: impl_abi.to_string(),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        // Selector that does not match `transfer(address,uint256)` (0xa9059cbb).
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(proxy),
            value: U256::ZERO,
            input: Bytes::from(vec![0x00, 0x01, 0x02, 0x03]),
        });
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("ETHEREUM_MAINNET".to_string()),
                    abi_mappings,
                })),
            }),
            developer_config: None,
        };

        let converter = EthereumVisualSignConverter::new();
        let payload = converter
            .to_visual_sign_payload(EthereumTransactionWrapper::new(tx), options)
            .unwrap();

        let impl_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Implementation")
            .expect("Implementation field must be present even when impl ABI did not decode");
        let SignablePayloadField::AddressV2 { address_v2, .. } = impl_field else {
            panic!("Implementation field is not AddressV2");
        };
        assert_eq!(address_v2.address, impl_addr.to_string());
        assert_eq!(
            address_v2.badge_text.as_deref(),
            Some("Proxy implementation (unresolved)"),
        );
    }

    #[test]
    fn test_proxy_resolution_is_single_hop() {
        // A -> B where B is also registered as a proxy pointing to C.
        // Resolution must stop at B (single hop): the calldata is the selector
        // for C's `doThing()` function, but C's ABI must never be consulted.
        let proxy_a: Address = "0x0000000000000000000000000000000000000020"
            .parse()
            .unwrap();
        let proxy_b: Address = "0x0000000000000000000000000000000000000021"
            .parse()
            .unwrap();
        let impl_c: Address = "0x0000000000000000000000000000000000000022"
            .parse()
            .unwrap();

        let c_abi = r#"[{"type":"function","name":"doThing","inputs":[],"outputs":[],"stateMutability":"nonpayable"}]"#;
        let abi_mappings: std::collections::BTreeMap<String, Abi> = [
            (
                proxy_a.to_string(),
                Abi {
                    value: "[]".to_string(),
                    abi_type: Some(AbiType::Proxy as i32),
                    implementation_address: Some(proxy_b.to_string()),
                    ..Default::default()
                },
            ),
            (
                proxy_b.to_string(),
                Abi {
                    value: "[]".to_string(),
                    abi_type: Some(AbiType::Proxy as i32),
                    implementation_address: Some(impl_c.to_string()),
                    ..Default::default()
                },
            ),
            (
                impl_c.to_string(),
                Abi {
                    value: c_abi.to_string(),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        let do_thing_hash = keccak256(b"doThing()");
        let do_thing_selector = [
            do_thing_hash[0],
            do_thing_hash[1],
            do_thing_hash[2],
            do_thing_hash[3],
        ];
        let tx = TypedTransaction::Legacy(TxLegacy {
            chain_id: Some(ChainId::from(1u64)),
            nonce: 0,
            gas_price: 1_000_000_000u128,
            gas_limit: 50_000,
            to: alloy_primitives::TxKind::Call(proxy_a),
            value: U256::ZERO,
            input: Bytes::from(do_thing_selector.to_vec()),
        });
        let options = VisualSignOptions {
            decode_transfers: false,
            transaction_name: None,
            metadata: Some(ChainMetadata {
                metadata: Some(chain_metadata::Metadata::Ethereum(EthereumMetadata {
                    network_id: Some("ETHEREUM_MAINNET".to_string()),
                    abi_mappings,
                })),
            }),
            developer_config: None,
        };

        let converter = EthereumVisualSignConverter::new();
        let payload = converter
            .to_visual_sign_payload(EthereumTransactionWrapper::new(tx), options)
            .unwrap();

        // C's ABI must not have been used — single-hop stops at B.
        let rendered = serde_json::to_string(&payload).unwrap();
        assert!(
            !rendered.contains("doThing"),
            "C's ABI must not be used under single-hop resolution; rendered: {rendered}",
        );
        // B is the unresolved implementation (B's empty ABI didn't decode the selector).
        let impl_field = payload
            .fields
            .iter()
            .find(|f| f.label() == "Implementation")
            .expect("expected unresolved Implementation field pointing at B");
        let SignablePayloadField::AddressV2 { address_v2, .. } = impl_field else {
            panic!("Implementation is not AddressV2");
        };
        assert_eq!(address_v2.address, proxy_b.to_string());
        assert_eq!(
            address_v2.badge_text.as_deref(),
            Some("Proxy implementation (unresolved)"),
        );
    }
}
