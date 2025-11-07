use super::config::SurfpoolConfig;
use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
};
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Manages the lifecycle of a Surfpool validator instance
pub struct SurfpoolManager {
    process: Option<Child>,
    rpc_url: String,
    ws_url: String,
    #[allow(dead_code)]
    config: SurfpoolConfig,
}

impl SurfpoolManager {
    /// Start a new Surfpool instance with the given configuration
    pub async fn start(config: SurfpoolConfig) -> Result<Self> {
        info!("Starting Surfpool with config: {:?}", config);

        // Determine ports
        let rpc_port = config.rpc_port.unwrap_or_else(|| Self::find_free_port());
        let ws_port = config.ws_port.unwrap_or_else(|| Self::find_free_port());

        let rpc_url = format!("http://127.0.0.1:{}", rpc_port);
        let ws_url = format!("ws://127.0.0.1:{}", ws_port);

        // Build command arguments
        let mut args = vec![
            "--rpc-port".to_string(),
            rpc_port.to_string(),
            "--ws-port".to_string(),
            ws_port.to_string(),
            "--log".to_string(),
        ];

        if let Some(fork_url) = &config.fork_url {
            args.push("--url".to_string());
            args.push(fork_url.clone());
        }

        if let Some(ledger_path) = &config.ledger_path {
            args.push("--ledger".to_string());
            args.push(ledger_path.to_string_lossy().to_string());
        }

        if config.reset_ledger {
            args.push("--reset".to_string());
        }

        debug!("Spawning surfpool with args: {:?}", args);

        // Spawn the process
        let child = Command::new("surfpool")
            .args(&args)
            .spawn()
            .context("Failed to spawn surfpool process. Is surfpool installed?")?;

        let manager = Self {
            process: Some(child),
            rpc_url: rpc_url.clone(),
            ws_url,
            config,
        };

        // Wait for RPC to be ready
        manager
            .wait_ready()
            .await
            .context("Surfpool failed to become ready")?;

        info!("Surfpool started successfully at {}", rpc_url);

        Ok(manager)
    }

    /// Wait for the Surfpool RPC server to be ready
    pub async fn wait_ready(&self) -> Result<()> {
        let client = self.rpc_client();
        let max_attempts = 30;
        let delay = Duration::from_millis(500);

        for attempt in 1..=max_attempts {
            debug!("Checking if Surfpool is ready (attempt {}/{})", attempt, max_attempts);

            match client.get_version() {
                Ok(version) => {
                    info!("Surfpool is ready! Version: {:?}", version);
                    return Ok(());
                }
                Err(e) => {
                    if attempt == max_attempts {
                        return Err(anyhow::anyhow!(
                            "Surfpool did not become ready after {} attempts: {}",
                            max_attempts,
                            e
                        ));
                    }
                    warn!("Surfpool not ready yet (attempt {}): {}", attempt, e);
                    thread::sleep(delay);
                }
            }
        }

        Err(anyhow::anyhow!("Surfpool readiness check failed"))
    }

    /// Get an RPC client for this Surfpool instance
    pub fn rpc_client(&self) -> RpcClient {
        RpcClient::new_with_commitment(self.rpc_url.clone(), CommitmentConfig::confirmed())
    }

    /// Get the RPC URL
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    /// Get the WebSocket URL
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Request an airdrop to the given address
    pub async fn airdrop(&self, pubkey: &Pubkey, lamports: u64) -> Result<Signature> {
        let client = self.rpc_client();
        let signature = client
            .request_airdrop(pubkey, lamports)
            .context("Failed to request airdrop")?;

        // Wait for confirmation
        loop {
            if let Ok(status) = client.get_signature_status(&signature) {
                if status.is_some() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(signature)
    }

    /// Find a free TCP port
    fn find_free_port() -> u16 {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("Failed to bind to find free port");
        listener.local_addr()
            .expect("Failed to get local addr")
            .port()
    }
}

impl Drop for SurfpoolManager {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            info!("Stopping Surfpool process");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signer::{keypair::Keypair, Signer};

    #[tokio::test]
    #[ignore] // Requires surfpool to be installed
    async fn test_surfpool_lifecycle() {
        let config = SurfpoolConfig::default();
        let manager = SurfpoolManager::start(config).await.unwrap();

        let client = manager.rpc_client();
        let version = client.get_version().unwrap();
        assert!(!version.solana_core.is_empty());
    }

    #[tokio::test]
    #[ignore] // Requires surfpool to be installed
    async fn test_airdrop() {
        let config = SurfpoolConfig::default();
        let manager = SurfpoolManager::start(config).await.unwrap();

        let keypair = Keypair::new();
        let signature = manager.airdrop(&keypair.pubkey(), 1_000_000_000).await.unwrap();
        assert_ne!(signature, Signature::default());

        let client = manager.rpc_client();
        let balance = client.get_balance(&keypair.pubkey()).unwrap();
        assert_eq!(balance, 1_000_000_000);
    }
}
