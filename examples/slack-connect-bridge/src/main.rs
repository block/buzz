#![deny(unsafe_code)]
//! Operator-run reference bridge between Buzz channels and Slack Connect.

mod bridge;
mod config;
mod slack;
mod state;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;

use crate::{
    bridge::Bridge,
    config::Config,
    slack::{run_webhook_server, WebhookServerState},
};

const DELIVERY_QUEUE_CAPACITY: usize = 256;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;
    run().await
}

async fn run() -> Result<()> {
    let config = Config::from_env()?;
    let listen_addr = config.listen_addr;
    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("failed to bind Slack webhook listener on {listen_addr}"))?;
    let (delivery_tx, delivery_rx) = mpsc::channel(DELIVERY_QUEUE_CAPACITY);
    let (webhook_state, webhook_control) =
        WebhookServerState::new(config.slack_signing_secret.clone(), delivery_tx);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server = tokio::spawn(run_webhook_server(listener, webhook_state, shutdown_rx));

    let bridge = Bridge::initialize(config, delivery_rx, webhook_control).await?;
    tokio::select! {
        bridge_result = bridge.run() => {
            let _ = shutdown_tx.send(true);
            let server_result = server.await.context("Slack webhook server task panicked")?;
            bridge_result?;
            server_result
        }
        server_result = &mut server => {
            let _ = shutdown_tx.send(true);
            server_result.context("Slack webhook server task panicked")??;
            anyhow::bail!("Slack webhook server stopped unexpectedly")
        }
    }
}

fn init_tracing() -> Result<()> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new("buzz_slack_connect_bridge=info"),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing: {error}"))
}
