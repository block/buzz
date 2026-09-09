//! Command-path regression: `set_persona_shared` publish-retry team-refresh.
//!
//! Both variants call `publish_and_refresh_teams_at` — the extracted command
//! core that production `set_persona_shared` delegates to. They cover:
//! - Pre-fold: persona definitions in `personas.json` (legacy path).
//! - Post-fold: definitions as keyless records in `managed-agents.json`; no
//!   `personas.json` present (the normal state after Phase 1A.2 fold).
//!
//! Deletion mutations:
//! - Removing the `refresh_for_persona_at` call from `publish_and_refresh_teams_at`
//!   turns BOTH tests RED; restoring returns GREEN.
//! - Reverting the loader to single-store (`personas.json` only) turns only the
//!   post-fold test RED, proving the dual-store fix is load-bearing.

use super::{member, prepare_team_publication_at, scoped_db, team_with_members, write_stores};
use crate::managed_agents::retention::{get_retained_event, open_retention_db};
use crate::managed_agents::{AgentDefinition, TeamRecord};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;

fn post_fold_write_stores(
    base_dir: &std::path::Path,
    teams: &[TeamRecord],
    personas: &[AgentDefinition],
) {
    std::fs::write(
        base_dir.join("teams.json"),
        serde_json::to_string(teams).unwrap(),
    )
    .unwrap();
    // Post-fold: definitions live as keyless records in managed-agents.json.
    // personas.json is absent (retired by the fold migration).
    let records: Vec<crate::managed_agents::ManagedAgentRecord> = personas
        .iter()
        .cloned()
        .map(|p| p.into_agent_record())
        .collect();
    std::fs::write(
        base_dir.join("managed-agents.json"),
        serde_json::to_string(&records).unwrap(),
    )
    .unwrap();
    assert!(
        !base_dir.join("personas.json").exists(),
        "post-fold fixture must not have personas.json"
    );
}

async fn run_publish_and_refresh_for_team(
    dir: &std::path::Path,
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    persona_def: &AgentDefinition,
    persona_id: &str,
) {
    let (event, retained, persona) = crate::commands::personas::prepare_persona_publication_at(
        db_path,
        keys,
        persona_def,
        Some(true),
    )
    .unwrap();
    let prepared = crate::commands::personas::PreparedPersonaPublication {
        scope: crate::managed_agents::retention::RetentionScope {
            db_path: db_path.to_path_buf(),
            relay_url: "http://127.0.0.1:1".to_string(),
            owner_keys: keys.clone(),
        },
        event,
        retained,
        persona,
    };
    let state = crate::app_state::build_app_state();
    crate::commands::personas::publish_and_refresh_teams_at(
        &state, prepared, dir, keys, db_path, persona_id,
    )
    .await
    .unwrap();
}

/// Pre-fold variant: definitions in `personas.json`.
#[tokio::test]
async fn test_set_persona_shared_publish_retry_refreshes_shared_team_head_pre_fold() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    let m1_before = member("m1", "Original prompt.");
    let t = team_with_members("team-retry", "Retry Team", vec!["m1".to_string()]);
    prepare_team_publication_at(
        &db_path,
        &keys,
        &t,
        std::slice::from_ref(&m1_before),
        Some(true),
    )
    .unwrap();
    let head_before = {
        let conn = open_retention_db(&db_path).unwrap();
        get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-retry")
            .unwrap()
            .expect("shared head must exist before retry")
    };
    assert!(head_before.content.contains("Original prompt."));

    let m1_after = member("m1", "Updated prompt after publish-retry.");
    write_stores(
        dir.path(),
        std::slice::from_ref(&t),
        std::slice::from_ref(&m1_after),
    );
    run_publish_and_refresh_for_team(
        dir.path(),
        &db_path,
        &keys,
        &member("m1", "Updated prompt after publish-retry."),
        "m1",
    )
    .await;

    let head_after = {
        let conn = open_retention_db(&db_path).unwrap();
        get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-retry")
            .unwrap()
            .expect("shared head must still exist after retry-path refresh")
    };
    assert!(
        head_after
            .content
            .contains("Updated prompt after publish-retry."),
        "team 30178 must reflect updated persona content (pre-fold)"
    );
    assert!(head_after.pending_sync, "refreshed head must be queued");
    assert!(head_after.created_at >= head_before.created_at);
}

/// Post-fold variant: definitions as keyless records in `managed-agents.json`;
/// `personas.json` absent. Reverting the loader to `personas.json` only turns
/// this test RED (zero definitions → retraction instead of refresh).
#[tokio::test]
async fn test_set_persona_shared_publish_retry_refreshes_shared_team_head_post_fold() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    let m1_before = member("m1", "Original prompt.");
    let t = team_with_members("team-retry", "Retry Team", vec!["m1".to_string()]);
    prepare_team_publication_at(
        &db_path,
        &keys,
        &t,
        std::slice::from_ref(&m1_before),
        Some(true),
    )
    .unwrap();
    let head_before = {
        let conn = open_retention_db(&db_path).unwrap();
        get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-retry")
            .unwrap()
            .expect("shared head must exist before retry")
    };
    assert!(head_before.content.contains("Original prompt."));

    let m1_after = member("m1", "Updated prompt after publish-retry.");
    post_fold_write_stores(
        dir.path(),
        std::slice::from_ref(&t),
        std::slice::from_ref(&m1_after),
    );
    run_publish_and_refresh_for_team(
        dir.path(),
        &db_path,
        &keys,
        &member("m1", "Updated prompt after publish-retry."),
        "m1",
    )
    .await;

    // The 30178 team head must reflect the updated prompt — not be retracted.
    let head_after = {
        let conn = open_retention_db(&db_path).unwrap();
        get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-retry")
            .unwrap()
            .expect("shared head must exist after post-fold retry — not retracted")
    };
    assert!(
        head_after
            .content
            .contains("Updated prompt after publish-retry."),
        "team 30178 must reflect updated persona content (post-fold, managed-agents.json)"
    );
    assert!(head_after.pending_sync, "refreshed head must be queued");
    assert!(head_after.created_at >= head_before.created_at);
}
