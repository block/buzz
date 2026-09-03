//! Durable, owner-and-relay-scoped Bestie designation storage.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{retention::open_retention_db, storage::atomic_write_json_restricted};

const RECOVERY_JOURNAL_FILE: &str = "bestie-assignment-recovery.json";

/// The one durable Bestie designation in a retention scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BestieAssignment {
    pub agent_pubkey: String,
}

fn ensure_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bestie_assignments (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            agent_pubkey TEXT NOT NULL
        );",
    )
    .map_err(|error| format!("failed to create bestie assignment table: {error}"))
}

/// Read the designation for the already-scoped retention database.
pub fn get_assignment(conn: &Connection) -> Result<Option<BestieAssignment>, String> {
    ensure_table(conn)?;
    conn.query_row(
        "SELECT agent_pubkey FROM bestie_assignments WHERE singleton = 1",
        [],
        |row| {
            Ok(BestieAssignment {
                agent_pubkey: row.get(0)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("failed to read bestie assignment: {error}"))
}

/// Atomically create or replace the one designation in this scope.
pub fn replace_assignment(
    conn: &mut Connection,
    agent_pubkey: &str,
) -> Result<BestieAssignment, String> {
    ensure_table(conn)?;
    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    let transaction = conn
        .transaction()
        .map_err(|error| format!("failed to begin bestie assignment transaction: {error}"))?;
    transaction
        .execute(
            "INSERT INTO bestie_assignments (singleton, agent_pubkey)
             VALUES (1, ?1)
             ON CONFLICT(singleton) DO UPDATE SET agent_pubkey = excluded.agent_pubkey",
            params![normalized],
        )
        .map_err(|error| format!("failed to replace bestie assignment: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit bestie assignment: {error}"))?;
    get_assignment(conn)?.ok_or_else(|| "bestie assignment was not persisted".to_string())
}

/// Clear the designation without changing or stopping the agent.
pub fn clear_assignment(conn: &mut Connection) -> Result<(), String> {
    ensure_table(conn)?;
    let transaction = conn
        .transaction()
        .map_err(|error| format!("failed to begin bestie clear transaction: {error}"))?;
    transaction
        .execute("DELETE FROM bestie_assignments WHERE singleton = 1", [])
        .map_err(|error| format!("failed to clear bestie assignment: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit bestie clear: {error}"))
}

/// Whether the same agent is still designated after an asynchronous operation.
pub fn assignment_matches(conn: &Connection, agent_pubkey: &str) -> Result<bool, String> {
    ensure_table(conn)?;
    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    Ok(get_assignment(conn)?.is_some_and(|assignment| assignment.agent_pubkey == normalized))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ScopedAssignment {
    agent_pubkey: String,
    path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct AssignmentRecoveryJournal {
    assignments: Vec<ScopedAssignment>,
    version: u8,
}

fn recovery_journal_path(base_dir: &Path) -> PathBuf {
    base_dir.join(RECOVERY_JOURNAL_FILE)
}

fn persist_recovery_journal(
    base_dir: &Path,
    assignments: &[ScopedAssignment],
) -> Result<(), String> {
    fs::create_dir_all(base_dir)
        .map_err(|error| format!("failed to create agents directory: {error}"))?;
    let payload = serde_json::to_vec_pretty(&AssignmentRecoveryJournal {
        assignments: assignments.to_vec(),
        version: 1,
    })
    .map_err(|error| format!("failed to serialize Bestie recovery journal: {error}"))?;
    atomic_write_json_restricted(&recovery_journal_path(base_dir), &payload)
        .map_err(|error| format!("failed to persist Bestie recovery journal: {error}"))
}

fn load_recovery_journal(base_dir: &Path) -> Result<Option<AssignmentRecoveryJournal>, String> {
    let path = recovery_journal_path(base_dir);
    let payload = match fs::read(&path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read Bestie recovery journal {}: {error}",
                path.display()
            ))
        }
    };
    let journal: AssignmentRecoveryJournal = serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to parse Bestie recovery journal: {error}"))?;
    if journal.version != 1 {
        return Err(format!(
            "unsupported Bestie recovery journal version {}",
            journal.version
        ));
    }
    let retention_dir = base_dir.join("retention");
    for assignment in &journal.assignments {
        if assignment.path.parent() != Some(retention_dir.as_path())
            || assignment.path.extension().and_then(|value| value.to_str()) != Some("db")
        {
            return Err(format!(
                "Bestie recovery journal contains an invalid retention path: {}",
                assignment.path.display()
            ));
        }
    }
    Ok(Some(journal))
}

fn remove_recovery_journal(base_dir: &Path) -> Result<(), String> {
    let path = recovery_journal_path(base_dir);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove Bestie recovery journal {}: {error}",
            path.display()
        )),
    }
}

fn retention_db_paths(base_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let retention_dir = base_dir.join("retention");
    let entries = match fs::read_dir(&retention_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to read retention directory {}: {error}",
                retention_dir.display()
            ))
        }
    };

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect retention directory {}: {error}",
                retention_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("db") {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn matching_assignments(
    base_dir: &Path,
    agent_pubkey: &str,
) -> Result<Vec<ScopedAssignment>, String> {
    let normalized = agent_pubkey.trim().to_ascii_lowercase();
    let mut assignments = Vec::new();
    // Read and validate every scope before mutating any of them. A broken later
    // database therefore cannot leave an already-cleared prefix behind.
    for path in retention_db_paths(base_dir)? {
        let conn = open_retention_db(&path)?;
        ensure_table(&conn)?;
        if assignment_matches(&conn, &normalized)? {
            assignments.push(ScopedAssignment {
                agent_pubkey: normalized.clone(),
                path,
            });
        }
    }
    Ok(assignments)
}

fn clear_scope(assignment: &ScopedAssignment) -> Result<(), String> {
    let conn = open_retention_db(&assignment.path)?;
    conn.execute(
        "DELETE FROM bestie_assignments WHERE singleton = 1 AND agent_pubkey = ?1",
        params![assignment.agent_pubkey],
    )
    .map_err(|error| {
        format!(
            "failed to clear bestie assignment in {}: {error}",
            assignment.path.display()
        )
    })?;
    Ok(())
}

fn apply_to_assignments(
    assignments: &[ScopedAssignment],
    mut apply: impl FnMut(&ScopedAssignment) -> Result<(), String>,
    action: &str,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for assignment in assignments {
        if let Err(error) = apply(assignment) {
            failures.push(format!("{}: {error}", assignment.path.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to {action} Bestie assignments: {}",
            failures.join("; ")
        ))
    }
}

fn restore_scope(assignment: &ScopedAssignment) -> Result<(), String> {
    let mut conn = open_retention_db(&assignment.path)?;
    replace_assignment(&mut conn, &assignment.agent_pubkey).map(|_| ())
}

fn restore_assignments(assignments: &[ScopedAssignment]) -> Result<(), String> {
    apply_to_assignments(assignments, restore_scope, "restore")
}

/// Replay a durable interrupted-deletion journal.
///
/// The managed-agent store is authoritative for which side of the operation
/// committed: a retained agent gets its exact pre-delete assignments restored;
/// an absent agent gets those exact assignments cleared. The journal is only
/// removed after every scope reaches that deterministic state.
pub fn recover_pending_assignment_cleanup(
    base_dir: &Path,
    agent_exists: impl FnOnce(&str) -> bool,
) -> Result<(), String> {
    let Some(journal) = load_recovery_journal(base_dir)? else {
        return Ok(());
    };
    let agent_pubkey = journal
        .assignments
        .first()
        .map(|assignment| assignment.agent_pubkey.as_str())
        .ok_or_else(|| "Bestie recovery journal contains no assignments".to_string())?;
    if journal
        .assignments
        .iter()
        .any(|assignment| assignment.agent_pubkey != agent_pubkey)
    {
        return Err("Bestie recovery journal contains multiple agents".to_string());
    }
    if agent_exists(agent_pubkey) {
        restore_assignments(&journal.assignments)?;
    } else {
        apply_to_assignments(&journal.assignments, clear_scope, "clear")?;
    }
    remove_recovery_journal(base_dir)
}

fn clear_scoped_assignments(
    assignments: &[ScopedAssignment],
    mut clear: impl FnMut(&ScopedAssignment) -> Result<(), String>,
) -> Result<(), String> {
    for assignment in assignments {
        clear(assignment)?;
    }
    Ok(())
}

fn rollback_with_journal<T>(
    base_dir: &Path,
    assignments: &[ScopedAssignment],
    error: String,
    restore: impl FnMut(&ScopedAssignment) -> Result<(), String>,
) -> Result<T, String> {
    match apply_to_assignments(assignments, restore, "restore") {
        Ok(()) => match remove_recovery_journal(base_dir) {
            Ok(()) => Err(error),
            Err(journal_error) => Err(format!("{error}; {journal_error}")),
        },
        Err(restore_error) => Err(format!("{error}; {restore_error}")),
    }
}

fn with_agent_assignments_cleared_using<T>(
    base_dir: &Path,
    agent_pubkey: &str,
    delete: impl FnOnce() -> Result<T, String>,
    clear: impl FnMut(&ScopedAssignment) -> Result<(), String>,
    mut restore: impl FnMut(&ScopedAssignment) -> Result<(), String>,
) -> Result<T, String> {
    if load_recovery_journal(base_dir)?.is_some() {
        return Err("pending Bestie assignment recovery must complete before deletion".to_string());
    }
    let assignments = matching_assignments(base_dir, agent_pubkey)?;
    if assignments.is_empty() {
        return delete();
    }
    persist_recovery_journal(base_dir, &assignments)?;
    if let Err(error) = clear_scoped_assignments(&assignments, clear) {
        return rollback_with_journal(base_dir, &assignments, error, &mut restore);
    }
    match delete() {
        Ok(value) => {
            if let Err(error) = remove_recovery_journal(base_dir) {
                // The authoritative managed-agent write already committed.
                // Keep the journal as a durable cleanup record; launch/command
                // recovery will observe the absent agent, re-clear these exact
                // scopes idempotently, and retry journal removal.
                eprintln!("buzz-desktop: {error}; cleanup will retry");
            }
            Ok(value)
        }
        Err(error) => rollback_with_journal(base_dir, &assignments, error, &mut restore),
    }
}

/// Run agent deletion work with this agent's community-scoped Bestie
/// assignments temporarily cleared.
///
/// Call this while holding `managed_agents_store_lock`. Every matching scope is
/// snapshotted before the first write. A partial clear, or any later stop/save
/// failure returned by `delete`, restores the snapshot before the error is
/// propagated. Assignments remain cleared only when `delete` succeeds.
pub fn with_agent_assignments_cleared<T>(
    base_dir: &Path,
    agent_pubkey: &str,
    delete: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    with_agent_assignments_cleared_using(base_dir, agent_pubkey, delete, clear_scope, restore_scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        Connection::open_in_memory().unwrap_or_else(|error| panic!("open test db: {error}"))
    }

    #[test]
    fn assignment_is_singleton_and_idempotent() {
        let mut conn = connection();
        let first = replace_assignment(&mut conn, &"A".repeat(64))
            .unwrap_or_else(|error| panic!("assign first: {error}"));
        assert_eq!(first.agent_pubkey, "a".repeat(64));

        let same = replace_assignment(&mut conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("reassign same: {error}"));
        assert_eq!(same.agent_pubkey, "a".repeat(64));

        let replaced = replace_assignment(&mut conn, &"b".repeat(64))
            .unwrap_or_else(|error| panic!("replace: {error}"));
        assert_eq!(replaced.agent_pubkey, "b".repeat(64));
    }

    #[test]
    fn stale_resolver_is_fenced_after_replace_and_clear_is_idempotent() {
        let mut conn = connection();
        replace_assignment(&mut conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("assign: {error}"));
        replace_assignment(&mut conn, &"b".repeat(64))
            .unwrap_or_else(|error| panic!("replace: {error}"));
        assert!(!assignment_matches(&conn, &"a".repeat(64))
            .unwrap_or_else(|error| panic!("check stale assignment: {error}")));
        clear_assignment(&mut conn).unwrap_or_else(|error| panic!("clear: {error}"));
        clear_assignment(&mut conn).unwrap_or_else(|error| panic!("clear again: {error}"));
        assert_eq!(
            get_assignment(&conn).unwrap_or_else(|error| panic!("read: {error}")),
            None
        );
    }

    #[test]
    fn deleting_agent_clears_every_matching_scope_and_preserves_other_assignments() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let agent = "a".repeat(64);
        let other = "b".repeat(64);
        let first_path = retention_dir.join("first.db");
        let second_path = retention_dir.join("second.db");
        let third_path = retention_dir.join("third.db");
        replace_assignment(
            &mut open_retention_db(&first_path)
                .unwrap_or_else(|error| panic!("open first db: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("assign first scope: {error}"));
        replace_assignment(
            &mut open_retention_db(&second_path)
                .unwrap_or_else(|error| panic!("open second db: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("assign second scope: {error}"));
        replace_assignment(
            &mut open_retention_db(&third_path)
                .unwrap_or_else(|error| panic!("open third db: {error}")),
            &other,
        )
        .unwrap_or_else(|error| panic!("assign third scope: {error}"));

        with_agent_assignments_cleared(dir.path(), &agent, || Ok(()))
            .unwrap_or_else(|error| panic!("clear agent assignments: {error}"));
        assert_eq!(
            get_assignment(
                &open_retention_db(&first_path)
                    .unwrap_or_else(|error| panic!("reopen first db: {error}"))
            )
            .unwrap_or_else(|error| panic!("read first scope: {error}")),
            None
        );
        assert_eq!(
            get_assignment(
                &open_retention_db(&third_path)
                    .unwrap_or_else(|error| panic!("reopen third db: {error}"))
            )
            .unwrap_or_else(|error| panic!("read third scope: {error}"))
            .map(|assignment| assignment.agent_pubkey),
            Some(other)
        );
    }

    #[test]
    fn later_scope_clear_failure_restores_the_already_cleared_prefix() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let agent = "a".repeat(64);
        for name in ["first.db", "second.db"] {
            replace_assignment(
                &mut open_retention_db(&retention_dir.join(name))
                    .unwrap_or_else(|error| panic!("open {name}: {error}")),
                &agent,
            )
            .unwrap_or_else(|error| panic!("assign {name}: {error}"));
        }

        let result = with_agent_assignments_cleared_using(
            dir.path(),
            &agent,
            || Ok(()),
            |assignment| {
                if assignment.path.ends_with("second.db") {
                    Err("injected later retention DB failure".to_string())
                } else {
                    clear_scope(assignment)
                }
            },
            restore_scope,
        );

        assert!(result.is_err());
        for name in ["first.db", "second.db"] {
            let conn = open_retention_db(&retention_dir.join(name))
                .unwrap_or_else(|error| panic!("reopen {name}: {error}"));
            assert!(assignment_matches(&conn, &agent)
                .unwrap_or_else(|error| panic!("read {name}: {error}")));
        }
    }

    fn assert_later_deletion_failure_restores_assignment(failure: &str) {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let path = retention_dir.join("owner.db");
        let agent = "a".repeat(64);
        replace_assignment(
            &mut open_retention_db(&path)
                .unwrap_or_else(|error| panic!("open assignment db: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("assign agent: {error}"));

        let result = with_agent_assignments_cleared(dir.path(), &agent, || {
            Err::<(), _>(failure.to_string())
        });

        assert_eq!(result, Err(failure.to_string()));
        let conn = open_retention_db(&path)
            .unwrap_or_else(|error| panic!("reopen assignment db: {error}"));
        assert!(assignment_matches(&conn, &agent)
            .unwrap_or_else(|error| panic!("read restored assignment: {error}")));
    }

    #[test]
    fn stop_failure_after_cleanup_restores_assignment() {
        assert_later_deletion_failure_restores_assignment("injected stop failure");
    }

    #[test]
    fn save_failure_after_cleanup_restores_assignment() {
        assert_later_deletion_failure_restores_assignment("injected save failure");
    }

    #[test]
    fn failed_rollback_leaves_a_durable_journal_that_repairs_on_restart() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let retention_dir = dir.path().join("retention");
        fs::create_dir_all(&retention_dir)
            .unwrap_or_else(|error| panic!("create retention dir: {error}"));
        let agent = "a".repeat(64);
        let first_path = retention_dir.join("first.db");
        let second_path = retention_dir.join("second.db");
        for path in [&first_path, &second_path] {
            replace_assignment(
                &mut open_retention_db(path)
                    .unwrap_or_else(|error| panic!("open {}: {error}", path.display())),
                &agent,
            )
            .unwrap_or_else(|error| panic!("assign {}: {error}", path.display()));
        }

        let result = with_agent_assignments_cleared_using(
            dir.path(),
            &agent,
            || Err::<(), _>("injected managed-agent save failure".to_string()),
            clear_scope,
            |assignment| {
                if assignment.path == second_path {
                    Err("injected restore failure".to_string())
                } else {
                    restore_scope(assignment)
                }
            },
        );

        assert!(result
            .as_ref()
            .is_err_and(|error| error.contains("injected restore failure")));
        assert!(recovery_journal_path(dir.path()).exists());
        assert!(!assignment_matches(
            &open_retention_db(&second_path)
                .unwrap_or_else(|error| panic!("reopen second scope: {error}")),
            &agent,
        )
        .unwrap_or_else(|error| panic!("read torn scope: {error}")));

        recover_pending_assignment_cleanup(dir.path(), |pubkey| pubkey == agent)
            .unwrap_or_else(|error| panic!("replay durable recovery: {error}"));

        for path in [&first_path, &second_path] {
            assert!(assignment_matches(
                &open_retention_db(path)
                    .unwrap_or_else(|error| panic!("reopen {}: {error}", path.display())),
                &agent,
            )
            .unwrap_or_else(|error| panic!("read repaired {}: {error}", path.display())));
        }
        assert!(!recovery_journal_path(dir.path()).exists());
    }
}
