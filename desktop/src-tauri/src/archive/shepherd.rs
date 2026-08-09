//! Owner-local persistence for redacted Shepherd execution evidence.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    app_state::AppState,
    managed_agents::shepherd::{normalize_shepherd_export, ShepherdEvidenceEnvelope},
};

use super::{identity_pubkey, now_secs, run_archive_db_task};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportShepherdEvidenceRequest {
    pub agent_pubkey: String,
    pub channel_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub source_run_ref: String,
    pub export_json: String,
}

/// One persisted external-execution evidence record.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredShepherdEvidence {
    pub agent_pubkey: String,
    pub channel_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub source_run_ref: String,
    pub imported_at: i64,
    pub evidence: ShepherdEvidenceEnvelope,
}

fn required(value: String, name: &str, max: usize) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() || value.len() > max {
        return Err(format!("{name} must contain 1 to {max} characters"));
    }
    Ok(value)
}

fn validate_agent_pubkey(value: String) -> Result<String, String> {
    let value = required(value, "agentPubkey", 64)?.to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("agentPubkey must be a 64-character hexadecimal public key".to_string());
    }
    Ok(value)
}

/// Validate, redact, and persist one Shepherd trace under an exact Buzz scope.
#[tauri::command]
pub async fn import_shepherd_evidence(
    state: State<'_, AppState>,
    request: ImportShepherdEvidenceRequest,
) -> Result<StoredShepherdEvidence, String> {
    let identity = identity_pubkey(&state)?;
    let agent_pubkey = validate_agent_pubkey(request.agent_pubkey)?;
    let channel_id = required(request.channel_id, "channelId", 256)?;
    let session_id = required(request.session_id, "sessionId", 256)?;
    let turn_id = request
        .turn_id
        .map(|value| required(value, "turnId", 256))
        .transpose()?;
    let source_run_ref = required(request.source_run_ref, "sourceRunRef", 256)?;
    let evidence = normalize_shepherd_export(&request.export_json, Some(source_run_ref.clone()))?;
    let imported_at = now_secs();
    let record = StoredShepherdEvidence {
        agent_pubkey,
        channel_id,
        session_id,
        turn_id,
        source_run_ref,
        imported_at,
        evidence,
    };
    let stored = record.clone();
    run_archive_db_task(move |conn| persist(conn, &identity, &stored)).await?;
    Ok(record)
}

fn persist(
    conn: &Connection,
    identity: &str,
    record: &StoredShepherdEvidence,
) -> Result<(), String> {
    let evidence_json = serde_json::to_string(&record.evidence)
        .map_err(|error| format!("failed to encode Shepherd evidence: {error}"))?;
    conn.execute(
        "INSERT INTO shepherd_evidence
         (identity_pubkey, agent_pubkey, channel_id, session_id, turn_id,
          source_run_ref, evidence_json, imported_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (identity_pubkey, source_run_ref) DO UPDATE SET
           agent_pubkey = excluded.agent_pubkey,
           channel_id = excluded.channel_id,
           session_id = excluded.session_id,
           turn_id = excluded.turn_id,
           evidence_json = excluded.evidence_json,
           imported_at = excluded.imported_at",
        params![
            identity,
            record.agent_pubkey,
            record.channel_id,
            record.session_id,
            record.turn_id,
            record.source_run_ref,
            evidence_json,
            record.imported_at,
        ],
    )
    .map_err(|error| format!("failed to persist Shepherd evidence: {error}"))?;
    Ok(())
}

/// Read Shepherd evidence for one exact Buzz agent/channel/session scope.
#[tauri::command]
pub async fn read_shepherd_evidence(
    state: State<'_, AppState>,
    agent_pubkey: String,
    channel_id: String,
    session_id: String,
) -> Result<Vec<StoredShepherdEvidence>, String> {
    let identity = identity_pubkey(&state)?;
    let agent_pubkey = validate_agent_pubkey(agent_pubkey)?;
    let channel_id = required(channel_id, "channelId", 256)?;
    let session_id = required(session_id, "sessionId", 256)?;
    run_archive_db_task(move |conn| read(conn, &identity, &agent_pubkey, &channel_id, &session_id))
        .await
}

fn read(
    conn: &Connection,
    identity: &str,
    agent_pubkey: &str,
    channel_id: &str,
    session_id: &str,
) -> Result<Vec<StoredShepherdEvidence>, String> {
    let mut statement = conn
        .prepare(
            "SELECT agent_pubkey, channel_id, session_id, turn_id,
                    source_run_ref, imported_at, evidence_json
             FROM shepherd_evidence
             WHERE identity_pubkey = ?1 AND agent_pubkey = ?2
               AND channel_id = ?3 AND session_id = ?4
             ORDER BY imported_at ASC, source_run_ref ASC",
        )
        .map_err(|error| format!("failed to prepare Shepherd evidence read: {error}"))?;
    let rows = statement
        .query_map(
            params![identity, agent_pubkey, channel_id, session_id],
            |row| {
                let evidence_json: String = row.get(6)?;
                let evidence = serde_json::from_str(&evidence_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(StoredShepherdEvidence {
                    agent_pubkey: row.get(0)?,
                    channel_id: row.get(1)?,
                    session_id: row.get(2)?,
                    turn_id: row.get(3)?,
                    source_run_ref: row.get(4)?,
                    imported_at: row.get(5)?,
                    evidence,
                })
            },
        )
        .map_err(|error| format!("failed to read Shepherd evidence: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to decode Shepherd evidence row: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::store::{open_archive_db, SCHEMA};

    fn record(run: &str) -> StoredShepherdEvidence {
        let export = serde_json::json!({
            "total_effects": 1,
            "effect_types": ["task_started"],
            "timeline": [{"_sequence": 1, "effect_type": "task_started", "prompt": "secret"}]
        });
        StoredShepherdEvidence {
            agent_pubkey: "a".repeat(64),
            channel_id: "channel".into(),
            session_id: "session".into(),
            turn_id: Some("turn".into()),
            source_run_ref: run.into(),
            imported_at: 7,
            evidence: normalize_shepherd_export(&export.to_string(), Some(run.into()))
                .expect("normalize"),
        }
    }

    #[test]
    fn persists_reads_and_replaces_by_owner_and_run() {
        let directory = tempfile::tempdir().expect("tempdir");
        let conn = open_archive_db(&directory.path().join("archive.db")).expect("open");
        let first = record("run-1");
        persist(&conn, "owner-a", &first).expect("persist");
        persist(&conn, "owner-a", &first).expect("idempotent replace");
        assert_eq!(
            read(&conn, "owner-a", &first.agent_pubkey, "channel", "session")
                .expect("read")
                .len(),
            1
        );
        assert!(
            read(&conn, "owner-b", &first.agent_pubkey, "channel", "session")
                .expect("read other owner")
                .is_empty()
        );
    }

    #[test]
    fn schema_creates_the_shepherd_table() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(SCHEMA).expect("schema");
        conn.prepare("SELECT source_run_ref FROM shepherd_evidence")
            .expect("table exists");
    }
}
