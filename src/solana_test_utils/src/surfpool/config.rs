/// Configuration for a surfpool local validator instance.
#[derive(Debug, Clone)]
pub struct SurfpoolConfig {
    /// RPC URL of the mainnet node to fork from (`--rpc-url`).
    /// Defaults to SOLANA_RPC_URL env var, then the public mainnet endpoint.
    pub fork_url: Option<String>,
    /// Local RPC port (`--port`). None = auto-select a free port.
    pub rpc_port: Option<u16>,
}

impl Default for SurfpoolConfig {
    fn default() -> Self {
        // Prefer SOLANA_RPC_URL (set in .env / CI secrets) over the public endpoint.
        let fork_url = std::env::var("SOLANA_RPC_URL")
            .ok()
            .or_else(|| Some("https://api.mainnet-beta.solana.com".to_string()));

        Self {
            fork_url,
            rpc_port: None,
        }
    }
}
