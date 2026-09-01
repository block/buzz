//! Search visibility must not depend on the wake's physical FTS projection.
use super::*;
use buzz_core::{kind::KIND_WORKFLOW_MENTION_WAKE, workflow_wake::WorkflowMentionWake};
use serde_json::{json, Value};

async fn assert_search(f: &Fixture, filter: Value, expected: &[&str]) {
    let body = axum::body::Bytes::from(serde_json::to_vec(&json!([filter])).expect("body"));
    let response =
        crate::api::bridge::query_events(State(f.state.clone()), f.headers(), body.clone())
            .await
            .expect("HTTP search");
    let mut http_ids: Vec<&str> = response
        .0
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["id"].as_str().expect("event id"))
        .collect();
    http_ids.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(
        http_ids, expected,
        "HTTP returns authorized events, not raw hits"
    );

    let (conn, mut frames) = f.connection();
    let filters = serde_json::from_slice(&body).expect("filters");
    crate::handlers::req::handle_req("search".into(), filters, conn.clone(), f.state.clone()).await;
    let mut ws_ids = Vec::new();
    loop {
        let frame = next_frame(&mut frames);
        if frame[0] == "EOSE" {
            break;
        }
        assert_eq!(frame[0], "EVENT", "unexpected search frame: {frame}");
        ws_ids.push(frame[2]["id"].as_str().expect("event id").to_owned());
    }
    ws_ids.sort_unstable();
    assert_eq!(ws_ids, expected, "WS uses the same visibility boundary");
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn indexed_wakes_are_filtered_by_actual_http_and_ws_search_boundaries() {
    let f = Fixture::new().await;
    let revision = f.revision(Timestamp::now().as_secs()).await;
    let run = f
        .state
        .db
        .create_workflow_run(
            f.community,
            f.workflow,
            Some(revision.id.as_bytes()),
            None,
            None,
        )
        .await
        .expect("run");
    let message = RelayActionSink::new(&f.state)
        .send_message(
            WorkflowMessageContext {
                community_id: f.community,
                run_id: run,
                step_id: "notify".into(),
                definition_event_id: Some(revision.id.as_bytes().to_vec()),
            },
            &f.channel.to_string(),
            "@Worker searchneedle",
            "@Worker searchneedle",
            &f.owner.public_key().to_hex(),
            None,
        )
        .await
        .expect("ordinary sink stores message and wake");
    let filter = json!({"kinds":[KIND_WORKFLOW_MENTION_WAKE], "#p":[f.agent.public_key().to_hex()], "#h":[f.channel.to_string()]});
    let body = axum::body::Bytes::from(serde_json::to_vec(&json!([filter])).expect("body"));
    let response = crate::api::bridge::query_events(State(f.state.clone()), f.headers(), body)
        .await
        .expect("ordinary wake replay");
    assert_eq!(response.0.as_array().expect("events").len(), 1);
    let wake: Event = serde_json::from_value(response.0[0].clone()).expect("canonical wake");
    let other = WorkflowMentionWake::new(
        f.owner.public_key(),
        f.channel,
        run,
        revision.id,
        nostr::EventId::from_hex(&message).expect("message id"),
    )
    .sign(&f.state.relay_keypair)
    .expect("other recipient");
    // Represent arbitrary legacy/malformed storage without relaxing ingress.
    // Reuse canonical tags but sign nonempty content; parse must reject it.
    let malformed = EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_MENTION_WAKE as u16),
        "searchneedle",
    )
    .tags(wake.tags.clone())
    .sign_with_keys(&f.state.relay_keypair)
    .expect("malformed wake");
    for event in [&other, &malformed] {
        f.state
            .db
            .insert_event(f.community, event, Some(f.channel))
            .await
            .expect("legacy row");
    }
    // Desired-state's existing broad policy really indexes these rows. This
    // assertion prevents the authorization test from passing vacuously via NULL.
    let candidates = f
        .state
        .search
        .search(&buzz_search::SearchQuery {
            community: f.community,
            q: "-neverpresentqzx".into(),
            channel_scope: buzz_search::ChannelScope::Channels(vec![f.channel]),
            kinds: Some(vec![KIND_WORKFLOW_MENTION_WAKE as i32]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 100,
            mode: buzz_search::SearchMode::FullText,
        })
        .await
        .expect("raw NOT-only candidate search");
    for event in [&wake, &other, &malformed] {
        assert!(
            candidates
                .hits
                .iter()
                .any(|hit| hit.event_id == event.id.to_bytes()),
            "fixture must expose even the empty wake to candidate search"
        );
    }
    let wake_id = wake.id.to_hex();
    // IDs bypass the filter-level p gate; result-level recipient + shape gates
    // must still reject the other recipient and the malformed row.
    let ids = json!([message, wake_id, other.id.to_hex(), malformed.id.to_hex()]);
    let broad = json!({"ids":ids, "search":"-neverpresentqzx", "#h":[f.channel.to_string()]});
    assert_search(&f, broad.clone(), &[&message, &wake_id]).await;
    assert_search(
        &f,
        json!({"ids":ids, "search":"searchneedle", "#h":[f.channel.to_string()]}),
        &[&message],
    )
    .await;
    assert_search(&f, json!({"kinds":[9,40002,45001,45003], "search":"-neverpresentqzx", "#h":[f.channel.to_string()]}), &[&message]).await;
    assert_search(&f, json!({"kinds":[KIND_WORKFLOW_MENTION_WAKE], "#p":[f.agent.public_key().to_hex()], "search":"-neverpresentqzx"}), &[&wake_id]).await;

    let unauthorized = axum::body::Bytes::from(serde_json::to_vec(&json!([{
        "kinds":[KIND_WORKFLOW_MENTION_WAKE], "#p":[f.owner.public_key().to_hex()], "search":"-neverpresentqzx"
    }])).expect("body"));
    assert_eq!(
        crate::api::bridge::query_events(State(f.state.clone()), f.headers(), unauthorized.clone())
            .await
            .expect_err("foreign recipient filter")
            .0,
        StatusCode::FORBIDDEN
    );
    let (conn, mut frames) = f.connection();
    crate::handlers::req::handle_req(
        "denied".into(),
        serde_json::from_slice(&unauthorized).expect("filters"),
        conn.clone(),
        f.state.clone(),
    )
    .await;
    assert_eq!(next_frame(&mut frames)[0], "CLOSED");

    f.state
        .db
        .remove_member(
            f.community,
            f.channel,
            &f.agent.public_key().to_bytes(),
            &f.owner.public_key().to_bytes(),
        )
        .await
        .expect("remove member");
    // The open channel remains readable; the public control must still return,
    // while wake membership is checked from DB rather than the channel cache.
    assert_search(&f, broad, &[&message]).await;
}
