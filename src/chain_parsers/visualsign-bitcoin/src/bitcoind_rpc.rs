//! A Bitcoin Core node as a [`WalletSource`] (prototype, `bitcoind-rpc`
//! feature, CLI only).
//!
//! Meant for a watch-only descriptor wallet on the signer's own offline
//! machine: public descriptors only, networking off. That node is the trust
//! anchor, so nothing it returns needs a signature.
//!
//! The client speaks JSON-RPC over HTTP/1.1 on a plain [`TcpStream`], with
//! basic auth from the cookie file or a user and password. It can only send
//! the read-only methods in [`ALLOWED_METHODS`], and `listdescriptors` only
//! with `private=false`; anything else is refused before a connection is made.

use crate::wallet_source::{
    AddressInfo, SourceAccounts, WalletSource, WalletSourceError, accounts_from_descriptors,
    display_text,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use bitcoin::{Address, Network, Script, Transaction, Txid};
use serde_json::{Value, json};

/// The only methods this client sends. All are read-only.
pub const ALLOWED_METHODS: [&str; 5] = [
    "listdescriptors",
    "getaddressinfo",
    "gettransaction",
    "getwalletinfo",
    "getblockchaininfo",
];

/// Largest response accepted. `listdescriptors` and `gettransaction` replies
/// are far smaller.
const MAX_RESPONSE: u64 = 32 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
/// Core's "Invalid or non-wallet transaction id".
const RPC_INVALID_ADDRESS_OR_KEY: i64 = -5;

/// How to authenticate to the node.
#[derive(Debug, Clone)]
pub enum RpcAuth {
    /// The node's `.cookie` file.
    Cookie(std::path::PathBuf),
    UserPassword {
        user: String,
        password: String,
    },
}

/// Why a call failed.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum RpcError {
    #[error("refused: {0}")]
    Refused(String),
    #[error("RPC transport: {0}")]
    Transport(String),
    #[error("RPC error {code}: {message}")]
    Node { code: i64, message: String },
}

impl From<RpcError> for WalletSourceError {
    fn from(e: RpcError) -> Self {
        Self(e.to_string())
    }
}

/// What the node says about its wallet and chain.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NodeInfo {
    pub wallet_name: String,
    pub private_keys_enabled: bool,
    /// `getblockchaininfo.chain`: main, test, testnet4, signet or regtest.
    pub chain: String,
    pub blocks: u64,
}

/// A read-only connection to one wallet of a Bitcoin Core node.
#[derive(Debug)]
pub struct BitcoindRpc {
    host: String,
    port: u16,
    path: String,
    authorization: String,
    network: Network,
    account_name: Option<String>,
    info: Option<NodeInfo>,
    next_id: AtomicU64,
}

/// Core's default RPC port for `network`.
#[must_use]
pub fn default_port(network: Network) -> u16 {
    match network {
        Network::Bitcoin => 8332,
        Network::Testnet => 18332,
        Network::Testnet4 => 48332,
        Network::Signet => 38332,
        Network::Regtest => 18443,
    }
}

/// `http://host[:port][/]` to host and port. TLS is not supported: the node
/// is expected on the same machine.
fn parse_url(url: &str, network: Network) -> Result<(String, u16), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("RPC URL must start with http:// (got '{url}')"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.contains(']') => (
            host,
            port.parse()
                .map_err(|_| format!("invalid port in RPC URL '{url}'"))?,
        ),
        _ => (authority, default_port(network)),
    };
    if host.is_empty() {
        return Err(format!("no host in RPC URL '{url}'"));
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Ok((host.to_string(), port))
}

/// Percent-encode a wallet name for the `/wallet/<name>` path.
fn encode_path(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn read_cookie(path: &Path) -> Result<String, String> {
    let cookie = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read cookie file {}: {e}", path.display()))?;
    let cookie = cookie.trim();
    if !cookie.contains(':') {
        return Err(format!("{} is not an RPC cookie file", path.display()));
    }
    Ok(cookie.to_string())
}

impl BitcoindRpc {
    /// Configure a client for `wallet` (or the node's only wallet) without
    /// contacting the node.
    pub fn new(
        url: &str,
        wallet: Option<&str>,
        auth: &RpcAuth,
        network: Network,
    ) -> Result<Self, String> {
        let (host, port) = parse_url(url, network)?;
        let credentials = match auth {
            RpcAuth::Cookie(path) => read_cookie(path)?,
            RpcAuth::UserPassword { user, password } => format!("{user}:{password}"),
        };
        Ok(Self {
            host,
            port,
            path: wallet.map_or_else(
                || "/".to_string(),
                |w| format!("/wallet/{}", encode_path(w)),
            ),
            authorization: format!("Basic {}", b64.encode(credentials)),
            network,
            account_name: None,
            info: None,
            next_id: AtomicU64::new(1),
        })
    }

    /// Name the accounts read from this wallet. Defaults to the wallet name.
    #[must_use]
    pub fn with_account_name(mut self, name: Option<String>) -> Self {
        self.account_name = name;
        self
    }

    /// Read the wallet and chain info, and check the node is on the expected
    /// network.
    pub fn connect(mut self) -> Result<Self, WalletSourceError> {
        let wallet = self.call("getwalletinfo", &[])?;
        let chain = self.call("getblockchaininfo", &[])?;
        let info = NodeInfo {
            wallet_name: wallet["walletname"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            private_keys_enabled: wallet["private_keys_enabled"].as_bool().unwrap_or(true),
            chain: chain["chain"].as_str().unwrap_or_default().to_string(),
            blocks: chain["blocks"].as_u64().unwrap_or_default(),
        };
        if info.chain != self.network.to_core_arg() {
            return Err(WalletSourceError(format!(
                "node is on {}, expected {}",
                info.chain,
                self.network.to_core_arg()
            )));
        }
        self.info = Some(info);
        Ok(self)
    }

    /// Wallet and chain info read by [`Self::connect`].
    #[must_use]
    pub fn info(&self) -> Option<&NodeInfo> {
        self.info.as_ref()
    }

    /// Send one JSON-RPC call. Only [`ALLOWED_METHODS`] are sent, and
    /// `listdescriptors` only as `listdescriptors false`.
    pub fn call(&self, method: &str, params: &[Value]) -> Result<Value, RpcError> {
        if !ALLOWED_METHODS.contains(&method) {
            return Err(RpcError::Refused(format!(
                "{method} is not on the read-only allowlist"
            )));
        }
        if method == "listdescriptors" && params != [Value::Bool(false)] {
            return Err(RpcError::Refused(
                "listdescriptors is only sent with private=false".to_string(),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "1.0", "id": id, "method": method, "params": params});
        let (status, body) = self
            .post(&body.to_string())
            .map_err(|e| RpcError::Transport(e.to_string()))?;
        let reply: Value = serde_json::from_slice(&body).map_err(|_| {
            RpcError::Transport(match status {
                401 => "authentication failed (check the cookie or password)".to_string(),
                _ => format!("HTTP {status} without a JSON-RPC reply"),
            })
        })?;
        match &reply["error"] {
            Value::Null => Ok(reply["result"].clone()),
            error => Err(RpcError::Node {
                code: error["code"].as_i64().unwrap_or_default(),
                message: error["message"].as_str().unwrap_or_default().to_string(),
            }),
        }
    }

    fn post(&self, body: &str) -> std::io::Result<(u16, Vec<u8>)> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAuthorization: {auth}\r\n\
             Content-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            path = self.path,
            host = self.host,
            port = self.port,
            auth = self.authorization,
            len = body.len(),
        );
        stream.write_all(request.as_bytes())?;
        read_response(BufReader::new(stream.take(MAX_RESPONSE)))
    }
}

fn bad(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

/// Read an HTTP/1.1 response: status, headers, then a `Content-Length`,
/// chunked, or read-to-close body.
fn read_response(mut reader: impl BufRead) -> std::io::Result<(u16, Vec<u8>)> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("not an HTTP response"))?;
    let mut length = None;
    let mut chunked = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(bad("truncated HTTP headers"));
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.parse().map_err(|_| bad("bad Content-Length"))?);
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                chunked = value.eq_ignore_ascii_case("chunked");
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = line.trim().split(';').next().unwrap_or_default();
            let size = usize::from_str_radix(size, 16).map_err(|_| bad("bad chunk size"))?;
            if size == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            reader.read_exact(&mut body[start..])?;
            line.clear();
            reader.read_line(&mut line)?;
        }
    } else if let Some(length) = length {
        body.resize(length, 0);
        reader.read_exact(&mut body)?;
    } else {
        reader.read_to_end(&mut body)?;
    }
    Ok((status, body))
}

impl WalletSource for BitcoindRpc {
    fn network(&self) -> Network {
        self.network
    }

    fn description(&self) -> String {
        let Some(info) = &self.info else {
            return "not connected".to_string();
        };
        let wallet = display_text(&info.wallet_name).unwrap_or_else(|| "default".to_string());
        if info.blocks == 0 {
            format!("wallet {wallet}, no chain sync")
        } else {
            format!("wallet {wallet}, chain at block {}", info.blocks)
        }
    }

    fn accounts(&self) -> Result<SourceAccounts, WalletSourceError> {
        let reply = self.call("listdescriptors", &[Value::Bool(false)])?;
        let descriptors: Vec<&str> = reply["descriptors"]
            .as_array()
            .ok_or_else(|| WalletSourceError("listdescriptors: no descriptors".to_string()))?
            .iter()
            .filter_map(|d| d["desc"].as_str())
            .collect();
        let name = self
            .account_name
            .as_deref()
            .or(self.info.as_ref().map(|i| i.wallet_name.as_str()))
            .and_then(display_text)
            .unwrap_or_else(|| "Wallet".to_string());
        Ok(accounts_from_descriptors(descriptors, &name))
    }

    fn transaction(&self, txid: Txid) -> Result<Option<Transaction>, WalletSourceError> {
        let reply = match self.call("gettransaction", &[json!(txid.to_string())]) {
            Ok(reply) => reply,
            Err(RpcError::Node { code, .. }) if code == RPC_INVALID_ADDRESS_OR_KEY => {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let hex = reply["hex"]
            .as_str()
            .ok_or_else(|| WalletSourceError(format!("gettransaction {txid}: no hex")))?;
        bitcoin::consensus::encode::deserialize_hex(hex)
            .map(Some)
            .map_err(|e| WalletSourceError(format!("gettransaction {txid}: {e}")))
    }

    fn address_info(&self, script: &Script) -> Result<AddressInfo, WalletSourceError> {
        let Ok(address) = Address::from_script(script, self.network) else {
            return Ok(AddressInfo::default());
        };
        let reply = self.call("getaddressinfo", &[json!(address.to_string())])?;
        // `labels` is a list of names (older releases: objects with a name).
        let label = reply["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str().or_else(|| l["name"].as_str()))
            .find(|l| !l.is_empty())
            .map(str::to_string);
        Ok(AddressInfo {
            ismine: reply["ismine"].as_bool(),
            label,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn client() -> BitcoindRpc {
        // Port 9 (discard): nothing should ever be sent there.
        BitcoindRpc::new(
            "http://127.0.0.1:9",
            Some("airgap"),
            &RpcAuth::UserPassword {
                user: "u".to_string(),
                password: "p".to_string(),
            },
            Network::Regtest,
        )
        .unwrap()
    }

    #[test]
    fn methods_off_the_allowlist_are_refused_before_connecting() {
        let rpc = client();
        for method in [
            "dumpprivkey",
            "walletprocesspsbt",
            "signrawtransactionwithwallet",
            "sendrawtransaction",
            "importdescriptors",
            "stop",
        ] {
            let err = rpc.call(method, &[]).unwrap_err();
            assert_eq!(
                err,
                RpcError::Refused(format!("{method} is not on the read-only allowlist"))
            );
        }
    }

    #[test]
    fn listdescriptors_is_only_sent_without_private_keys() {
        let rpc = client();
        for params in [vec![], vec![Value::Bool(true)], vec![json!("true")]] {
            assert!(matches!(
                rpc.call("listdescriptors", &params),
                Err(RpcError::Refused(_))
            ));
        }
    }

    #[test]
    fn urls_and_wallet_paths() {
        assert_eq!(
            parse_url("http://127.0.0.1:18443", Network::Regtest).unwrap(),
            ("127.0.0.1".to_string(), 18443)
        );
        assert_eq!(
            parse_url("http://localhost/", Network::Bitcoin).unwrap(),
            ("localhost".to_string(), 8332)
        );
        assert_eq!(
            parse_url("http://[::1]:8332", Network::Bitcoin).unwrap(),
            ("::1".to_string(), 8332)
        );
        assert!(parse_url("https://127.0.0.1:8332", Network::Bitcoin).is_err());
        assert_eq!(encode_path("my wallet/1"), "my%20wallet%2F1");
    }

    #[test]
    fn reads_length_chunked_and_close_delimited_bodies() {
        let length = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbodyEXTRA";
        assert_eq!(read_response(&length[..]).unwrap(), (200, b"body".to_vec()));
        let chunked =
            b"HTTP/1.1 500 Internal\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(
            read_response(&chunked[..]).unwrap(),
            (500, b"abcde".to_vec())
        );
        let close = b"HTTP/1.1 401 Unauthorized\r\n\r\n";
        assert_eq!(read_response(&close[..]).unwrap(), (401, vec![]));
    }
}
