/// Configuration for a Surfpool validator instance.
///
/// Maps to `surfpool start` CLI flags. See `surfpool start --help` for details.
#[derive(Debug, Clone)]
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
