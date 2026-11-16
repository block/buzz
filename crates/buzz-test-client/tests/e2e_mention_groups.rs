//! NIP-MG live WS/HTTP journey on an explicitly selected isolated local relay.
//!
//! BUZZ_MG_TEST_RELAY_URL=ws://127.0.0.1:15891 cargo test -p buzz-test-client \
//!   --test e2e_mention_groups -- --ignored --nocapture
//! The relay must use the feature schema, open membership, and a fresh community.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use buzz_core::kind::{
    KIND_MENTION_GROUP_STATE, RELAY_ADMIN_DELETE_MENTION_GROUP, RELAY_ADMIN_PUT_MENTION_GROUP,
    RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
};
use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, PublicKey, Tag};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn local_relay_urls() -> (String, String) {
    let ws = std::env::var("BUZZ_MG_TEST_RELAY_URL")
        .expect("set BUZZ_MG_TEST_RELAY_URL to an isolated local relay");
    let mut url = url::Url::parse(&ws).unwrap();
    assert!(
        matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        ),
        "local relay required"
    );
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => panic!("WebSocket relay URL required"),
    };
    url.set_scheme(scheme).unwrap();
    (ws, url.as_str().trim_end_matches('/').to_string())
}

fn put(keys: &Keys, id: Uuid, prev: i64, name: &str) -> Event {
    put_with_defaults(keys, id, prev, name, &[keys.public_key()], &[])
}

fn put_with_defaults(
    keys: &Keys,
    id: Uuid,
    prev: i64,
    name: &str,
    members: &[PublicKey],
    channels: &[Uuid],
) -> Event {
    let mut tags = vec![
        Tag::parse(["d", &id.to_string()]).unwrap(),
        Tag::parse(["prev", &prev.to_string()]).unwrap(),
        Tag::parse(["name", name]).unwrap(),
    ];
    tags.extend(members.iter().copied().map(Tag::public_key));
    tags.extend(
        channels
            .iter()
            .map(|id| Tag::parse(["channel", &id.to_string()]).unwrap()),
    );
    EventBuilder::new(Kind::Custom(RELAY_ADMIN_PUT_MENTION_GROUP as u16), "")
        .allow_self_tagging()
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

async fn post(http: &str, keys: &Keys, path: &str, body: &Value) -> (StatusCode, Value) {
    let url = format!("{http}{path}");
    let bytes = serde_json::to_vec(body).unwrap();
    let auth = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&bytes))]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .post(&url)
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!(
                "Nostr {}",
                BASE64.encode(serde_json::to_vec(&auth).unwrap())
            ),
        )
        .body(bytes)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json().await.unwrap();
    (status, body)
}

async fn query_state(
    http: &str,
    reader: &Keys,
    relay: PublicKey,
    kind: u32,
    d: &str,
) -> Vec<Event> {
    let (status, body) = post(
        http,
        reader,
        "/query",
        &json!([{
            "kinds":[kind], "authors":[relay.to_hex()], "#d":[d], "consistency":"strong"
        }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

fn field<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.as_slice()[0] == name)
        .and_then(Tag::content)
}

async fn live_head(
    reader: &mut BuzzTestClient,
    sub: &str,
    relay: PublicKey,
    version: &str,
) -> Event {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "missing live group revision {version}"
        );
        match reader
            .recv_event(remaining)
            .await
            .expect("live group delivery")
        {
            RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == sub => {
                assert_eq!(event.kind.as_u16() as u32, KIND_MENTION_GROUP_STATE);
                assert_eq!(event.pubkey, relay);
                assert!(event.verify_id() && event.verify_signature());
                if field(&event, "version") == Some(version) {
                    return *event;
                }
            }
            RelayMessage::Closed { message, .. } => panic!("subscription closed: {message}"),
            _ => {}
        }
    }
}

#[tokio::test]
#[ignore = "requires an isolated open local relay with the feature schema"]
async fn http_commands_ws_live_delivery_and_tombstone_reads_share_one_authoritative_head() {
    let (ws, http) = local_relay_urls();
    let info: Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .get(format!("{http}/"))
        .header("accept", "application/nostr+json")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        info["mention_groups"]["membership"], "open",
        "isolated open relay required"
    );
    assert_eq!(info["mention_groups"]["default_manage"], "everyone");
    assert_eq!(info["mention_groups"]["version"], 1);
    assert_eq!(info["mention_groups"]["max_members"], 20);
    let relay = PublicKey::from_hex(info["self"].as_str().expect("relay signing key")).unwrap();
    let actor = Keys::generate();
    let outsider = Keys::generate();
    let eligible = Keys::generate();
    let blocked = Keys::generate();
    let id = Uuid::new_v4();
    let name = format!("mg-e2e-{id}");
    let sub = format!("mg-{id}");
    let mut writer = BuzzTestClient::connect(&ws, &actor).await.unwrap();
    let mut reader = BuzzTestClient::connect(&ws, &outsider).await.unwrap();
    let bounded_id = Uuid::new_v4();
    let bounded_name = format!("mg-limit-{bounded_id}");
    let bounded_members: Vec<_> = (0..21).map(|_| Keys::generate().public_key()).collect();
    let oversized = put_with_defaults(&actor, bounded_id, 0, &bounded_name, &bounded_members, &[]);
    let refusal = "invalid: a group can have at most 20 members";
    let (status, body) = post(&http, &actor, "/events", &json!(oversized)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], refusal);
    let ack = writer.send_event(oversized).await.unwrap();
    assert!(!ack.accepted, "{ack:?}");
    assert_eq!(ack.message, refusal);
    let boundary = put_with_defaults(
        &actor,
        bounded_id,
        0,
        &bounded_name,
        &bounded_members[..20],
        &[],
    );
    let (status, body) = post(&http, &actor, "/events", &json!(boundary)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true);
    let heads = query_state(
        &http,
        &actor,
        relay,
        KIND_MENTION_GROUP_STATE,
        &bounded_id.to_string(),
    )
    .await;
    assert_eq!(heads.len(), 1);
    assert_eq!(
        heads[0]
            .tags
            .iter()
            .filter(|tag| tag.as_slice()[0] == "p")
            .count(),
        20
    );
    assert!(
        query_state(&http, &actor, relay, KIND_MENTION_GROUP_STATE, "settings")
            .await
            .is_empty(),
        "fresh community with implicit settings revision zero required"
    );
    let channel = Uuid::new_v4();
    let create = EventBuilder::new(Kind::Custom(9007), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["name", &format!("mg-default-{channel}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
            Tag::parse(["unlisted", "false"]).unwrap(),
        ])
        .sign_with_keys(&actor)
        .unwrap();
    let (status, created) = post(&http, &actor, "/events", &json!(create)).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["accepted"], true);
    let refuse_adds = EventBuilder::new(
        Kind::Custom(10100),
        json!({"channel_add_policy":"nobody"}).to_string(),
    )
    .sign_with_keys(&blocked)
    .unwrap();
    let (status, configured) = post(&http, &blocked, "/events", &json!(refuse_adds)).await;
    assert_eq!(status, StatusCode::OK, "{configured}");
    assert_eq!(configured["accepted"], true);
    let members = [
        actor.public_key(),
        eligible.public_key(),
        blocked.public_key(),
    ];
    let filter = json!({"kinds":[39010], "authors":[relay.to_hex()], "#d":[id.to_string()]});
    reader
        .subscribe(
            &sub,
            vec![serde_json::from_value::<Filter>(filter.clone()).unwrap()],
        )
        .await
        .unwrap();
    assert!(reader
        .collect_until_eose(&sub, Duration::from_secs(10))
        .await
        .unwrap()
        .is_empty());

    let original = put_with_defaults(&actor, id, 0, &name, &members, &[channel]);
    let (status, accepted) = post(&http, &actor, "/events", &json!(original)).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["event_id"], original.id.to_hex());
    let original_message = accepted["message"].as_str().unwrap().to_owned();
    let joins: Value = serde_json::from_str(
        original_message
            .strip_prefix("joins: ")
            .expect("join accounting in acceptance"),
    )
    .unwrap();
    assert_eq!(joins["joined"], 1);
    assert_eq!(joins["skipped"]["already_member"], 1);
    assert_eq!(joins["skipped"]["target_policy_nobody"], 1);
    let initial = live_head(&mut reader, &sub, relay, "1").await;
    assert_eq!(field(&initial, "name"), Some(name.as_str()));
    for member in members {
        assert!(
            initial
                .tags
                .iter()
                .any(|tag| tag.as_slice()[0] == "p"
                    && tag.content() == Some(member.to_hex().as_str()))
        );
    }
    assert!(!initial.tags.iter().any(|tag| tag.as_slice()[0] == "p"
        && tag.content() == Some(outsider.public_key().to_hex().as_str())));
    assert_eq!(
        field(&initial, "channel"),
        Some(channel.to_string().as_str())
    );
    let rosters = query_state(&http, &actor, relay, 39002, &channel.to_string()).await;
    assert_eq!(rosters.len(), 1);
    assert!(rosters[0].verify_signature());
    for (member, expected) in [
        (actor.public_key(), true),
        (eligible.public_key(), true),
        (blocked.public_key(), false),
    ] {
        assert_eq!(
            rosters[0]
                .tags
                .iter()
                .any(|tag| tag.as_slice()[0] == "p"
                    && tag.content() == Some(member.to_hex().as_str())),
            expected
        );
    }

    for (event, expected_status, prefix) in [
        (
            put(&actor, id, 0, "Stale overwrite"),
            StatusCode::CONFLICT,
            "conflict:",
        ),
        (
            put(&actor, Uuid::new_v4(), 0, &name),
            StatusCode::CONFLICT,
            "duplicate:",
        ),
        (
            put(&actor, Uuid::new_v4(), 0, ""),
            StatusCode::BAD_REQUEST,
            "invalid:",
        ),
    ] {
        let (status, body) = post(&http, &actor, "/events", &json!(event)).await;
        assert_eq!(status, expected_status, "{body}");
        assert!(
            body["error"].as_str().unwrap().starts_with(prefix),
            "{body}"
        );
    }

    let edit = put_with_defaults(
        &actor,
        id,
        1,
        &format!("{name} renamed"),
        &members,
        &[channel],
    );
    let ack = writer.send_event(edit.clone()).await.unwrap();
    assert!(ack.accepted, "{ack:?}");
    assert_eq!(ack.event_id, edit.id.to_hex());
    let edited = live_head(&mut reader, &sub, relay, "2").await;
    assert!(edited.created_at > initial.created_at);
    let (status, duplicate) = post(&http, &actor, "/events", &json!(original)).await;
    assert_eq!(status, StatusCode::OK, "{duplicate}");
    assert_eq!(duplicate["accepted"], true);
    assert_eq!(
        duplicate["message"],
        format!("duplicate: {original_message}")
    );

    let policy = |manage: &str, prev: &str| {
        EventBuilder::new(
            Kind::Custom(RELAY_ADMIN_SET_MENTION_GROUP_POLICY as u16),
            "",
        )
        .tags([
            Tag::parse(["manage", manage]).unwrap(),
            Tag::parse(["prev", prev]).unwrap(),
        ])
        .sign_with_keys(&actor)
        .unwrap()
    };
    let admins = policy("admins", "0");
    let (status, policy_ack) = post(&http, &actor, "/events", &json!(admins)).await;
    assert_eq!(status, StatusCode::OK, "{policy_ack}");
    assert_eq!(policy_ack["accepted"], true);
    let settings = query_state(
        &http,
        &outsider,
        relay,
        KIND_MENTION_GROUP_STATE,
        "settings",
    )
    .await;
    assert_eq!(settings.len(), 1);
    assert!(settings[0].verify_id() && settings[0].verify_signature());
    assert_eq!(field(&settings[0], "manage"), Some("admins"));
    assert_eq!(field(&settings[0], "version"), Some("1"));
    assert_eq!(
        field(&settings[0], "command"),
        Some(admins.id.to_hex().as_str())
    );
    assert!(settings[0].content.is_empty());
    let outsider_put = put(&outsider, Uuid::new_v4(), 0, "Forbidden group");
    let (status, denied) = post(&http, &outsider, "/events", &json!(outsider_put)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    assert!(
        denied["error"].as_str().unwrap().starts_with("restricted:"),
        "{denied}"
    );
    // This relay has no owner/admin roster. The same admitted actor can restore
    // everyone management through the rosterless settings authority rule.
    let everyone = policy("everyone", "1");
    let (status, restored) = post(&http, &actor, "/events", &json!(everyone)).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["accepted"], true);
    let restored = query_state(
        &http,
        &outsider,
        relay,
        KIND_MENTION_GROUP_STATE,
        "settings",
    )
    .await;
    assert_eq!(restored.len(), 1);
    assert_eq!(field(&restored[0], "manage"), Some("everyone"));
    assert_eq!(field(&restored[0], "version"), Some("2"));
    assert_eq!(
        field(&restored[0], "command"),
        Some(everyone.id.to_hex().as_str())
    );

    let forged = EventBuilder::new(Kind::Custom(KIND_MENTION_GROUP_STATE as u16), "")
        .tags(edited.tags.iter().cloned())
        .sign_with_keys(&actor)
        .unwrap();
    let ack = writer.send_event(forged.clone()).await.unwrap();
    assert!(
        !ack.accepted && ack.message.contains("relay-only"),
        "{ack:?}"
    );
    let (status, body) = post(&http, &actor, "/events", &json!(forged)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("relay-only"),
        "{body}"
    );

    let delete = EventBuilder::new(Kind::Custom(RELAY_ADMIN_DELETE_MENTION_GROUP as u16), "")
        .tags([
            Tag::parse(["d", &id.to_string()]).unwrap(),
            Tag::parse(["prev", "2"]).unwrap(),
        ])
        .sign_with_keys(&actor)
        .unwrap();
    let (status, deleted) = post(&http, &actor, "/events", &json!(delete)).await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["accepted"], true);
    let tombstone = live_head(&mut reader, &sub, relay, "3").await;
    assert!(tombstone.created_at > edited.created_at);
    assert_eq!(field(&tombstone, "deleted"), Some("true"));
    assert!(field(&tombstone, "name").is_none() && field(&tombstone, "p").is_none());

    let mut query = filter;
    query["consistency"] = json!("strong");
    let (status, heads) = post(&http, &outsider, "/query", &json!([query])).await;
    assert_eq!(status, StatusCode::OK, "{heads}");
    let events: Vec<Event> = serde_json::from_value(heads).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, tombstone.id);
    reader.close_subscription(&sub).await.unwrap();
    reader.disconnect().await.unwrap();
    writer.disconnect().await.unwrap();
}
