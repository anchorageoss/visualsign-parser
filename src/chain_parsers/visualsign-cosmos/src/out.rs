//! Field accumulation shared by the direct and amino renderers.

use visualsign::field_builders::{create_address_field, create_amount_field, create_text_field};
use visualsign::vsptrait::VisualSignError;
use visualsign::{AnnotatedPayloadField, SignablePayloadField};

/// Fields being built for one payload.
#[derive(Default)]
pub(crate) struct Out {
    fields: Vec<AnnotatedPayloadField>,
}

impl Out {
    pub(crate) fn text(
        &mut self,
        label: &str,
        text: impl AsRef<str>,
    ) -> Result<(), VisualSignError> {
        self.fields.push(create_text_field(label, text.as_ref())?);
        Ok(())
    }

    /// An account address (bech32 on Cosmos chains; the counterparty's format
    /// for IBC receivers).
    pub(crate) fn address(&mut self, label: &str, address: &str) -> Result<(), VisualSignError> {
        self.fields.push(create_address_field(
            label, address, None, None, None, None,
        )?);
        Ok(())
    }

    /// A coin list: one amount field per coin, amounts left in the base
    /// denomination (Cosmos denoms carry no fixed decimals). A coin whose
    /// amount is not a plain number is shown as text rather than refused.
    pub(crate) fn coins(
        &mut self,
        label: &str,
        coins: &[(&str, &str)],
    ) -> Result<(), VisualSignError> {
        if coins.is_empty() {
            return self.text(label, "none");
        }
        for (amount, denom) in coins {
            match create_amount_field(label, amount, denom) {
                Ok(field) => self.fields.push(field),
                Err(_) => self.text(label, format!("{amount} {denom}"))?,
            }
        }
        Ok(())
    }

    pub(crate) fn into_fields(self) -> Vec<SignablePayloadField> {
        self.fields
            .into_iter()
            .map(|f| f.signable_payload_field)
            .collect()
    }
}

/// `prefix / label`, or just `label` at the top level.
pub(crate) fn join(prefix: &str, label: &str) -> String {
    if prefix.is_empty() {
        label.to_string()
    } else {
        format!("{prefix} / {label}")
    }
}

/// cosmos/gov `VoteOption`.
pub(crate) fn vote_option(option: i64) -> Option<&'static str> {
    Some(match option {
        1 => "Yes",
        2 => "Abstain",
        3 => "No",
        4 => "No with Veto",
        _ => return None,
    })
}

/// Deepest authz `MsgExec` nesting shown; deeper is refused.
pub const MAX_EXEC_DEPTH: usize = 3;
