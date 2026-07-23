#[tokio::main]
async fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
}
