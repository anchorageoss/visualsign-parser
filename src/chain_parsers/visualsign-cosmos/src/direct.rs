//! `SIGN_MODE_DIRECT`: the protobuf `SignDoc` that the signer's key signs.
//!
//! Every layer (`SignDoc`, `TxBody`, `AuthInfo`) must be the canonical
//! encoding of the fields shown. A layer that carries fields this parser does
//! not model is refused rather than shown incompletely. Inside a message, an
//! unmodelled field is surfaced as an `Undecoded Fields` entry.

use crate::out::{MAX_EXEC_DEPTH, Out, join, vote_option};
use crate::proto::{self, Any, AuthInfo, Coin, SignDoc, TxBody, decode_exact};
use sha2::{Digest, Sha256};
use visualsign::SignablePayloadField;
use visualsign::vsptrait::{TransactionParseError, VisualSignError};

/// Type URL of the Cosmos SDK v0.55 ML-DSA-65 public key.
pub const ML_DSA_65_PUBKEY_TYPE_URL: &str = "/cosmos.crypto.mldsa65.PubKey";

const SECP256K1_PUBKEY_TYPE_URL: &str = "/cosmos.crypto.secp256k1.PubKey";
const MULTISIG_PUBKEY_TYPE_URL: &str = "/cosmos.crypto.multisig.LegacyAminoPubKey";

/// A parsed `SignDoc` with its inner `TxBody` and `AuthInfo`.
#[derive(Debug, Clone)]
pub(crate) struct DirectDoc {
    sign_doc: SignDoc,
    body: TxBody,
    auth_info: AuthInfo,
}

fn canonical<M: prost::Message + Default>(
    bytes: &[u8],
    what: &str,
) -> Result<M, TransactionParseError> {
    decode_exact::<M>(bytes).map_err(|e| {
        TransactionParseError::DecodeError(match e {
            None => format!("{what} does not parse"),
            Some(_) => format!("{what} is not a canonical encoding of the displayed fields"),
        })
    })
}

impl DirectDoc {
    pub(crate) fn parse(sign_doc_bytes: &[u8]) -> Result<Self, TransactionParseError> {
        let sign_doc: SignDoc = canonical(sign_doc_bytes, "SignDoc")?;
        let body: TxBody = canonical(&sign_doc.body_bytes, "TxBody")?;
        let auth_info: AuthInfo = canonical(&sign_doc.auth_info_bytes, "AuthInfo")?;
        if body.messages.is_empty() {
            return Err(TransactionParseError::InvalidFormat(
                "SignDoc has no messages".to_string(),
            ));
        }
        Ok(Self {
            sign_doc,
            body,
            auth_info,
        })
    }

    pub(crate) fn render(&self) -> Result<Vec<SignablePayloadField>, VisualSignError> {
        let mut out = Out::default();
        out.text("Chain ID", &self.sign_doc.chain_id)?;
        out.text("Account Number", self.sign_doc.account_number.to_string())?;
        for (i, m) in self.body.messages.iter().enumerate() {
            render_any(m, &format!("Message {}", i + 1), 0, &mut out)?;
        }
        render_body_extras(&self.body, &mut out)?;
        render_auth_info(&self.auth_info, &mut out)?;
        Ok(out.into_fields())
    }
}

fn render_body_extras(body: &TxBody, out: &mut Out) -> Result<(), VisualSignError> {
    if !body.memo.is_empty() {
        out.text("Memo", &body.memo)?;
    }
    if body.timeout_height != 0 {
        out.text("Timeout Height", body.timeout_height.to_string())?;
    }
    if body.unordered {
        out.text("Unordered", "yes")?;
    }
    if let Some(t) = &body.timeout_timestamp {
        out.text(
            "Timeout Timestamp",
            format!("{}s {}ns (unix)", t.seconds, t.nanos),
        )?;
    }
    for e in body
        .extension_options
        .iter()
        .chain(&body.non_critical_extension_options)
    {
        out.text(
            "Extension Option",
            format!("{} (not fully decoded)", e.type_url),
        )?;
    }
    Ok(())
}

fn render_auth_info(auth: &AuthInfo, out: &mut Out) -> Result<(), VisualSignError> {
    match &auth.fee {
        Some(fee) => {
            out.coins("Fee", &coin_pairs(&fee.amount))?;
            out.text("Gas Limit", fee.gas_limit.to_string())?;
            if !fee.payer.is_empty() {
                out.address("Fee Payer", &fee.payer)?;
            }
            if !fee.granter.is_empty() {
                out.address("Fee Granter", &fee.granter)?;
            }
        }
        None => out.text("Fee", "missing")?,
    }
    if let Some(tip) = &auth.tip {
        out.text(
            "Tip (deprecated)",
            format!(
                "{} from {}",
                coins_text(&coin_pairs(&tip.amount)),
                tip.tipper
            ),
        )?;
    }
    for (i, si) in auth.signer_infos.iter().enumerate() {
        let l = format!("Signer {}", i + 1);
        match &si.public_key {
            Some(pk) => render_pubkey(pk, &join(&l, "Public Key"), out, 0)?,
            None => out.text(&join(&l, "Public Key"), "not included (already on chain)")?,
        }
        match si.mode_info.as_ref().and_then(|m| m.sum.as_ref()) {
            Some(proto::mode_info::Sum::Single(s)) => {
                out.text(&join(&l, "Sign Mode"), sign_mode(s.mode))?;
            }
            Some(proto::mode_info::Sum::Multi(_)) => {
                out.text(&join(&l, "Sign Mode"), "multisig")?;
            }
            None => out.text(&join(&l, "Sign Mode"), "missing")?,
        }
        out.text(&join(&l, "Sequence"), si.sequence.to_string())?;
    }
    Ok(())
}

fn coin_pairs(coins: &[Coin]) -> Vec<(&str, &str)> {
    coins
        .iter()
        .map(|c| (c.amount.as_str(), c.denom.as_str()))
        .collect()
}

pub(crate) fn coins_text(coins: &[(&str, &str)]) -> String {
    if coins.is_empty() {
        return "none".to_string();
    }
    coins
        .iter()
        .map(|(amount, denom)| format!("{amount} {denom}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sign_mode(mode: i32) -> String {
    match mode {
        1 => "direct".into(),
        2 => "textual".into(),
        3 => "direct aux".into(),
        127 => "legacy amino JSON".into(),
        191 => "EIP-191".into(),
        m => format!("mode {m}"),
    }
}

/// Show a public key `Any`: secp256k1, ML-DSA-65, or a legacy amino multisig
/// with its threshold and members.
fn render_pubkey(
    pk: &Any,
    label: &str,
    out: &mut Out,
    depth: usize,
) -> Result<(), VisualSignError> {
    match pk.type_url.as_str() {
        SECP256K1_PUBKEY_TYPE_URL => match decode_exact::<proto::KeyBytes>(&pk.value) {
            Ok(k) => out.text(label, format!("secp256k1 {}", hex::encode(&k.key))),
            Err(_) => out.text(label, "malformed secp256k1 key"),
        },
        ML_DSA_65_PUBKEY_TYPE_URL => match decode_exact::<proto::KeyBytes>(&pk.value) {
            Ok(k) => out.text(
                label,
                format!(
                    "ml-dsa-65 ({} bytes, sha256 {})",
                    k.key.len(),
                    hex::encode(Sha256::digest(&k.key))
                ),
            ),
            Err(_) => out.text(label, "malformed ml-dsa-65 key"),
        },
        MULTISIG_PUBKEY_TYPE_URL if depth == 0 => {
            match decode_exact::<proto::LegacyAminoPubKey>(&pk.value) {
                Ok(m) => {
                    out.text(
                        label,
                        format!("multisig {} of {}", m.threshold, m.public_keys.len()),
                    )?;
                    for (i, member) in m.public_keys.iter().enumerate() {
                        render_pubkey(member, &join(label, &format!("Member {}", i + 1)), out, 1)?;
                    }
                    Ok(())
                }
                Err(_) => out.text(label, "malformed multisig key"),
            }
        }
        other => out.text(label, format!("{other} (not fully decoded)")),
    }
}

/// Decode a message body, noting whether it carried fields we do not show.
fn decode_msg<M: prost::Message + Default>(any: &Any) -> Result<(M, bool), VisualSignError> {
    match decode_exact::<M>(&any.value) {
        Ok(m) => Ok((m, false)),
        Err(Some(m)) => Ok((m, true)),
        Err(None) => Err(VisualSignError::DecodeError(format!(
            "{} does not parse",
            any.type_url
        ))),
    }
}

/// Render one message. `depth` counts enclosing `MsgExec`s.
fn render_any(any: &Any, prefix: &str, depth: usize, out: &mut Out) -> Result<(), VisualSignError> {
    let p = |l: &str| join(prefix, l);
    let mut undecoded = false;
    match any.type_url.as_str() {
        "/cosmos.bank.v1beta1.MsgSend" => {
            let (m, u) = decode_msg::<proto::MsgSend>(any)?;
            undecoded = u;
            out.text(&p("Type"), "Send")?;
            out.address(&p("From"), &m.from_address)?;
            out.address(&p("To"), &m.to_address)?;
            out.coins(&p("Amount"), &coin_pairs(&m.amount))?;
        }
        "/cosmos.staking.v1beta1.MsgDelegate" | "/cosmos.staking.v1beta1.MsgUndelegate" => {
            let (m, u) = decode_msg::<proto::MsgDelegate>(any)?;
            undecoded = u;
            let kind = if any.type_url.ends_with("MsgDelegate") {
                "Delegate"
            } else {
                "Undelegate"
            };
            out.text(&p("Type"), kind)?;
            out.address(&p("Delegator"), &m.delegator_address)?;
            out.address(&p("Validator"), &m.validator_address)?;
            out.coins(&p("Amount"), &coin_pairs(m.amount.as_slice()))?;
        }
        "/cosmos.staking.v1beta1.MsgBeginRedelegate" => {
            let (m, u) = decode_msg::<proto::MsgBeginRedelegate>(any)?;
            undecoded = u;
            out.text(&p("Type"), "Redelegate")?;
            out.address(&p("Delegator"), &m.delegator_address)?;
            out.address(&p("From Validator"), &m.validator_src_address)?;
            out.address(&p("To Validator"), &m.validator_dst_address)?;
            out.coins(&p("Amount"), &coin_pairs(m.amount.as_slice()))?;
        }
        "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward" => {
            let (m, u) = decode_msg::<proto::MsgWithdrawDelegatorReward>(any)?;
            undecoded = u;
            out.text(&p("Type"), "Withdraw Rewards")?;
            out.address(&p("Delegator"), &m.delegator_address)?;
            out.address(&p("Validator"), &m.validator_address)?;
        }
        "/cosmos.gov.v1beta1.MsgVote" | "/cosmos.gov.v1.MsgVote" => {
            let (m, u) = decode_msg::<proto::MsgVote>(any)?;
            undecoded = u;
            out.text(&p("Type"), "Vote")?;
            out.text(&p("Proposal"), m.proposal_id.to_string())?;
            out.address(&p("Voter"), &m.voter)?;
            match vote_option(i64::from(m.option)) {
                Some(o) => out.text(&p("Option"), o)?,
                None => out.text(&p("Option"), format!("{} (unrecognized)", m.option))?,
            }
            if !m.metadata.is_empty() {
                out.text(&p("Metadata"), &m.metadata)?;
            }
        }
        "/cosmos.authz.v1beta1.MsgExec" => {
            let (m, u) = decode_msg::<proto::MsgExec>(any)?;
            undecoded = u;
            if depth >= MAX_EXEC_DEPTH {
                return Err(VisualSignError::ConversionError(
                    "authz MsgExec nested too deep".to_string(),
                ));
            }
            if m.msgs.is_empty() {
                return Err(VisualSignError::ConversionError(
                    "MsgExec without messages".to_string(),
                ));
            }
            out.text(&p("Type"), "Authz Exec")?;
            out.address(&p("Grantee"), &m.grantee)?;
            for (i, inner) in m.msgs.iter().enumerate() {
                render_any(inner, &p(&format!("Message {}", i + 1)), depth + 1, out)?;
            }
        }
        "/ibc.applications.transfer.v1.MsgTransfer" => {
            let (m, u) = decode_msg::<proto::MsgTransfer>(any)?;
            undecoded = u;
            out.text(&p("Type"), "IBC Transfer")?;
            out.address(&p("Sender"), &m.sender)?;
            out.address(&p("Receiver"), &m.receiver)?;
            out.coins(&p("Amount"), &coin_pairs(m.token.as_slice()))?;
            out.text(
                &p("Channel"),
                format!("{} / {}", m.source_port, m.source_channel),
            )?;
            if let Some(h) = m.timeout_height.filter(|h| h.revision_height != 0) {
                out.text(
                    &p("Timeout Height"),
                    format!("{}-{}", h.revision_number, h.revision_height),
                )?;
            }
            if m.timeout_timestamp != 0 {
                out.text(
                    &p("Timeout Timestamp (ns)"),
                    m.timeout_timestamp.to_string(),
                )?;
            }
            if !m.memo.is_empty() {
                out.text(&p("Memo"), &m.memo)?;
            }
        }
        "/cosmwasm.wasm.v1.MsgExecuteContract" => {
            let (m, u) = decode_msg::<proto::MsgExecuteContract>(any)?;
            undecoded = u;
            out.text(&p("Type"), "Wasm Execute")?;
            out.address(&p("Sender"), &m.sender)?;
            out.address(&p("Contract"), &m.contract)?;
            // The contract interprets the message: it can be shown, not explained.
            match std::str::from_utf8(&m.msg) {
                Ok(s) => out.text(&p("Message (contract-defined JSON)"), s)?,
                Err(_) => out.text(
                    &p("Message (contract-defined, not UTF-8)"),
                    format!("0x{}", hex::encode(&m.msg)),
                )?,
            }
            out.coins(&p("Funds"), &coin_pairs(&m.funds))?;
        }
        other => {
            out.text(&p("Type"), format!("{other} (not fully decoded)"))?;
            out.text(&p("Value"), format!("0x{}", hex::encode(&any.value)))?;
        }
    }
    if undecoded {
        out.text(&p("Undecoded Fields"), "present (not shown)")?;
    }
    Ok(())
}
