//! Postgres-backed end-to-end tests for system channels: every read path
//! that does not name a system channel leaves it out, and naming it reads it
//! under the ordinary rules (member, or open channel).

use std::sync::Arc;

use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, edit_channel, http_auth, test_conn};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    host: String,
}

async fn fixture() -> Fixture {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let host = format!("system-channels-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    let tenant = TenantContext::resolved(community, &host);
    Fixture {
        state,
        tenant,
        host,
    }
}

/// Ingest an event that `keys` signs and return its ID.
async fn publish(f: &Fixture, keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> String {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().copied()).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign");
    let id = event.id.to_hex();
    let result = ingest_event(&f.state, &f.tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
    id
}

/// The current relay-signed group-state event ID of `kind` for `channel`.
async fn group_state_id(f: &Fixture, channel: Uuid, kind: i32) -> String {
    f.state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![kind]),
            d_tag: Some(channel.to_string()),
            limit: Some(1),
            ..EventQuery::for_community(f.tenant.community())
        })
        .await
        .expect("query group state")
        .first()
        .unwrap_or_else(|| panic!("no kind:{kind} for {channel}"))
        .event
        .id
        .to_hex()
}

/// Run one historical REQ and return the event IDs sent before EOSE. A
/// CLOSED reply counts as no events.
async fn req(f: &Fixture, reader: &Keys, filter: Value) -> Vec<String> {
    let (conn, mut rx) = test_conn(&f.tenant, reader);
    let filter: nostr::Filter = serde_json::from_value(filter).expect("filter");
    crate::handlers::req::handle_req("s".into(), vec![filter], vec![None], conn, f.state.clone())
        .await;
    let mut ids = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => ids.push(message[2]["id"].as_str().expect("id").to_owned()),
            Some("EOSE") | Some("CLOSED") => return ids,
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no EOSE; events so far: {ids:?}");
}

/// Run one WebSocket COUNT and return the count. A CLOSED reply counts as 0.
async fn count(f: &Fixture, reader: &Keys, filter: Value) -> u64 {
    let (conn, mut rx) = test_conn(&f.tenant, reader);
    let filter: nostr::Filter = serde_json::from_value(filter).expect("filter");
    crate::handlers::count::handle_count("c".into(), vec![filter], conn, f.state.clone()).await;
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("COUNT") => return message[2]["count"].as_u64().expect("count"),
            Some("CLOSED") => return 0,
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no COUNT reply");
}

/// POST `filters` to an HTTP bridge route as `reader`. Returns the JSON body
/// of a 200, or `None` for a refusal.
async fn bridge(f: &Fixture, reader: &Keys, path: &str, filters: Value) -> Option<Value> {
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    let response = crate::router::build_router(f.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::HOST, &f.host)
                .header("x-pubkey", reader.public_key().to_hex())
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&filters).unwrap()))
                .expect("request"),
        )
        .await
        .expect("router");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    if status.is_success() {
        Some(serde_json::from_slice(&body).expect("json body"))
    } else {
        assert!(
            status.is_client_error(),
            "{path}: {status} {}",
            String::from_utf8_lossy(&body)
        );
        None
    }
}

fn bridge_ids(body: Option<Value>) -> Vec<String> {
    let Some(body) = body else {
        return Vec::new();
    };
    let events = body
        .as_array()
        .or_else(|| body["events"].as_array())
        .unwrap_or_else(|| panic!("bridge query body: {body}"));
    events
        .iter()
        .map(|event| event["id"].as_str().expect("id").to_owned())
        .collect()
}

fn bridge_count(body: Option<Value>) -> u64 {
    body.map_or(0, |body| {
        body["count"]
            .as_u64()
            .unwrap_or_else(|| panic!("bridge count body: {body}"))
    })
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// The relay leaves system channels out of every read that does not name
/// them, and a read that names one follows the ordinary rules.
/// Mutation: drop `channel_type <> 'system'` from either half of
/// `get_accessible_channel_ids` → RED (implicit reads list them). Drop the
/// named-channel scope → RED (members cannot read their own system channel).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn system_channels_are_read_only_by_name() {
    let f = fixture().await;
    let owner = Keys::generate();
    let outsider = Keys::generate();

    let private = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let open = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        open,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the system channel");
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        ordinary,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the ordinary channel");

    let word = format!("needle{}", Uuid::new_v4().simple());
    let mut message = std::collections::HashMap::new();
    for channel in [private, open, ordinary] {
        let id = publish(&f, &owner, 9, &word, &[&["h", &channel.to_string()]]).await;
        message.insert(channel, id);
    }
    let ordinary_message = vec![message[&ordinary].clone()];

    for reader in [&owner, &outsider] {
        let who = if std::ptr::eq(reader, &owner) {
            "member"
        } else {
            "outsider"
        };

        // Implicit scope: no channel named.
        assert_eq!(
            req(&f, reader, json!({"kinds": [9]})).await,
            ordinary_message,
            "{who}: REQ without #h"
        );
        assert_eq!(
            req(&f, reader, json!({"kinds": [9], "search": word})).await,
            ordinary_message,
            "{who}: NIP-50 search"
        );
        assert_eq!(
            count(&f, reader, json!({"kinds": [9]})).await,
            1,
            "{who}: COUNT"
        );
        assert_eq!(
            bridge_ids(bridge(&f, reader, "/query", json!([{"kinds": [9]}])).await),
            ordinary_message,
            "{who}: bridge /query"
        );
        assert_eq!(
            bridge_ids(
                bridge(
                    &f,
                    reader,
                    "/query",
                    json!([{"kinds": [9], "search": word}])
                )
                .await
            ),
            ordinary_message,
            "{who}: bridge search"
        );
        assert_eq!(
            bridge_count(bridge(&f, reader, "/count", json!([{"kinds": [9]}])).await),
            1,
            "{who}: bridge /count"
        );
        assert_eq!(
            req(&f, reader, json!({"kinds": [39000]})).await,
            vec![group_state_id(&f, ordinary, 39000).await],
            "{who}: channel list"
        );

        // Named scope: member, or the channel is open.
        for (channel, readable) in [(private, std::ptr::eq(reader, &owner)), (open, true)] {
            let h = channel.to_string();
            let expected_message = if readable {
                vec![message[&channel].clone()]
            } else {
                Vec::new()
            };
            let expected_count = u64::from(readable);
            assert_eq!(
                req(&f, reader, json!({"kinds": [9], "#h": [h]})).await,
                expected_message,
                "{who}: REQ #h {channel}"
            );
            assert_eq!(
                count(&f, reader, json!({"kinds": [9], "#h": [h]})).await,
                expected_count,
                "{who}: COUNT #h {channel}"
            );
            assert_eq!(
                bridge_ids(bridge(&f, reader, "/query", json!([{"kinds": [9], "#h": [h]}])).await),
                expected_message,
                "{who}: bridge /query #h {channel}"
            );
            assert_eq!(
                bridge_count(
                    bridge(&f, reader, "/count", json!([{"kinds": [9], "#h": [h]}])).await
                ),
                expected_count,
                "{who}: bridge /count #h {channel}"
            );
            assert_eq!(
                req(&f, reader, json!({"kinds": [9], "#h": [h], "search": word})).await,
                expected_message,
                "{who}: search #h {channel}"
            );
            let expected_metadata = if readable {
                vec![group_state_id(&f, channel, 39000).await]
            } else {
                Vec::new()
            };
            assert_eq!(
                req(&f, reader, json!({"kinds": [39000], "#d": [h]})).await,
                expected_metadata,
                "{who}: metadata #d {channel}"
            );
        }
    }

    // The member's own channel list (kind:39002 #p:[me]) leaves them out.
    assert_eq!(
        req(
            &f,
            &owner,
            json!({"kinds": [39002], "#p": [owner.public_key().to_hex()]})
        )
        .await,
        vec![group_state_id(&f, ordinary, 39002).await],
        "member list by #p"
    );
}

/// Adding a member to a system channel sends no kind:44100 notification,
/// which an existing client would show as a new channel. An ordinary
/// channel still sends one, so the check is not vacuous.
/// Mutation: drop the system check in `emit_membership_notification` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn system_channels_send_no_membership_notifications() {
    let f = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let member_hex = member.public_key().to_hex();

    let system = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    for channel in [system, ordinary] {
        publish(
            &f,
            &owner,
            9000,
            "",
            &[&["h", &channel.to_string()], &["p", &member_hex]],
        )
        .await;
    }
    // Notifications are written after the reply; give the task a moment.
    let mut notified = Vec::new();
    for _ in 0..50 {
        notified = f
            .state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION as i32]),
                p_tag_hex: Some(member_hex.clone()),
                ..EventQuery::for_community(f.tenant.community())
            })
            .await
            .expect("query notifications")
            .into_iter()
            .map(|stored| {
                stored
                    .event
                    .tags
                    .iter()
                    .find(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
                    .and_then(|tag| tag.as_slice().get(1).cloned())
                    .expect("h tag")
            })
            .collect::<Vec<_>>();
        if !notified.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(sorted(notified), vec![ordinary.to_string()]);
}

/// A group-state filter with `#t:["system"]` or any `#P` finds the system
/// channels the reader can read, and still matches as usual: a creator plus
/// a label finds that creator's one labeled channel.
/// Mutation: make `filter_opts_in` return false → RED (discovery finds
/// nothing). Drop the readability check from the opt-in → RED (an outsider
/// finds the private channel).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn system_channels_are_found_by_type_or_creator() {
    let f = fixture().await;
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let label = "agent-attention";

    let private = create_channel(
        &f.state,
        &f.tenant,
        &owner,
        &[&["channel_type", "system"], &["t", label]],
    )
    .await;
    let open = create_channel(&f.state, &f.tenant, &owner, &[&["channel_type", "system"]]).await;
    edit_channel(
        &f.state,
        &f.tenant,
        &owner,
        open,
        &[&["visibility", "open"]],
    )
    .await
    .expect("open the system channel");
    let ordinary = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    // Another creator's channel with the same label: #P must exclude it.
    let lookalike = create_channel(&f.state, &f.tenant, &outsider, &[&["t", label]]).await;

    let meta = |channel| group_state_id(&f, channel, 39000);
    let (private_meta, open_meta, ordinary_meta, lookalike_meta) = (
        meta(private).await,
        meta(open).await,
        meta(ordinary).await,
        meta(lookalike).await,
    );

    // #t:["system"]: members find both; outsiders find only the open one.
    assert_eq!(
        sorted(req(&f, &owner, json!({"kinds": [39000], "#t": ["system"]})).await),
        sorted(vec![private_meta.clone(), open_meta.clone()]),
        "member #t:system"
    );
    assert_eq!(
        req(&f, &outsider, json!({"kinds": [39000], "#t": ["system"]})).await,
        vec![open_meta.clone()],
        "outsider #t:system"
    );
    // A member lists its own system channels from kind:39002.
    assert_eq!(
        sorted(
            req(
                &f,
                &owner,
                json!({"kinds": [39002], "#p": [owner_hex], "#t": ["system"]})
            )
            .await
        ),
        sorted(vec![
            group_state_id(&f, private, 39002).await,
            group_state_id(&f, open, 39002).await,
        ]),
        "member 39002 #p:[me] #t:system"
    );
    // #P opts in, so a creator's channels include its system channels.
    assert_eq!(
        sorted(req(&f, &owner, json!({"kinds": [39000], "#P": [owner_hex]})).await),
        sorted(vec![private_meta.clone(), open_meta.clone(), ordinary_meta]),
        "member #P"
    );
    // Creator plus label finds exactly the one channel, in SQL.
    let lookup = json!({"kinds": [39000], "#P": [owner_hex], "#t": [label], "limit": 1});
    assert_eq!(
        req(&f, &owner, lookup.clone()).await,
        vec![private_meta.clone()],
        "creator + label"
    );
    assert!(
        req(&f, &outsider, lookup.clone()).await.is_empty(),
        "an outsider cannot find a private system channel"
    );
    // A label alone is not an opt-in.
    assert_eq!(
        req(&f, &owner, json!({"kinds": [39000], "#t": [label]})).await,
        vec![lookalike_meta],
        "label alone"
    );
    // The same opt-in on COUNT and the HTTP bridge.
    assert_eq!(
        count(&f, &owner, json!({"kinds": [39000], "#t": ["system"]})).await,
        2,
        "COUNT #t:system"
    );
    assert_eq!(
        bridge_ids(bridge(&f, &owner, "/query", json!([lookup])).await),
        vec![private_meta],
        "bridge creator + label"
    );
    assert_eq!(
        bridge_count(
            bridge(
                &f,
                &outsider,
                "/count",
                json!([{"kinds": [39000], "#t": ["system"]}])
            )
            .await
        ),
        1,
        "bridge /count outsider #t:system"
    );
}
