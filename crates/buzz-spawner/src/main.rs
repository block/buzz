//! `buzz-spawner` binary entry point.

use std::sync::Arc;

use anyhow::Result;
use buzz_spawner::{
    config::Config,
    container::{ContainerOps, DockerOps},
    daemon::Daemon,
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "buzz_spawner=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let containers: Arc<dyn ContainerOps> = Arc::new(DockerOps::connect()?);

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("received SIGINT");
            let _ = shutdown_tx.send(true);
        }
    });

    let mut daemon = Daemon::start(config, containers).await?;
    // An initial pass before the first tick, so a restart re-adopts running
    // containers and re-requests any attestation that expired while down.
    daemon.reconcile().await?;
    daemon.run(shutdown_rx).await
}
