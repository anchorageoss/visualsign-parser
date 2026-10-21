//! `SIGN_MODE_LEGACY_AMINO_JSON`: the `StdSignDoc` JSON that the signer's key
//! signs.
//!
//! The SDK signs `MustSortJSON(json.Marshal(doc))`: keys sorted, no
//! whitespace, Go string escaping (`<`, `>`, `&`, U+2028, U+2029 as `\u`
//! escapes). The parser re-serializes the parsed document the same way and
//! refuses input that is not byte-identical, so the bytes signed are exactly
//! the document shown. Strings with control characters other than `\n`, `\r`,
//! `\t`, and numbers other than small integers, are refused rather than
//! guessed at.

use crate::out::{MAX_EXEC_DEPTH, Out, join, vote_option};
use serde_json::Value;
use std::fmt::Write as _;
use visualsign::SignablePayloadField;
use visualsign::vsptrait::{TransactionParseError, VisualSignError};

fn malformed(what: impl Into<String>) -> VisualSignError {
    VisualSignError::ConversionError(format!("amino JSON: {}", what.into()))
}

/// Largest integer Go's float64 round trip keeps exact.
const MAX_SAFE_INT: u64 = 1 << 53;

fn write_string(s: &str, out: &mut String) -> Result<(), String> {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c if u32::from(c) < 0x20 => return Err("control character in string".to_string()),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(())
}

/// Go `json.Marshal` of the value decoded into `interface{}`, keys sorted.
fn canonical(v: &Value, out: &mut String) -> Result<(), String> {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let ok = n.as_u64().is_some_and(|u| u <= MAX_SAFE_INT)
                || n.as_i64().is_some_and(|i| i.unsigned_abs() <= MAX_SAFE_INT);
            if !ok {
                return Err("number outside the exact integer range".to_string());
            }
            let _ = write!(out, "{n}");
        }
        Value::String(s) => write_string(s, out)?,
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out)?;
            }
            out.push(']');
        }
        Value::Object(m) => {
            // Sort explicitly: serde_json's map order depends on features.
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out)?;
                out.push(':');
                canonical(&m[k], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// A parsed `StdSignDoc`, known to be byte-identical to its canonical form.
#[derive(Debug, Clone)]
pub(crate) struct AminoDoc {
    value: Value,
}

impl AminoDoc {
    pub(crate) fn parse(text: &str) -> Result<Self, TransactionParseError> {
        let invalid =
            |what: String| TransactionParseError::DecodeError(format!("amino JSON: {what}"));
        let value: Value = serde_json::from_str(text).map_err(|e| invalid(e.to_string()))?;
        let mut canon = String::with_capacity(text.len());
        canonical(&value, &mut canon).map_err(invalid)?;
        if canon != text {
            return Err(invalid(
                "not canonical (keys sorted, no whitespace, Go escaping)".to_string(),
            ));
        }
        Ok(Self { value })
    }

    /// Render a canonical `StdSignDoc`.
    pub(crate) fn render(&self) -> Result<Vec<SignablePayloadField>, VisualSignError> {
        let mut doc = Obj::new(&self.value, "sign doc")?;
        let mut out = Out::default();
        out.text("Chain ID", doc.str("chain_id")?)?;
        out.text("Account Number", doc.int("account_number")?)?;
        out.text("Sequence", doc.int("sequence")?)?;
        match doc.get("msgs") {
            Some(Value::Array(a)) if !a.is_empty() => {
                for (i, m) in a.iter().enumerate() {
                    render_msg(m, &format!("Message {}", i + 1), 0, &mut out)?;
                }
            }
            _ => return Err(malformed("no messages")),
        }
        let memo = doc.str("memo")?;
        if !memo.is_empty() {
            out.text("Memo", memo)?;
        }
        let fee = doc.get("fee").ok_or_else(|| malformed("missing fee"))?;
        let mut fee = Obj::new(fee, "fee")?;
        out.coins("Fee", &fee.coins("amount")?.pairs())?;
        out.text("Gas Limit", fee.int("gas")?)?;
        let payer = fee.str("payer")?;
        if !payer.is_empty() {
            out.address("Fee Payer", &payer)?;
        }
        let granter = fee.str("granter")?;
        if !granter.is_empty() {
            out.address("Fee Granter", &granter)?;
        }
        fee.finish("Fee", &mut out)?;
        if doc.map.contains_key("timeout_height") {
            out.text("Timeout Height", doc.int("timeout_height")?)?;
        }
        doc.finish("", &mut out)?;
        Ok(out.into_fields())
    }
}

/// Owned coin list read from amino JSON.
struct CoinList(Vec<(String, String)>);

impl CoinList {
    fn pairs(&self) -> Vec<(&str, &str)> {
        self.0
            .iter()
            .map(|(a, d)| (a.as_str(), d.as_str()))
            .collect()
    }
}

/// A JSON object, with each key looked up at most once; leftover keys are
/// shown as unrecognized.
struct Obj<'a> {
    map: &'a serde_json::Map<String, Value>,
    used: Vec<&'static str>,
}

impl<'a> Obj<'a> {
    fn new(v: &'a Value, what: &str) -> Result<Self, VisualSignError> {
        match v {
            Value::Object(map) => Ok(Self {
                map,
                used: Vec::new(),
            }),
            _ => Err(malformed(format!("{what} is not an object"))),
        }
    }

    fn get(&mut self, k: &'static str) -> Option<&'a Value> {
        self.used.push(k);
        self.map.get(k)
    }

    /// String field; amino omits empty strings.
    fn str(&mut self, k: &'static str) -> Result<String, VisualSignError> {
        match self.get(k) {
            None => Ok(String::new()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(malformed(format!("{k} is not a string"))),
        }
    }

    /// Integer that amino may write as a string or a number.
    fn int(&mut self, k: &'static str) -> Result<String, VisualSignError> {
        match self.get(k) {
            None => Ok("0".into()),
            Some(Value::String(s)) if !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) => {
                Ok(s.clone())
            }
            Some(Value::Number(n)) if n.is_u64() => Ok(n.to_string()),
            Some(_) => Err(malformed(format!("{k} is not an integer"))),
        }
    }

    fn coins(&mut self, k: &'static str) -> Result<CoinList, VisualSignError> {
        match self.get(k) {
            None | Some(Value::Null) => Ok(CoinList(Vec::new())),
            Some(Value::Array(a)) => {
                let mut coins = Vec::new();
                for c in a {
                    coins.push(coin(c)?);
                }
                Ok(CoinList(coins))
            }
            Some(v @ Value::Object(_)) => Ok(CoinList(vec![coin(v)?])),
            Some(_) => Err(malformed(format!("{k} is not a coin list"))),
        }
    }

    /// Show keys that were never looked up.
    fn finish(self, prefix: &str, out: &mut Out) -> Result<(), VisualSignError> {
        let mut extra: Vec<(&String, &Value)> = self
            .map
            .iter()
            .filter(|(k, _)| !self.used.contains(&k.as_str()))
            .collect();
        extra.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in extra {
            out.text(
                &join(prefix, &format!("Unrecognized Field {k}")),
                v.to_string(),
            )?;
        }
        Ok(())
    }

    /// Refuse keys that were never looked up (inside small fixed shapes).
    fn finish_strict(self, what: &str) -> Result<(), VisualSignError> {
        if self.map.keys().any(|k| !self.used.contains(&k.as_str())) {
            return Err(malformed(format!("unexpected key in {what}")));
        }
        Ok(())
    }
}

fn coin(v: &Value) -> Result<(String, String), VisualSignError> {
    let mut o = Obj::new(v, "coin")?;
    let pair = (o.str("amount")?, o.str("denom")?);
    o.finish_strict("coin")?;
    Ok(pair)
}

fn render_msg(v: &Value, prefix: &str, depth: usize, out: &mut Out) -> Result<(), VisualSignError> {
    let mut msg = Obj::new(v, "message")?;
    let ty = msg.str("type")?;
    let value = msg
        .get("value")
        .ok_or_else(|| malformed("message without value"))?;
    msg.finish_strict("message")?;
    let p = |l: &str| join(prefix, l);
    let mut m = Obj::new(value, "message value")?;
    match ty.as_str() {
        "cosmos-sdk/MsgSend" => {
            out.text(&p("Type"), "Send")?;
            out.address(&p("From"), &m.str("from_address")?)?;
            out.address(&p("To"), &m.str("to_address")?)?;
            out.coins(&p("Amount"), &m.coins("amount")?.pairs())?;
        }
        "cosmos-sdk/MsgDelegate" | "cosmos-sdk/MsgUndelegate" => {
            out.text(
                &p("Type"),
                if ty.ends_with("MsgDelegate") {
                    "Delegate"
                } else {
                    "Undelegate"
                },
            )?;
            out.address(&p("Delegator"), &m.str("delegator_address")?)?;
            out.address(&p("Validator"), &m.str("validator_address")?)?;
            out.coins(&p("Amount"), &m.coins("amount")?.pairs())?;
        }
        "cosmos-sdk/MsgBeginRedelegate" => {
            out.text(&p("Type"), "Redelegate")?;
            out.address(&p("Delegator"), &m.str("delegator_address")?)?;
            out.address(&p("From Validator"), &m.str("validator_src_address")?)?;
            out.address(&p("To Validator"), &m.str("validator_dst_address")?)?;
            out.coins(&p("Amount"), &m.coins("amount")?.pairs())?;
        }
        "cosmos-sdk/MsgWithdrawDelegationReward" => {
            out.text(&p("Type"), "Withdraw Rewards")?;
            out.address(&p("Delegator"), &m.str("delegator_address")?)?;
            out.address(&p("Validator"), &m.str("validator_address")?)?;
        }
        "cosmos-sdk/MsgVote" | "cosmos-sdk/v1/MsgVote" => {
            out.text(&p("Type"), "Vote")?;
            out.text(&p("Proposal"), m.int("proposal_id")?)?;
            out.address(&p("Voter"), &m.str("voter")?)?;
            let option = match m.get("option") {
                Some(Value::Number(n)) => n.as_i64().and_then(vote_option),
                Some(Value::String(s)) => match s.as_str() {
                    "VOTE_OPTION_YES" => Some("Yes"),
                    "VOTE_OPTION_ABSTAIN" => Some("Abstain"),
                    "VOTE_OPTION_NO" => Some("No"),
                    "VOTE_OPTION_NO_WITH_VETO" => Some("No with Veto"),
                    _ => None,
                },
                _ => None,
            };
            match option {
                Some(o) => out.text(&p("Option"), o)?,
                None => out.text(&p("Option"), "unrecognized")?,
            }
            let metadata = m.str("metadata")?;
            if !metadata.is_empty() {
                out.text(&p("Metadata"), metadata)?;
            }
        }
        "cosmos-sdk/MsgExec" => {
            if depth >= MAX_EXEC_DEPTH {
                return Err(VisualSignError::ConversionError(
                    "authz MsgExec nested too deep".to_string(),
                ));
            }
            out.text(&p("Type"), "Authz Exec")?;
            out.address(&p("Grantee"), &m.str("grantee")?)?;
            match m.get("msgs") {
                Some(Value::Array(a)) if !a.is_empty() => {
                    for (i, inner) in a.iter().enumerate() {
                        render_msg(inner, &p(&format!("Message {}", i + 1)), depth + 1, out)?;
                    }
                }
                _ => return Err(malformed("MsgExec without messages")),
            }
        }
        "cosmos-sdk/MsgTransfer" => {
            out.text(&p("Type"), "IBC Transfer")?;
            out.address(&p("Sender"), &m.str("sender")?)?;
            out.address(&p("Receiver"), &m.str("receiver")?)?;
            out.coins(&p("Amount"), &m.coins("token")?.pairs())?;
            out.text(
                &p("Channel"),
                format!("{} / {}", m.str("source_port")?, m.str("source_channel")?),
            )?;
            if let Some(h) = m.get("timeout_height") {
                let mut h = Obj::new(h, "timeout_height")?;
                let height = format!(
                    "{}-{}",
                    h.int("revision_number")?,
                    h.int("revision_height")?
                );
                h.finish_strict("timeout_height")?;
                if height != "0-0" {
                    out.text(&p("Timeout Height"), height)?;
                }
            }
            let ts = m.int("timeout_timestamp")?;
            if ts != "0" {
                out.text(&p("Timeout Timestamp (ns)"), ts)?;
            }
            let memo = m.str("memo")?;
            if !memo.is_empty() {
                out.text(&p("Memo"), memo)?;
            }
        }
        "wasm/MsgExecuteContract" => {
            out.text(&p("Type"), "Wasm Execute")?;
            out.address(&p("Sender"), &m.str("sender")?)?;
            out.address(&p("Contract"), &m.str("contract")?)?;
            let msg = m.get("msg").map(Value::to_string).unwrap_or_default();
            out.text(&p("Message (contract-defined JSON)"), msg)?;
            out.coins(&p("Funds"), &m.coins("funds")?.pairs())?;
        }
        other => {
            out.text(&p("Type"), format!("{other} (not fully decoded)"))?;
            out.text(&p("Value"), value.to_string())?;
            return Ok(());
        }
    }
    m.finish(prefix, out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn canon(s: &str) -> String {
        let v: Value = serde_json::from_str(s).unwrap();
        let mut out = String::new();
        canonical(&v, &mut out).unwrap();
        out
    }

    #[test]
    fn sorts_keys_and_escapes_like_go() {
        let bs = '\\';
        let want = format!(
            "{{\"a\":{{\"c\":[true,null],\"d\":\"{bs}u003c{bs}u0026{bs}u003e\"}},\"b\":1}}"
        );
        assert_eq!(canon(r#"{"b":1,"a":{"d":"<&>","c":[true,null]}}"#), want);
        let line_sep = format!("\"x{bs}nx {bs}u2028\"");
        assert_eq!(canon(&line_sep), line_sep);
    }

    #[test]
    fn refuses_what_go_would_render_differently() {
        let v: Value = serde_json::from_str("1.5").unwrap();
        assert!(canonical(&v, &mut String::new()).is_err());
        let v: Value = serde_json::from_str("18446744073709551615").unwrap();
        assert!(canonical(&v, &mut String::new()).is_err());
        let v: Value = serde_json::from_str("\"\\u0001\"").unwrap();
        assert!(canonical(&v, &mut String::new()).is_err());
    }
}
