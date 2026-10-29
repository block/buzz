//! Loopback regressions use the production router, synthetic signed grants,
//! and controlled transport/completion boundaries; no APNs or external services.
use super::*;
use crate::authority::*;
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};

struct Controls {
    sends: AtomicUsize,
    finishes: AtomicUsize,
    completed: AtomicUsize,
    send_gate: Semaphore,
    finish_gate: Semaphore,
}
impl Controls {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sends: AtomicUsize::new(0),
            finishes: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            send_gate: Semaphore::new(0),
            finish_gate: Semaphore::new(0),
        })
    }
}
struct ControlledTransport(Arc<Controls>);
#[async_trait::async_trait]
impl PushTransport for ControlledTransport {
    async fn send(&self, _: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        assert_eq!(endpoint, "01020304");
        self.0.sends.fetch_add(1, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(15), self.0.send_gate.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        DeliveryOutcome::Retry {
            retry_after_seconds: None,
        }
    }
}
struct ControlledStore {
    inner: Arc<MemoryAuthorityStore>,
    c: Arc<Controls>,
}

// Forward real admission, quota and replay behavior; delay only completion.
#[async_trait::async_trait]
impl AuthorityStore for ControlledStore {
    async fn ready(&self) -> Result<(), AuthorityError> {
        self.inner.ready().await
    }
    async fn put_challenge(&self, challenge: Challenge) -> Result<(), AuthorityError> {
        self.inner.put_challenge(challenge).await
    }
    async fn consume_challenge(
        &self,
        id: Uuid,
        value: [u8; 32],
        now: i64,
    ) -> Result<(), AuthorityError> {
        self.inner.consume_challenge(id, value, now).await
    }
    async fn create_installation(
        &self,
        installation: NewInstallation,
        now: i64,
    ) -> Result<(), AuthorityError> {
        self.inner.create_installation(installation, now).await
    }
    async fn matching_installation(
        &self,
        app_attest_key_id: &[u8],
        token_fingerprint: [u8; 32],
        endpoint_epoch: i64,
        expires_at: i64,
        now: i64,
    ) -> Result<Option<Installation>, AuthorityError> {
        self.inner
            .matching_installation(
                app_attest_key_id,
                token_fingerprint,
                endpoint_epoch,
                expires_at,
                now,
            )
            .await
    }
    async fn installation(&self, id: Uuid, now: i64) -> Result<Installation, AuthorityError> {
        self.inner.installation(id, now).await
    }
    async fn installation_for_revocation(
        &self,
        id: Uuid,
        now: i64,
    ) -> Result<Installation, AuthorityError> {
        self.inner.installation_for_revocation(id, now).await
    }
    async fn upsert_delegation(
        &self,
        delegation: Delegation,
        assertion: AssertionCounter,
    ) -> Result<(), AuthorityError> {
        self.inner.upsert_delegation(delegation, assertion).await
    }
    async fn rotate_endpoint(
        &self,
        installation_id: Uuid,
        expected_epoch: i64,
        new_epoch: i64,
        token_ciphertext: Vec<u8>,
        token_fingerprint: [u8; 32],
        assertion: AssertionCounter,
    ) -> Result<(), AuthorityError> {
        self.inner
            .rotate_endpoint(
                installation_id,
                expected_epoch,
                new_epoch,
                token_ciphertext,
                token_fingerprint,
                assertion,
            )
            .await
    }
    async fn revoke_delegation(
        &self,
        installation_id: Uuid,
        relay_pubkey: &str,
        expected_generation: i64,
        assertion: AssertionCounter,
    ) -> Result<(), AuthorityError> {
        self.inner
            .revoke_delegation(
                installation_id,
                relay_pubkey,
                expected_generation,
                assertion,
            )
            .await
    }
    async fn revoke_installation(
        &self,
        installation_id: Uuid,
        expected_epoch: i64,
        new_epoch: i64,
        assertion: AssertionCounter,
    ) -> Result<(), AuthorityError> {
        self.inner
            .revoke_installation(installation_id, expected_epoch, new_epoch, assertion)
            .await
    }
    async fn authorize_delivery(
        &self,
        grant_hash: [u8; 32],
        relay_pubkey: &str,
        event_id: &str,
        request_id: Uuid,
        request_expires_at: i64,
        quota_window_seconds: i64,
        quota_max_deliveries: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError> {
        self.inner
            .authorize_delivery(
                grant_hash,
                relay_pubkey,
                event_id,
                request_id,
                request_expires_at,
                quota_window_seconds,
                quota_max_deliveries,
                now,
            )
            .await
    }
    async fn finish_delivery(
        &self,
        permit: DeliveryPermit,
        disposition: DeliveryDisposition,
    ) -> Result<(), AuthorityError> {
        self.c.finishes.fetch_add(1, Ordering::SeqCst);
        self.c.finish_gate.acquire().await.unwrap().forget();
        let result = self.inner.finish_delivery(permit, disposition).await;
        self.c.completed.fetch_add(1, Ordering::SeqCst);
        result
    }
    async fn reap_expired(&self, now: i64) -> Result<(), AuthorityError> {
        self.inner.reap_expired(now).await
    }
}

struct Server {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn new(app: axum::Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self { addr, task }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
    async fn upload(&self, path: &str, body: &[u8], declared: usize, auth: &str) -> TcpStream {
        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        let header = format!("POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: {declared}\r\nAuthorization: {auth}\r\n\r\n", self.addr);
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        stream
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn wait_count(counter: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while counter.load(Ordering::SeqCst) < expected {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn fresh_body(template: &[u8]) -> Vec<u8> {
    let mut body: serde_json::Value = serde_json::from_slice(template).unwrap();
    body["request_id"] = Uuid::new_v4().to_string().into();
    serde_json::to_vec(&body).unwrap()
}
async fn delivery_cancellation_retains_capacity(h2: bool) {
    let c = Controls::new();
    let controls = c.clone();
    let (app, keys, template) = fixture_with(
        Arc::new(ControlledTransport(c.clone())),
        move |inner| Arc::new(ControlledStore { inner, c: controls }),
        10_000, // Isolate ingress while retaining real grant/quota/replay admission.
    )
    .await;
    let server = Server::new(app).await;
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(3));
    builder = if h2 {
        builder.http2_prior_knowledge()
    } else {
        builder.http1_only()
    };
    let client = builder.build().unwrap();
    let control = client
        .post(server.url(crate::http::DELIVERY_PATH))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(control.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        control.version(),
        if h2 {
            reqwest::Version::HTTP_2
        } else {
            reqwest::Version::HTTP_11
        }
    );
    for n in 1..=8 {
        let body = fresh_body(&template);
        let auth = signed_header(&keys, DIRECT_URL, "POST", &body);
        if h2 {
            let request = client
                .post(server.url(crate::http::DELIVERY_PATH))
                .header("authorization", auth)
                .body(body);
            let task = tokio::spawn(async move { request.send().await });
            wait_count(&c.sends, n).await;
            task.abort(); // Dropping response interest sends RST_STREAM(CANCEL).
            let _ = task.await;
        } else {
            let stream = server
                .upload(crate::http::DELIVERY_PATH, &body, body.len(), &auth)
                .await;
            wait_count(&c.sends, n).await;
            drop(stream); // Close the actual HTTP/1 connection after send-begin.
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Capacity must remain occupied both while sending and while finishing.
    for finishing in [false, true] {
        if finishing {
            c.send_gate.add_permits(8);
            wait_count(&c.finishes, 8).await;
        }
        let body = fresh_body(&template);
        let response = client
            .post(server.url(crate::http::DELIVERY_PATH))
            .header(
                "authorization",
                signed_header(&keys, DIRECT_URL, "POST", &body),
            )
            .body(body)
            .send()
            .await
            .expect("occupied delivery capacity must reject promptly");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["retry-after"], "1");
        assert_eq!(c.sends.load(Ordering::SeqCst), 8);
    }
    c.finish_gate.add_permits(8);
    wait_count(&c.completed, 8).await;
    // Detached bookkeeping finishes on disconnect and releases capacity.
    c.send_gate.add_permits(1);
    c.finish_gate.add_permits(1);
    let body = fresh_body(&template);
    let response = client
        .post(server.url(crate::http::DELIVERY_PATH))
        .header(
            "authorization",
            signed_header(&keys, DIRECT_URL, "POST", &body),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(c.sends.load(Ordering::SeqCst), 9);
    assert_eq!(c.completed.load(Ordering::SeqCst), 9);
}

#[tokio::test]
async fn http1_disconnect_retains_send_and_completion_capacity() {
    delivery_cancellation_retains_capacity(false).await;
}
#[tokio::test]
async fn http2_cancel_retains_send_and_completion_capacity() {
    delivery_cancellation_retains_capacity(true).await;
}

async fn read_status(stream: &mut TcpStream) -> u16 {
    let mut status = Vec::new();
    // Includes the production 20-second body timeout.
    tokio::time::timeout(Duration::from_secs(23), async {
        while !status.ends_with(b"\r\n") {
            status.push(stream.read_u8().await.unwrap());
            assert!(status.len() < 128);
        }
    })
    .await
    .unwrap();
    String::from_utf8(status)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn one_sources_slow_revocations_leave_both_handlers_available() {
    let (app, _, _, _) = fixture().await;
    let server = Server::new(app).await;
    let paths = ["/v1/delegations/revoke", "/v1/installations/revoke"];
    let client = reqwest::Client::builder()
        .local_address("127.0.0.2".parse::<std::net::IpAddr>().unwrap())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let mut uploads = Vec::new();
    // Four unauthenticated partial bodies from the real TCP source 127.0.0.1.
    for path in paths.into_iter().cycle().take(4) {
        uploads.push(server.upload(path, b"{", 1024, "").await);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    for path in paths {
        let response = client
            .post(server.url(path))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{path} must reach its handler"
        );
        let response = client
            .post(server.url(path))
            .body(vec![b' '; crate::model::MAX_REQUEST_BYTES + 1])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
    // The excess uploads are rejected promptly; admitted uploads keep the
    // existing real 20-second timeout and then return their source capacity.
    let mut statuses = Vec::new();
    for stream in &mut uploads {
        statuses.push(read_status(stream).await);
    }
    statuses.sort_unstable();
    assert_eq!(statuses, [408, 408, 429, 429]);
    let own_source = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    for path in paths {
        assert_eq!(
            own_source
                .post(server.url(path))
                .body("{}")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    // Disconnect also returns capacity; finish two new partial bodies after
    // dropping a previous pair, rather than waiting for their timeout.
    let first = server.upload(paths[0], b"{", 1024, "").await;
    let second = server.upload(paths[1], b"{", 1024, "").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop((first, second));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut a = server.upload(paths[0], b"{", 2, "").await;
    let mut b = server.upload(paths[1], b"{", 2, "").await;
    a.write_all(b"}").await.unwrap();
    b.write_all(b"}").await.unwrap();
    assert_eq!(read_status(&mut a).await, 400);
    assert_eq!(read_status(&mut b).await, 400);
}
