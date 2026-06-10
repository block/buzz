use std::net::SocketAddr;
use std::sync::Arc;

use sprout_pair_relay::{run_server, Relay};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let addr: SocketAddr = ([127, 0, 0, 1], 5000).into();
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("fatal: failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    let relay = Arc::new(Relay::new());
    run_server(listener, relay).await;
}
