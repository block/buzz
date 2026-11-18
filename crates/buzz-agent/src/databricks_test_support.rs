//! Local-only synthetic OAuth/catalog transport. No browser, ambient cache or credentials.
use super::*;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    task::JoinSet,
};
use tokio_rustls::{rustls, TlsAcceptor};

#[derive(Clone, Debug)]
pub(super) struct Request {
    pub path: String,
    pub body: String,
    pub authorization: Option<String>,
}
impl Request {
    pub fn form(&self) -> HashMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect()
    }
}
pub(super) enum Reply {
    Json(u16, serde_json::Value),
    #[cfg(unix)]
    Raw(String),
    #[cfg(unix)]
    Stall,
}
pub(super) struct Server {
    pub base: String,
    pub cert: Option<reqwest::Certificate>,
    pub requests: Arc<Mutex<Vec<Request>>>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub active: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(
        tls: bool,
        handler: impl Fn(&str, &Request) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!(
            "{}://localhost:{}",
            if tls { "https" } else { "http" },
            listener.local_addr().unwrap().port()
        );
        let (acceptor, cert) = if tls {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let der = cert.cert.der().clone();
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![der.clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der())
                    .into(),
            )
            .unwrap();
            (
                Some(TlsAcceptor::from(Arc::new(config))),
                Some(reqwest::Certificate::from_der(der.as_ref()).unwrap()),
            )
        } else {
            (None, None)
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let handler = Arc::new(handler);
        let (base2, requests2, active2) = (base.clone(), requests.clone(), active.clone());
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.unwrap();
                        let (handler, base, requests, active, acceptor) = (handler.clone(), base2.clone(), requests2.clone(), active2.clone(), acceptor.clone());
                        children.spawn(async move {
                            if let Some(acceptor) = acceptor {
                                if let Ok(stream) = acceptor.accept(stream).await { serve(stream, base, handler, requests, active).await; }
                            } else { serve(stream, base, handler, requests, active).await; }
                        });
                    }
                    _ = children.join_next(), if !children.is_empty() => {}
                }
            }
        });
        Self {
            base,
            cert,
            requests,
            active,
            task,
        }
    }
    pub fn builder(&self) -> ClientBuilder {
        let builder = Client::builder().no_proxy();
        match &self.cert {
            Some(cert) => builder.add_root_certificate(cert.clone()),
            None => builder,
        }
    }
    pub fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path.starts_with(path))
            .count()
    }
    #[cfg(unix)]
    pub async fn wait_for(&self, path: &str) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.count(path) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    base: String,
    handler: Arc<impl Fn(&str, &Request) -> Reply>,
    requests: Arc<Mutex<Vec<Request>>>,
    active: Arc<AtomicUsize>,
) {
    active.fetch_add(1, Ordering::SeqCst);
    let _guard = Active(active);
    let mut bytes = Vec::new();
    let mut buf = [0; 4096];
    let header_end = loop {
        let Ok(n) = stream.read(&mut buf).await else {
            return;
        };
        if n == 0 {
            return;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        assert!(bytes.len() < 32 * 1024);
    };
    let header = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length = header
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(|v| v.parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let n = stream.read(&mut buf).await.unwrap();
        if n == 0 {
            return;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let request = Request {
        path: header
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .into(),
        body: String::from_utf8(bytes[header_end..header_end + length].to_vec()).unwrap(),
        authorization: header.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.trim().into())
        }),
    };
    requests.lock().unwrap().push(request.clone());
    let response = match handler(&base, &request) {
        Reply::Json(status, body) => {
            let body = body.to_string();
            format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        }
        #[cfg(unix)]
        Reply::Raw(raw) => raw,
        #[cfg(unix)]
        Reply::Stall => {
            // Send headers, then stall the body until the client cancels.
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n")
                .await;
            let _ = stream.flush().await;
            let _ = stream.read(&mut buf).await;
            return;
        }
    };
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[derive(Default)]
pub(super) struct Opener {
    pub urls: Mutex<Vec<String>>,
    pub fail: bool,
    pub callback: bool,
    pub denial: bool,
}
impl BrowserOpener for Opener {
    fn open(&self, raw: &str) -> Result<(), String> {
        self.urls.lock().unwrap().push(raw.into());
        if self.fail {
            return Err(format!("secret-opener-error {raw}"));
        }
        if self.callback {
            let url = Url::parse(raw).unwrap();
            let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(params["code_challenge_method"], "S256");
            assert_eq!(params["client_id"], "databricks-cli");
            let mut redirect = Url::parse(&params["redirect_uri"]).unwrap();
            if self.denial {
                redirect
                    .query_pairs_mut()
                    .append_pair("error", "secret-denial");
            } else {
                redirect
                    .query_pairs_mut()
                    .append_pair("code", "synthetic-code")
                    .append_pair("state", &params["state"]);
            }
            tokio::spawn(async move {
                let _ = Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .get(redirect)
                    .send()
                    .await;
            });
        }
        Ok(())
    }
}
pub(super) fn discovery(base: &str) -> Reply {
    Reply::Json(
        200,
        serde_json::json!({"authorization_endpoint": format!("{base}/authorize"), "token_endpoint": format!("{base}/token")}),
    )
}
#[cfg(unix)]
pub(super) fn seed(root: &Path, base: &str, namespace: &str, token: &str, expired: bool) {
    use sha2::{Digest, Sha256};
    let key = format!(
        "{base}/oidc/.well-known/oauth-authorization-server|databricks-cli|all-apis,offline_access"
    );
    let dir = root.join(namespace);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{}.json", hex::encode(Sha256::digest(key)))), serde_json::json!({"access_token": token, "refresh_token": "synthetic-refresh", "expires_at": if expired { 1 } else { 4_000_000_000u64 }}).to_string()).unwrap();
}
#[cfg(unix)]
pub(super) fn chunked(status: u16, body: &str) -> Reply {
    Reply::Raw(format!("HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len()))
}
