use super::config::SurfpoolConfig;
use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey, signature::Signature};
use std::net::TcpListener;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Manages the lifecycle of a Surfpool validator instance.
///
/// Spawns a `surfpool` subprocess on [`start`](Self::start), polls until the
/// RPC server is ready, and kills the process on [`Drop`].
pub struct SurfpoolManager {
    process: Option<Child>,
    rpc_url: String,
    ws_url: String,
}

impl SurfpoolManager {
    /// Start a new Surfpool instance with the given configuration.
    pub async fn start(config: SurfpoolConfig) -> Result<Self> {
        info!("Starting Surfpool with config: {:?}", config);

        let rpc_port = config.rpc_port.map_or_else(Self::find_free_port, Ok)?;
        let ws_port = config.ws_port.map_or_else(Self::find_free_port, Ok)?;

        let rpc_url = format!("http://127.0.0.1:{rpc_port}");
        let ws_url = format!("ws://127.0.0.1:{ws_port}");

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

        let child = Command::new("surfpool")
            .args(&args)
            .spawn()
            .context("Failed to spawn surfpool process. Is surfpool installed?")?;

        let manager = Self {
            process: Some(child),
            rpc_url: rpc_url.clone(),
            ws_url,
        };

        manager
            .wait_ready()
            .await
            .context("Surfpool failed to become ready")?;

        info!("Surfpool started successfully at {}", rpc_url);
        Ok(manager)
    }

    /// Poll the RPC server until it responds (up to 30 attempts, 500ms apart).
    pub async fn wait_ready(&self) -> Result<()> {
        let client = self.rpc_client();
        let max_attempts = 30;
        let delay = Duration::from_millis(500);

        for attempt in 1..=max_attempts {
            debug!(
                "Checking if Surfpool is ready (attempt {}/{})",
                attempt, max_attempts
            );
            match client.get_version() {
                Ok(version) => {
                    info!("Surfpool is ready! Version: {:?}", version);
                    return Ok(());
                }
                Err(e) => {
                    if attempt == max_attempts {
                        return Err(anyhow::anyhow!(
                            "Surfpool did not become ready after {max_attempts} attempts: {e}"
                        ));
                    }
                    warn!("Surfpool not ready yet (attempt {}): {}", attempt, e);
                    thread::sleep(delay);
                }
            }
        }

        Err(anyhow::anyhow!("Surfpool readiness check failed"))
    }

    /// Return an RPC client pointed at this instance.
    pub fn rpc_client(&self) -> RpcClient {
        RpcClient::new_with_commitment(self.rpc_url.clone(), CommitmentConfig::confirmed())
    }

    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Request an airdrop and wait for confirmation (bounded).
    pub async fn airdrop(&self, pubkey: &Pubkey, lamports: u64) -> Result<Signature> {
        let client = self.rpc_client();
        let signature = client
            .request_airdrop(pubkey, lamports)
            .context("Failed to request airdrop")?;

        let max_attempts = 60;
        for _ in 0..max_attempts {
            if let Ok(Some(_status)) = client.get_signature_status(&signature) {
                return Ok(signature);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err(anyhow::anyhow!(
            "Airdrop confirmation timed out after {max_attempts} attempts"
        ))
    }

    /// Find a free TCP port by binding to port 0.
    fn find_free_port() -> Result<u16> {
        let listener = TcpListener::bind("127.0.0.1:0").context("Failed to bind ephemeral port")?;
        let port = listener
            .local_addr()
            .context("Failed to get local address")?
            .port();
        Ok(port)
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
