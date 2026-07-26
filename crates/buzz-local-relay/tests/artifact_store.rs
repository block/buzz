use std::path::PathBuf;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_local_relay::identity::LocalIdentityAdapter;
use buzz_local_relay::{serve, LocalRelay, ReplicationDisabled, StorageMode};
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url;
use nostr::{EventBuilder, Keys, Tag};
use reqwest::StatusCode;
use serde_json::Value;
use tokio::net::TcpListener;
use uuid::Uuid;

fn nip98_header(keys: &Keys, method: HttpMethod, url: &str, body: &[u8]) -> String {
    let data = HttpData::new(Url::parse(url).expect("HTTP auth URL parses"), method)
        .payload(Sha256Hash::hash(body));
    let nonce = Uuid::new_v4().to_string();
    let event = EventBuilder::http_auth(data)
        .tag(Tag::parse(["nonce", nonce.as_str()]).expect("nonce tag parses"))
        .sign_with_keys(keys)
        .expect("NIP-98 event signs");
    let encoded = BASE64.encode(serde_json::to_vec(&event).expect("auth event serializes"));
    format!("Nostr {encoded}")
}

#[tokio::test]
async fn artifact_store_round_trips_and_fails_closed() {
    let artifacts_dir =
        std::env::temp_dir().join(format!("buzz-artifacts-test-{}", Uuid::new_v4()));
    let relay = LocalRelay::open_full(
        StorageMode::Ephemeral,
        Arc::new(ReplicationDisabled),
        Some(Arc::new(LocalIdentityAdapter::new())),
        Some(artifacts_dir.clone()),
    )
    .await
    .expect("relay opens");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("address available");
    let server = tokio::spawn(async move {
        serve(listener, relay).await.expect("relay serves");
    });
    let keys = Keys::generate();
    let client = reqwest::Client::new();
    let content = b"portable relay conformance evidence, immutable".to_vec();
    let expected_hash = Sha256Hash::hash(&content).to_string();
    let upload_url = format!("http://{address}/artifacts");

    let anonymous = client
        .post(&upload_url)
        .body(content.clone())
        .send()
        .await
        .expect("anonymous upload completes");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let uploaded = client
        .post(&upload_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::POST, &upload_url, &content),
        )
        .body(content.clone())
        .send()
        .await
        .expect("upload completes");
    assert_eq!(uploaded.status(), StatusCode::OK);
    let descriptor: Value = uploaded.json().await.expect("descriptor parses");
    assert_eq!(descriptor["sha256"], expected_hash.as_str());
    assert_eq!(descriptor["size"], content.len());

    let fetch_url = format!("http://{address}/artifacts/{expected_hash}");
    let fetched = client
        .get(&fetch_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::GET, &fetch_url, b""),
        )
        .send()
        .await
        .expect("fetch completes");
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(fetched.bytes().await.expect("bytes read").to_vec(), content);

    let bad_id_url = format!("http://{address}/artifacts/not-a-hash");
    let bad_id = client
        .get(&bad_id_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::GET, &bad_id_url, b""),
        )
        .send()
        .await
        .expect("bad ID request completes");
    assert_eq!(bad_id.status(), StatusCode::BAD_REQUEST);

    let missing_hash = Sha256Hash::hash(b"absent").to_string();
    let missing_url = format!("http://{address}/artifacts/{missing_hash}");
    let missing = client
        .get(&missing_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::GET, &missing_url, b""),
        )
        .send()
        .await
        .expect("missing request completes");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    // Corrupt the blob on disk: content verification must fail closed
    // rather than serving bytes that no longer match their identity.
    let stored: PathBuf = artifacts_dir.join(&expected_hash);
    std::fs::write(&stored, b"tampered").expect("corruption written");
    let corrupted = client
        .get(&fetch_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::GET, &fetch_url, b""),
        )
        .send()
        .await
        .expect("corrupted fetch completes");
    assert_eq!(corrupted.status(), StatusCode::BAD_REQUEST);

    std::fs::remove_dir_all(&artifacts_dir).ok();
    server.abort();
}

#[tokio::test]
async fn head_probe_reports_presence_without_transfer() {
    let artifacts_dir =
        std::env::temp_dir().join(format!("buzz-artifacts-head-{}", Uuid::new_v4()));
    let relay = LocalRelay::open_full(
        StorageMode::Ephemeral,
        Arc::new(ReplicationDisabled),
        Some(Arc::new(LocalIdentityAdapter::new())),
        Some(artifacts_dir.clone()),
    )
    .await
    .expect("relay opens");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("address available");
    let server = tokio::spawn(async move {
        serve(listener, relay).await.expect("relay serves");
    });
    let keys = Keys::generate();
    let client = reqwest::Client::new();
    let content = b"walker existence probe".to_vec();
    let hash = Sha256Hash::hash(&content).to_string();
    let head_url = format!("http://{address}/artifacts/{hash}");

    let absent = client
        .head(&head_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::GET, &head_url, b""),
        )
        .send()
        .await
        .expect("absent probe completes");
    assert_eq!(absent.status(), StatusCode::NOT_FOUND);

    let upload_url = format!("http://{address}/artifacts");
    client
        .post(&upload_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::POST, &upload_url, &content),
        )
        .body(content.clone())
        .send()
        .await
        .expect("upload completes");

    let present = client
        .head(&head_url)
        .header(
            "authorization",
            nip98_header(&keys, HttpMethod::GET, &head_url, b""),
        )
        .send()
        .await
        .expect("present probe completes");
    assert_eq!(present.status(), StatusCode::OK);
    assert_eq!(present.bytes().await.expect("no body").len(), 0);

    std::fs::remove_dir_all(&artifacts_dir).ok();
    server.abort();
}
