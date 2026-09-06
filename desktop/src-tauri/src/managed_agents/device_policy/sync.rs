//! Release only lifecycle coordinates explicitly authored for a local identity.
use std::collections::HashSet;

/// Decide whether a pending coordinate belongs to a registered local identity.
pub(crate) fn allows_coordinate(keys: &HashSet<String>, kind: u32, d_tag: &str) -> bool {
    match kind {
        30177 | 9035 => keys.contains(d_tag),
        5 => d_tag
            .strip_prefix("30177:")
            .is_some_and(|key| keys.contains(key)),
        _ => false,
    }
}

fn ensure_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS device_local_agent_keys (pubkey TEXT PRIMARY KEY NOT NULL)",
    )
    .map_err(|e| format!("Cannot initialize local agent sync permissions: {e}"))
}

/// Called only by the explicit local lifecycle retention path, after its record
/// is saved. Inbound replay and old queue scans never enroll identities here.
pub(crate) fn register(conn: &rusqlite::Connection, pubkey: &str) -> Result<(), String> {
    ensure_table(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO device_local_agent_keys (pubkey) VALUES (?1)",
        [pubkey],
    )
    .map_err(|e| format!("Cannot retain local agent sync permission: {e}"))?;
    Ok(())
}

/// Durable across deletion and restart so a failed local archive can retry.
pub(crate) fn registered(conn: &rusqlite::Connection) -> Result<HashSet<String>, String> {
    ensure_table(conn)?;
    let mut statement = conn
        .prepare("SELECT pubkey FROM device_local_agent_keys LIMIT 5001")
        .map_err(|e| e.to_string())?;
    let keys: HashSet<String> = statement
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if keys.len() > 5000 {
        return Err("Local agent sync permission limit exceeded".into());
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_agent_publication_never_releases_old_deletions_or_runnable_templates() {
        let keys = HashSet::from(["new-local-key".into()]);
        assert!(allows_coordinate(&keys, 30177, "new-local-key"));
        assert!(allows_coordinate(&keys, 5, "30177:new-local-key"));
        assert!(allows_coordinate(&keys, 9035, "new-local-key"));
        for (kind, coordinate) in [
            (30177, "remote-key"),
            (5, "30177:remote-key"),
            (9035, "remote-key"),
            (30175, "new-local-key"),
            (30176, "new-local-key"),
            (30178, "new-local-key"),
            (5, "30175:new-local-key"),
        ] {
            assert!(
                !allows_coordinate(&keys, kind, coordinate),
                "released {kind}:{coordinate}"
            );
        }
    }

    #[test]
    fn only_registered_local_keys_survive_database_reopen_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        assert!(registered(&conn).unwrap().is_empty());
        register(&conn, "local-key").unwrap();
        drop(conn);
        let conn = rusqlite::Connection::open(path).unwrap();
        let keys = registered(&conn).unwrap();
        assert!(allows_coordinate(&keys, 9035, "local-key"));
        assert!(!allows_coordinate(&keys, 5, "30177:historical-key"));
    }
}
