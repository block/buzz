use rusqlite::{params, Connection};

use super::{get_retained_event, tombstone_retention_d_tag, RetainedEvent};

/// Load retained events for one owner and kind.
pub fn get_retained_events(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE kind = ?1 AND pubkey = ?2
             ORDER BY d_tag",
        )
        .map_err(|e| format!("failed to prepare query: {e}"))?;

    let rows = stmt
        .query_map(params![kind, pubkey], |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| format!("failed to query retained events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read retained event row: {e}"))
}

/// Whether a retained NIP-09 tombstone suppresses an upsert at the target
/// coordinate. A strictly newer upsert may recreate the coordinate.
pub fn retained_tombstone_covers(
    conn: &Connection,
    target_kind: u32,
    pubkey: &str,
    d_tag: &str,
    created_at: i64,
) -> Result<bool, String> {
    Ok(get_retained_event(
        conn,
        5,
        pubkey,
        &tombstone_retention_d_tag(target_kind, d_tag),
    )?
    .is_some_and(|tombstone| tombstone.created_at >= created_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::retention::{open_retention_db, retain_event};

    #[test]
    fn tombstone_suppression_is_delivery_order_independent() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
        retain_event(
            &conn,
            &RetainedEvent {
                kind: 5,
                pubkey: "owner".to_string(),
                d_tag: tombstone_retention_d_tag(30177, "agent-pubkey"),
                content: String::new(),
                created_at: 2000,
                raw_event: "{}".to_string(),
                pending_sync: false,
            },
        )
        .unwrap();

        for created_at in [1999, 2000] {
            assert!(
                retained_tombstone_covers(&conn, 30177, "owner", "agent-pubkey", created_at)
                    .unwrap()
            );
        }
        assert!(
            !retained_tombstone_covers(&conn, 30177, "owner", "agent-pubkey", 2001).unwrap(),
            "a newer upsert may legitimately recreate the coordinate"
        );
    }
}
