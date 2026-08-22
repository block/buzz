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

fn seal_snapshot_archive_fence(
    snapshot_json: &str,
    current_revision: i64,
    current_unindexed: i64,
) -> Result<String, String> {
    let mut snapshot: serde_json::Value = serde_json::from_str(snapshot_json)
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
    let source_dropped = projection
        .get("sourceDroppedObserverEvents")
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("Today snapshot must disclose source-dropped observer events")?;
    if (excluded > 0 || source_dropped > 0)
        && projection
            .get("bounded")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err("Today snapshot with observer evidence gaps must be bounded".into());
    }
    if current_revision < declared_revision {
        return Err("Today snapshot archive revision moved backwards".into());
    }
    let revision_drift = current_revision - declared_revision;
    if revision_drift > 0 {
        invalidate_snapshot_journal_truth(&mut snapshot)?;
    }
    let current_unindexed = current_unindexed.max(declared);
    let current_excluded = excluded
        .checked_add(current_unindexed - declared)
        .ok_or("Today snapshot exclusion count overflow")?;
    let projection = snapshot
        .pointer_mut("/surface/snapshotProjection")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("Today snapshot must include snapshotProjection")?;
    projection.insert(
        "archiveRevisionAtPublish".into(),
        serde_json::Value::from(current_revision),
    );
    projection.insert(
        "archiveRevisionDrift".into(),
        serde_json::Value::from(revision_drift),
    );
    projection.insert(
        "truthInvalidatedByArchiveDrift".into(),
        serde_json::Value::Bool(revision_drift > 0),
    );
    projection.insert(
        "unindexedObserverFrames".into(),
        serde_json::Value::from(current_unindexed),
    );
    projection.insert(
        "excludedObserverFrames".into(),
        serde_json::Value::from(current_excluded),
    );
    if revision_drift > 0 || current_excluded > 0 {
        projection.insert("bounded".into(), serde_json::Value::Bool(true));
    }
    serde_json::to_string(&snapshot)
        .map_err(|error| format!("serialize fenced Today snapshot: {error}"))
}

/// A forward archive revision means the reconstructed journal set is no
/// longer current. Preserve the bounded historical rows, but never sign stale
/// completion or verification as present truth.
fn invalidate_snapshot_journal_truth(snapshot: &mut serde_json::Value) -> Result<(), String> {
    let surface = snapshot
        .get_mut("surface")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("Today snapshot surface must be an object")?;
    let Some(journals) = surface
        .get_mut("journals")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    for journal in journals.iter_mut() {
        let Some(journal) = journal.as_object_mut() else {
            continue;
        };
        journal.insert(
            "status".into(),
            serde_json::Value::String("incomplete".into()),
        );
        journal.insert(
            "proofState".into(),
            serde_json::Value::String("UNKNOWN".into()),
        );
        journal.insert(
            "summary".into(),
            serde_json::Value::String(
                "Archive changed during publication; refresh before relying on this journal."
                    .into(),
            ),
        );
        journal.insert(
            "summarySource".into(),
            serde_json::Value::String("auto".into()),
        );
        journal.insert(
            "claimedCompletionWithoutEvidence".into(),
            serde_json::Value::Bool(false),
        );
        journal.insert("archiveRevisionStale".into(), serde_json::Value::Bool(true));
        if let Some(events) = journal
            .get_mut("events")
            .and_then(serde_json::Value::as_array_mut)
        {
            for event in events {
                let Some(event) = event.as_object_mut() else {
                    continue;
                };
                if event.get("proofState").and_then(serde_json::Value::as_str) == Some("VERIFIED") {
                    event.insert(
                        "proofState".into(),
                        serde_json::Value::String("UNKNOWN".into()),
                    );
                }
            }
        }
    }
    if let Some(counts) = surface
        .get_mut("counts")
        .and_then(serde_json::Value::as_object_mut)
    {
        counts.insert("failed".into(), serde_json::Value::from(0));
        counts.insert("inProgress".into(), serde_json::Value::from(0));
        counts.insert("claimedWithoutEvidence".into(), serde_json::Value::from(0));
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
            let snapshot_json =
                seal_snapshot_archive_fence(&snapshot_json, current_revision, current_unindexed)?;
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
    use super::seal_snapshot_archive_fence;

    #[test]
    fn snapshot_fence_discloses_new_archive_activity() {
        let snapshot = r#"{"surface":{"counts":{"journals":1,"failed":0,"inProgress":0,"claimedWithoutEvidence":0},"journals":[{"status":"completed","proofState":"VERIFIED","summary":"Verified complete","summarySource":"owner","claimedCompletionWithoutEvidence":false,"events":[{"proofState":"VERIFIED"}]}],"snapshotProjection":{"archiveRevision":7,"bounded":true,"excludedObserverFrames":2,"sourceDroppedObserverEvents":0,"unindexedObserverFrames":2}}}"#;
        let sealed = seal_snapshot_archive_fence(snapshot, 8, 3).unwrap();
        let sealed: serde_json::Value = serde_json::from_str(&sealed).unwrap();
        let projection = &sealed["surface"]["snapshotProjection"];
        assert_eq!(projection["archiveRevision"], 7);
        assert_eq!(projection["archiveRevisionAtPublish"], 8);
        assert_eq!(projection["archiveRevisionDrift"], 1);
        assert_eq!(projection["unindexedObserverFrames"], 3);
        assert_eq!(projection["excludedObserverFrames"], 3);
        assert_eq!(projection["bounded"], true);
        assert_eq!(projection["truthInvalidatedByArchiveDrift"], true);
        let journal = &sealed["surface"]["journals"][0];
        assert_eq!(journal["status"], "incomplete");
        assert_eq!(journal["proofState"], "UNKNOWN");
        assert_eq!(journal["summarySource"], "auto");
        assert_eq!(journal["events"][0]["proofState"], "UNKNOWN");
        assert_eq!(journal["archiveRevisionStale"], true);
        assert!(seal_snapshot_archive_fence(snapshot, 6, 2)
            .unwrap_err()
            .contains("moved backwards"));
        let false_complete = r#"{"surface":{"snapshotProjection":{"archiveRevision":7,"bounded":false,"excludedObserverFrames":2,"sourceDroppedObserverEvents":0,"unindexedObserverFrames":2}}}"#;
        assert!(seal_snapshot_archive_fence(false_complete, 7, 2)
            .unwrap_err()
            .contains("must be bounded"));
        let undisclosed_gap = r#"{"surface":{"snapshotProjection":{"archiveRevision":7,"bounded":false,"excludedObserverFrames":0,"sourceDroppedObserverEvents":1,"unindexedObserverFrames":0}}}"#;
        assert!(seal_snapshot_archive_fence(undisclosed_gap, 7, 0)
            .unwrap_err()
            .contains("must be bounded"));
    }
}
