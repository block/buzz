//! Manual/e2e harness: `cargo run -p buzz-remote-access --example serve -- <member-pubkey-hex> <origin> [port]`
//! Starts the loopback listener with real shell terminals enabled and prints `listening 127.0.0.1:<port>`.
use std::sync::Arc;

use buzz_remote_access::{PtyBackend, RemoteServer, ServerConfig, TerminalMode};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let member = nostr::PublicKey::parse(&a[1]).expect("member pubkey");
    let mut cfg = ServerConfig::new(member, "dev-desktop", vec![a[2].clone()]);
    cfg.backend = Some(Arc::new(PtyBackend::new(TerminalMode::Shell)));
    let srv = RemoteServer::start(cfg, a.get(3).and_then(|p| p.parse().ok()).unwrap_or(0)).await.unwrap();
    srv.set_terminals_enabled(true);
    println!("listening {}", srv.addr);
    tokio::signal::ctrl_c().await.ok();
}
