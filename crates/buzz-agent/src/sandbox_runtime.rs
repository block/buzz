//! Trusted-launcher services for a confined native agent. No arbitrary URLs or file operations.
use crate::{
    auth::TokenSource,
    config::{Config, Provider},
    AgentError,
};
use async_trait::async_trait;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::path::Path;
use std::{
    io::Read,
    sync::{Arc, OnceLock},
    time::Duration,
};

/// Nonsecret startup marker. Set to `stdin-v1` and send a four-byte big-endian
/// length followed by the JSON [`BrokerConfig`] before ACP input. Never put the
/// capability in the environment or arguments.
///
/// The trusted launcher must isolate the agent from tool process inspection.
/// On macOS this requires inherited kernel denials for other-process `process-info*` and
/// `mach-priv-task-port` and `kern.procargs*` sysctl reads, including processes
/// in the same sandbox. The startup channel does not replace OS isolation.
pub const BROKER_ENV: &str = "BUZZ_SANDBOX_AUTH_BROKER";
/// Path to the immutable certificate snapshot supplied by the launcher.
pub const ROOTS_ENV: &str = "BUZZ_SANDBOX_TLS_ROOTS";

/// Connection capability delivered privately before any tools are started.
#[derive(Clone, Serialize, Deserialize)]
pub struct BrokerConfig {
    /// Loopback TCP port selected by the trusted launcher.
    pub port: u16,
    /// Random bearer capability authenticating requests to the launcher.
    pub secret: String,
    /// Exact configured model service URL; prevents cross-provider reuse.
    pub host: String,
}
#[derive(Serialize, Deserialize)]
struct Request {
    rejected: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Response {
    token: String,
}
#[derive(Clone)]
struct BrokerState {
    secret: String,
    source: Arc<dyn TokenSource>,
}

static BROKER: OnceLock<BrokerConfig> = OnceLock::new();
const MAX_BOOTSTRAP_BYTES: usize = 16 * 1024;

pub(crate) fn initialize() -> Result<(), AgentError> {
    let Some(marker) = std::env::var_os(BROKER_ENV) else {
        return Ok(());
    };
    if marker != "stdin-v1" {
        return Err(AgentError::Llm(
            "broker capability must be delivered through stdin-v1".into(),
        ));
    }
    // Unlike clearing environ, this denies same-UID reads of the initial
    // environment, memory, and descriptors through procfs/ptrace on Linux.
    #[cfg(target_os = "linux")]
    nix::sys::prctl::set_dumpable(false)
        .map_err(|_| AgentError::Llm("cannot protect broker process from inspection".into()))?;
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return Err(AgentError::Llm(
            "credential broker unsupported on this platform".into(),
        ));
    }
    let broker = read_bootstrap(&mut std::io::stdin().lock())?;
    BROKER
        .set(broker)
        .map_err(|_| AgentError::Llm("broker already initialized".into()))
}

fn read_bootstrap(reader: &mut impl Read) -> Result<BrokerConfig, AgentError> {
    let invalid = |_| AgentError::Llm("invalid broker startup frame".into());
    let mut header = [0; 4];
    reader.read_exact(&mut header).map_err(invalid)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_BOOTSTRAP_BYTES {
        return Err(AgentError::Llm("invalid broker startup frame size".into()));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).map_err(invalid)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| AgentError::Llm("invalid broker startup configuration".into()))
}

pub(crate) fn configured() -> Result<bool, AgentError> {
    if BROKER.get().is_none() && std::env::var_os(BROKER_ENV).is_some() {
        return Err(AgentError::Llm("broker startup was not initialized".into()));
    }
    Ok(BROKER.get().is_some())
}

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

pub(crate) fn token_source(cfg: &Config) -> Result<Option<Arc<dyn TokenSource>>, AgentError> {
    let Some(broker) = BROKER.get() else {
        if std::env::var_os(BROKER_ENV).is_some() {
            return Err(AgentError::Llm("broker startup was not initialized".into()));
        }
        return Ok(None);
    };
    if !matches!(cfg.provider, Provider::Databricks | Provider::DatabricksV2)
        || !cfg.api_key.is_empty()
    {
        return Ok(None);
    }
    if broker.host != cfg.base_url || broker.port == 0 || broker.secret.len() < 32 {
        return Err(AgentError::Llm(
            "sandbox auth broker provider mismatch".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| AgentError::Llm("sandbox auth client unavailable".into()))?;
    Ok(Some(Arc::new(BrokerClient {
        broker: broker.clone(),
        client,
    })))
}
struct BrokerClient {
    broker: BrokerConfig,
    client: reqwest::Client,
}
impl BrokerClient {
    async fn request(&self, rejected: Option<&str>) -> Result<String, AgentError> {
        let response = self
            .client
            .post(format!("http://127.0.0.1:{}/token", self.broker.port))
            .bearer_auth(&self.broker.secret)
            .json(&Request {
                rejected: rejected.map(str::to_owned),
            })
            .send()
            .await
            .map_err(|_| AgentError::Llm("sandbox sign-in service unavailable".into()))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(AgentError::LlmAuth(
                "Refresh sign-in from the desktop model picker".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(AgentError::Llm(
                "sandbox sign-in service unavailable".into(),
            ));
        }
        Ok(response
            .json::<Response>()
            .await
            .map_err(|_| AgentError::Llm("invalid sandbox sign-in response".into()))?
            .token)
    }
}
#[async_trait]
impl TokenSource for BrokerClient {
    async fn bearer(&self) -> Result<String, AgentError> {
        self.request(None).await
    }
    async fn refresh_now(&self, rejected: &str) -> Result<String, AgentError> {
        self.request(Some(rejected)).await
    }
}

/// The launcher fixes the workspace and cache. The child can only request a
/// bearer or refresh a rejected bearer, never choose a destination or open a browser.
/// The task handle preserves the server result for the launcher to observe.
pub fn serve(
    listener: tokio::net::TcpListener,
    host: &str,
    secret: String,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, AgentError> {
    let source =
        crate::auth::PkceOAuthTokenSource::new(crate::llm::databricks_pkce_config(host, None))?;
    Ok(serve_source(listener, secret, source))
}
fn serve_source(
    listener: tokio::net::TcpListener,
    secret: String,
    source: Arc<dyn TokenSource>,
) -> tokio::task::JoinHandle<std::io::Result<()>> {
    let app = Router::new()
        .route("/token", post(token))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(BrokerState { secret, source });
    tokio::spawn(async move { axum::serve(listener, app).await })
}
async fn token(
    State(state): State<BrokerState>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<Json<Response>, StatusCode> {
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        != Some(format!("Bearer {}", state.secret).as_str())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let result = tokio::time::timeout(Duration::from_secs(55), async {
        match request.rejected {
            Some(value) => state.source.refresh_now(&value).await,
            None => state.source.bearer_no_browser().await,
        }
    })
    .await
    .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?;
    let token = result.map_err(|error| match error {
        AgentError::LlmAuth(_) => StatusCode::UNAUTHORIZED,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    })?;
    Ok(Json(Response { token }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_preserves_following_acp_input() {
        let config = BrokerConfig {
            port: 1234,
            secret: "synthetic-capability".into(),
            host: "https://model.example".into(),
        };
        let bytes = serde_json::to_vec(&config).unwrap();
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(bytes);
        frame.extend(b"{\"jsonrpc\":\"2.0\"}\n");
        let mut reader = std::io::Cursor::new(frame);
        assert_eq!(read_bootstrap(&mut reader).unwrap().secret, config.secret);
        let mut acp = String::new();
        reader.read_to_string(&mut acp).unwrap();
        assert_eq!(acp, "{\"jsonrpc\":\"2.0\"}\n");
    }

    #[test]
    fn bootstrap_rejects_invalid_frames_without_echoing_their_contents() {
        for frame in [
            vec![],
            vec![0, 0],
            0u32.to_be_bytes().to_vec(),
            ((MAX_BOOTSTRAP_BYTES + 1) as u32).to_be_bytes().to_vec(),
            [8u32.to_be_bytes().as_slice(), b"private"].concat(),
            [7u32.to_be_bytes().as_slice(), b"private"].concat(),
        ] {
            let error = read_bootstrap(&mut frame.as_slice()).err().unwrap();
            assert!(!error.to_string().contains("private"));
        }
    }

    #[tokio::test]
    async fn broker_rejects_unauthorized_callers_and_serves_fixed_source() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = serve_source(
            listener,
            "test-broker-capability".into(),
            Arc::new(crate::auth::StaticTokenSource::new("synthetic-bearer")),
        );
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{port}/token");
        let denied = client
            .post(&url)
            .json(&Request { rejected: None })
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let allowed = client
            .post(&url)
            .bearer_auth("test-broker-capability")
            .json(&Request { rejected: None })
            .send()
            .await
            .unwrap();
        assert_eq!(
            allowed.json::<Response>().await.unwrap().token,
            "synthetic-bearer"
        );
        task.abort();
    }
    struct FailingSource {
        authentication: bool,
    }
    #[async_trait]
    impl TokenSource for FailingSource {
        async fn bearer(&self) -> Result<String, AgentError> {
            if self.authentication {
                Err(AgentError::LlmAuth("private auth diagnostic".into()))
            } else {
                Err(AgentError::Llm("private infrastructure diagnostic".into()))
            }
        }
    }

    #[tokio::test]
    async fn broker_preserves_authentication_and_service_error_classes() {
        for authentication in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let secret = "test-broker-capability".to_owned();
            let task = serve_source(
                listener,
                secret.clone(),
                Arc::new(FailingSource { authentication }),
            );
            let client = BrokerClient {
                broker: BrokerConfig {
                    port,
                    secret,
                    host: "unused".into(),
                },
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
            };
            for result in [client.bearer().await, client.refresh_now("rejected").await] {
                let error = result.unwrap_err();
                assert_eq!(matches!(error, AgentError::LlmAuth(_)), authentication);
                assert_eq!(matches!(error, AgentError::Llm(_)), !authentication);
                assert!(!error.to_string().contains("private"));
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn broker_timeout_response_is_a_service_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = Router::new().route("/token", post(|| async { StatusCode::GATEWAY_TIMEOUT }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await });
        let client = BrokerClient {
            broker: BrokerConfig {
                port,
                secret: "test-broker-capability".into(),
                host: "unused".into(),
            },
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        };
        assert!(matches!(client.bearer().await, Err(AgentError::Llm(_))));
        task.abort();
    }

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
