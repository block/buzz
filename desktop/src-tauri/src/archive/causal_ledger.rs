use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use tauri::State;

use crate::app_state::AppState;

use super::{identity_pubkey, now_secs, run_archive_db_task};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerEntryEnvelope {
    sequence: u64,
    previous_hash: String,
    hash: String,
    experiment: ExperimentIdentity,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExperimentIdentity {
    experiment_id: String,
}

/// Return the current owner's immutable causal-ledger journal in chain order.
#[tauri::command]
pub async fn read_causal_ledger(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let identity = identity_pubkey(&state)?;
    run_archive_db_task(move |conn| read_entries(conn, &identity)).await
}

fn read_entries(conn: &Connection, identity: &str) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT entry_json FROM causal_ledger_entries
                 WHERE identity_pubkey = ?1 ORDER BY sequence ASC",
        )
        .map_err(|error| format!("failed to prepare causal ledger read: {error}"))?;
    let rows = statement
        .query_map(params![identity], |row| row.get::<_, String>(0))
        .map_err(|error| format!("failed to read causal ledger: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to decode causal ledger row: {error}"))
}

/// Transactionally append one owner-scoped hash-linked causal-ledger entry.
#[tauri::command]
pub async fn append_causal_ledger_entry(
    state: State<'_, AppState>,
    entry_json: String,
) -> Result<(), String> {
    let identity = identity_pubkey(&state)?;
    let entry: LedgerEntryEnvelope = serde_json::from_str(&entry_json)
        .map_err(|error| format!("invalid causal ledger entry: {error}"))?;
    if entry.sequence == 0 || entry.hash.len() != 64 || entry.previous_hash.len() != 64 {
        return Err("invalid causal ledger chain fields".to_string());
    }
    let recorded_at = now_secs();
    run_archive_db_task(move |conn| append_entry(conn, &identity, &entry_json, entry, recorded_at))
        .await
}

fn append_entry(
    conn: &Connection,
    identity: &str,
    entry_json: &str,
    entry: LedgerEntryEnvelope,
    recorded_at: i64,
) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| format!("failed to begin causal ledger append: {error}"))?;
    let result = (|| {
        let tail = conn
            .query_row(
                "SELECT sequence, hash FROM causal_ledger_entries
                 WHERE identity_pubkey = ?1 ORDER BY sequence DESC LIMIT 1",
                params![identity],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("failed to read causal ledger tail: {error}"))?;
        let expected_sequence = tail.as_ref().map_or(1, |(sequence, _)| sequence + 1);
        let expected_previous = tail
            .as_ref()
            .map_or(GENESIS_HASH, |(_, hash)| hash.as_str());

        if entry.sequence != expected_sequence || entry.previous_hash != expected_previous {
            return Err(format!(
                "causal ledger append conflict: expected sequence {expected_sequence}"
            ));
        }
        conn
            .execute(
                "INSERT INTO causal_ledger_entries
                 (identity_pubkey, sequence, experiment_id, previous_hash, hash, entry_json, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    identity,
                    entry.sequence,
                    entry.experiment.experiment_id,
                    entry.previous_hash,
                    entry.hash,
                    entry_json,
                    recorded_at
                ],
            )
            .map_err(|error| format!("failed to append causal ledger entry: {error}"))?;
        Ok(())
    })();
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| format!("failed to commit causal ledger append: {error}")),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::store::open_archive_db;

    fn envelope(sequence: u64, previous_hash: &str) -> (String, LedgerEntryEnvelope) {
        let hash = format!("{sequence:064x}");
        let experiment_id = format!("experiment-{sequence}");
        let json = serde_json::json!({
            "sequence": sequence,
            "previousHash": previous_hash,
            "hash": hash,
            "experiment": { "experimentId": experiment_id }
        })
        .to_string();
        let parsed = serde_json::from_str(&json).expect("test envelope should decode");
        (json, parsed)
    }

    #[test]
    fn persists_ten_thousand_owner_scoped_entries_on_disk_and_reopens() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("archive.db");
        let conn = open_archive_db(&path).expect("open archive database");
        let mut previous = GENESIS_HASH.to_string();
        for sequence in 1..=10_000 {
            let (json, entry) = envelope(sequence, &previous);
            previous = entry.hash.clone();
            append_entry(&conn, "owner-a", &json, entry, 0).expect("append entry");
        }
        drop(conn);

        let reopened = open_archive_db(&path).expect("reopen archive database");
        assert_eq!(
            read_entries(&reopened, "owner-a").expect("read").len(),
            10_000
        );
        assert!(read_entries(&reopened, "owner-b").expect("read").is_empty());
    }

    #[test]
    fn rejects_a_non_contiguous_chain_without_writing_it() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(crate::archive::store::SCHEMA)
            .expect("initialize schema");
        let (json, entry) = envelope(2, GENESIS_HASH);
        assert!(append_entry(&conn, "owner", &json, entry, 0).is_err());
        assert!(read_entries(&conn, "owner").expect("read").is_empty());
    }
}
