//! A failed recovery must stop the actual selective HTTP publisher, not merely
//! fail a helper after stale 30177 rows have already gone out.
use super::*;
use crate::app_state::build_app_state;
use crate::managed_agents::retention::{
    active_retention_scope, open_retention_db, retain_event, RetainedEvent,
};
use nostr::JsonUtil;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn selective_flush_retries_atomic_deletion_before_any_http_publication() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let received = Arc::new(Mutex::new(Vec::<nostr::Event>::new()));
    let events = received.clone();
    let router = axum::Router::new().route(
        "/events",
        axum::routing::post(move |body: String| {
            let events = events.clone();
            async move {
                let event = nostr::Event::from_json(body).unwrap();
                event.verify().unwrap();
                let id = event.id.to_hex();
                events.lock().unwrap().push(event);
                axum::Json(serde_json::json!({"event_id": id, "accepted": true, "message": ""}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = dir.path().to_string_lossy().into_owned();
    let state = build_app_state();
    state
        .agent_device_policy
        .set(Ok(DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        }))
        .unwrap();
    *state.relay_url_override.lock().unwrap() = Some(url);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(context)
        .unwrap();
    assert_eq!(app.path().app_data_dir().unwrap(), dir.path());
    let scope = active_retention_scope(app.handle(), &app.state()).unwrap();
    let agent = nostr::Keys::generate().public_key().to_hex();
    // Future-dated but within the existing relay publication window (900s).
    let future = nostr::Timestamp::now().as_secs() + 300;
    let head = nostr::EventBuilder::new(nostr::Kind::Custom(30177),
        r#"{"name":"Deleted local agent","persona_id":"deleted-persona","parallelism":1,"respond_to":"owner-only"}"#)
        .tags([nostr::Tag::parse(["d", &agent]).unwrap()])
        .custom_created_at(nostr::Timestamp::from_secs(future))
        .sign_with_keys(&scope.owner_keys).unwrap();
    let conn = open_retention_db(&scope.db_path).unwrap();
    sync::register(&conn, &agent).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: 30177,
            pubkey: scope.owner_keys.public_key().to_hex(),
            d_tag: agent,
            content: head.content.clone(),
            created_at: future as i64,
            raw_event: head.as_json(),
            pending_sync: true,
        },
    )
    .unwrap();
    // A valid empty persisted store, not a missing store, witnesses removal.
    let store = super::super::managed_agents_base_dir(app.handle())
        .unwrap()
        .join("managed-agents.json");
    std::fs::write(store, "[]").unwrap();
    conn.execute_batch("CREATE TRIGGER deny_archive BEFORE INSERT ON persona_events WHEN NEW.kind = 9035 BEGIN SELECT RAISE(ABORT, 'archive blocked'); END;").unwrap();
    let first =
        super::super::persona_events::flush_active_pending_events(app.handle(), &app.state()).await;
    assert!(first.unwrap_err().contains("archive blocked"));
    assert!(received.lock().unwrap().is_empty());
    conn.execute_batch("DROP TRIGGER deny_archive").unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        super::super::persona_events::flush_active_pending_events(app.handle(), &app.state()),
    )
    .await
    .unwrap();
    let again =
        super::super::persona_events::flush_active_pending_events(app.handle(), &app.state()).await;
    server.abort();
    assert_eq!(result, Ok(2));
    assert_eq!(again, Ok(0));
    let received = received.lock().unwrap();
    let mut kinds: Vec<_> = received.iter().map(|e| e.kind.as_u16()).collect();
    kinds.sort();
    assert_eq!(kinds, vec![5, 9035]);
    assert!(
        received
            .iter()
            .find(|e| e.kind.as_u16() == 5)
            .unwrap()
            .created_at
            .as_secs()
            > future
    );
    assert!(received
        .iter()
        .find(|e| e.kind.as_u16() == 9035)
        .unwrap()
        .content
        .contains("deleted-persona"));
}
