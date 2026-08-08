use super::*;
use crate::{
    app_state::build_app_state,
    commands::teams::pending::prepare_team_publication_at,
    managed_agents::{
        retention::{get_retained_event, open_retention_db, RetentionScope},
        AgentDefinition,
    },
};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn member(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: "One".to_string(),
        avatar_url: None,
        system_prompt: "Do the work.".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn team() -> TeamRecord {
    TeamRecord {
        id: "team-abc".to_string(),
        name: "Catalog Team".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec!["m1".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: Some(PathBuf::from("/local/only/path")),
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

async fn spawn_relay(accepted: bool) -> String {
    use axum::{routing::post, Router};

    let app = Router::new().route(
        "/events",
        post(move |body: String| async move {
            let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            serde_json::json!({
                "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                "accepted": accepted,
                "message": if accepted { "" } else { "policy rejection" }
            })
            .to_string()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

fn prepared(
    db_path: &std::path::Path,
    relay_url: String,
    keys: nostr::Keys,
    shared: bool,
) -> PreparedTeamPublication {
    let (event, retained, team) =
        prepare_team_publication_at(db_path, &keys, &team(), &[member("m1")], Some(shared))
            .unwrap();
    PreparedTeamPublication {
        scope: RetentionScope {
            db_path: db_path.to_path_buf(),
            relay_url,
            owner_keys: keys,
        },
        event,
        retained,
        team,
    }
}

fn retained_head(
    db_path: &std::path::Path,
    owner: &str,
) -> crate::managed_agents::retention::RetainedEvent {
    get_retained_event(
        &open_retention_db(db_path).unwrap(),
        KIND_TEAM_CATALOG,
        owner,
        "team-abc",
    )
    .unwrap()
    .expect("the head is retained before the relay is ever contacted")
}

#[tokio::test]
async fn test_accepted_share_reports_published_and_clears_the_pending_flag() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let prepared = prepared(&db_path, spawn_relay(true).await, keys, true);
    let state = build_app_state();

    let result = publish_prepared_team(&state, prepared).await.unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Published
    );
    assert!(result.relay_message.is_none());
    assert!(result.team.shared);
    assert!(
        !retained_head(&db_path, &owner).pending_sync,
        "a confirmed publish must not be republished by the flush loop"
    );
}

#[tokio::test]
async fn test_relay_rejection_stays_durably_queued() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let prepared = prepared(&db_path, spawn_relay(false).await, keys, true);
    let state = build_app_state();

    let result = publish_prepared_team(&state, prepared).await.unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Queued
    );
    assert!(result
        .relay_message
        .as_deref()
        .is_some_and(|message| message.contains("relay rejected event")));
    assert!(retained_head(&db_path, &owner).pending_sync);
}

#[tokio::test]
async fn test_unavailable_relay_stays_durably_queued() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let prepared = prepared(&db_path, relay_url, keys, true);
    let state = build_app_state();

    let result = publish_prepared_team(&state, prepared).await.unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Queued
    );
    assert!(
        retained_head(&db_path, &owner).pending_sync,
        "an offline share must survive for the flush loop rather than failing the command"
    );
}

#[tokio::test]
async fn test_unshare_leaves_an_untagged_head_retained_after_publication() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let relay_url = spawn_relay(true).await;
    let state = build_app_state();
    publish_prepared_team(
        &state,
        prepared(&db_path, relay_url.clone(), keys.clone(), true),
    )
    .await
    .unwrap();

    let result = publish_prepared_team(&state, prepared(&db_path, relay_url, keys, false))
        .await
        .unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Published
    );
    assert!(!result.team.shared);
    let row = retained_head(&db_path, &owner);
    assert!(
        !buzz_core_pkg::kind::event_is_shared(
            &<nostr::Event as nostr::JsonUtil>::from_json(&row.raw_event).unwrap()
        ),
        "unshare retracts by replacement, so the coordinate stays readable by its author"
    );
}
