use std::path::PathBuf;

/// Configuration for a Surfpool validator instance.
#[derive(Debug, Clone)]
pub struct SurfpoolConfig {
    /// RPC URL to fork from (e.g., mainnet-beta via Helius).
    pub fork_url: Option<String>,
    /// Local RPC port (auto-selected if `None`).
    pub rpc_port: Option<u16>,
    /// Local WebSocket port (auto-selected if `None`).
    pub ws_port: Option<u16>,
    /// Ledger directory path.
    pub ledger_path: Option<PathBuf>,
    /// Reset ledger on startup.
    pub reset_ledger: bool,
    /// Log level for the surfpool process.
    pub log_level: String,
}

impl Default for SurfpoolConfig {
    fn default() -> Self {
        let fork_url = std::env::var("HELIUS_API_KEY")
            .ok()
            .map(|key| format!("https://mainnet.helius-rpc.com/?api-key={key}"))
            .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
            .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());

        Self {
            fork_url: Some(fork_url),
            rpc_port: None,
            ws_port: None,
            ledger_path: None,
            reset_ledger: true,
            log_level: "info".to_string(),
        }
    }
}

impl SurfpoolConfig {
    pub fn with_fork_url(mut self, url: impl Into<String>) -> Self {
        self.fork_url = Some(url.into());
        self
    }

    pub fn with_rpc_port(mut self, port: u16) -> Self {
        self.rpc_port = Some(port);
        self
    }

    pub fn with_ws_port(mut self, port: u16) -> Self {
        self.ws_port = Some(port);
        self
    }

    pub fn with_ledger_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.ledger_path = Some(path.into());
        self
    }

    pub fn with_reset_ledger(mut self, reset: bool) -> Self {
        self.reset_ledger = reset;
        self
    }

    pub fn with_log_level(mut self, level: impl Into<String>) -> Self {
        self.log_level = level.into();
        self
    }
}
