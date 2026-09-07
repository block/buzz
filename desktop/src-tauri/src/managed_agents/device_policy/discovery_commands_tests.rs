//! Exercise actual IPC command bodies, with an isolated disk and loopback relay.
use super::*;
use crate::app_state::build_app_state;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
async fn invalid_policy_does_not_disable_people_or_relay_agent_commands() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let queries = Arc::new(AtomicUsize::new(0));
    let count = queries.clone();
    let relay_key = nostr::Keys::generate().public_key().to_hex();
    let server = axum::Router::new()
        .route(
            "/",
            axum::routing::get(move || async move {
                axum::Json(serde_json::json!({"self": relay_key}))
            }),
        )
        .route(
            "/query",
            axum::routing::post(move |headers: axum::http::HeaderMap| {
                count.fetch_add(1, Ordering::SeqCst);
                assert!(headers
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("Nostr "));
                async { axum::Json(serde_json::json!([])) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, server).await.unwrap();
    });
    let state = build_app_state();
    state
        .agent_device_policy
        .set(Err("malformed device policy".into()))
        .unwrap();
    *state.relay_url_override.lock().unwrap() = Some(url);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let people = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::commands::search_users(
            "Scout".into(),
            Some(8),
            None,
            app.state(),
            app.handle().clone(),
        ),
    )
    .await
    .unwrap();
    let agents = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::commands::list_relay_agents(app.state(), app.handle().clone()),
    )
    .await
    .unwrap();
    task.abort();
    assert!(people.unwrap().users.is_empty());
    assert!(agents.unwrap().is_empty());
    assert!(
        queries.load(Ordering::SeqCst) >= 3,
        "both commands must reach the authenticated relay"
    );
    assert_eq!(
        require_hosting(app.handle()),
        Err("malformed device policy".into())
    );
}

#[tokio::test]
async fn invalid_policy_lists_personas_inactive_without_persisting_activation() {
    let dir = tempfile::tempdir().unwrap();
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    // Path::join preserves this absolute test path on every desktop platform.
    // Do not override HOME or touch the real app's data/keyring.
    context.config_mut().identifier = dir.path().to_string_lossy().into_owned();
    let state = build_app_state();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(context)
        .unwrap();
    assert_eq!(app.path().app_data_dir().unwrap(), dir.path());
    let base = super::super::managed_agents_base_dir(app.handle()).unwrap();
    std::fs::write(base.join("agent-device-policy.json"), "broken").unwrap();
    let record: super::super::ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "", "slug": "remote-definition", "name": "Scout", "is_active": true,
        "relay_url": "", "acp_command": "buzz-acp", "agent_command": "goose", "agent_args": [],
        "mcp_command": "", "turn_timeout_seconds": 320,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    let store = base.join("managed-agents.json");
    std::fs::write(&store, serde_json::to_vec(&[record]).unwrap()).unwrap();
    let personas = crate::commands::list_personas(app.handle().clone())
        .await
        .unwrap();
    assert!(
        !personas
            .iter()
            .find(|p| p.id == "remote-definition")
            .unwrap()
            .is_active
    );
    let persisted: Vec<super::super::ManagedAgentRecord> =
        serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
    assert!(
        persisted
            .iter()
            .find(|p| p.slug.as_deref() == Some("remote-definition"))
            .unwrap()
            .is_active
    );
    let status = get_agent_device_policy(app.handle().clone()).unwrap();
    assert!(status
        .load_error
        .unwrap()
        .contains("Invalid agent device policy"));
    assert!(status.active_client_only);
    assert!(require_hosting(app.handle()).is_err());
}
