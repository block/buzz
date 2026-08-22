//! End-to-end privacy, immutability, and replay contract for Stage 1 section workspaces.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, Event, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const IMPORT_KIND: u16 = 9050;
const PROJECTION_KIND: u16 = 30623;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".into())
}

fn http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .into()
}

fn reader_filter(kind: u16, reader: &Keys) -> Filter {
    Filter::new().kind(Kind::Custom(kind)).custom_tag(
        SingleLetterTag::lowercase(Alphabet::P),
        reader.public_key().to_hex(),
    )
}

async fn ws_query(client: &mut BuzzTestClient, name: &str, filter: Filter) -> Vec<Event> {
    client.subscribe(name, vec![filter]).await.unwrap();
    client
        .collect_until_eose(name, Duration::from_secs(5))
        .await
        .unwrap()
}

fn nip98(keys: &Keys, url: &str, body: &str) -> String {
    let payload = hex::encode(Sha256::digest(body.as_bytes()));
    let event = EventBuilder::new(Kind::Custom(27_235), "")
        .tags([
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &payload]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_string(&event).unwrap())
    )
}

async fn http_query(http: &Client, reader: &Keys, filter: Filter) -> Vec<Value> {
    let url = format!("{}/query", http_url());
    let body = serde_json::to_string(&vec![filter]).unwrap();
    let response = http
        .post(&url)
        .header("Authorization", nip98(reader, &url, &body))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "query status={}",
        response.status()
    );
    response.json().await.unwrap()
}

async fn http_count(http: &Client, reader: &Keys, filter: Filter) -> u64 {
    let url = format!("{}/count", http_url());
    let body = serde_json::to_string(&vec![filter]).unwrap();
    let response = http
        .post(&url)
        .header("Authorization", nip98(reader, &url, &body))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "count status={}",
        response.status()
    );
    response.json::<Value>().await.unwrap()["count"]
        .as_u64()
        .unwrap()
}

fn import_event(owner: &Keys, source: &Event, action_id: uuid::Uuid, marker: &str) -> Event {
    let content = json!({
        "version": 1,
        "action_id": action_id,
        "source_event_id": source.id.to_hex(),
        "source_hash": marker,
        "key_epoch": 1,
        "owner_key_envelope": format!("owner-envelope-{marker}"),
        "sections": [],
        "assignments": []
    })
    .to_string();
    EventBuilder::new(Kind::Custom(IMPORT_KIND), content)
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
            Tag::parse(["action", &action_id.to_string()]).unwrap(),
        ])
        .allow_self_tagging()
        .sign_with_keys(owner)
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn owner_import_projection_is_private_immutable_and_one_shot() {
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner).await.unwrap();
    let mut stranger_client = BuzzTestClient::connect(&relay_url(), &stranger)
        .await
        .unwrap();
    let http = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let source = EventBuilder::new(Kind::Custom(30078), "legacy section state")
        .sign_with_keys(&owner)
        .unwrap();
    assert!(
        owner_client
            .send_event(source.clone())
            .await
            .unwrap()
            .accepted
    );

    let action_id = uuid::Uuid::new_v4();
    let import = import_event(&owner, &source, action_id, &"33".repeat(32));
    let accepted = owner_client.send_event(import.clone()).await.unwrap();
    assert!(accepted.accepted, "{}", accepted.message);

    let imports = ws_query(
        &mut owner_client,
        "owner-import",
        reader_filter(IMPORT_KIND, &owner),
    )
    .await;
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].id, import.id);
    let projections = ws_query(
        &mut owner_client,
        "owner-projection",
        reader_filter(PROJECTION_KIND, &owner),
    )
    .await;
    assert_eq!(projections.len(), 1);
    let projection = projections[0].clone();

    for (kind, event) in [(IMPORT_KIND, &import), (PROJECTION_KIND, &projection)] {
        let owner_events = http_query(&http, &owner, reader_filter(kind, &owner)).await;
        assert_eq!(owner_events.len(), 1);
        assert_eq!(owner_events[0]["id"], event.id.to_hex());
        assert_eq!(
            http_count(&http, &owner, reader_filter(kind, &owner)).await,
            1
        );

        assert!(ws_query(
            &mut stranger_client,
            &format!("stranger-kind-{kind}"),
            reader_filter(kind, &stranger),
        )
        .await
        .is_empty());
        assert!(http_query(&http, &stranger, reader_filter(kind, &stranger))
            .await
            .is_empty());
        assert_eq!(
            http_count(&http, &stranger, reader_filter(kind, &stranger)).await,
            0
        );
        assert!(ws_query(
            &mut stranger_client,
            &format!("stranger-id-{kind}"),
            Filter::new().id(event.id),
        )
        .await
        .is_empty());
        assert!(http_query(&http, &stranger, Filter::new().id(event.id))
            .await
            .is_empty());
        assert_eq!(
            http_count(&http, &stranger, Filter::new().id(event.id)).await,
            0
        );
        assert!(ws_query(
            &mut stranger_client,
            &format!("stranger-search-{kind}"),
            Filter::new().id(event.id).search("owner-envelope"),
        )
        .await
        .is_empty());
        assert!(http_query(
            &http,
            &stranger,
            Filter::new().id(event.id).search("owner-envelope"),
        )
        .await
        .is_empty());

        let deletion = EventBuilder::new(Kind::Custom(5), "")
            .tags([Tag::parse(["e", &event.id.to_hex()]).unwrap()])
            .sign_with_keys(&owner)
            .unwrap();
        let rejected = owner_client.send_event(deletion).await.unwrap();
        assert!(
            !rejected.accepted,
            "kind {kind} event-id deletion was accepted"
        );
    }

    let coordinate = format!(
        "{}:{}:{}:{}",
        PROJECTION_KIND,
        projection.pubkey.to_hex(),
        owner.public_key().to_hex(),
        owner.public_key().to_hex()
    );
    let address_deletion = EventBuilder::new(Kind::Custom(5), "")
        .tags([Tag::parse(["a", &coordinate]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(
        !owner_client
            .send_event(address_deletion)
            .await
            .unwrap()
            .accepted
    );

    let replay = owner_client.send_event(import.clone()).await.unwrap();
    assert!(replay.accepted, "{}", replay.message);
    assert!(replay.message.starts_with("duplicate:"));
    assert_eq!(
        ws_query(
            &mut owner_client,
            "projection-after-replay",
            reader_filter(PROJECTION_KIND, &owner),
        )
        .await
        .len(),
        1
    );

    let changed = import_event(&owner, &source, uuid::Uuid::new_v4(), &"44".repeat(32));
    let conflict = owner_client.send_event(changed).await.unwrap();
    assert!(!conflict.accepted);
    assert!(
        conflict.message.starts_with("conflict:"),
        "{}",
        conflict.message
    );
}
