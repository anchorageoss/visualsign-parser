use std::net::TcpListener;
use std::process::{Child, Command};
use std::time::Duration;

use anyhow::Context;
use solana_client::rpc_client::RpcClient;

use super::SurfpoolConfig;

/// Manages a surfpool subprocess for the lifetime of a test.
///
/// Spawns the `surfpool` binary, waits for its RPC to become ready, and kills
/// it on drop. Uses `std::process::Command` (not tokio) so that `Drop` can
/// kill the child synchronously.
pub struct SurfpoolManager {
    process: Child,
    pub rpc_url: String,
}

impl SurfpoolManager {
    /// Spawn surfpool and wait up to `timeout` for it to be ready.
    pub async fn start(config: SurfpoolConfig) -> anyhow::Result<Self> {
        let rpc_port = config.rpc_port.unwrap_or_else(Self::find_free_port);
        let rpc_url = format!("http://127.0.0.1:{rpc_port}");

        // surfpool v1.1.1 CLI: `surfpool start [OPTIONS]`
        // --no-tui   : disable interactive dashboard (required for non-TTY / CI)
        // --no-deploy: skip looking for txtx.yml manifest in cwd
        let mut args = vec![
            "start".to_string(),
            "--port".to_string(),
            rpc_port.to_string(),
            "--no-tui".to_string(),
            "--no-deploy".to_string(),
        ];
        if let Some(url) = &config.fork_url {
            args.extend(["--rpc-url".to_string(), url.clone()]);
        }
        let _ = config.rpc_port; // consumed above via rpc_port local

        tracing::debug!("spawning: surfpool {}", args.join(" "));

        let process = Command::new("surfpool")
            .args(&args)
            .spawn()
            .context(
                "failed to spawn surfpool — install with: \
                 cargo install surfpool --git https://github.com/txtx/surfpool",
            )?;

        Ok(Self { process, rpc_url })
    }

    /// Poll the RPC until surfpool answers `getVersion`, or return an error
    /// after `timeout`.
    pub async fn wait_ready(&self, timeout: Duration) -> anyhow::Result<()> {
        let client = self.rpc_client();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if client.get_version().is_ok() {
                tracing::debug!("surfpool ready at {}", self.rpc_url);
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "surfpool did not become ready within {timeout:?} at {}",
                    self.rpc_url
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Return a synchronous `RpcClient` pointed at this surfpool instance.
    pub fn rpc_client(&self) -> RpcClient {
        RpcClient::new(self.rpc_url.clone())
    }

    /// Bind port 0 and return the assigned port number.
    ///
    /// Note: there is a brief TOCTOU window between dropping the listener and
    /// surfpool binding the port. This is acceptable in test environments.
    fn find_free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .expect("failed to bind ephemeral port")
            .local_addr()
            .expect("no local addr")
            .port()
    }
}

impl Drop for SurfpoolManager {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
