#[tokio::main]
async fn main() {
    // Select the rustls crypto provider explicitly.
    //
    // The workspace ends up with BOTH providers compiled in: crates pin
    // `rustls` to `ring`, while `reqwest` -> `hyper-rustls` /
    // `rustls-platform-verifier` enable `aws-lc-rs`. With two providers
    // available rustls cannot pick one at runtime and panics on the first TLS
    // handshake ("Could not automatically determine the process-level
    // CryptoProvider from Rustls crate features"). A crate-level feature pin
    // cannot fix this because Cargo unifies features across the graph, so the
    // choice has to be made here. Ignore the error: a provider may already be
    // installed by an embedding caller, which is not a failure.
    let _ = rustls::crypto::ring::default_provider().install_default();
    std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
}
