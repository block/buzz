//! Real ingest/query regressions. Use only an isolated relay and test accounts.
//! Set RELAY_URL, BUZZ_TEST_OWNER_PRIVATE_KEY, BUZZ_TEST_MEMBER_PRIVATE_KEY,
//! and BUZZ_TEST_CHANNEL_ID, then run this ignored test target.

use base64::{engine::general_purpose::STANDARD, Engine};
use nostr::hashes::{sha256, Hash};
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use uuid::Uuid;

struct Fixture {
    url: String,
    owner: Keys,
    member: Keys,
    channel: Uuid,
}

impl Fixture {
    fn new() -> Self {
        Self {
            url: std::env::var("RELAY_URL")
                .expect("isolated RELAY_URL")
                .replace("ws://", "http://")
                .replace("wss://", "https://")
                .trim_end_matches('/')
                .to_owned(),
            owner: Keys::parse(&std::env::var("BUZZ_TEST_OWNER_PRIVATE_KEY").unwrap()).unwrap(),
            member: Keys::parse(&std::env::var("BUZZ_TEST_MEMBER_PRIVATE_KEY").unwrap()).unwrap(),
            channel: std::env::var("BUZZ_TEST_CHANNEL_ID")
                .unwrap()
                .parse()
                .unwrap(),
        }
    }

    async fn post(&self, keys: &Keys, path: &str, body: Value) -> Value {
        let url = format!("{}{path}", self.url);
        let body = body.to_string();
        let http = HttpData::new(url.parse().unwrap(), HttpMethod::POST)
            .payload(sha256::Hash::hash(body.as_bytes()));
        let auth = EventBuilder::http_auth(http)
            .tag(Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap())
            .sign_with_keys(keys)
            .unwrap();
        reqwest::Client::new()
            .post(url)
            .header(
                "Authorization",
                format!("Nostr {}", STANDARD.encode(auth.as_json())),
            )
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn submit(&self, keys: &Keys, event: &Event) -> Value {
        self.post(keys, "/events", serde_json::to_value(event).unwrap())
            .await
    }

    async fn accepted(&self, event: &Event) {
        let response = self.submit(&self.owner, event).await;
        assert_eq!(response["accepted"], true, "{response}");
    }

    async fn query(&self, filter: Value) -> Vec<Event> {
        serde_json::from_value(self.post(&self.owner, "/query", json!([filter])).await).unwrap()
    }

    async fn definition(&self, id: Uuid) -> Vec<Event> {
        self.query(json!({"kinds": [30620], "#d": [id.to_string()]}))
            .await
    }

    fn deletion(&self, id: Uuid, timestamp: Timestamp) -> Event {
        buzz_sdk::build_workflow_delete(&self.owner.public_key().to_hex(), id)
            .unwrap()
            .custom_created_at(timestamp)
            .sign_with_keys(&self.owner)
            .unwrap()
    }

    async fn create(&self, id: Uuid, timestamp: Timestamp) {
        let event = buzz_sdk::build_workflow_def(self.channel, id,
            "name: delete-regression\ntrigger:\n  on: schedule\n  cron: '0 0 1 1 *'\nsteps:\n  - id: marker\n    action: send_message\n    text: isolated deletion regression\n")
            .unwrap().custom_created_at(timestamp).sign_with_keys(&self.owner).unwrap();
        self.accepted(&event).await;
        assert_eq!(self.definition(id).await.len(), 1);
    }
}

#[tokio::test]
#[ignore = "requires isolated relay and two test accounts"]
async fn deletion_removes_get_list_and_execution_and_replay_is_safe() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    let now = Timestamp::now();
    f.create(id, now).await;

    // A different authenticated channel member cannot delete this coordinate.
    let deletion = f.deletion(id, now);
    let forged = EventBuilder::new(Kind::Custom(5), "")
        .tags(deletion.tags.clone())
        .sign_with_keys(&f.member)
        .unwrap();
    let rejected = f.submit(&f.member, &forged).await;
    assert_ne!(rejected["accepted"], true, "{rejected}");
    assert_eq!(f.definition(id).await.len(), 1);

    f.accepted(&deletion).await;
    for _ in 0..2 {
        assert!(
            f.definition(id).await.is_empty(),
            "get must omit deleted definition"
        );
        let list = f
            .query(json!({"kinds": [30620], "#h": [f.channel.to_string()]}))
            .await;
        assert!(
            list.iter()
                .all(|event| event.tags.identifier() != Some(id.to_string().as_str())),
            "list must omit deleted definition"
        );
        let trigger = buzz_sdk::build_workflow_trigger(id)
            .unwrap()
            .sign_with_keys(&f.owner)
            .unwrap();
        let response = f.submit(&f.owner, &trigger).await;
        assert_ne!(
            response["accepted"], true,
            "deleted workflow must not execute: {response}"
        );
        assert!(
            response.to_string().contains("workflow not found"),
            "{response}"
        );
        f.accepted(&deletion).await;
    }
}

#[tokio::test]
#[ignore = "requires isolated relay and two test accounts"]
async fn stale_deletion_preserves_newer_definition_and_runtime() {
    let f = Fixture::new();
    let id = Uuid::new_v4();
    let now = Timestamp::now();
    f.create(id, now).await;
    f.accepted(&f.deletion(id, Timestamp::from(now.as_secs() - 1)))
        .await;
    assert_eq!(f.definition(id).await.len(), 1);
    let trigger = buzz_sdk::build_workflow_trigger(id)
        .unwrap()
        .sign_with_keys(&f.owner)
        .unwrap();
    f.accepted(&trigger).await;
    f.accepted(&f.deletion(id, now)).await;
    assert!(f.definition(id).await.is_empty());
}
