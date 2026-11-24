//! JSON-RPC layer. Everything testable lives in the other modules; this
//! one only moves bytes to an RPC endpoint, behind the [`Rpc`] trait so
//! tests (and record modes) need no network.

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use std::time::Duration;

use crate::json::{self, Json};

/// One getAccountInfo reply, base64-decoded.
#[derive(Clone, Debug)]
pub struct AccountInfo {
    /// Account data, or `None` when the account does not exist.
    pub data: Option<Vec<u8>>,
    /// Owning program (base58), or `None` when the account does not exist.
    pub owner: Option<String>,
    /// Slot the reply was served at (`result.context.slot`).
    pub slot: u64,
}

/// getAccountInfo over JSON-RPC. `data_slice` mirrors the RPC `dataSlice`
/// parameter as `(offset, length)` and limits the returned bytes (base64
/// encoding still applies).
pub trait Rpc {
    fn get_account_info(
        &self,
        address: &str,
        data_slice: Option<(u64, u64)>,
    ) -> Result<AccountInfo>;
}

/// Blocking HTTPS JSON-RPC client with a 60-second timeout, like the
/// Python script's `urllib.request.urlopen(req, timeout=60)`.
pub struct HttpRpc {
    url: String,
    client: reqwest::blocking::Client,
}

impl HttpRpc {
    pub fn new(url: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("failed to build the RPC client")?;
        Ok(Self {
            url: url.to_string(),
            client,
        })
    }
}

impl Rpc for HttpRpc {
    fn get_account_info(
        &self,
        address: &str,
        data_slice: Option<(u64, u64)>,
    ) -> Result<AccountInfo> {
        let mut account = vec![
            ("encoding", Json::str("base64")),
            ("commitment", Json::str("finalized")),
        ];
        if let Some((offset, length)) = data_slice {
            account.push((
                "dataSlice",
                Json::object(vec![
                    (
                        "offset",
                        Json::Int(i64::try_from(offset).context("offset too large")?),
                    ),
                    (
                        "length",
                        Json::Int(i64::try_from(length).context("length too large")?),
                    ),
                ]),
            ));
        }
        let request = Json::object(vec![
            ("jsonrpc", Json::str("2.0")),
            ("id", Json::Int(1)),
            ("method", Json::str("getAccountInfo")),
            (
                "params",
                Json::Array(vec![Json::str(address), Json::object(account)]),
            ),
        ]);
        let reply_text = self
            .client
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json::canonical(&request))
            .send()
            .with_context(|| format!("RPC request to {} failed", self.url))?
            .text()
            .context("failed to read the RPC reply")?;
        parse_account_info_reply(&reply_text)
    }
}

/// Decode a getAccountInfo reply: `(data, owner, context.slot)`. `data` and
/// `owner` are `None` when the account does not exist.
fn parse_account_info_reply(reply_text: &str) -> Result<AccountInfo> {
    let reply = json::parse(reply_text).map_err(anyhow::Error::msg)?;
    if let Some(error) = reply.get("error") {
        bail!("RPC error: {}", json::canonical(error));
    }
    let result = reply
        .get("result")
        .ok_or_else(|| anyhow!("RPC reply has no 'result': {}", json::canonical(&reply)))?;
    let slot = result
        .get("context")
        .and_then(|context| context.get("slot"))
        .and_then(Json::as_u64)
        .ok_or_else(|| anyhow!("RPC reply has no integer 'context.slot'"))?;
    let value = result.get("value");
    if value.is_none() || value.is_some_and(Json::is_null) {
        return Ok(AccountInfo {
            data: None,
            owner: None,
            slot,
        });
    }
    let owner = value
        .and_then(|value| value.get("owner"))
        .and_then(Json::as_str)
        .ok_or_else(|| anyhow!("RPC reply has no string 'owner'"))?
        .to_string();
    let encoded = value
        .and_then(|value| value.get("data"))
        .and_then(|data| data.at(0))
        .and_then(Json::as_str)
        .ok_or_else(|| anyhow!("RPC reply has no base64 'data[0]'"))?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("RPC reply data is not valid base64")?;
    Ok(AccountInfo {
        data: Some(data),
        owner: Some(owner),
        slot,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn account_found_reply_decodes() {
        let payload = b"hello account";
        let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
        let reply = format!(
            "{{\"jsonrpc\":\"2.0\",\"result\":{{\"context\":{{\"slot\":123}},\"value\":{{\"data\":[\"{encoded}\",\"base64\"],\"owner\":\"BPFLoaderUpgradeab1e11111111111111111111111\",\"executable\":true}}}}}}"
        );
        let info = parse_account_info_reply(&reply).unwrap();
        assert_eq!(info.slot, 123);
        assert_eq!(info.data, Some(payload.to_vec()));
        assert_eq!(
            info.owner,
            Some("BPFLoaderUpgradeab1e11111111111111111111111".to_string())
        );
    }

    #[test]
    fn missing_account_reply_decodes_to_none_data() {
        let reply = "{\"jsonrpc\":\"2.0\",\"result\":{\"context\":{\"slot\":9},\"value\":null}}";
        let info = parse_account_info_reply(reply).unwrap();
        assert_eq!(info.slot, 9);
        assert_eq!(info.data, None);
        assert_eq!(info.owner, None);
    }

    #[test]
    fn rpc_error_reply_is_an_error() {
        let reply = "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32000,\"message\":\"not found\"}}";
        let err = parse_account_info_reply(reply).unwrap_err().to_string();
        assert!(err.contains("RPC error:"), "{err}");
    }

    #[test]
    fn malformed_replies_are_errors() {
        for reply in [
            "",
            "{}",
            "{\"result\":{}}",
            "{\"result\":{\"context\":{},\"value\":{}}}",
        ] {
            assert!(
                parse_account_info_reply(reply).is_err(),
                "should have rejected {reply}"
            );
        }
    }
}
