use std::collections::VecDeque;
use std::sync::Mutex;

use buzz_media::CommunityStorage;
use nostr::Keys;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

use super::*;

fn id(byte: u8) -> Uuid {
    Uuid::from_bytes([byte; 16])
}

fn community(byte: u8, active: bool) -> StorageUsageCommunity {
    StorageUsageCommunity {
        id: id(byte),
        host: format!("c{byte}.example"),
        active,
    }
}

fn snapshot_of(entries: &[(u8, u64, u64)]) -> BucketSnapshot {
    BucketSnapshot {
        per_community: entries
            .iter()
            .map(|(byte, bytes, objects)| {
                (
                    id(*byte),
                    CommunityStorage {
                        bytes: *bytes,
                        objects: *objects,
                    },
                )
            })
            .collect(),
        ..Default::default()
    }
}

fn stored(snapshot: &BucketSnapshot, completed_at: i64) -> StoredStorageSnapshot {
    StoredStorageSnapshot {
        snapshot: serde_json::to_value(snapshot).expect("encode snapshot"),
        completed_at: DateTime::from_timestamp(completed_at, 0).expect("timestamp"),
        duration_ms: 1,
        max_objects: 10,
        code_sha: "test".to_string(),
    }
}

struct MockSource {
    stored: Mutex<Option<StoredStorageSnapshot>>,
    communities: Vec<StorageUsageCommunity>,
}

impl MockSource {
    fn new(stored: Option<StoredStorageSnapshot>, communities: Vec<StorageUsageCommunity>) -> Self {
        Self {
            stored: Mutex::new(stored),
            communities,
        }
    }

    fn replace(&self, stored: StoredStorageSnapshot) {
        *self.stored.lock().expect("source lock") = Some(stored);
    }
}

#[async_trait::async_trait]
impl UsageSource for MockSource {
    async fn completed_at(&self) -> Result<Option<DateTime<Utc>>, String> {
        Ok(self
            .stored
            .lock()
            .expect("source lock")
            .as_ref()
            .map(|stored| stored.completed_at))
    }

    async fn snapshot(&self) -> Result<Option<StoredStorageSnapshot>, String> {
        Ok(self.stored.lock().expect("source lock").clone())
    }

    async fn communities(&self) -> Result<Vec<StorageUsageCommunity>, String> {
        Ok(self.communities.clone())
    }
}

/// Answers each POST with the next queued response, then 204.
#[derive(Default)]
struct MockTransport {
    responses: Mutex<VecDeque<Result<StatusCode, String>>>,
    bodies: Mutex<Vec<serde_json::Value>>,
}

impl MockTransport {
    fn answering(responses: impl IntoIterator<Item = Result<StatusCode, String>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            bodies: Mutex::default(),
        }
    }

    fn take_bodies(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut *self.bodies.lock().expect("bodies lock"))
    }
}

#[async_trait::async_trait]
impl DeliveryTransport for MockTransport {
    async fn post(
        &self,
        _url: &url::Url,
        _authorization: &str,
        body: Vec<u8>,
    ) -> Result<StatusCode, String> {
        self.bodies
            .lock()
            .expect("bodies lock")
            .push(serde_json::from_slice(&body).expect("batch JSON"));
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .unwrap_or(Ok(StatusCode::NO_CONTENT))
    }
}

fn endpoint() -> url::Url {
    url::Url::parse("https://usage.example/storage").expect("endpoint")
}

/// A snapshot that spans three batches: 2 * BATCH_SIZE + 1 communities.
fn three_batch_source(completed_at: i64) -> MockSource {
    let count = 2 * BATCH_SIZE + 1;
    let mut per_community = HashMap::new();
    let mut communities = Vec::new();
    for index in 0..count {
        let id = Uuid::from_u128(index as u128 + 1);
        per_community.insert(
            id,
            CommunityStorage {
                bytes: 10,
                objects: 1,
            },
        );
        communities.push(StorageUsageCommunity {
            id,
            host: format!("c{index}.example"),
            active: true,
        });
    }
    let snapshot = BucketSnapshot {
        per_community,
        ..Default::default()
    };
    MockSource::new(Some(stored(&snapshot, completed_at)), communities)
}

/// Each body's `measured_at` and community count.
fn batch_shape(bodies: &[serde_json::Value]) -> Vec<(&str, usize)> {
    bodies
        .iter()
        .map(|body| {
            (
                body["measured_at"].as_str().expect("measured_at"),
                body["communities"].as_array().expect("communities").len(),
            )
        })
        .collect()
}

/// RFC 3339 form of the 1_700_000_000 test snapshot time.
const MEASURED_AT: &str = "2023-11-14T22:13:20Z";

#[test]
fn delivery_url_accepts_plain_http_endpoints_only() {
    for valid in [
        "https://usage.example/storage",
        "http://kgoose:8080/storage",
        " https://usage.example/storage ",
    ] {
        assert!(parse_delivery_url(valid).is_ok(), "{valid}");
    }
    for invalid in [
        "",
        "not a url",
        "ftp://usage.example/storage",
        "https://user:pass@usage.example/storage",
        "https://usage.example/storage?token=secret",
        "https://usage.example/storage#fragment",
    ] {
        assert!(parse_delivery_url(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn records_cover_snapshot_communities_and_zero_absent_active_ones() {
    let snapshot = snapshot_of(&[
        (1, 100, 2), // active, stored
        (2, 50, 1),  // archived but still storing bytes
        (5, 7, 1),   // no community row: no host to report
    ]);
    let communities = vec![
        community(1, true),
        community(2, false),
        community(3, true),  // active, nothing stored: explicit zero
        community(4, false), // inactive, nothing stored: omitted
    ];

    let records = usage_records(&snapshot, &communities);

    let record = |host: &str, bytes: u64, objects: u64| CommunityUsage {
        community_host: host.to_string(),
        storage_bytes: bytes,
        storage_objects: objects,
    };
    assert_eq!(
        records,
        vec![
            record("c1.example", 100, 2),
            record("c2.example", 50, 1),
            record("c3.example", 0, 0),
        ]
    );
}

#[tokio::test]
async fn new_snapshot_is_delivered_once_in_bounded_batches() {
    let source = three_batch_source(1_700_000_000);
    let transport = MockTransport::default();
    let mut handled = None;

    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;

    assert_eq!(outcome, CheckOutcome::Delivered);
    let bodies = transport.take_bodies();
    assert_eq!(
        batch_shape(&bodies),
        vec![
            (MEASURED_AT, BATCH_SIZE),
            (MEASURED_AT, BATCH_SIZE),
            (MEASURED_AT, 1)
        ]
    );

    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;
    assert_eq!(outcome, CheckOutcome::Unchanged);
    assert!(
        transport.take_bodies().is_empty(),
        "delivered snapshot re-sent"
    );

    source.replace(stored(&snapshot_of(&[(1, 1, 1)]), 1_700_086_400));
    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;
    assert_eq!(outcome, CheckOutcome::Delivered);
    let bodies = transport.take_bodies();
    assert_eq!(bodies.len(), 3, "every active community is reported");
    assert_eq!(bodies[0]["measured_at"], "2023-11-15T22:13:20Z");
}

#[tokio::test]
async fn retryable_failures_resend_the_whole_snapshot_next_check() {
    for failure in [
        Ok(StatusCode::REQUEST_TIMEOUT),
        Ok(StatusCode::TOO_MANY_REQUESTS),
        Ok(StatusCode::SERVICE_UNAVAILABLE),
        Err("connection refused".to_string()),
    ] {
        let source = three_batch_source(1_700_000_000);
        let transport = MockTransport::answering([Ok(StatusCode::OK), failure.clone()]);
        let mut handled = None;

        let outcome = check_once(
            &source,
            &transport,
            &endpoint(),
            &Keys::generate(),
            &mut handled,
        )
        .await;
        assert_eq!(outcome, CheckOutcome::Retry, "{failure:?}");
        assert_eq!(handled, None, "{failure:?}");
        assert_eq!(transport.take_bodies().len(), 2, "{failure:?}");

        let outcome = check_once(
            &source,
            &transport,
            &endpoint(),
            &Keys::generate(),
            &mut handled,
        )
        .await;
        assert_eq!(outcome, CheckOutcome::Delivered, "{failure:?}");
        assert_eq!(
            batch_shape(&transport.take_bodies()),
            vec![
                (MEASURED_AT, BATCH_SIZE),
                (MEASURED_AT, BATCH_SIZE),
                (MEASURED_AT, 1)
            ],
            "{failure:?}"
        );
    }
}

#[tokio::test]
async fn client_error_abandons_only_that_snapshot() {
    let source = three_batch_source(1_700_000_000);
    let transport = MockTransport::answering([Ok(StatusCode::BAD_REQUEST)]);
    let mut handled = None;

    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;
    assert_eq!(outcome, CheckOutcome::Rejected);
    assert_eq!(transport.take_bodies().len(), 1, "stopped after rejection");

    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;
    assert_eq!(outcome, CheckOutcome::Unchanged);
    assert!(
        transport.take_bodies().is_empty(),
        "rejected snapshot re-sent"
    );

    source.replace(stored(&snapshot_of(&[(1, 1, 1)]), 1_700_086_400));
    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;
    assert_eq!(outcome, CheckOutcome::Delivered);
}

#[tokio::test]
async fn missing_snapshot_sends_nothing() {
    let source = MockSource::new(None, vec![community(1, true)]);
    let transport = MockTransport::default();
    let mut handled = None;

    let outcome = check_once(
        &source,
        &transport,
        &endpoint(),
        &Keys::generate(),
        &mut handled,
    )
    .await;

    assert_eq!(outcome, CheckOutcome::Unchanged);
    assert!(transport.take_bodies().is_empty());
}

async fn read_http_request(stream: &mut TcpStream) -> (String, Vec<u8>, String) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .expect("read request line");
    let mut content_length = 0;
    let mut authorization = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read header");
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().expect("valid content length");
            } else if name.eq_ignore_ascii_case("authorization") {
                authorization = value.trim().to_owned();
            }
        }
    }
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .await
        .expect("read request body");
    (request_line, body, authorization)
}

#[tokio::test]
async fn http_delivery_is_signed_by_the_relay_key() {
    use nostr::nips::nip98::{verify_auth_header, HttpMethod};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener");
    let url = url::Url::parse(&format!(
        "http://{}/storage",
        listener.local_addr().expect("listener address")
    ))
    .expect("HTTP endpoint");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept delivery");
        let request = read_http_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .await
            .expect("write success response");
        request
    });
    let relay_keypair = Keys::generate();
    let source = MockSource::new(
        Some(stored(&snapshot_of(&[(1, 100, 2)]), 1_700_000_000)),
        vec![community(1, true)],
    );
    let transport = ReqwestDeliveryTransport {
        http: build_delivery_http_client().expect("delivery client"),
    };
    let mut handled = None;

    let outcome = check_once(&source, &transport, &url, &relay_keypair, &mut handled).await;

    assert_eq!(outcome, CheckOutcome::Delivered);
    let (request_line, body, authorization) = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("HTTP server completed")
        .expect("HTTP server task");
    assert!(request_line.starts_with("POST /storage HTTP/1.1"));
    let signer = verify_auth_header(
        &authorization,
        &url,
        HttpMethod::POST,
        nostr::Timestamp::now(),
        Some(&body),
    )
    .expect("valid signature binding the endpoint and body");
    assert_eq!(signer, relay_keypair.public_key());
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("batch JSON");
    assert_eq!(
        payload,
        serde_json::json!({
            "measured_at": MEASURED_AT,
            "communities": [{
                "community_host": "c1.example",
                "storage_bytes": 100,
                "storage_objects": 2,
            }],
        })
    );
}
