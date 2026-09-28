//! Local TLS verification for agents started by trusted external launchers.
use crate::AgentError;
#[cfg(test)]
use std::{path::Path, sync::Arc};

/// Path to the immutable certificate snapshot supplied by the launcher.
pub const ROOTS_ENV: &str = "BUZZ_SANDBOX_TLS_ROOTS";

/// Native trust is captured before confinement. Verification remains enabled,
/// using rustls locally instead of macOS trustd (which can bypass network policy).
pub fn http_builder() -> Result<reqwest::ClientBuilder, AgentError> {
    buzz_runtime_support::tls::http_builder().map_err(|e| AgentError::Llm(e.to_string()))
}
#[cfg(test)]
fn builder_with_roots(
    builder: reqwest::ClientBuilder,
    path: &Path,
) -> Result<reqwest::ClientBuilder, AgentError> {
    buzz_runtime_support::tls::builder_with_roots(builder, path)
        .map_err(|e| AgentError::Llm(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    #[test]
    fn missing_trust_snapshot_fails_closed() {
        assert!(builder_with_roots(
            reqwest::Client::builder(),
            Path::new("/nonexistent/sandbox-certs.json")
        )
        .is_err());
    }
    #[tokio::test]
    async fn snapshot_verifier_accepts_trusted_cert_and_rejects_wrong_hostname() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let der = cert.cert.der().clone();
        let key = tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(
            cert.signing_key.serialize_der(),
        );
        let config = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key.into())
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(mut stream) = acceptor.accept(socket).await {
                        let mut buffer = [0; 4096];
                        let _ = stream.read(&mut buffer).await;
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
                    }
                });
            }
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            serde_json::to_vec(&vec![der.as_ref().to_vec()]).unwrap(),
        )
        .unwrap();
        let client = builder_with_roots(reqwest::Client::builder().no_proxy(), file.path())
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            client
                .get(format!("https://localhost:{port}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert!(client
            .get(format!("https://127.0.0.1:{port}"))
            .send()
            .await
            .is_err());
        let untrusted = reqwest::Client::builder()
            .no_proxy()
            .tls_certs_only(Vec::<reqwest::Certificate>::new())
            .build()
            .unwrap();
        assert!(untrusted
            .get(format!("https://localhost:{port}"))
            .send()
            .await
            .is_err());
        task.abort();
    }
}
