#[tokio::main]
async fn main() {
    // Workspace feature unification compiles both rustls providers (ring +
    // aws-lc-rs). Without an explicit install, the first wss:// publish panics
    // inside tokio-tungstenite (#2308 / #2329 / #2457). Idempotent if another
    // path already installed a provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
}
