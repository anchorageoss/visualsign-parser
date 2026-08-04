//! Per-action rendering: Transfer, FunctionCall, AddKey, etc.
//!
//! Each renderer returns the action-specific [`SignablePayloadField`]s. The
//! transaction-level fields (network, signer, receiver) are produced by the
//! converter; an action renderer contributes only what is intrinsic to the
//! action (e.g. a Transfer's amount).

use near_primitives::account::AccessKeyPermission;
use near_primitives::action::Action;
use serde::Deserialize;
use visualsign::SignablePayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_number_field, create_raw_data_field,
    create_text_field,
};

use crate::fmt::{format_near, format_tgas};

/// NEAR's native token symbol, used for `AmountV2` abbreviations.
const NEAR_SYMBOL: &str = "NEAR";

/// Render the action-specific fields for a single [`Action`].
///
/// `total_actions` is the number of actions in the transaction this action
/// belongs to. When there is more than one, `Transfer` and `FunctionCall`
/// (the two variants decoded in detail) are prefixed with an `"Action"`
/// boundary field, the same label the fallback branch below always uses --
/// otherwise a multi-action transaction with e.g. two transfers renders two
/// identically-labelled `"Amount"` fields with no indication which action
/// each belongs to.
pub fn render_action(
    action: &Action,
    total_actions: usize,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    match action {
        Action::Transfer(transfer) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            let amount = format_near(transfer.deposit.as_yoctonear());
            fields
                .push(create_amount_field("Amount", &amount, NEAR_SYMBOL)?.signable_payload_field);
            Ok(fields)
        }
        Action::FunctionCall(fc) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_text_field("Method", &charset_safe(&fc.method_name))?.signable_payload_field,
            );
            match decode_known_method_args(&fc.method_name, &fc.args)? {
                Some(args_fields) => fields.extend(args_fields),
                None if !fc.args.is_empty() => {
                    fields.push(create_raw_data_field(&fc.args, None)?.signable_payload_field);
                }
                None => {}
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
        Action::CreateAccount(_) => Ok(vec![
            create_text_field("Action", action_label(action))?.signable_payload_field,
        ]),
        Action::DeleteAccount(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_address_field(
                    "Beneficiary",
                    a.beneficiary_id.as_str(),
                    None,
                    None,
                    None,
                    None,
                )?
                .signable_payload_field,
            );
            Ok(fields)
        }
        Action::AddKey(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_text_field("Public Key", &a.public_key.to_string())?.signable_payload_field,
            );
            fields.push(
                create_text_field("Permission", permission_label(&a.access_key.permission))?
                    .signable_payload_field,
            );
            Ok(fields)
        }
        Action::DeleteKey(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_text_field("Public Key", &a.public_key.to_string())?.signable_payload_field,
            );
            Ok(fields)
        }
        Action::Stake(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_amount_field("Stake", &format_near(a.stake.as_yoctonear()), NEAR_SYMBOL)?
                    .signable_payload_field,
            );
            fields.push(
                create_text_field("Validator Public Key", &a.public_key.to_string())?
                    .signable_payload_field,
            );
            Ok(fields)
        }
        Action::TransferToGasKey(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_amount_field(
                    "Amount",
                    &format_near(a.deposit.as_yoctonear()),
                    NEAR_SYMBOL,
                )?
                .signable_payload_field,
            );
            Ok(fields)
        }
        Action::WithdrawFromGasKey(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_amount_field("Amount", &format_near(a.amount.as_yoctonear()), NEAR_SYMBOL)?
                    .signable_payload_field,
            );
            Ok(fields)
        }
        Action::DeterministicStateInit(a) => {
            let mut fields = action_boundary_field(action, total_actions)?;
            fields.push(
                create_amount_field(
                    "Deposit",
                    &format_near(a.deposit.as_yoctonear()),
                    NEAR_SYMBOL,
                )?
                .signable_payload_field,
            );
            // `state_init` (the derived account's code/data) has no cheap
            // field-level render, same as DeployContract's raw wasm; flag it
            // rather than implying the deposit is the whole picture.
            fields.push(
                create_text_field("State Init", "(not fully decoded)")?.signable_payload_field,
            );
            Ok(fields)
        }
        // A NEP-366 meta-transaction: `sender_id`/`receiver_id` distinct from
        // the outer transaction's, and an arbitrary nested action batch
        // (including FunctionCall/AddKey/DeleteAccount) executed on the
        // sender's behalf. Nothing in this stack constructs or requires one;
        // refuse rather than risk a partial render of a nested transaction.
        Action::Delegate(_) => Err(VisualSignError::ValidationError(
            "NEAR Delegate (meta-transaction) actions are not supported for signing display"
                .to_string(),
        )),
        // No cheap field-level render available (raw wasm / a contract
        // identifier); surface that the label is incomplete rather than
        // implying a full understanding of the action. This is already an
        // unconditional "Action" field, so it needs no total_actions-gated
        // boundary marker of its own.
        other @ (Action::DeployContract(_)
        | Action::DeployGlobalContract(_)
        | Action::UseGlobalContract(_)) => Ok(vec![
            create_text_field(
                "Action",
                &format!("{} (not fully decoded)", action_label(other)),
            )?
            .signable_payload_field,
        ]),
    }
}

/// The `"Action"` boundary field prepended to a decoded action's own fields
/// when `total_actions > 1`; empty otherwise, since a single-action
/// transaction's fields need no boundary to disambiguate.
fn action_boundary_field(
    action: &Action,
    total_actions: usize,
) -> Result<Vec<SignablePayloadField>, VisualSignError> {
    if total_actions > 1 {
        Ok(vec![
            create_text_field("Action", action_label(action))?.signable_payload_field,
        ])
    } else {
        Ok(Vec::new())
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
/// included) returns `None`, leaving the caller to fall back to a raw-data
/// field -- a partially decoded arg set must not masquerade as a fully
/// understood call.
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
                create_address_field(
                    "Recipient",
                    &charset_safe(&parsed.receiver_id),
                    None,
                    None,
                    None,
                    None,
                )?
                .signable_payload_field,
            );
            push_amount_and_notes(&mut fields, &parsed.amount, &parsed.memo, &parsed.msg)?;
        }
        "ft_withdraw" => {
            let Ok(parsed) = serde_json::from_slice::<FtWithdrawArgs>(args) else {
                return Ok(None);
            };
            fields.push(
                create_address_field(
                    "Token",
                    &charset_safe(&parsed.token),
                    None,
                    None,
                    None,
                    None,
                )?
                .signable_payload_field,
            );
            fields.push(
                create_address_field(
                    "Recipient",
                    &charset_safe(&parsed.receiver_id),
                    None,
                    None,
                    None,
                    None,
                )?
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
        fields.push(create_text_field("Memo", &charset_safe(memo))?.signable_payload_field);
    }
    if let Some(msg) = msg.as_deref().filter(|m| !m.is_empty()) {
        fields.push(create_text_field("Message", &charset_safe(msg))?.signable_payload_field);
    }
    Ok(())
}

/// Strips everything except printable ASCII and spaces, so a transaction's
/// JSON args (`memo`, `msg`, `method_name`) cannot smuggle a newline into a
/// text field's fallback text. The core crate's charset validator permits
/// `\n` as the wallet's documented multi-line separator, so an unfiltered
/// attacker-controlled string can render as extra apparent confirmed fields
/// on the signing screen.
fn charset_safe(text: &str) -> String {
    text.chars()
        .filter(|&c| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\'))
        .collect()
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

/// Human-readable label for an access key's permission scope.
fn permission_label(permission: &AccessKeyPermission) -> &'static str {
    match permission {
        AccessKeyPermission::FullAccess => "Full Access",
        AccessKeyPermission::FunctionCall(_) => "Function Call (Restricted)",
        AccessKeyPermission::GasKeyFullAccess(_) => "Gas Key (Full Access)",
        AccessKeyPermission::GasKeyFunctionCall(..) => "Gas Key (Function Call, Restricted)",
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
        let fields = render_action(&action, 1).expect("render");
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
        let fields = render_action(&action, 1).expect("render");
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
        let fields = render_action(&action, 1).expect("render");
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
    fn undecodable_or_unknown_args_fall_back_to_raw_data() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        for (method, args) in [
            ("ft_transfer_call", &b"not json"[..]),
            // Unknown extra field: partially understood args must not render
            // as if fully decoded, but the raw bytes still surface.
            (
                "ft_transfer_call",
                &br#"{"receiver_id":"a.near","amount":"1","extra":true}"#[..],
            ),
            // Unknown method: args stay opaque but visible.
            ("do_something", &br#"{"receiver_id":"a.near"}"#[..]),
        ] {
            let action = Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: method.to_string(),
                args: args.to_vec(),
                gas: Gas::from_gas(1_000_000_000_000),
                deposit: Balance::from_yoctonear(0),
            }));
            let fields = render_action(&action, 1).expect("render");
            let labels: Vec<&str> = fields.iter().map(field_label).collect();
            assert_eq!(labels, ["Method", "Raw Data", "Gas"], "case: {method}");
        }
    }

    #[test]
    fn empty_args_render_no_raw_data_field() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "do_something".to_string(),
            args: Vec::new(),
            gas: Gas::from_gas(1_000_000_000_000),
            deposit: Balance::from_yoctonear(0),
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Method", "Gas"]);
    }

    #[test]
    fn method_name_with_embedded_newline_is_sanitized() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "deposit\nAmount: 0.000001 NEAR".to_string(),
            args: Vec::new(),
            gas: Gas::from_gas(1_000_000_000_000),
            deposit: Balance::from_yoctonear(0),
        }));
        let fields = render_action(&action, 1).expect("render");
        let SignablePayloadField::TextV2 { text_v2, .. } = &fields[0] else {
            panic!("expected TextV2, got {:?}", fields[0]);
        };
        assert!(!text_v2.text.contains('\n'));
        assert_eq!(text_v2.text, "depositAmount: 0.000001 NEAR");
    }

    #[test]
    fn msg_with_embedded_newline_is_sanitized() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "ft_transfer_call".to_string(),
            args: br#"{"receiver_id":"attacker.near","amount":"1000000000","msg":"deposit\nAmount: 0.000001 NEAR\nRecipient: alice.near"}"#.to_vec(),
            gas: Gas::from_gas(100_000_000_000_000),
            deposit: Balance::from_yoctonear(1),
        }));
        let fields = render_action(&action, 1).expect("render");
        let message_field = fields
            .iter()
            .find(|f| field_label(f) == "Message")
            .expect("Message field present");
        let SignablePayloadField::TextV2 { text_v2, .. } = message_field else {
            panic!("expected TextV2, got {message_field:?}");
        };
        assert!(!text_v2.text.contains('\n'));
    }

    #[test]
    fn receiver_id_and_token_with_embedded_newline_are_sanitized() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "ft_withdraw".to_string(),
            args: br#"{"token":"wrap.near\nAmount: 999 NEAR","receiver_id":"bob.near\nAmount: 999 NEAR","amount":"7"}"#.to_vec(),
            gas: Gas::from_gas(100_000_000_000_000),
            deposit: Balance::from_yoctonear(1),
        }));
        let fields = render_action(&action, 1).expect("render");
        for label in ["Token", "Recipient"] {
            let field = fields
                .iter()
                .find(|f| field_label(f) == label)
                .unwrap_or_else(|| panic!("{label} field present"));
            let SignablePayloadField::AddressV2 { common, address_v2 } = field else {
                panic!("expected AddressV2, got {field:?}");
            };
            assert!(!common.fallback_text.contains('\n'), "field: {label}");
            assert!(!address_v2.address.contains('\n'), "field: {label}");
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
        let fields = render_action(&action, 1).expect("render");
        assert_eq!(fields.len(), 1);
        match &fields[0] {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Action");
                assert_eq!(text_v2.text, "Create Account");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    // Regression coverage for a rendering-ambiguity gap: without a boundary
    // marker, two Transfers in one transaction would both render as a bare
    // "Amount" field with nothing distinguishing which action each belongs
    // to.
    #[test]
    fn multi_action_transfer_gets_action_boundary_label() {
        let action = Action::Transfer(TransferAction {
            deposit: Balance::from_yoctonear(1_500_000_000_000_000_000_000_000),
        });
        let fields = render_action(&action, 2).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Action", "Amount"]);
        match &fields[0] {
            SignablePayloadField::TextV2 { text_v2, .. } => assert_eq!(text_v2.text, "Transfer"),
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn multi_action_function_call_gets_action_boundary_label() {
        use near_primitives::action::FunctionCallAction;
        use near_primitives::types::Gas;
        let action = Action::FunctionCall(Box::new(FunctionCallAction {
            method_name: "do_something".to_string(),
            args: Vec::new(),
            gas: Gas::from_gas(1_000_000_000_000),
            deposit: Balance::from_yoctonear(0),
        }));
        let fields = render_action(&action, 2).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Action", "Method", "Gas"]);
        match &fields[0] {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "Function Call");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn single_action_transfer_has_no_boundary_label() {
        let action = Action::Transfer(TransferAction {
            deposit: Balance::from_yoctonear(1),
        });
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Amount"]);
    }

    #[test]
    fn delete_account_renders_beneficiary() {
        use near_primitives::action::DeleteAccountAction;
        let action = Action::DeleteAccount(DeleteAccountAction {
            beneficiary_id: "bob.near".parse().expect("valid account id"),
        });
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Beneficiary"]);
        match &fields[0] {
            SignablePayloadField::AddressV2 { address_v2, .. } => {
                assert_eq!(address_v2.address, "bob.near");
            }
            other => panic!("expected AddressV2, got {other:?}"),
        }
    }

    #[test]
    fn add_key_full_access_renders_public_key_and_permission() {
        use near_crypto::{KeyType, PublicKey};
        use near_primitives::account::{AccessKey, AccessKeyPermission};
        use near_primitives::action::AddKeyAction;
        let action = Action::AddKey(Box::new(AddKeyAction {
            public_key: PublicKey::empty(KeyType::ED25519),
            access_key: AccessKey {
                nonce: 0,
                permission: AccessKeyPermission::FullAccess,
            },
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Public Key", "Permission"]);
        match &fields[1] {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "Full Access");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn add_key_function_call_permission_is_labeled_restricted() {
        use near_crypto::{KeyType, PublicKey};
        use near_primitives::account::{AccessKey, AccessKeyPermission, FunctionCallPermission};
        use near_primitives::action::AddKeyAction;
        let action = Action::AddKey(Box::new(AddKeyAction {
            public_key: PublicKey::empty(KeyType::ED25519),
            access_key: AccessKey {
                nonce: 0,
                permission: AccessKeyPermission::FunctionCall(FunctionCallPermission {
                    allowance: None,
                    receiver_id: "contract.near".to_string(),
                    method_names: Vec::new(),
                }),
            },
        }));
        let fields = render_action(&action, 1).expect("render");
        match &fields[1] {
            SignablePayloadField::TextV2 { text_v2, .. } => {
                assert_eq!(text_v2.text, "Function Call (Restricted)");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn delete_key_renders_public_key() {
        use near_crypto::{KeyType, PublicKey};
        use near_primitives::action::DeleteKeyAction;
        let action = Action::DeleteKey(Box::new(DeleteKeyAction {
            public_key: PublicKey::empty(KeyType::ED25519),
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Public Key"]);
    }

    #[test]
    fn stake_renders_stake_amount_and_validator_key() {
        use near_crypto::{KeyType, PublicKey};
        use near_primitives::action::StakeAction;
        let action = Action::Stake(Box::new(StakeAction {
            stake: Balance::from_yoctonear(1_000_000_000_000_000_000_000_000),
            public_key: PublicKey::empty(KeyType::ED25519),
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Stake", "Validator Public Key"]);
    }

    #[test]
    fn transfer_to_gas_key_renders_amount() {
        use near_crypto::{KeyType, PublicKey};
        use near_primitives::action::TransferToGasKeyAction;
        let action = Action::TransferToGasKey(Box::new(TransferToGasKeyAction {
            public_key: PublicKey::empty(KeyType::ED25519),
            deposit: Balance::from_yoctonear(1_000_000_000_000_000_000_000_000),
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Amount"]);
    }

    #[test]
    fn withdraw_from_gas_key_renders_amount() {
        use near_crypto::{KeyType, PublicKey};
        use near_primitives::action::WithdrawFromGasKeyAction;
        let action = Action::WithdrawFromGasKey(Box::new(WithdrawFromGasKeyAction {
            public_key: PublicKey::empty(KeyType::ED25519),
            amount: Balance::from_yoctonear(1_000_000_000_000_000_000_000_000),
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Amount"]);
    }

    #[test]
    fn deterministic_state_init_renders_deposit_and_flags_state_init_as_undecoded() {
        use near_primitives::action::DeterministicStateInitAction;
        use near_primitives::deterministic_account_id::{
            DeterministicAccountStateInit, DeterministicAccountStateInitV1,
        };
        use near_primitives::global_contract::GlobalContractIdentifier;
        use near_primitives::hash::CryptoHash;
        let action = Action::DeterministicStateInit(Box::new(DeterministicStateInitAction {
            state_init: DeterministicAccountStateInit::V1(DeterministicAccountStateInitV1 {
                code: GlobalContractIdentifier::CodeHash(CryptoHash::default()),
                data: Default::default(),
            }),
            deposit: Balance::from_yoctonear(1_000_000_000_000_000_000_000_000),
        }));
        let fields = render_action(&action, 1).expect("render");
        let labels: Vec<&str> = fields.iter().map(field_label).collect();
        assert_eq!(labels, ["Deposit", "State Init"]);
        let state_init_field = &fields[1];
        let SignablePayloadField::TextV2 { text_v2, .. } = state_init_field else {
            panic!("expected TextV2, got {state_init_field:?}");
        };
        assert!(text_v2.text.contains("not fully decoded"));
    }

    #[test]
    fn deploy_contract_label_is_marked_not_fully_decoded() {
        use near_primitives::action::DeployContractAction;
        let action = Action::DeployContract(DeployContractAction { code: vec![0u8; 4] });
        let fields = render_action(&action, 1).expect("render");
        assert_eq!(fields.len(), 1);
        match &fields[0] {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                assert_eq!(common.label, "Action");
                assert_eq!(text_v2.text, "Deploy Contract (not fully decoded)");
            }
            other => panic!("expected TextV2, got {other:?}"),
        }
    }

    #[test]
    fn delegate_action_is_refused() {
        use near_crypto::{KeyType, PublicKey, Signature};
        use near_primitives::action::delegate::{DelegateAction, SignedDelegateAction};
        let action = Action::Delegate(Box::new(SignedDelegateAction {
            delegate_action: DelegateAction {
                sender_id: "alice.near".parse().expect("valid account id"),
                receiver_id: "contract.near".parse().expect("valid account id"),
                actions: Vec::new(),
                nonce: 0,
                max_block_height: 0,
                public_key: PublicKey::empty(KeyType::ED25519),
            },
            signature: Signature::empty(KeyType::ED25519),
        }));
        let result = render_action(&action, 1);
        assert!(matches!(result, Err(VisualSignError::ValidationError(_))));
    }
}
