/// Reduce an RPC URL to scheme and host.
///
/// Datasource URLs carry credentials in three places -- the query string
/// (`?api-key=...`), the path (`/v2/<key>`), and the userinfo
/// (`user:pass@host`) -- and all of them reach logs through `Debug` output and
/// the spawn argument list. Only the scheme and host are loggable.
pub(crate) fn redact_url_credentials(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, url),
    };

    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let (host, has_userinfo) = match authority.rsplit_once('@') {
        Some((_userinfo, host)) => (host, true),
        None => (authority, false),
    };

    let mut redacted = String::new();
    if let Some(scheme) = scheme {
        redacted.push_str(scheme);
        redacted.push_str("://");
    }
    if has_userinfo {
        redacted.push_str("<redacted>@");
    }
    redacted.push_str(host);
    if authority_end < rest.len() {
        redacted.push_str("/<redacted>");
    }
    redacted
}

/// Configuration for a Surfpool validator instance.
///
/// Maps to `surfpool start` CLI flags. See `surfpool start --help` for details.
#[derive(Clone)]
pub struct SurfpoolConfig {
    /// Datasource RPC URL to fork from (`-u`/`--rpc-url`).
    pub rpc_url: Option<String>,
    /// Local Simnet RPC port (`-p`/`--port`). Auto-selected if `None`.
    pub port: Option<u16>,
    /// Local Simnet WebSocket port (`-w`/`--ws-port`). Auto-selected if `None`.
    pub ws_port: Option<u16>,
    /// Log level (`-l`/`--log-level`).
    pub log_level: String,
    /// Use CI-adequate settings (`--ci`).
    pub ci: bool,
}

/// Renders `rpc_url` through [`redact_url_credentials`] so `{:?}` on a config
/// keeps the datasource credentials out of logs.
impl std::fmt::Debug for SurfpoolConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfpoolConfig")
            .field(
                "rpc_url",
                &self.rpc_url.as_deref().map(redact_url_credentials),
            )
            .field("port", &self.port)
            .field("ws_port", &self.ws_port)
            .field("log_level", &self.log_level)
            .field("ci", &self.ci)
            .finish()
    }
}

impl Default for SurfpoolConfig {
    fn default() -> Self {
        let rpc_url = std::env::var("HELIUS_API_KEY")
            .ok()
            .map(|key| format!("https://mainnet.helius-rpc.com/?api-key={key}"))
            .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
            .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());

        Self {
            rpc_url: Some(rpc_url),
            port: None,
            ws_port: None,
            log_level: "info".to_string(),
            ci: true,
        }
    }
}

impl SurfpoolConfig {
    pub fn with_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.rpc_url = Some(url.into());
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_ws_port(mut self, port: u16) -> Self {
        self.ws_port = Some(port);
        self
    }

    pub fn with_log_level(mut self, level: impl Into<String>) -> Self {
        self.log_level = level.into();
        self
    }

    pub fn with_ci(mut self, ci: bool) -> Self {
        self.ci = ci;
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query_string_credentials() {
        assert_eq!(
            redact_url_credentials("https://mainnet.helius-rpc.com/?api-key=SECRET"),
            "https://mainnet.helius-rpc.com/<redacted>"
        );
    }

    #[test]
    fn redacts_path_embedded_credentials() {
        assert_eq!(
            redact_url_credentials("https://rpc.example.com/v2/SECRET"),
            "https://rpc.example.com/<redacted>"
        );
    }

    #[test]
    fn keeps_credential_free_urls_intact() {
        assert_eq!(
            redact_url_credentials("https://api.mainnet-beta.solana.com"),
            "https://api.mainnet-beta.solana.com"
        );
        assert_eq!(
            redact_url_credentials("http://127.0.0.1:8899"),
            "http://127.0.0.1:8899"
        );
    }

    #[test]
    fn redacts_userinfo_credentials() {
        let redacted = redact_url_credentials("https://user:pass@rpc.example.com/v2/SECRET");
        assert_eq!(redacted, "https://<redacted>@rpc.example.com/<redacted>");
        assert!(!redacted.contains("pass"));
    }

    #[test]
    fn redacts_userinfo_without_a_path() {
        assert_eq!(
            redact_url_credentials("https://user:pass@rpc.example.com"),
            "https://<redacted>@rpc.example.com"
        );
    }

    #[test]
    fn redacts_schemeless_input() {
        assert_eq!(
            redact_url_credentials("rpc.example.com/SECRET"),
            "rpc.example.com/<redacted>"
        );
    }

    #[test]
    fn debug_output_hides_the_api_key() {
        let config = SurfpoolConfig::default()
            .with_rpc_url("https://mainnet.helius-rpc.com/?api-key=SECRET");
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("SECRET"),
            "Debug output must not carry the api key: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug output must mark the redaction: {rendered}"
        );
    }
}
