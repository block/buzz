//! Postgres-backed tests for generic tag filters the relay matches in SQL
//! before `LIMIT`.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

use super::group_state_postgres_tests::{
    community, create_channel, req_event_ids, state, test_conn,
};
use crate::state::AppState;

/// A kind:7 reaction by `keys` to `address`, created at `at`.
fn reaction(keys: &Keys, address: &str, at: u64, channel: Option<Uuid>) -> Event {
    let mut tags = vec![
        Tag::parse(["a", address]).unwrap(),
        Tag::parse(["k", "30175"]).unwrap(),
    ];
    if let Some(channel) = channel {
        tags.push(Tag::parse(["h", &channel.to_string()]).unwrap());
    }
    EventBuilder::new(Kind::Reaction, "+")
        .tags(tags)
        .custom_created_at(Timestamp::from(at))
        .sign_with_keys(keys)
        .unwrap()
}

/// Run one WebSocket COUNT as `reader` and return the count.
async fn count(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    filter: serde_json::Value,
) -> u64 {
    let (conn, mut rx) = test_conn(tenant, reader);
    let filter: nostr::Filter = serde_json::from_value(filter).expect("filter");
    crate::handlers::count::handle_count("c".into(), vec![filter], conn, state.clone()).await;
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: serde_json::Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("COUNT") => return message[2]["count"].as_u64().expect("count"),
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no COUNT reply");
}

/// A selective `#a` REQ returns the reactions to one address even when they
/// are older than a full page of reactions to other addresses, and COUNT
/// counts exactly them. An empty `#a` matches nothing.
/// Mutation: drop `a_tags` from `filter_to_query_params` → RED (the page holds
/// only the newest reactions to the other address, which the post-filter
/// drops); drop the empty-`a_tags` early return from `count_events_on` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn address_reaction_reqs_are_not_starved_by_limit() {
    let state = state().await;
    let tenant = community(&state, "a-tag-pushdown").await;
    let owner = Keys::generate();
    let reactor = Keys::generate();
    let reader = Keys::generate();
    let needle = format!("30175:{}:needle", owner.public_key().to_hex());
    let hay = format!("30175:{}:hay", owner.public_key().to_hex());
    let now = Timestamp::now().as_secs();

    let mut expected = Vec::new();
    for at in [now - 100, now - 99] {
        let event = reaction(&reactor, &needle, at, None);
        expected.push(event.id.to_hex());
        state
            .db
            .insert_event(tenant.community(), &event, None)
            .await
            .expect("insert needle reaction");
    }
    expected.reverse(); // newest first
    for i in 0..6 {
        state
            .db
            .insert_event(
                tenant.community(),
                &reaction(&reactor, &hay, now - 50 + i, None),
                None,
            )
            .await
            .expect("insert hay reaction");
    }

    let filter = serde_json::json!({"kinds": [7], "#a": [needle], "limit": 2});
    assert_eq!(
        req_event_ids(&state, &tenant, &reader, filter.clone()).await,
        expected,
        "#a address match"
    );
    assert_eq!(count(&state, &tenant, &reader, filter).await, 2, "#a count");
    assert_eq!(
        count(
            &state,
            &tenant,
            &reader,
            serde_json::json!({"kinds": [7], "#a": []})
        )
        .await,
        0,
        "empty #a count"
    );
}

/// `#a` with a multi-value `#h` reads the authorized channel set: the
/// reference fence must not combine with the per-channel read into invalid
/// SQL, and the older match still beats a page of newer reactions.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn address_reaction_reqs_over_several_channels() {
    let state = state().await;
    let tenant = community(&state, "a-tag-channels").await;
    let member = Keys::generate();
    let first = create_channel(&state, &tenant, &member, &[]).await;
    let second = create_channel(&state, &tenant, &member, &[]).await;
    let needle = format!("30175:{}:needle", member.public_key().to_hex());
    let hay = format!("30175:{}:hay", member.public_key().to_hex());
    let now = Timestamp::now().as_secs();

    let event = reaction(&member, &needle, now - 100, Some(first));
    let expected = vec![event.id.to_hex()];
    state
        .db
        .insert_event(tenant.community(), &event, Some(first))
        .await
        .expect("insert needle reaction");
    for (i, channel) in [first, second, first, second].into_iter().enumerate() {
        state
            .db
            .insert_event(
                tenant.community(),
                &reaction(&member, &hay, now - 50 + i as u64, Some(channel)),
                Some(channel),
            )
            .await
            .expect("insert hay reaction");
    }

    let filter = serde_json::json!({
        "kinds": [7],
        "#h": [first.to_string(), second.to_string()],
        "#a": [needle],
        "limit": 1
    });
    assert_eq!(
        req_event_ids(&state, &tenant, &member, filter.clone()).await,
        expected
    );
    assert_eq!(count(&state, &tenant, &member, filter).await, 1);
}

/// POST one filter to the HTTP bridge as `reader` and return the JSON reply.
async fn bridge(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    path: &str,
    filter: &serde_json::Value,
) -> serde_json::Value {
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    let request = Request::post(path)
        .header(header::HOST, tenant.host())
        .header("x-pubkey", reader.public_key().to_hex())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&[filter]).unwrap()))
        .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(status.is_success(), "{path}: {status}");
    serde_json::from_slice(&body).expect("bridge JSON")
}

/// An empty `authors`, `ids` or tag set matches nothing, so COUNT is 0 and
/// REQ is empty on WebSocket and HTTP, even beside a matching `#a`.
/// Mutation: drop the empty-set sentinel from `filter_to_query_params` → RED
/// (each COUNT counts the `#a` match, or every kind:7 without `#a`).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn empty_sets_beside_an_address_match_nothing() {
    let Some(state) = crate::api::bridge::postgres_tests::bridge_handler_test_state().await else {
        panic!("requires local Postgres and Redis");
    };
    let tenant = community(&state, "a-tag-empty-sets").await;
    let reactor = Keys::generate();
    let reader = Keys::generate();
    let needle = format!("30175:{}:needle", reader.public_key().to_hex());
    let event = EventBuilder::new(Kind::Reaction, "+")
        .tags([
            Tag::parse(["a", &needle]).unwrap(),
            Tag::parse(["e", &"cd".repeat(32)]).unwrap(),
            Tag::parse(["p", &reader.public_key().to_hex()]).unwrap(),
            Tag::parse(["k", "30175"]).unwrap(),
        ])
        .sign_with_keys(&reactor)
        .unwrap();
    state
        .db
        .insert_event(tenant.community(), &event, None)
        .await
        .expect("insert reaction");

    let base = serde_json::json!({"kinds": [7], "#a": [needle]});
    assert_eq!(count(&state, &tenant, &reader, base.clone()).await, 1);
    let mut filters: Vec<_> = ["authors", "ids", "#e", "#p", "#d", "#k", "#t", "#A"]
        .into_iter()
        .map(|key| {
            let mut filter = base.clone();
            filter[key] = serde_json::json!([]);
            filter
        })
        .collect();
    filters.push(serde_json::json!({"kinds": [7], "authors": []}));

    for filter in filters {
        let ws_count = count(&state, &tenant, &reader, filter.clone()).await;
        let http_count = bridge(&state, &tenant, &reader, "/count", &filter).await["count"].clone();
        let ws_req = req_event_ids(&state, &tenant, &reader, filter.clone()).await;
        let http_req = bridge(&state, &tenant, &reader, "/query", &filter).await;
        assert_eq!(
            (ws_count, http_count, ws_req.len()),
            (0, serde_json::json!(0), 0),
            "{filter}"
        );
        assert_eq!(http_req, serde_json::json!([]), "{filter} /query");
    }
}
