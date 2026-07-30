//! Per-action rendering: Transfer, FunctionCall, AddKey, etc.
//!
//! Each renderer returns the action-specific [`SignablePayloadField`]s. The
//! transaction-level fields (network, signer, receiver) are produced by the
//! converter; an action renderer contributes only what is intrinsic to the
//! action (e.g. a Transfer's amount).

use near_primitives::action::Action;
use serde::Deserialize;
use visualsign::SignablePayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_number_field, create_text_field,
};

use crate::fmt::{format_near, format_tgas};

/// NEAR's native token symbol, used for `AmountV2` abbreviations.
const NEAR_SYMBOL: &str = "NEAR";

/// Render the action-specific fields for a single [`Action`].
pub fn render_action(action: &Action) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    match action {
        Action::Transfer(transfer) => {
            let amount = format_near(transfer.deposit.as_yoctonear());
            Ok(vec![
                create_amount_field("Amount", &amount, NEAR_SYMBOL)?.signable_payload_field,
            ])
        }
        Action::FunctionCall(fc) => {
            let mut fields =
                vec![create_text_field("Method", &fc.method_name)?.signable_payload_field];
            if let Some(args_fields) = decode_known_method_args(&fc.method_name, &fc.args)? {
                fields.extend(args_fields);
            }
            let deposit = fc.deposit.as_yoctonear();
            if deposit > 0 {
                fields.push(
                    create_amount_field("Deposit", &format_near(deposit), NEAR_SYMBOL)?
                        .signable_payload_field,
                );
            }
            fields.push(
                create_text_field("Gas", &format!("{} Tgas", format_tgas(fc.gas.as_gas())))?
                    .signable_payload_field,
            );
            Ok(fields)
        }
        // Other variants are not yet decoded in detail; surface the action
        // kind so the payload still names what is being signed.
        other => Ok(vec![
            create_text_field("Action", action_label(other))?.signable_payload_field,
        ]),
    }
}

/// NEP-141 `ft_transfer` / `ft_transfer_call` args (the deposit flow calls
/// the token contract with the verifier as `receiver_id`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FtTransferArgs {
    receiver_id: String,
    amount: String,
    #[serde(default)]
    memo: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

/// Verifier `ft_withdraw` args (the withdraw flow calls the verifier).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FtWithdrawArgs {
    token: String,
    receiver_id: String,
    amount: String,
    #[serde(default)]
    memo: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

/// Decode args for the token-movement methods a deposit/withdraw flow uses,
/// so the true beneficiary is displayed rather than opaque bytes. Fail-closed:
/// anything that does not parse as exactly the known shape (unknown fields
/// included) renders nothing extra, leaving the generic view -- a partially
/// decoded arg set must not masquerade as a fully understood call.
fn decode_known_method_args(
    method: &str,
    args: &[u8],
) -> Result<Option<Vec<SignablePayloadField>>, VisualSignError> {
    let mut fields = Vec::new();
    match method {
        "ft_transfer" | "ft_transfer_call" => {
            let Ok(parsed) = serde_json::from_slice::<FtTransferArgs>(args) else {
                return Ok(None);
            };
            fields.push(
                create_address_field("Recipient", &parsed.receiver_id, None, None, None, None)?
                    .signable_payload_field,
            );
            push_amount_and_notes(&mut fields, &parsed.amount, &parsed.memo, &parsed.msg)?;
        }
        "ft_withdraw" => {
            let Ok(parsed) = serde_json::from_slice::<FtWithdrawArgs>(args) else {
                return Ok(None);
            };
            fields.push(
                create_address_field("Token", &parsed.token, None, None, None, None)?
                    .signable_payload_field,
            );
            fields.push(
                create_address_field("Recipient", &parsed.receiver_id, None, None, None, None)?
                    .signable_payload_field,
            );
            push_amount_and_notes(&mut fields, &parsed.amount, &parsed.memo, &parsed.msg)?;
        }
        _ => return Ok(None),
    }
    Ok(Some(fields))
}

/// Shared tail of the decoded-args fields: the raw token amount plus any
/// non-empty memo/msg. Amounts stay in raw token units -- decimals belong to
/// per-token metadata, which has no trustworthy source here yet. The number
/// field validates the amount is numeric, so a malformed amount rejects the
/// payload instead of rendering.
fn push_amount_and_notes(
    fields: &mut Vec<SignablePayloadField>,
    amount: &str,
    memo: &Option<String>,
    msg: &Option<String>,
) -> Result<(), VisualSignError> {
    fields.push(create_number_field("Amount", amount, "raw token units")?.signable_payload_field);
    if let Some(memo) = memo.as_deref().filter(|m| !m.is_empty()) {
        fields.push(create_text_field("Memo", memo)?.signable_payload_field);
    }
    if let Some(msg) = msg.as_deref().filter(|m| !m.is_empty()) {
        fields.push(create_text_field("Message", msg)?.signable_payload_field);
    }
    Ok(())
}

/// Human-readable label for an action variant.
pub(crate) fn action_label(action: &Action) -> &'static str {
    match action {
        Action::CreateAccount(_) => "Create Account",
        Action::DeployContract(_) => "Deploy Contract",
        Action::FunctionCall(_) => "Function Call",
        Action::Transfer(_) => "Transfer",
        Action::Stake(_) => "Stake",
        Action::AddKey(_) => "Add Key",
        Action::DeleteKey(_) => "Delete Key",
        Action::DeleteAccount(_) => "Delete Account",
        Action::Delegate(_) => "Delegate",
        Action::DeployGlobalContract(_) => "Deploy Global Contract",
        Action::UseGlobalContract(_) => "Use Global Contract",
        Action::DeterministicStateInit(_) => "Deterministic State Init",
        Action::TransferToGasKey(_) => "Transfer To Gas Key",
        Action::WithdrawFromGasKey(_) => "Withdraw From Gas Key",
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use near_primitives::action::{CreateAccountAction, TransferAction};
    use near_primitives::types::Balance;
    use visualsign::SignablePayloadField;

    #[test]
    fn transfer_renders_single_amount_field() {
        let action = Action::Transfer(TransferAction {
            deposit: Balance::from_yoctonear(1_500_000_000_000_000_000_000_000),
        });
        let fields = render_action(&action).expect("render");
        assert_eq!(fields.len(), 1);
        match &fields[0] {
            SignablePayloadField::AmountV2 { common, amount_v2 } => {
                assert_eq!(amount_v2.amount, "1.5");
                assert_eq!(amount_v2.abbreviation.as_deref(), Some("NEAR"));
                assert_eq!(common.label, "Amount");
                assert_eq!(common.fallback_text, "1.5 NEAR");
            }
            other => panic!("expected AmountV2, got {other:?}"),
        }
    }

    #[test]
    fn ft_transfer_call_args_render_recipient_and_amount() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "ft_transfer_call".to_string(),
            args: br#"{"receiver_id":"intents.near","amount":"1000000","msg":""}"#.to_vec(),
            gas: Gas::from_gas(100_000_000_000_000),
            deposit: Balance::from_yoctonear(1),
        }));
        let fields = render_action(&action).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Method", "Recipient", "Amount", "Deposit", "Gas"]);
        // Empty msg is skipped, not rendered as an empty field.
        assert!(!labels.contains(&"Message"));
    }

    #[test]
    fn ft_withdraw_args_render_token_recipient_amount_and_msg() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "ft_withdraw".to_string(),
            args: br#"{"token":"wrap.near","receiver_id":"bob.near","amount":"7","msg":"0xdead"}"#
                .to_vec(),
            gas: Gas::from_gas(100_000_000_000_000),
            deposit: Balance::from_yoctonear(1),
        }));
        let fields = render_action(&action).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(
            labels,
            [
                "Method",
                "Token",
                "Recipient",
                "Amount",
                "Message",
                "Deposit",
                "Gas"
            ]
        );
    }

    #[test]
    fn undecodable_or_unknown_args_fall_back_to_generic_view() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        for (method, args) in [
            ("ft_transfer_call", &b"not json"[..]),
            // Unknown extra field: partially understood args must not render.
            (
                "ft_transfer_call",
                &br#"{"receiver_id":"a.near","amount":"1","extra":true}"#[..],
            ),
            // Unknown method: args stay opaque.
            ("do_something", &br#"{"receiver_id":"a.near"}"#[..]),
        ] {
            let action = Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: method.to_string(),
                args: args.to_vec(),
                gas: Gas::from_gas(1_000_000_000_000),
                deposit: Balance::from_yoctonear(0),
            }));
            let fields = render_action(&action).expect("render");
            let labels: Vec<&str> = fields.iter().map(field_label).collect();
            assert_eq!(labels, ["Method", "Gas"], "case: {method}");
        }
    }

    fn field_label(field: &SignablePayloadField) -> &str {
        match field {
            SignablePayloadField::TextV2 { common, .. }
            | SignablePayloadField::AmountV2 { common, .. }
            | SignablePayloadField::AddressV2 { common, .. }
            | SignablePayloadField::Number { common, .. } => &common.label,
            other => panic!("unexpected field variant: {other:?}"),
        }
    }

    #[test]
    fn unsupported_action_renders_label_text_field() {
        let action = Action::CreateAccount(CreateAccountAction {});
        let fields = render_action(&action).expect("render");
        assert_eq!(fields.len(), 1);
        match &fields[0] {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Action");
                assert_eq!(text_v2.text, "Create Account");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }
}
