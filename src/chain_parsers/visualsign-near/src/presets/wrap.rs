//! `wrap.near`: the NEAR <-> wNEAR contract.
//!
//! A NEAR dApp rather than a cross-chain protocol, so it is decoded here rather
//! than in `visualsign-intents`: nothing outside NEAR can produce a call to it.
//!
//! Keyed by account as well as method. `near_deposit` and `near_withdraw` are
//! this contract's own methods, not NEP-141 standard ones, so another contract
//! could define the same names with a different meaning -- unlike `ft_transfer`
//! and `ft_withdraw`, which mean one thing on every token and are keyed by
//! method alone in [`crate::actions`].

use serde::Deserialize;
use visualsign::SignablePayloadField;
use visualsign::errors::VisualSignError;
use visualsign::field_builders::create_amount_field;

use crate::fmt::format_near;

/// The wNEAR contract on each network.
const WRAP_CONTRACTS: [&str; 2] = ["wrap.near", "wrap.testnet"];

const NEAR_DEPOSIT: &str = "near_deposit";
const NEAR_WITHDRAW: &str = "near_withdraw";

/// wNEAR carries the same 24 decimals as NEAR, so amounts format alike.
const WNEAR_SYMBOL: &str = "wNEAR";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NearWithdrawArgs {
    amount: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The action label for a wrap contract call this build fully understands, or
/// `None`.
///
/// `None` when the args do not decode, so the payload title cannot claim more
/// than the rendered fields do -- the discipline `partially_decoded_label`
/// applies to the deploy variants, enforced here by construction rather than by
/// a separate qualifier.
#[must_use]
pub fn label(receiver_id: &str, method: &str, args: &[u8]) -> Option<&'static str> {
    if !WRAP_CONTRACTS.contains(&receiver_id) {
        return None;
    }
    match method {
        // Takes no args; the NEAR being wrapped arrives as the attached deposit.
        NEAR_DEPOSIT if args.is_empty() || serde_json::from_slice::<NoArgs>(args).is_ok() => {
            Some("Wrap")
        }
        NEAR_WITHDRAW if withdraw_amount(args).is_some() => Some("Unwrap"),
        _ => None,
    }
}

/// Decode a wrap contract call's args.
///
/// Only `near_withdraw` carries any: `near_deposit` takes none, and the NEAR it
/// wraps arrives as the attached deposit, which the FunctionCall renderer
/// already shows. Fail-closed like the standard-method decoder -- anything that
/// is not exactly the known shape returns `None` and falls back to raw data,
/// because a partially decoded arg set must not look fully understood.
pub fn decode_args(
    receiver_id: &str,
    method: &str,
    args: &[u8],
) -> Result<Option<Vec<SignablePayloadField>>, VisualSignError> {
    if !WRAP_CONTRACTS.contains(&receiver_id) || method != NEAR_WITHDRAW {
        return Ok(None);
    }
    let Some(amount) = withdraw_amount(args) else {
        return Ok(None);
    };
    Ok(Some(vec![
        create_amount_field("Amount", &format_near(amount), WNEAR_SYMBOL)?.signable_payload_field,
    ]))
}

/// The amount a `near_withdraw` moves, or `None` when the args are not exactly
/// the known shape.
///
/// The single parse behind both [`label`] and [`decode_args`]: `amount` is a
/// JSON string, so a value that deserializes can still fail to be a number, and
/// a label derived from a weaker check than the fields would let the title claim
/// an unwrap the body never rendered.
fn withdraw_amount(args: &[u8]) -> Option<u128> {
    serde_json::from_slice::<NearWithdrawArgs>(args)
        .ok()?
        .amount
        .parse::<u128>()
        .ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const ONE_NEAR: &str = "1000000000000000000000000";

    fn amount_text(fields: &[SignablePayloadField]) -> String {
        fields
            .iter()
            .find_map(|f| match f {
                SignablePayloadField::AmountV2 { common, .. } if common.label == "Amount" => {
                    Some(common.fallback_text.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no Amount field in {fields:?}"))
    }

    #[test]
    fn deposit_is_a_wrap() {
        assert_eq!(label("wrap.near", "near_deposit", b""), Some("Wrap"));
        assert_eq!(label("wrap.near", "near_deposit", b"{}"), Some("Wrap"));
    }

    #[test]
    fn withdraw_is_an_unwrap_and_renders_its_amount() {
        let args = format!(r#"{{"amount":"{ONE_NEAR}"}}"#);
        assert_eq!(
            label("wrap.near", "near_withdraw", args.as_bytes()),
            Some("Unwrap")
        );
        let fields = decode_args("wrap.near", "near_withdraw", args.as_bytes())
            .expect("decode")
            .expect("wrap.near near_withdraw is decoded");
        assert!(
            amount_text(&fields).contains("wNEAR"),
            "an unwrap moves wNEAR, not NEAR: {fields:?}"
        );
    }

    /// The testnet deployment is the same contract under another account.
    #[test]
    fn the_testnet_contract_is_recognized() {
        assert_eq!(label("wrap.testnet", "near_deposit", b""), Some("Wrap"));
    }

    /// `near_deposit` and `near_withdraw` are this contract's own names, not
    /// NEP-141 standard ones, so another contract's identically-named method
    /// must not borrow the label.
    #[test]
    fn another_contract_with_the_same_method_names_is_not_a_wrap() {
        let args = format!(r#"{{"amount":"{ONE_NEAR}"}}"#);
        assert_eq!(label("token.example.near", "near_deposit", b""), None);
        assert_eq!(
            label("token.example.near", "near_withdraw", args.as_bytes()),
            None
        );
        assert!(
            decode_args("token.example.near", "near_withdraw", args.as_bytes())
                .expect("decode")
                .is_none()
        );
    }

    /// The title may not claim more than the fields show, so args that do not
    /// decode yield no label and the call falls back to a raw-data render.
    #[test]
    fn args_that_do_not_decode_yield_no_label() {
        for bad in [
            &b"not json"[..],
            // `amount` missing
            &b"{}"[..],
            // an unknown field means this build does not understand the call
            br#"{"amount":"1","receiver_id":"alice.near"}"#,
            // an amount that is not an integer
            br#"{"amount":"lots"}"#,
        ] {
            assert_eq!(label("wrap.near", "near_withdraw", bad), None, "{bad:?}");
            assert!(
                decode_args("wrap.near", "near_withdraw", bad)
                    .expect("decode")
                    .is_none(),
                "{bad:?}"
            );
        }
    }

    /// A deposit takes no args at all, so anything present means this build does
    /// not understand the call.
    #[test]
    fn a_deposit_carrying_args_yields_no_label() {
        assert_eq!(
            label("wrap.near", "near_deposit", br#"{"amount":"1"}"#),
            None
        );
    }

    #[test]
    fn an_unrelated_method_on_the_wrap_contract_is_not_labelled() {
        assert_eq!(label("wrap.near", "ft_transfer", b"{}"), None);
    }
}
