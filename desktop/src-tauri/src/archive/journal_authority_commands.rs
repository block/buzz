//! Tauri command boundary for owner-authorized Activity Ledger artifacts.

use tauri::State;

use super::journal_authority::{
    self, JournalAuthorityArtifact, JournalVerificationInput, OwnerJournalOverrideInput,
};
use super::today_snapshot::{self, TodaySnapshotReceipt};
use super::{identity_pubkey, now_secs, observer_revision, observer_time, store};
use crate::app_state::AppState;
use crate::managed_agents::nest_dir;
use crate::relay::relay_ws_url_with_override;

// ── Activity Ledger owner authority ─────────────────────────────────────────

fn active_relay_scope(state: &AppState, requested_relay_url: &str) -> Result<String, String> {
    let active = journal_authority::normalize_relay_scope(&relay_ws_url_with_override(state))?;
    let requested = journal_authority::normalize_relay_scope(requested_relay_url)?;
    if requested != active {
        return Err("journal authority request relay is not the active relay".into());
    }
    Ok(active)
}

/// Persist an owner-authenticated journal summary override. The backend signs
/// the artifact with the active identity; callers never receive key material.
#[tauri::command]
pub async fn upsert_owner_journal_override(
    state: State<'_, AppState>,
    relay_url: String,
    input: OwnerJournalOverrideInput,
) -> Result<JournalAuthorityArtifact, String> {
    let keys = state.signing_keys()?;
    let identity_pk = keys.public_key().to_hex();
    let relay_url = active_relay_scope(&state, &relay_url)?;
    let agent_pubkey = journal_authority::normalize_agent_scope(&input.agent_pubkey)?;
    let now = now_secs();
    state
        .archive_db
        .with_conn(move |conn| {
            let revision = journal_authority::next_revision(
                conn,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
                input.journal_id.trim(),
                journal_authority::JournalAuthorityArtifactType::OwnerOverride,
            )?;
            let raw =
                journal_authority::build_owner_override_event(&keys, &relay_url, &input, revision)?;
            journal_authority::upsert_signed_artifact(
                conn,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
                &raw,
                now,
            )
        })
        .await
}

/// Persist an independent owner verification. It cannot be created without a
/// receipt reference and one or more source observer event IDs that are
/// currently present and signature-valid in this owner's archive.
#[tauri::command]
pub async fn upsert_journal_verification(
    state: State<'_, AppState>,
    relay_url: String,
    input: JournalVerificationInput,
) -> Result<JournalAuthorityArtifact, String> {
    let keys = state.signing_keys()?;
    let identity_pk = keys.public_key().to_hex();
    let relay_url = active_relay_scope(&state, &relay_url)?;
    let agent_pubkey = journal_authority::normalize_agent_scope(&input.agent_pubkey)?;
    let now = now_secs();
    state
        .archive_db
        .with_conn(move |conn| {
            let revision = journal_authority::next_revision(
                conn,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
                input.journal_id.trim(),
                journal_authority::JournalAuthorityArtifactType::Verification,
            )?;
            let raw =
                journal_authority::build_verification_event(&keys, &relay_url, &input, revision)?;
            let artifact = journal_authority::validate_signed_artifact(
                &raw,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
            )?;
            journal_authority::validate_archived_verification_sources(conn, &keys, &artifact)?;
            journal_authority::upsert_signed_artifact(
                conn,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
                &raw,
                now,
            )
        })
        .await
}

/// Read the current owner override and/or verification for one journal. Every
/// signed artifact and every verification source is revalidated fail-closed.
#[tauri::command]
pub async fn get_journal_authority_artifacts(
    state: State<'_, AppState>,
    relay_url: String,
    agent_pubkey: String,
    journal_id: String,
) -> Result<Vec<JournalAuthorityArtifact>, String> {
    let keys = state.signing_keys()?;
    let identity_pk = keys.public_key().to_hex();
    let relay_url = active_relay_scope(&state, &relay_url)?;
    let agent_pubkey = journal_authority::normalize_agent_scope(&agent_pubkey)?;
    let journal_id = journal_id.trim().to_owned();
    state
        .archive_db
        .with_conn(move |conn| {
            let artifacts = journal_authority::get_journal_authority_artifacts(
                conn,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
                &journal_id,
            )?;
            for artifact in &artifacts {
                journal_authority::validate_archived_verification_sources(conn, &keys, artifact)?;
            }
            Ok(artifacts)
        })
        .await
}

/// Bounded owner-only authority query used by Today surfaces and local
/// read-only consumers. No signing or secret key data is returned.
#[tauri::command]
pub async fn query_journal_authority_artifacts(
    state: State<'_, AppState>,
    relay_url: String,
    agent_pubkey: String,
    start_created_at: i64,
    end_created_at: i64,
    limit: Option<i64>,
) -> Result<Vec<JournalAuthorityArtifact>, String> {
    let keys = state.signing_keys()?;
    let identity_pk = keys.public_key().to_hex();
    let relay_url = active_relay_scope(&state, &relay_url)?;
    let agent_pubkey = journal_authority::normalize_agent_scope(&agent_pubkey)?;
    state
        .archive_db
        .with_conn(move |conn| {
            let artifacts = journal_authority::query_journal_authority_artifacts(
                conn,
                &identity_pk,
                &relay_url,
                &agent_pubkey,
                start_created_at,
                end_created_at,
                limit.unwrap_or(200),
            )?;
            for artifact in &artifacts {
                journal_authority::validate_archived_verification_sources(conn, &keys, artifact)?;
            }
            Ok(artifacts)
        })
        .await
}

fn validate_snapshot_archive_fence(
    snapshot_json: &str,
    current_revision: i64,
    current_unindexed: i64,
) -> Result<(), String> {
    let snapshot: serde_json::Value = serde_json::from_str(snapshot_json)
        .map_err(|error| format!("invalid Today snapshot JSON: {error}"))?;
    let projection = snapshot
        .pointer("/surface/snapshotProjection")
        .and_then(serde_json::Value::as_object)
        .ok_or("Today snapshot must include snapshotProjection")?;
    let declared_revision = projection
        .get("archiveRevision")
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("Today snapshot must disclose archiveRevision")?;
    let declared = projection
        .get("unindexedObserverFrames")
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("Today snapshot must disclose unindexedObserverFrames")?;
    let excluded = projection
        .get("excludedObserverFrames")
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= declared)
        .ok_or("Today snapshot exclusions must cover unindexed observer frames")?;
    if excluded > 0
        && projection
            .get("bounded")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err("Today snapshot with excluded observer frames must be bounded".into());
    }
    if declared_revision != current_revision {
        return Err(format!(
            "Today snapshot archive revision changed: declared {declared_revision}, current {current_revision}"
        ));
    }
    if current_unindexed > declared {
        return Err(format!(
            "Today snapshot archive exclusions changed: declared {declared}, current {current_unindexed}"
        ));
    }
    Ok(())
}

/// Publish under both the process-exclusive archive guard and a SQLite
/// immediate transaction so no in-process or second-process writer can
/// overtake the signed archive revision before atomic file replacement.
#[tauri::command]
pub async fn write_owner_today_snapshot(
    state: State<'_, AppState>,
    snapshot_json: String,
) -> Result<TodaySnapshotReceipt, String> {
    let keys = state.signing_keys()?;
    let identity_pk = keys.public_key().to_hex();
    let relay_url = relay_ws_url_with_override(&state);
    let nest = nest_dir().ok_or("cannot resolve nest directory for Today snapshot")?;
    state
        .archive_db
        .with_exclusive_conn(move |conn| {
            if !observer_time::backfill_missing(conn, &identity_pk, &relay_url, &keys)? {
                return Err("Today snapshot archive fence requires completed backfill".into());
            }
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )
            .map_err(|error| format!("begin Today snapshot archive fence: {error}"))?;
            let current_revision = observer_revision::current(&tx, &identity_pk, &relay_url)?;
            let current_unindexed =
                store::count_unindexed_observer_frames(&tx, &identity_pk, &relay_url)?;
            validate_snapshot_archive_fence(&snapshot_json, current_revision, current_unindexed)?;
            let receipt = today_snapshot::write_owner_today_snapshot(
                &nest,
                &keys,
                &identity_pk,
                &relay_url,
                &snapshot_json,
                now_secs(),
            )?;
            tx.commit()
                .map_err(|error| format!("finish Today snapshot archive fence: {error}"))?;
            Ok(receipt)
        })
        .await
}

/// Read and revalidate the current owner's unexpired Today snapshot.
#[tauri::command]
pub fn read_owner_today_snapshot(state: State<'_, AppState>) -> Result<String, String> {
    let identity_pk = identity_pubkey(&state)?;
    let relay_url = relay_ws_url_with_override(&state);
    let nest = nest_dir().ok_or("cannot resolve nest directory for Today snapshot")?;
    today_snapshot::read_owner_today_snapshot(&nest, &identity_pk, &relay_url, now_secs())
}

#[cfg(test)]
mod snapshot_fence_tests {
    use super::validate_snapshot_archive_fence;

    #[test]
    fn snapshot_fence_rejects_a_newly_hidden_observer_frame() {
        let snapshot = r#"{"surface":{"snapshotProjection":{"archiveRevision":7,"bounded":true,"excludedObserverFrames":2,"unindexedObserverFrames":2}}}"#;
        assert!(validate_snapshot_archive_fence(snapshot, 7, 2).is_ok());
        assert!(validate_snapshot_archive_fence(snapshot, 7, 1).is_ok());
        assert!(validate_snapshot_archive_fence(snapshot, 8, 2)
            .unwrap_err()
            .contains("revision changed"));
        assert!(validate_snapshot_archive_fence(snapshot, 7, 3)
            .unwrap_err()
            .contains("declared 2, current 3"));
        let false_complete = r#"{"surface":{"snapshotProjection":{"archiveRevision":7,"bounded":false,"excludedObserverFrames":2,"unindexedObserverFrames":2}}}"#;
        assert!(validate_snapshot_archive_fence(false_complete, 7, 2)
            .unwrap_err()
            .contains("must be bounded"));
    }
}
