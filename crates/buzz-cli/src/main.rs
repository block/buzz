#[tokio::main]
async fn main() {
    install_rustls_crypto_provider();
    std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
}

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    #[test]
    fn installs_rustls_crypto_provider_before_websocket_setup() {
        assert!(rustls::crypto::CryptoProvider::get_default().is_none());

        super::install_rustls_crypto_provider();

        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
