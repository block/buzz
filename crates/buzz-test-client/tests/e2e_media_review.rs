//! Live report -> private file review -> report closure journey.
//! Requires the relay's database and BUZZ_MEDIA_READ_MODE=enforce.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use reqwest::{Client, Method};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn auth(keys: &Keys, host: &str, verb: &str, hash: &str) -> String {
    let event = EventBuilder::new(Kind::from(24242), "media review test")
        .tags([
            Tag::parse(["t", verb]).unwrap(),
            Tag::parse(["server", host]).unwrap(),
            Tag::parse(["x", hash]).unwrap(),
            Tag::parse(["expiration", &(Timestamp::now().as_secs() + 55).to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!("Nostr {}", URL_SAFE_NO_PAD.encode(event.as_json()))
}

async fn read(http: &Client, keys: &Keys, base: &str, host: &str, hash: &str, expected: u16) {
    for method in [Method::GET, Method::HEAD] {
        let response = http
            .request(method.clone(), format!("{base}/media/{hash}"))
            .header("authorization", auth(keys, host, "get", hash))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            expected,
            "{method} for {}",
            keys.public_key()
        );
    }
}

#[tokio::test]
#[ignore]
async fn open_report_grants_only_audited_moderator_attachment_access() {
    let base = std::env::var("RELAY_HTTP_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let host = url::Url::parse(&base).unwrap().authority().to_owned();
    let ws = base
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    let database = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into()); // sadscan:disable np.postgres.1 -- local test-only credentials
    let pool = sqlx::PgPool::connect(&database).await.unwrap();
    let community: Uuid = sqlx::query_scalar("SELECT id FROM communities WHERE host = $1")
        .bind(&host)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (owner, moderator, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
    // Seed only the queue role; all upload, share, report, read, and closure I/O is real.
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1,$2,'admin')")
        .bind(community)
        .bind(moderator.public_key().to_hex())
        .execute(&pool)
        .await
        .unwrap();
    let http = Client::new();
    let channel = Uuid::new_v4().to_string();
    let mut author = BuzzTestClient::connect(&ws, &owner).await.unwrap();
    let create = EventBuilder::new(Kind::from(9007), "")
        .tags([
            Tag::parse(["h", &channel]).unwrap(),
            Tag::parse(["name", &channel]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "private"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(author.send_event(create).await.unwrap().accepted);
    let mut sources = Vec::new();
    for _ in 0..2 {
        let bytes = Uuid::new_v4().as_bytes().repeat(8);
        let hash = hex::encode(Sha256::digest(&bytes));
        let response = http
            .put(format!("{base}/upload"))
            .header("authorization", auth(&owner, &host, "upload", &hash))
            .header("x-sha-256", &hash)
            .body(bytes)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let event = EventBuilder::new(Kind::from(9), format!("{base}/media/{hash}"))
            .tags([Tag::parse(["h", &channel]).unwrap()])
            .sign_with_keys(&owner)
            .unwrap();
        let id = event.id;
        assert!(author.send_event(event).await.unwrap().accepted);
        sources.push((hash, id));
    }
    let (hash, event_id) = &sources[0];
    read(&http, &moderator, &base, &host, hash, 404).await;
    let report = EventBuilder::new(
        Kind::from(1984),
        format!("unrelated /media/{}", sources[1].0),
    )
    .allow_self_tagging()
    .tags([
        Tag::parse(["e", &event_id.to_hex(), "spam"]).unwrap(),
        Tag::parse(["p", &owner.public_key().to_hex(), "spam"]).unwrap(),
    ])
    .sign_with_keys(&owner)
    .unwrap();
    let report_event_id = report.id;
    let result = author.send_event(report).await.unwrap();
    assert!(result.accepted, "report rejected: {}", result.message);
    read(&http, &moderator, &base, &host, hash, 200).await;
    read(&http, &stranger, &base, &host, hash, 404).await;
    read(&http, &moderator, &base, &host, &sources[1].0, 404).await;
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE community_id=$1 AND action='moderation_media_read' AND actor_pubkey=$2 AND object_id=$3")
        .bind(community).bind(moderator.public_key().to_bytes().to_vec()).bind(hash).fetch_one(&pool).await.unwrap();
    assert_eq!(audited, 2, "GET and HEAD must each commit an audit entry");
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(community)
            .bind(report_event_id.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        stored, 0,
        "reports must remain outside the public event log"
    );
    let mut reviewer = BuzzTestClient::connect(&ws, &moderator).await.unwrap();
    let close = EventBuilder::new(Kind::from(9044), "")
        .tags([
            Tag::parse(["report", &report_event_id.to_hex()]).unwrap(),
            Tag::parse(["status", "dismissed"]).unwrap(),
            Tag::parse(["action", "dismiss"]).unwrap(),
        ])
        .sign_with_keys(&moderator)
        .unwrap();
    let result = reviewer.send_event(close).await.unwrap();
    assert!(
        result.accepted,
        "report closure rejected: {}",
        result.message
    );
    read(&http, &moderator, &base, &host, hash, 404).await;
    read(&http, &owner, &base, &host, hash, 200).await;

    // A later edit adds a different attachment. The report targets the logical
    // message, and deleting it must revoke the exception even while open.
    let edited_hash = &sources[1].0;
    let edit = EventBuilder::new(Kind::from(40003), format!("{base}/media/{edited_hash}"))
        .tags([
            Tag::parse(["h", &channel]).unwrap(),
            Tag::parse(["e", &event_id.to_hex()]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(author.send_event(edit).await.unwrap().accepted);
    let reopened = EventBuilder::new(Kind::from(1984), "review edited attachment")
        .allow_self_tagging()
        .tags([
            Tag::parse(["e", &event_id.to_hex(), "spam"]).unwrap(),
            Tag::parse(["p", &owner.public_key().to_hex(), "spam"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(author.send_event(reopened).await.unwrap().accepted);
    read(&http, &moderator, &base, &host, edited_hash, 200).await;
    read(&http, &stranger, &base, &host, edited_hash, 404).await;
    let deletion = EventBuilder::new(Kind::EventDeletion, "delete edited message")
        .tags([
            Tag::parse(["h", &channel]).unwrap(),
            Tag::parse(["e", &event_id.to_hex()]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(author.send_event(deletion).await.unwrap().accepted);
    read(&http, &moderator, &base, &host, edited_hash, 404).await;
    // Its independent private post survives, so the channel owner still reads it.
    read(&http, &owner, &base, &host, edited_hash, 200).await;
    author.disconnect().await.unwrap();
    reviewer.disconnect().await.unwrap();
}
