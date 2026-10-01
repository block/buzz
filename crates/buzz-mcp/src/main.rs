#![deny(unsafe_code)]
//! Main entry point for the `buzz-mcp` binary.

use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

use buzz_mcp::{run_stdio, OrbitMcpServer};

/// Orbit Model Context Protocol Server CLI
#[derive(Parser, Debug)]
#[command(name = "buzz-mcp", about = "Orbit Model Context Protocol (MCP) Server")]
struct Cli {
    /// Run with stdio transport (default)
    #[arg(long, default_value_t = true)]
    stdio: bool,

    /// Run with HTTP/SSE transport
    #[arg(long)]
    sse: bool,

    /// Port for SSE / HTTP transport
    #[arg(long, default_value_t = 3456)]
    port: u16,

    /// Optional root base directory for Orbit brain data (defaults to ~/.orbit/brain)
    #[arg(long)]
    base_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // If stdio mode, log to stderr to avoid corrupting stdio transport
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cli = Cli::parse();

    let server = if let Some(base_path) = cli.base_dir {
        OrbitMcpServer::open_at(base_path)?
    } else {
        OrbitMcpServer::default_local()?
    };

    if cli.sse {
        tracing::info!("Starting Orbit MCP server over HTTP on port {}", cli.port);
        // Minimal HTTP health/info endpoint for SSE / HTTP discovery
        let app = axum::Router::new()
            .route("/health", axum::routing::get(|| async { "OK" }))
            .route("/info", axum::routing::get(|| async {
                serde_json::json!({
                    "name": "orbit-mcp",
                    "version": env!("CARGO_PKG_VERSION"),
                    "protocol": "2024-11-05",
                }).to_string()
            }));

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cli.port)).await?;
        axum::serve(listener, app).await?;
    } else {
        tracing::info!("Starting Orbit MCP server over stdio");
        run_stdio(server).await?;
    }

    Ok(())
}
