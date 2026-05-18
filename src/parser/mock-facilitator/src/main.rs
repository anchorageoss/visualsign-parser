// TODO(#231): Remove these exemptions and fix violations in a follow-up PR.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("MOCK_FACILITATOR_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8090);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("mock_facilitator {} listening on {addr}", env!("VERSION"));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, mock_facilitator::router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await.expect("failed to listen for ctrl-c");
    println!("Shutting down mock_facilitator");
}
