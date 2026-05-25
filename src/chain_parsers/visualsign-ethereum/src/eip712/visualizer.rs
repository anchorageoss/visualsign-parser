//! EIP-712 visualizer orchestrator: parse payload, match descriptor, render fields,
//! assemble final `SignablePayload` (or fall back to a structured tree walk).

use crate::eip712::descriptor::DescriptorField;
use crate::eip712::descriptor::registry::LayeredErc7730Registry;
use crate::eip712::fallback;
use crate::eip712::format::{RenderContext, render_field};
use crate::eip712::payload::Eip712Payload;
use crate::networks;
use crate::registry::ContractRegistry;
use std::collections::HashSet;
use std::sync::Arc;
use visualsign::SignablePayload;
use visualsign::field_builders::{create_address_field, create_text_field};
use visualsign::vsptrait::{
    Transaction, TransactionParseError, VisualSignConverter, VisualSignConverterFromString,
    VisualSignError, VisualSignOptions,
};

/// EIP-712 transaction wrapper (parallel to `EthereumTransactionWrapper`).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Eip712TransactionWrapper {
    payload: Eip712Payload,
}

impl Eip712TransactionWrapper {
    pub fn new(payload: Eip712Payload) -> Self {
        Self { payload }
    }
    pub fn inner(&self) -> &Eip712Payload {
        &self.payload
    }
}

impl Transaction for Eip712TransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        let payload = Eip712Payload::from_json(data)
            .map_err(|e| TransactionParseError::InvalidFormat(e.to_string()))?;
        Ok(Self { payload })
    }
    fn transaction_type(&self) -> String {
        "EthereumTypedData".to_string()
    }
}

pub struct Eip712VisualSignConverter {
    registry: Arc<ContractRegistry>,
}

impl Eip712VisualSignConverter {
    pub fn new() -> Self {
        let (contract_registry, _) = ContractRegistry::with_default_protocols();
        Self {
            registry: Arc::new(contract_registry),
        }
    }
    pub fn with_registry(registry: Arc<ContractRegistry>) -> Self {
        Self { registry }
    }
}

impl Default for Eip712VisualSignConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualSignConverter<Eip712TransactionWrapper> for Eip712VisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        wrapper: Eip712TransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let payload = wrapper.payload;
        let chain_id = payload.domain.chain_id.ok_or_else(|| {
            VisualSignError::DecodeError("EIP-712 payload missing domain.chainId".into())
        })?;

        let mut fields = header_fields(&payload, chain_id, &self.registry)?;

        // Only the embedded ERC-7730 registry is consulted on the live path. The
        // lookup also verifies the descriptor's declared EIP-712 schema matches
        // the payload's `types` field — without that check, an attacker could
        // submit a payload claiming a known `primaryType` but with reordered or
        // renamed fields, and we'd render the descriptor's expected labels
        // against the wrong values. EIP-712 has no separate ABI; the payload's
        // own `types` map is the authoritative schema, so we cross-check both.
        let layered = LayeredErc7730Registry::new(None);
        let descriptor = match layered.find_for_payload(&payload) {
            Some((d, _src)) => d,
            None => return tree_walk_payload(&payload, options, fields),
        };
        let format = match descriptor.display.formats.get(&payload.primary_type) {
            Some(f) => f,
            None => return tree_walk_payload(&payload, options, fields),
        };

        if let Some(intent) = &format.intent {
            fields.push(create_text_field("Intent", intent)?.signable_payload_field);
        }

        let excluded: HashSet<&str> = format.excluded.iter().map(|s| s.as_str()).collect();
        let render_ctx = RenderContext {
            chain_id,
            registry: Some(&self.registry),
            domain: &payload.domain,
            message: &payload.message,
        };

        let mut rendered: Vec<visualsign::SignablePayloadField> = Vec::new();
        let mut any_failed = false;
        for descf in format
            .fields
            .iter()
            .filter(|f: &&DescriptorField| f.path.as_deref().is_none_or(|p| !excluded.contains(p)))
        {
            // Skip group-style fields (label + nested fields) and constant fields (no path).
            if descf.path.is_none() || descf.format.is_none() {
                continue;
            }
            match render_field(descf, &render_ctx) {
                Ok(mut fs) => rendered.append(&mut fs),
                Err(e) => {
                    log::warn!(
                        "EIP-712 field render failed ({:?}): {e}; falling back to tree walk",
                        descf.path
                    );
                    any_failed = true;
                    break;
                }
            }
        }

        if any_failed {
            let mut walk = fallback::render(&payload)?;
            fields.push(
                create_text_field("Warning", "Descriptor render failed - showing raw payload")?
                    .signable_payload_field,
            );
            fields.append(&mut walk);
        } else {
            fields.append(&mut rendered);
        }

        let title = options
            .transaction_name
            .clone()
            .or_else(|| format.intent.clone())
            .unwrap_or_else(|| format!("Sign typed data: {}", payload.primary_type));

        Ok(SignablePayload::new(
            0,
            title,
            None,
            fields,
            "EthereumTypedData".into(),
        ))
    }
}

impl VisualSignConverterFromString<Eip712TransactionWrapper> for Eip712VisualSignConverter {
    fn to_visual_sign_payload_from_string(
        &self,
        transaction_data: &str,
        options: VisualSignOptions,
    ) -> Result<SignablePayload, VisualSignError> {
        let wrapper = Eip712TransactionWrapper::from_string(transaction_data)
            .map_err(VisualSignError::ParseError)?;
        self.to_visual_sign_payload(wrapper, options)
    }
}

fn tree_walk_payload(
    payload: &Eip712Payload,
    options: VisualSignOptions,
    mut fields: Vec<visualsign::SignablePayloadField>,
) -> Result<SignablePayload, VisualSignError> {
    fields.append(&mut fallback::render(payload)?);
    let title = options
        .transaction_name
        .unwrap_or_else(|| format!("Sign typed data: {}", payload.primary_type));
    Ok(SignablePayload::new(
        0,
        title,
        None,
        fields,
        "EthereumTypedData".into(),
    ))
}

fn header_fields(
    payload: &Eip712Payload,
    chain_id: u64,
    registry: &ContractRegistry,
) -> Result<Vec<visualsign::SignablePayloadField>, VisualSignError> {
    let mut out = Vec::with_capacity(4);
    out.push(
        create_text_field("Network", &networks::get_network_name(Some(chain_id)))?
            .signable_payload_field,
    );
    if let Some(vc) = payload.domain.verifying_contract {
        let name = registry.get_token_symbol(chain_id, vc);
        out.push(
            create_address_field(
                "Contract",
                &format!("{vc:?}"),
                name.as_deref(),
                None,
                None,
                None,
            )?
            .signable_payload_field,
        );
    }
    let msg_type = format!(
        "{}{} - {}",
        payload.domain.name.as_deref().unwrap_or("<unnamed>"),
        payload
            .domain
            .version
            .as_deref()
            .map(|v| format!(" v{v}"))
            .unwrap_or_default(),
        payload.primary_type,
    );
    out.push(create_text_field("Message Type", &msg_type)?.signable_payload_field);
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const UNKNOWN_PAYLOAD: &str = r#"{
      "domain": {"chainId": "0x1", "verifyingContract": "0x000000000000000000000000000000000000dead", "name": "X", "version": "1"},
      "primaryType": "Permit",
      "types": {
        "EIP712Domain": [
          {"name": "name", "type": "string"},
          {"name": "version", "type": "string"},
          {"name": "chainId", "type": "uint256"},
          {"name": "verifyingContract", "type": "address"}
        ],
        "Permit": [{"name": "value", "type": "uint256"}]
      },
      "message": {"value": "42"}
    }"#;

    #[test]
    fn unknown_payload_uses_fallback() {
        let wrapper = Eip712TransactionWrapper::from_string(UNKNOWN_PAYLOAD).unwrap();
        let conv = Eip712VisualSignConverter::new();
        let result = conv
            .to_visual_sign_payload(wrapper, VisualSignOptions::default())
            .unwrap();
        // Header (Network + Contract + Message Type = 3) + tree-walk leaf (value = 1) >= 4
        assert!(result.fields.len() >= 4);
        assert!(result.fields.iter().any(|f| matches!(
            f,
            visualsign::SignablePayloadField::TextV2 { common, .. } if common.label == "Network"
        )));
        assert!(result.fields.iter().any(|f| matches!(
            f,
            visualsign::SignablePayloadField::AddressV2 { common, .. } if common.label == "Contract"
        )));
    }
}
