use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_core::identity::{IdentityDenialCode, Principal};
use buzz_core::replication::{
    ReplicationCursor, ReplicationIngestOutcome, ReplicationRecord, ReplicationSourceId,
};
use buzz_local_relay::identity::{
    LocalAuthenticationEvidence, LocalIdentityAdapter, LocalPeerEvidence, RelayPeerTrust,
};
use buzz_local_relay::{serve, LocalRelay, ReplicationSourceAllowlist, StorageMode};
use futures_util::{SinkExt, StreamExt};
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url;
use nostr::{Event, EventBuilder, Filter, Keys, Kind, RelayUrl, Tag, Timestamp};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as ClientMessage;
use uuid::Uuid;

fn identity_vector() -> Value {
    serde_json::from_str(include_str!(
        "../../../specs/fixtures/portable-relay/identity-v0.1.json"
    ))
    .expect("identity vector parses")
}

fn nip98_header(keys: &Keys, url: &str, body: &[u8]) -> String {
    let data = HttpData::new(
        Url::parse(url).expect("HTTP auth URL parses"),
        HttpMethod::POST,
    )
    .payload(Sha256Hash::hash(body));
    let nonce = Uuid::new_v4().to_string();
    let event = EventBuilder::http_auth(data)
        .tag(Tag::parse(["nonce", nonce.as_str()]).expect("nonce tag parses"))
        .sign_with_keys(keys)
        .expect("NIP-98 event signs");
    let encoded = BASE64.encode(serde_json::to_vec(&event).expect("auth event serializes"));
    format!("Nostr {encoded}")
}

async fn start_secured_relay(
    identity: Arc<LocalIdentityAdapter>,
) -> (
    Arc<LocalRelay>,
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
) {
    let relay = LocalRelay::open_with_identity(StorageMode::Ephemeral, identity)
        .await
        .expect("secured relay opens");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener binds");
    let address = listener.local_addr().expect("address available");
    let server_relay = Arc::clone(&relay);
    let server = tokio::spawn(async move {
        serve(listener, server_relay)
            .await
            .expect("secured relay serves");
    });
    (relay, address, server)
}

async fn authenticated_post(keys: &Keys, url: &str, body: &[u8]) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .header("authorization", nip98_header(keys, url, body))
        .header("content-type", "application/json")
        .body(body.to_vec())
        .send()
        .await
        .expect("authenticated request completes")
}

#[test]
fn fixed_nip42_vector_authenticates_and_replay_fails_closed() {
    let vector = identity_vector();
    let proof = &vector["authentication_evidence"];
    let event: Event = serde_json::from_value(proof["event"].clone()).expect("proof event parses");
    let evaluation_time = Timestamp::from(
        proof["evaluation_time"]
            .as_u64()
            .expect("evaluation time is u64"),
    );
    let audience = proof["audience"].as_str().expect("audience is string");
    let challenge = proof["challenge"]
        .as_str()
        .expect("challenge is string")
        .to_string();
    let adapter = LocalIdentityAdapter::new();

    let wrong_audience = adapter
        .authenticate_at(
            LocalAuthenticationEvidence::Nip42 {
                event: event.clone(),
                challenge: challenge.clone(),
            },
            "wss://different.example/",
            evaluation_time,
        )
        .expect_err("proof for another audience is rejected");
    assert_eq!(
        wrong_audience.denial_code(),
        Some(IdentityDenialCode::AudienceMismatch)
    );

    let principal = adapter
        .authenticate_at(
            LocalAuthenticationEvidence::Nip42 {
                event: event.clone(),
                challenge: challenge.clone(),
            },
            audience,
            evaluation_time,
        )
        .expect("fixed proof authenticates");
    assert_eq!(
        principal.principal,
        Principal::Nostr {
            pubkey: event.pubkey
        }
    );

    let replay = adapter
        .authenticate_at(
            LocalAuthenticationEvidence::Nip42 { event, challenge },
            audience,
            evaluation_time,
        )
        .expect_err("replayed proof is rejected");
    assert_eq!(
        replay.denial_code(),
        Some(IdentityDenialCode::ReplayDetected)
    );
}

#[tokio::test]
async fn secured_http_binds_author_rejects_replay_and_never_journals_auth() {
    let (relay, address, server) = start_secured_relay(Arc::new(LocalIdentityAdapter::new())).await;
    let author = Keys::generate();
    let other = Keys::generate();
    let event = EventBuilder::new(Kind::TextNote, "attributable")
        .sign_with_keys(&author)
        .expect("event signs");
    let body = serde_json::to_vec(&event).expect("event serializes");
    let url = format!("http://{address}/events");
    let header = nip98_header(&author, &url, &body);
    let client = reqwest::Client::new();

    let anonymous = client
        .post(&url)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("anonymous request completes");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous
            .json::<Value>()
            .await
            .expect("anonymous response parses")["code"],
        "authentication_required"
    );

    let first = client
        .post(&url)
        .header("authorization", &header)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("first request completes");
    assert_eq!(first.status(), StatusCode::OK);

    let replay = client
        .post(&url)
        .header("authorization", &header)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("replay request completes");
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        replay
            .json::<Value>()
            .await
            .expect("replay response parses")["code"],
        "replay_detected"
    );

    let mismatched = EventBuilder::new(Kind::TextNote, "not yours")
        .sign_with_keys(&other)
        .expect("event signs");
    let mismatched_body = serde_json::to_vec(&mismatched).expect("event serializes");
    let denied = authenticated_post(&author, &url, &mismatched_body).await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        denied
            .json::<Value>()
            .await
            .expect("denial response parses")["code"],
        "author_mismatch"
    );

    assert!(relay
        .store()
        .query(&[Filter::new().id(mismatched.id)])
        .await
        .expect("query succeeds")
        .is_empty());
    assert_eq!(
        relay
            .store()
            .query(&[Filter::new().kind(Kind::Authentication)])
            .await
            .expect("auth history query succeeds")
            .len(),
        0
    );
    assert_eq!(
        relay
            .store()
            .query(&[Filter::new().kind(Kind::HttpAuth)])
            .await
            .expect("HTTP auth history query succeeds")
            .len(),
        0
    );

    server.abort();
}

#[tokio::test]
async fn protected_event_is_filtered_from_query_and_count_for_other_readers() {
    let (_relay, address, server) =
        start_secured_relay(Arc::new(LocalIdentityAdapter::new())).await;
    let recipient = Keys::generate();
    let sender = Keys::generate();
    let attacker = Keys::generate();
    let gift_wrap = EventBuilder::new(Kind::GiftWrap, "ciphertext")
        .tag(Tag::public_key(recipient.public_key()))
        .sign_with_keys(&Keys::generate())
        .expect("gift wrap signs");
    let event_body = serde_json::to_vec(&gift_wrap).expect("event serializes");
    let events_url = format!("http://{address}/events");
    let stored = authenticated_post(&sender, &events_url, &event_body).await;
    assert_eq!(stored.status(), StatusCode::OK);

    let filters = vec![Filter::new().id(gift_wrap.id)];
    let filter_body = serde_json::to_vec(&filters).expect("filters serialize");
    let query_url = format!("http://{address}/query");
    let recipient_query = authenticated_post(&recipient, &query_url, &filter_body).await;
    assert_eq!(recipient_query.status(), StatusCode::OK);
    assert_eq!(
        recipient_query
            .json::<Vec<Event>>()
            .await
            .expect("recipient query parses"),
        vec![gift_wrap.clone()]
    );
    let attacker_query = authenticated_post(&attacker, &query_url, &filter_body).await;
    assert_eq!(attacker_query.status(), StatusCode::OK);
    assert!(attacker_query
        .json::<Vec<Event>>()
        .await
        .expect("attacker query parses")
        .is_empty());

    let count_url = format!("http://{address}/count");
    let recipient_count = authenticated_post(&recipient, &count_url, &filter_body).await;
    assert_eq!(
        recipient_count
            .json::<Value>()
            .await
            .expect("recipient count parses")["count"],
        1
    );
    let attacker_count = authenticated_post(&attacker, &count_url, &filter_body).await;
    assert_eq!(
        attacker_count
            .json::<Value>()
            .await
            .expect("attacker count parses")["count"],
        0
    );

    server.abort();
}

#[tokio::test]
async fn websocket_requires_nip42_and_binds_direct_event_author() {
    let (relay, address, server) = start_secured_relay(Arc::new(LocalIdentityAdapter::new())).await;
    let keys = Keys::generate();
    let (mut websocket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
        .await
        .expect("WebSocket connects");

    let challenge_frame = tokio::time::timeout(Duration::from_secs(2), websocket.next())
        .await
        .expect("challenge arrives")
        .expect("stream remains open")
        .expect("challenge frame valid")
        .into_text()
        .expect("challenge is text");
    let challenge: Value = serde_json::from_str(&challenge_frame).expect("challenge parses");
    assert_eq!(challenge[0], "AUTH");

    let pre_auth_event = EventBuilder::new(Kind::TextNote, "too early")
        .sign_with_keys(&keys)
        .expect("event signs");
    websocket
        .send(ClientMessage::Text(
            json!(["EVENT", pre_auth_event]).to_string().into(),
        ))
        .await
        .expect("unauthenticated EVENT sends");
    let denied = tokio::time::timeout(Duration::from_secs(2), websocket.next())
        .await
        .expect("EVENT denial arrives")
        .expect("stream remains open")
        .expect("EVENT denial frame valid")
        .into_text()
        .expect("EVENT denial is text");
    let denied: Value = serde_json::from_str(&denied).expect("EVENT denial parses");
    assert_eq!(denied[2], false);
    assert_eq!(denied[3], "authentication_required");

    let relay_url_string = format!("ws://{address}/");
    let relay_url = RelayUrl::parse(&relay_url_string).expect("relay URL parses");
    let auth_event = EventBuilder::auth(
        challenge[1].as_str().expect("challenge is string"),
        relay_url,
    )
    .sign_with_keys(&keys)
    .expect("AUTH event signs");
    let auth_event_id = auth_event.id;
    websocket
        .send(ClientMessage::Text(
            json!(["AUTH", auth_event]).to_string().into(),
        ))
        .await
        .expect("AUTH sends");
    let auth_ok = tokio::time::timeout(Duration::from_secs(2), websocket.next())
        .await
        .expect("AUTH OK arrives")
        .expect("stream remains open")
        .expect("AUTH OK frame valid")
        .into_text()
        .expect("AUTH OK is text");
    let auth_ok: Value = serde_json::from_str(&auth_ok).expect("AUTH OK parses");
    assert_eq!(auth_ok[2], true);

    let event = EventBuilder::new(Kind::TextNote, "over secured WebSocket")
        .sign_with_keys(&keys)
        .expect("event signs");
    websocket
        .send(ClientMessage::Text(
            json!(["EVENT", event]).to_string().into(),
        ))
        .await
        .expect("EVENT sends");
    let event_ok = tokio::time::timeout(Duration::from_secs(2), websocket.next())
        .await
        .expect("EVENT OK arrives")
        .expect("stream remains open")
        .expect("EVENT OK frame valid")
        .into_text()
        .expect("EVENT OK is text");
    let event_ok: Value = serde_json::from_str(&event_ok).expect("EVENT OK parses");
    assert_eq!(event_ok[2], true);

    assert!(relay
        .store()
        .query(&[Filter::new().id(auth_event_id)])
        .await
        .expect("auth-event query succeeds")
        .is_empty());

    server.abort();
}

#[tokio::test]
async fn replication_peer_must_match_destination_source_binding() {
    let source = ReplicationSourceId::new("laptop-a/coherence");
    let peer = Keys::generate();
    let trust = RelayPeerTrust::new(
        source.clone(),
        "did:example:buzz-relay-a",
        [(
            peer.public_key(),
            "did:example:buzz-relay-a#node-key-1".to_string(),
        )],
    );
    let identity = Arc::new(LocalIdentityAdapter::with_peer_trust([trust]));
    let relay = LocalRelay::open_with_adapters(
        StorageMode::Ephemeral,
        Arc::new(ReplicationSourceAllowlist::new([source.clone()])),
        Some(identity),
    )
    .await
    .expect("destination opens");
    let event = EventBuilder::new(Kind::TextNote, "from peer")
        .sign_with_keys(&Keys::generate())
        .expect("event signs");
    let record = ReplicationRecord {
        source: source.clone(),
        cursor: ReplicationCursor::new("peer:1"),
        event: event.clone(),
    };
    let challenge = "b".repeat(64);
    let audience = "ws://destination.local/";
    let proof = EventBuilder::auth(
        &challenge,
        RelayUrl::parse(audience).expect("audience parses"),
    )
    .sign_with_keys(&peer)
    .expect("peer proof signs");
    let receipt = relay
        .ingest_replication_from_peer(
            LocalPeerEvidence {
                event: proof,
                challenge: challenge.clone(),
            },
            audience,
            record,
        )
        .await
        .expect("configured peer ingests");
    assert_eq!(receipt.outcome, ReplicationIngestOutcome::Stored);

    let attacker_proof = EventBuilder::auth(
        &challenge,
        RelayUrl::parse(audience).expect("audience parses"),
    )
    .sign_with_keys(&Keys::generate())
    .expect("attacker proof signs");
    let second = EventBuilder::new(Kind::TextNote, "spoofed peer")
        .sign_with_keys(&Keys::generate())
        .expect("event signs");
    let denied = relay
        .ingest_replication_from_peer(
            LocalPeerEvidence {
                event: attacker_proof,
                challenge,
            },
            audience,
            ReplicationRecord {
                source,
                cursor: ReplicationCursor::new("peer:2"),
                event: second.clone(),
            },
        )
        .await
        .expect_err("unbound peer is denied");
    assert_eq!(
        match denied {
            buzz_local_relay::ReplicationSinkError::Identity(error) => error.denial_code(),
            other => panic!("expected identity error, got {other}"),
        },
        Some(IdentityDenialCode::SourceMismatch)
    );
    assert!(relay
        .store()
        .query(&[Filter::new().id(second.id)])
        .await
        .expect("query succeeds")
        .is_empty());
}
