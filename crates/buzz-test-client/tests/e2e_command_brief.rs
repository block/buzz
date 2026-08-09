//! Real-route NIP-CB privacy and exact-ID replay tests.
//!
//! Run with a local relay:
//! `RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_command_brief -- --ignored`

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
use nostr::{Alphabet, Event, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use sha2::{Digest, Sha256};

const KIND_COMMAND_BRIEF: u16 = 44_210;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn fake_nip44_v2() -> String {
    format!("Ag{}", "A".repeat(130))
}

fn command_brief(keys: &Keys, run_id: &str, created_at: Timestamp) -> Event {
    EventBuilder::new(Kind::Custom(KIND_COMMAND_BRIEF), fake_nip44_v2())
        .tags([
            Tag::public_key(keys.public_key()),
            Tag::parse(["d", run_id]).expect("d"),
            Tag::parse(["status", "completed"]).expect("status"),
        ])
        .custom_created_at(created_at)
        .allow_self_tagging()
        .sign_with_keys(keys)
        .expect("sign command brief")
}

fn nip98_header(keys: &Keys, url: &str, body: &str) -> String {
    let payload = hex::encode(Sha256::digest(body.as_bytes()));
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", url]).expect("u"),
            Tag::parse(["method", "POST"]).expect("method"),
            Tag::parse(["payload", &payload]).expect("payload"),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).expect("nonce"),
        ])
        .sign_with_keys(keys)
        .expect("sign NIP-98");
    format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_string(&event).expect("serialize auth"))
    )
}

async fn submit(event: &Event, keys: &Keys) -> serde_json::Value {
    let url = format!("{}/events", relay_http_url());
    let body = serde_json::to_string(event).expect("serialize event");
    reqwest::Client::new()
        .post(&url)
        .header("Authorization", nip98_header(keys, &url, &body))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("submit event")
        .json()
        .await
        .expect("submit response")
}

fn owner_filter(owner: &Keys) -> Filter {
    Filter::new()
        .kind(Kind::Custom(KIND_COMMAND_BRIEF))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            owner.public_key().to_hex(),
        )
}

async fn query(pubkey: Option<&str>, filters: serde_json::Value) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(format!("{}/query", relay_http_url()))
        .header("Content-Type", "application/json")
        .json(&filters);
    if let Some(pubkey) = pubkey {
        request = request.header("X-Pubkey", pubkey);
    }
    request.send().await.expect("query")
}

async fn count(pubkey: Option<&str>, filters: serde_json::Value) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(format!("{}/count", relay_http_url()))
        .header("Content-Type", "application/json")
        .json(&filters);
    if let Some(pubkey) = pubkey {
        request = request.header("X-Pubkey", pubkey);
    }
    request.send().await.expect("count")
}

async fn community_id() -> uuid::Uuid {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
    let pool = sqlx::PgPool::connect(&url).await.expect("connect database");
    sqlx::query_scalar("SELECT id FROM communities WHERE lower(host)=lower('localhost:3000')")
        .fetch_one(&pool)
        .await
        .expect("test community")
}

async fn seed_bypassing_ingest(event: &Event) {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
    let pool = sqlx::PgPool::connect(&url).await.expect("connect database");
    let created_at =
        chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("timestamp");
    sqlx::query(
        "INSERT INTO events
         (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,d_tag)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,now(),$9)
         ON CONFLICT DO NOTHING",
    )
    .bind(community_id().await)
    .bind(event.id.as_bytes().as_slice())
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(created_at)
    .bind(KIND_COMMAND_BRIEF as i32)
    .bind(serde_json::to_value(&event.tags).expect("tags"))
    .bind(&event.content)
    .bind(event.sig.serialize().as_slice())
    .bind(
        event
            .tags
            .find(nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(
                Alphabet::D,
            )))
            .and_then(|tag| tag.content()),
    )
    .execute(&pool)
    .await
    .expect("seed malformed event");
}

#[tokio::test]
#[ignore]
async fn historical_exact_id_is_accepted_and_duplicate_replay_is_success() {
    let owner = Keys::generate();
    let event = command_brief(
        &owner,
        &format!("historical-{}", uuid::Uuid::new_v4()),
        Timestamp::from(Timestamp::now().as_secs() - 3_600),
    );
    let first = submit(&event, &owner).await;
    assert_eq!(first["accepted"], true, "first publish: {first}");
    assert_eq!(first["event_id"], event.id.to_hex());
    let duplicate = submit(&event, &owner).await;
    assert_eq!(
        duplicate["accepted"], true,
        "duplicate publish: {duplicate}"
    );
    assert_eq!(duplicate["event_id"], event.id.to_hex());
    assert!(
        duplicate["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("duplicate:")),
        "duplicate acknowledgement must be recognizable: {duplicate}"
    );
}

#[tokio::test]
#[ignore]
async fn real_req_count_id_live_and_http_search_routes_are_owner_private() {
    let owner = Keys::generate();
    let wrong = Keys::generate();
    let run_id = format!("routes-{}", uuid::Uuid::new_v4());
    let event = command_brief(&owner, &run_id, Timestamp::now());

    let mut owner_ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("owner connect");
    let mut wrong_ws = BuzzTestClient::connect(&relay_url(), &wrong)
        .await
        .expect("wrong connect");
    let mut unauth_live = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("unauthenticated live connect");
    let live_owner = format!("live-owner-{}", uuid::Uuid::new_v4());
    let live_wrong = format!("live-wrong-{}", uuid::Uuid::new_v4());
    owner_ws
        .subscribe(&live_owner, vec![Filter::new().id(event.id)])
        .await
        .expect("owner live REQ");
    wrong_ws
        .subscribe(&live_wrong, vec![Filter::new().id(event.id)])
        .await
        .expect("wrong live REQ");
    let live_unauth = format!("live-unauth-{}", uuid::Uuid::new_v4());
    unauth_live
        .subscribe(&live_unauth, vec![Filter::new().id(event.id)])
        .await
        .expect("unauthenticated live REQ");
    let _ = owner_ws
        .collect_until_eose(&live_owner, Duration::from_secs(5))
        .await
        .expect("owner EOSE");
    let _ = wrong_ws
        .collect_until_eose(&live_wrong, Duration::from_secs(5))
        .await
        .expect("wrong EOSE");
    loop {
        match unauth_live
            .recv_event(Duration::from_secs(5))
            .await
            .expect("unauthenticated live denial")
        {
            RelayMessage::Auth { .. } => {}
            RelayMessage::Notice { message } if message.contains("auth-required:") => break,
            RelayMessage::Event { .. } => {
                panic!("unauthenticated NIP-CB live REQ leaked an event")
            }
            RelayMessage::Count { .. } => panic!("unauthenticated live REQ leaked a count"),
            other => panic!("expected auth-required live denial, got {other:?}"),
        }
    }

    assert_eq!(submit(&event, &owner).await["accepted"], true);
    match owner_ws
        .recv_event(Duration::from_secs(5))
        .await
        .expect("owner live event")
    {
        RelayMessage::Event {
            event: delivered, ..
        } => assert_eq!(delivered.id, event.id),
        other => panic!("expected owner EVENT, got {other:?}"),
    }
    assert!(matches!(
        wrong_ws.recv_event(Duration::from_millis(500)).await,
        Err(TestClientError::Timeout)
    ));
    loop {
        match unauth_live.recv_event(Duration::from_millis(500)).await {
            Err(TestClientError::Timeout) => break,
            Ok(RelayMessage::Auth { .. } | RelayMessage::Closed { .. }) => {}
            Ok(RelayMessage::Notice { message }) if message.contains("auth-required:") => {}
            Ok(RelayMessage::Event { .. }) => {
                panic!("unauthenticated live route leaked an event after publish")
            }
            Ok(RelayMessage::Count { .. }) => {
                panic!("unauthenticated live route leaked existence after publish")
            }
            Ok(other) => panic!("unexpected unauthenticated live frame: {other:?}"),
            Err(error) => panic!("unauthenticated live receive failed: {error}"),
        }
    }

    let req_id = format!("owner-req-{}", uuid::Uuid::new_v4());
    owner_ws
        .subscribe(&req_id, vec![owner_filter(&owner)])
        .await
        .expect("owner REQ");
    let events = owner_ws
        .collect_until_eose(&req_id, Duration::from_secs(5))
        .await
        .expect("owner rows");
    assert!(events.iter().any(|row| row.id == event.id));

    let wrong_id = format!("wrong-id-{}", uuid::Uuid::new_v4());
    wrong_ws
        .subscribe(&wrong_id, vec![Filter::new().id(event.id)])
        .await
        .expect("wrong ID REQ");
    assert!(wrong_ws
        .collect_until_eose(&wrong_id, Duration::from_secs(5))
        .await
        .expect("wrong rows")
        .is_empty());

    let count_id = format!("owner-count-{}", uuid::Uuid::new_v4());
    owner_ws
        .send_raw(&serde_json::json!([
            "COUNT",
            count_id,
            owner_filter(&owner)
        ]))
        .await
        .expect("owner COUNT");
    match owner_ws
        .recv_event(Duration::from_secs(5))
        .await
        .expect("count response")
    {
        RelayMessage::Count { count, .. } => assert!(count >= 1),
        other => panic!("expected COUNT, got {other:?}"),
    }
    let wrong_count = format!("wrong-count-{}", uuid::Uuid::new_v4());
    wrong_ws
        .send_raw(&serde_json::json!([
            "COUNT",
            wrong_count,
            owner_filter(&owner)
        ]))
        .await
        .expect("wrong COUNT");
    assert!(matches!(
        wrong_ws
            .recv_event(Duration::from_secs(5))
            .await
            .expect("wrong COUNT response"),
        RelayMessage::Closed { .. }
    ));

    let mut unauthenticated = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("unauthenticated connect");
    let unauth_id = format!("unauth-req-{}", uuid::Uuid::new_v4());
    unauthenticated
        .subscribe(&unauth_id, vec![owner_filter(&owner)])
        .await
        .expect("unauthenticated REQ");
    loop {
        match unauthenticated
            .recv_event(Duration::from_secs(5))
            .await
            .expect("unauthenticated response")
        {
            RelayMessage::Closed { .. } => break,
            RelayMessage::Auth { .. } => {}
            RelayMessage::Notice { message } if message.contains("auth-required:") => break,
            RelayMessage::Event { .. } => panic!("unauthenticated NIP-CB read leaked an event"),
            RelayMessage::Count { .. } => panic!("unauthenticated REQ leaked a count"),
            other => panic!("expected auth-required denial, got {other:?}"),
        }
    }
    let unauth_count_id = format!("unauth-count-{}", uuid::Uuid::new_v4());
    unauthenticated
        .send_raw(&serde_json::json!([
            "COUNT",
            unauth_count_id,
            owner_filter(&owner)
        ]))
        .await
        .expect("unauthenticated COUNT");
    loop {
        match unauthenticated
            .recv_event(Duration::from_secs(5))
            .await
            .expect("unauthenticated COUNT denial")
        {
            RelayMessage::Auth { .. } => {}
            RelayMessage::Closed {
                subscription_id,
                message,
            } if message.contains("auth-required:") => {
                if subscription_id == unauth_count_id {
                    break;
                }
            }
            RelayMessage::Notice { message } if message.contains("auth-required:") => break,
            RelayMessage::Event { .. } => panic!("unauthenticated COUNT leaked an event"),
            RelayMessage::Count { .. } => panic!("unauthenticated COUNT leaked existence"),
            other => panic!("expected auth-required COUNT denial, got {other:?}"),
        }
    }

    let count_filter = serde_json::json!([{
        "kinds": [KIND_COMMAND_BRIEF],
        "#p": [owner.public_key().to_hex()]
    }]);
    let owner_count = count(Some(&owner.public_key().to_hex()), count_filter.clone()).await;
    assert!(owner_count.status().is_success());
    assert!(owner_count
        .json::<serde_json::Value>()
        .await
        .expect("owner HTTP count")["count"]
        .as_u64()
        .is_some_and(|value| value >= 1));
    assert!(
        !count(Some(&wrong.public_key().to_hex()), count_filter.clone())
            .await
            .status()
            .is_success()
    );
    assert!(!count(None, count_filter).await.status().is_success());

    let search = serde_json::json!([{
        "ids": [event.id.to_hex()],
        "search": "Ag",
        "limit": 10
    }]);
    for viewer in [owner.public_key().to_hex(), wrong.public_key().to_hex()] {
        let response = query(Some(&viewer), search.clone()).await;
        assert!(response.status().is_success());
        assert!(response
            .json::<Vec<serde_json::Value>>()
            .await
            .expect("search rows")
            .is_empty());
    }
    assert!(!query(None, search).await.status().is_success());
}

#[tokio::test]
#[ignore]
async fn malformed_stored_command_briefs_fail_closed_at_real_result_routes() {
    let owner = Keys::generate();
    let wrong = Keys::generate();
    let common = [
        Tag::parse(["d", &format!("fixture-{}", uuid::Uuid::new_v4())]).expect("d"),
        Tag::parse(["status", "completed"]).expect("status"),
    ];
    let variants = [
        common.to_vec(),
        vec![
            Tag::public_key(owner.public_key()),
            Tag::public_key(owner.public_key()),
            common[0].clone(),
            common[1].clone(),
        ],
        vec![
            Tag::public_key(wrong.public_key()),
            common[0].clone(),
            common[1].clone(),
        ],
    ];
    let mut ids = Vec::new();
    for (index, tags) in variants.into_iter().enumerate() {
        let event = EventBuilder::new(Kind::Custom(KIND_COMMAND_BRIEF), fake_nip44_v2())
            .tags(tags)
            .allow_self_tagging()
            .sign_with_keys(&owner)
            .expect("sign fixture");
        let p_count = event
            .tags
            .filter(nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(
                Alphabet::P,
            )))
            .count();
        assert_eq!(p_count, [0, 2, 1][index], "fixture p-tag cardinality");
        seed_bypassing_ingest(&event).await;
        ids.push(event.id);
    }

    let mut owner_ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("owner connect");
    for id in &ids {
        let sid = format!("malformed-{}", uuid::Uuid::new_v4());
        owner_ws
            .subscribe(&sid, vec![Filter::new().id(*id)])
            .await
            .expect("fixture REQ");
        assert!(owner_ws
            .collect_until_eose(&sid, Duration::from_secs(5))
            .await
            .expect("fixture rows")
            .is_empty());
    }
    let count_id = format!("malformed-count-{}", uuid::Uuid::new_v4());
    owner_ws
        .send_raw(&serde_json::json!([
            "COUNT",
            count_id,
            {"ids": ids.iter().map(|id| id.to_hex()).collect::<Vec<_>>()}
        ]))
        .await
        .expect("malformed COUNT");
    match owner_ws
        .recv_event(Duration::from_secs(5))
        .await
        .expect("malformed COUNT response")
    {
        RelayMessage::Count { count, .. } => assert_eq!(count, 0),
        other => panic!("expected COUNT, got {other:?}"),
    }
    let http_count = count(
        Some(&owner.public_key().to_hex()),
        serde_json::json!([{
            "ids": ids.iter().map(|id| id.to_hex()).collect::<Vec<_>>()
        }]),
    )
    .await;
    assert!(http_count.status().is_success());
    assert_eq!(
        http_count
            .json::<serde_json::Value>()
            .await
            .expect("HTTP count")["count"],
        0
    );
    let filters = serde_json::json!([{
        "ids": ids.iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
        "limit": 10
    }]);
    let response = query(Some(&owner.public_key().to_hex()), filters).await;
    assert!(response.status().is_success());
    assert!(response
        .json::<Vec<serde_json::Value>>()
        .await
        .expect("HTTP rows")
        .is_empty());
}
