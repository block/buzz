//! Production implementations of the ports: plain HTTP fetch and the Buzz client.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::ports::{CatalogRow, Fetcher, PortError, Relay, UploadedBlob, Uploader};
use crate::client::{normalize_write_response, parse_retry_hint_text, BuzzClient};
use crate::error::CliError;

/// Longest `Retry-After` honored, in seconds.
const MAX_RETRY_AFTER_SECS: u64 = 60;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Backoff and attempt limits for [`HttpFetcher`].
#[derive(Debug, Clone, Copy)]
pub struct FetchPolicy {
    pub attempts: u32,
    pub base_delay: Duration,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            base_delay: Duration::from_secs(1),
        }
    }
}

/// Fetches image sources over HTTP(S) or from `file://` paths.
///
/// Sends no credentials, caps the body at `max_bytes`, limits requests per
/// host and retries transient failures with backoff and `Retry-After`.
pub struct HttpFetcher {
    http: reqwest::Client,
    max_bytes: u64,
    per_host: usize,
    hosts: Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
    policy: FetchPolicy,
}

impl HttpFetcher {
    pub fn new(max_bytes: u64, per_host: usize, policy: FetchPolicy) -> Result<Self, CliError> {
        let http = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent("buzz-emoji-migrate")
            .build()
            .map_err(|e| CliError::Other(format!("http client init failed: {e}")))?;
        Ok(Self {
            http,
            max_bytes,
            per_host: per_host.max(1),
            hosts: Mutex::default(),
            policy,
        })
    }

    fn host_slots(&self, host: &str) -> Arc<tokio::sync::Semaphore> {
        let mut hosts = self.hosts.lock().unwrap_or_else(|p| p.into_inner());
        hosts
            .entry(host.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(self.per_host)))
            .clone()
    }

    fn read_local(&self, url: &url::Url) -> Result<Vec<u8>, PortError> {
        let path = url
            .to_file_path()
            .map_err(|()| PortError::Permanent(format!("not a local file path: {url}")))?;
        let file = std::fs::File::open(&path)
            .map_err(|e| PortError::Permanent(format!("cannot open {}: {e}", path.display())))?;
        let mut buf = Vec::new();
        file.take(self.max_bytes + 1)
            .read_to_end(&mut buf)
            .map_err(|e| PortError::Permanent(format!("cannot read {}: {e}", path.display())))?;
        if buf.len() as u64 > self.max_bytes {
            return Err(too_large(self.max_bytes));
        }
        Ok(buf)
    }

    async fn get_once(&self, url: &url::Url) -> Result<Vec<u8>, PortError> {
        let mut resp = self
            .http
            .get(url.clone())
            .send()
            .await
            .map_err(|e| PortError::transient(format!("request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let retry_after = retry_after(resp.headers());
            return Err(classify_status(status.as_u16(), retry_after));
        }
        if resp.content_length().is_some_and(|n| n > self.max_bytes) {
            return Err(too_large(self.max_bytes));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| PortError::transient(format!("body read failed: {e}")))?
        {
            if (body.len() + chunk.len()) as u64 > self.max_bytes {
                return Err(too_large(self.max_bytes));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn too_large(max: u64) -> PortError {
    PortError::Permanent(format!("image is larger than the {max} byte cap"))
}

/// `Retry-After` in whole seconds, capped. HTTP-date values are ignored.
pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.min(MAX_RETRY_AFTER_SECS)))
}

/// Map an HTTP failure status to a retry class.
pub fn classify_status(status: u16, retry_after: Option<Duration>) -> PortError {
    match status {
        408 | 425 | 429 | 500..=599 => PortError::Transient {
            message: format!("HTTP {status}"),
            retry_after,
        },
        _ => PortError::Permanent(format!("HTTP {status}")),
    }
}

impl Fetcher for HttpFetcher {
    async fn fetch(&self, source: &str) -> Result<Vec<u8>, PortError> {
        let url = url::Url::parse(source)
            .map_err(|e| PortError::Permanent(format!("invalid source '{source}': {e}")))?;
        match url.scheme() {
            "file" => return self.read_local(&url),
            "http" | "https" => {}
            other => {
                return Err(PortError::Permanent(format!(
                    "unsupported scheme '{other}'"
                )))
            }
        }
        let slots = self.host_slots(url.host_str().unwrap_or_default());
        let _slot = slots
            .acquire()
            .await
            .map_err(|e| PortError::Fatal(format!("host slots closed: {e}")))?;
        let mut last = PortError::transient("no attempt made");
        for attempt in 0..self.policy.attempts.max(1) {
            match self.get_once(&url).await {
                Ok(body) => return Ok(body),
                Err(PortError::Transient {
                    message,
                    retry_after,
                }) => {
                    let backoff = self.policy.base_delay * 2u32.pow(attempt.min(5));
                    let wait = retry_after
                        .unwrap_or(backoff)
                        .min(Duration::from_secs(MAX_RETRY_AFTER_SECS));
                    last = PortError::Transient {
                        message,
                        retry_after,
                    };
                    if attempt + 1 < self.policy.attempts {
                        tokio::time::sleep(wait).await;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

/// Map a client error to a retry class.
pub fn map_cli_error(e: CliError) -> PortError {
    match e {
        CliError::Relay { status, body } => match status {
            401 | 403 => PortError::Fatal(format!("relay refused credentials ({status}): {body}")),
            429 => PortError::Transient {
                retry_after: parse_retry_hint_text(&body).map(Duration::from_secs),
                message: format!("relay rate limit: {body}"),
            },
            408 | 500..=599 => PortError::transient(format!("relay error {status}: {body}")),
            _ => PortError::Permanent(format!("relay error {status}: {body}")),
        },
        CliError::Auth(m) | CliError::Key(m) => PortError::Fatal(m),
        CliError::Network(_) | CliError::DeliveryUnknown(_) => PortError::transient(e.to_string()),
        other => PortError::Permanent(other.to_string()),
    }
}

/// Read a write response: the event id on `accepted: true`, otherwise an error.
pub fn interpret_write_response(raw: &str) -> Result<String, PortError> {
    let v: serde_json::Value = serde_json::from_str(&normalize_write_response(raw))
        .map_err(|e| PortError::Permanent(format!("unreadable relay response: {e}")))?;
    let message = v
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    if v.get("accepted").and_then(|a| a.as_bool()) == Some(true) {
        return Ok(v
            .get("event_id")
            .and_then(|i| i.as_str())
            .unwrap_or_default()
            .to_string());
    }
    if message.starts_with("rate-limited") {
        return Err(PortError::Transient {
            retry_after: parse_retry_hint_text(message).map(Duration::from_secs),
            message: message.to_string(),
        });
    }
    Err(PortError::Permanent(format!(
        "relay rejected the event: {message}"
    )))
}

/// The Buzz client as uploader and relay.
pub struct BuzzPorts<'a> {
    pub client: &'a BuzzClient,
}

impl Uploader for BuzzPorts<'_> {
    async fn upload(&self, bytes: Vec<u8>) -> Result<UploadedBlob, PortError> {
        let desc = self
            .client
            .upload_bytes(bytes)
            .await
            .map_err(map_cli_error)?;
        Ok(UploadedBlob {
            url: desc.url,
            sha256: desc.sha256,
        })
    }
}

fn row_of(e: super::super::CatalogEntry) -> CatalogRow {
    CatalogRow {
        name: e.shortcode,
        url: e.url,
        author: e.author,
        event_id: e.event_id,
        other_authors: e.other_authors,
    }
}

fn entry_builder(name: &str, url: &str) -> Result<nostr::EventBuilder, PortError> {
    buzz_sdk::build_emoji_entry(name, url).map_err(|e| PortError::Permanent(e.to_string()))
}

impl BuzzPorts<'_> {
    async fn submit(&self, builder: nostr::EventBuilder) -> Result<String, PortError> {
        let event = self
            .client
            .sign_event(builder)
            .map_err(|e| PortError::Permanent(e.to_string()))?;
        let id = event.id.to_hex();
        let raw = self
            .client
            .submit_event(event)
            .await
            .map_err(map_cli_error)?;
        let event_id = interpret_write_response(&raw)?;
        Ok(if event_id.is_empty() { id } else { event_id })
    }
}

impl Relay for BuzzPorts<'_> {
    fn pubkey(&self) -> String {
        self.client.keys().public_key().to_hex()
    }

    async fn publish(&self, name: &str, url: &str) -> Result<String, PortError> {
        self.submit(entry_builder(name, url)?).await
    }

    async fn lookup(&self, names: &[String]) -> Result<Vec<CatalogRow>, PortError> {
        let found = super::super::fetch_catalog_by_names(self.client, names)
            .await
            .map_err(map_cli_error)?;
        Ok(found.into_iter().map(row_of).collect())
    }

    async fn search(&self, prefix: &str, limit: u32) -> Result<Vec<CatalogRow>, PortError> {
        let found = super::super::search_catalog(self.client, prefix, limit)
            .await
            .map_err(map_cli_error)?;
        Ok(found.into_iter().map(row_of).collect())
    }

    async fn delete(&self, name: &str) -> Result<(), PortError> {
        let builder = buzz_sdk::build_delete_emoji_entry(&self.pubkey(), name)
            .map_err(|e| PortError::Permanent(e.to_string()))?;
        self.submit(builder).await.map(|_| ())
    }

    async fn blob_exists(&self, url: &str) -> Result<bool, PortError> {
        self.client.media_exists(url).await.map_err(map_cli_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use std::sync::atomic::{AtomicU32, Ordering};

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn fetcher(max: u64, attempts: u32) -> HttpFetcher {
        HttpFetcher::new(
            max,
            2,
            FetchPolicy {
                attempts,
                base_delay: Duration::from_millis(5),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn retries_429_and_503_then_succeeds_and_honors_retry_after() {
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        let app = Router::new().route(
            "/e.png",
            get(move || {
                let n = h.fetch_add(1, Ordering::SeqCst);
                async move {
                    match n {
                        0 => (
                            StatusCode::TOO_MANY_REQUESTS,
                            [(header::RETRY_AFTER, "0")],
                            vec![],
                        ),
                        1 => (
                            StatusCode::SERVICE_UNAVAILABLE,
                            [(header::RETRY_AFTER, "0")],
                            vec![],
                        ),
                        _ => (
                            StatusCode::OK,
                            [(header::RETRY_AFTER, "0")],
                            b"pixels".to_vec(),
                        ),
                    }
                }
            }),
        );
        let base = serve(app).await;
        let body = fetcher(1024, 3)
            .fetch(&format!("{base}/e.png"))
            .await
            .unwrap();
        assert_eq!(body, b"pixels");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_the_attempt_limit_with_a_transient_error() {
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        let app = Router::new().route(
            "/e.png",
            get(move || {
                h.fetch_add(1, Ordering::SeqCst);
                async { StatusCode::BAD_GATEWAY }
            }),
        );
        let base = serve(app).await;
        let err = fetcher(1024, 3)
            .fetch(&format!("{base}/e.png"))
            .await
            .unwrap_err();
        assert!(matches!(err, PortError::Transient { .. }), "{err:?}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn client_errors_and_oversize_bodies_are_permanent_without_retry() {
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        let app = Router::new()
            .route(
                "/gone.png",
                get(move || {
                    h.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::NOT_FOUND }
                }),
            )
            .route("/big.png", get(|| async { vec![7u8; 5000] }))
            .route(
                "/chunked.png",
                get(|| async {
                    let chunks = (0..10).map(|_| Ok::<_, std::io::Error>(vec![1u8; 100]));
                    axum::body::Body::from_stream(futures_util::stream::iter(chunks))
                }),
            );
        let base = serve(app).await;
        let f = fetcher(1000, 3);
        assert!(matches!(
            f.fetch(&format!("{base}/gone.png")).await,
            Err(PortError::Permanent(_))
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        for path in ["/big.png"] {
            let err = f.fetch(&format!("{base}{path}")).await.unwrap_err();
            assert!(
                matches!(&err, PortError::Permanent(m) if m.contains("cap")),
                "{err:?}"
            );
        }
        let small = fetcher(999, 1);
        let err = small
            .fetch(&format!("{base}/chunked.png"))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, PortError::Permanent(m) if m.contains("cap")),
            "{err:?}"
        );
        assert_eq!(
            f.fetch(&format!("{base}/chunked.png")).await.unwrap().len(),
            1000
        );
    }

    #[tokio::test]
    async fn file_sources_respect_the_cap_and_other_schemes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, vec![1u8; 50]).unwrap();
        let url = url::Url::from_file_path(&path).unwrap().to_string();
        assert_eq!(fetcher(50, 1).fetch(&url).await.unwrap().len(), 50);
        assert!(matches!(
            fetcher(49, 1).fetch(&url).await,
            Err(PortError::Permanent(_))
        ));
        assert!(matches!(
            fetcher(50, 1).fetch("file:///definitely/missing.png").await,
            Err(PortError::Permanent(_))
        ));
        assert!(matches!(
            fetcher(50, 1).fetch("ftp://example.com/a.png").await,
            Err(PortError::Permanent(_))
        ));
    }

    #[test]
    fn status_classification_and_retry_after_parsing() {
        for (status, transient) in [
            (200u16, false),
            (400, false),
            (403, false),
            (404, false),
            (408, true),
            (429, true),
            (500, true),
            (503, true),
        ] {
            let e = classify_status(status, None);
            assert_eq!(
                matches!(e, PortError::Transient { .. }),
                transient,
                "{status}"
            );
        }
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(reqwest::header::RETRY_AFTER, "7".parse().unwrap());
        assert_eq!(retry_after(&h), Some(Duration::from_secs(7)));
        h.insert(reqwest::header::RETRY_AFTER, "99999".parse().unwrap());
        assert_eq!(retry_after(&h), Some(Duration::from_secs(60)));
        h.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&h), None);
    }

    #[test]
    fn client_errors_map_to_retry_classes() {
        let relay = |status, body: &str| CliError::Relay {
            status,
            body: body.into(),
        };
        assert!(matches!(map_cli_error(relay(401, "")), PortError::Fatal(_)));
        assert!(matches!(map_cli_error(relay(403, "")), PortError::Fatal(_)));
        assert!(matches!(
            map_cli_error(relay(413, "")),
            PortError::Permanent(_)
        ));
        assert!(matches!(
            map_cli_error(relay(422, "")),
            PortError::Permanent(_)
        ));
        assert!(matches!(
            map_cli_error(relay(503, "")),
            PortError::Transient { .. }
        ));
        assert_eq!(
            map_cli_error(relay(
                429,
                r#"{"error":"rate-limited: quota; retry in 4s"}"#
            )),
            PortError::Transient {
                message: r#"relay rate limit: {"error":"rate-limited: quota; retry in 4s"}"#.into(),
                retry_after: Some(Duration::from_secs(4)),
            }
        );
        assert!(matches!(
            map_cli_error(CliError::DeliveryUnknown("x".into())),
            PortError::Transient { .. }
        ));
        assert!(matches!(
            map_cli_error(CliError::Auth("x".into())),
            PortError::Fatal(_)
        ));
    }

    #[test]
    fn published_entries_carry_exactly_the_search_text_as_content() {
        let keys = nostr::Keys::generate();
        for (raw, url) in [
            (":Party_Parrot-2:", "https://cdn.example/a.gif"),
            ("thumbsup", "https://cdn.example/b.png"),
            ("a-b_c-d", "http://cdn.example/c.png"),
        ] {
            let event = entry_builder(raw, url)
                .and_then(|b| {
                    b.sign_with_keys(&keys)
                        .map_err(|e| PortError::Permanent(e.to_string()))
                })
                .unwrap();
            let name = buzz_sdk::normalize_custom_emoji_shortcode(raw).unwrap();
            assert_eq!(
                event.content,
                buzz_sdk::emoji_entry_search_text(&name),
                "{raw}"
            );
            assert_eq!(event.kind.as_u16(), 30180);
        }
        assert_eq!(
            entry_builder("party_parrot-2", "https://x.example/a.png")
                .and_then(|b| b
                    .sign_with_keys(&keys)
                    .map_err(|e| PortError::Permanent(e.to_string())))
                .unwrap()
                .content,
            "party parrot 2"
        );
    }

    #[test]
    fn only_accepted_true_counts_as_published() {
        assert_eq!(
            interpret_write_response(r#"{"event_id":"abc","accepted":true,"message":""}"#).unwrap(),
            "abc"
        );
        for raw in [
            r#"{"event_id":"abc","accepted":false,"message":"invalid: bad d tag"}"#,
            r#"{"event_id":"abc","message":"ok"}"#,
            r#"{"accepted":"true"}"#,
            "plain text",
        ] {
            assert!(
                matches!(interpret_write_response(raw), Err(PortError::Permanent(_))),
                "{raw}"
            );
        }
        let limited = interpret_write_response(
            r#"{"event_id":"abc","accepted":false,"message":"rate-limited: slow down; retry in 3s"}"#,
        );
        assert_eq!(
            limited,
            Err(PortError::Transient {
                message: "rate-limited: slow down; retry in 3s".into(),
                retry_after: Some(Duration::from_secs(3)),
            })
        );
    }
}
