//! Monotonic revision fence for owner-scoped observer archive projections.
//!
//! Every mutation that can change Today reconstruction advances a durable
//! `(identity, relay)` revision in the same SQLite transaction as the source
//! mutation. Multi-page readers can therefore restart instead of combining
//! pages from different archive states, and publishers can reject a projection
//! if accepted evidence changed after reconstruction.

use rusqlite::{params, Connection};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observer_archive_revisions (
    identity_pubkey TEXT NOT NULL,
    relay_url       TEXT NOT NULL,
    revision        INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url),
    CHECK (revision >= 0)
);

CREATE TRIGGER IF NOT EXISTS observer_revision_event_insert
AFTER INSERT ON archived_events WHEN NEW.kind = 24200 BEGIN
  INSERT INTO observer_archive_revisions VALUES (NEW.identity_pubkey, NEW.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
CREATE TRIGGER IF NOT EXISTS observer_revision_event_delete
AFTER DELETE ON archived_events WHEN OLD.kind = 24200 BEGIN
  INSERT INTO observer_archive_revisions VALUES (OLD.identity_pubkey, OLD.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
DROP TRIGGER IF EXISTS observer_revision_scope_insert;
CREATE TRIGGER observer_revision_scope_insert
AFTER INSERT ON archived_event_scopes
WHEN NEW.scope_type = 'owner_p'
 AND EXISTS (
   SELECT 1 FROM archived_events
    WHERE identity_pubkey = NEW.identity_pubkey
      AND relay_url = NEW.relay_url AND id = NEW.id AND kind = 24200
 ) BEGIN
  INSERT INTO observer_archive_revisions VALUES (NEW.identity_pubkey, NEW.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
DROP TRIGGER IF EXISTS observer_revision_scope_delete;
CREATE TRIGGER observer_revision_scope_delete
AFTER DELETE ON archived_event_scopes
WHEN OLD.scope_type = 'owner_p'
 AND EXISTS (
   SELECT 1 FROM archived_events
    WHERE identity_pubkey = OLD.identity_pubkey
      AND relay_url = OLD.relay_url AND id = OLD.id AND kind = 24200
 ) BEGIN
  INSERT INTO observer_archive_revisions VALUES (OLD.identity_pubkey, OLD.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
CREATE TRIGGER IF NOT EXISTS observer_revision_time_insert
AFTER INSERT ON observer_time_index BEGIN
  INSERT INTO observer_archive_revisions VALUES (NEW.identity_pubkey, NEW.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
CREATE TRIGGER IF NOT EXISTS observer_revision_time_update
AFTER UPDATE ON observer_time_index BEGIN
  INSERT INTO observer_archive_revisions VALUES (NEW.identity_pubkey, NEW.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
CREATE TRIGGER IF NOT EXISTS observer_revision_time_delete
AFTER DELETE ON observer_time_index BEGIN
  INSERT INTO observer_archive_revisions VALUES (OLD.identity_pubkey, OLD.relay_url, 1)
  ON CONFLICT (identity_pubkey, relay_url)
  DO UPDATE SET revision = revision + 1;
END;
"#;

pub(super) fn ensure_schema(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("begin observer archive revision migration: {error}"))?;
    tx.execute_batch(SCHEMA)
        .map_err(|error| format!("initialize observer archive revision: {error}"))?;
    tx.commit()
        .map_err(|error| format!("commit observer archive revision migration: {error}"))
}

pub(super) fn current(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE((SELECT revision FROM observer_archive_revisions
                          WHERE identity_pubkey = ?1 AND relay_url = ?2), 0)",
        params![identity_pubkey, relay_url],
        |row| row.get(0),
    )
    .map_err(|error| format!("read observer archive revision: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_mutations_advance_revision_transactionally() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE archived_events (
               identity_pubkey TEXT, relay_url TEXT, id TEXT, kind INTEGER
             );
             CREATE TABLE archived_event_scopes (
               identity_pubkey TEXT, relay_url TEXT, id TEXT,
               scope_type TEXT, scope_value TEXT
             );
             CREATE TABLE observer_time_index (
               identity_pubkey TEXT, relay_url TEXT, id TEXT,
               observed_start_at INTEGER, observed_end_at INTEGER
             );
             CREATE TABLE observer_archive_revisions (
               identity_pubkey TEXT, relay_url TEXT, revision INTEGER,
               PRIMARY KEY (identity_pubkey, relay_url)
             );
             CREATE TRIGGER observer_revision_scope_insert
             AFTER INSERT ON archived_event_scopes WHEN NEW.scope_type = 'owner_p' BEGIN
               INSERT INTO observer_archive_revisions VALUES (NEW.identity_pubkey, NEW.relay_url, 1)
               ON CONFLICT (identity_pubkey, relay_url)
               DO UPDATE SET revision = revision + 1;
             END;
             CREATE TRIGGER observer_revision_scope_delete
             AFTER DELETE ON archived_event_scopes WHEN OLD.scope_type = 'owner_p' BEGIN
               INSERT INTO observer_archive_revisions VALUES (OLD.identity_pubkey, OLD.relay_url, 1)
               ON CONFLICT (identity_pubkey, relay_url)
               DO UPDATE SET revision = revision + 1;
             END;",
        )
        .unwrap();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 0);

        conn.execute(
            "INSERT INTO archived_events VALUES ('owner','relay','metric',44200)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO archived_event_scopes VALUES ('owner','relay','metric','owner_p','owner')",
            [],
        )
        .unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 0);

        conn.execute(
            "INSERT INTO archived_events VALUES ('owner','relay','event',24200)",
            [],
        )
        .unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 1);

        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO observer_time_index VALUES ('owner','relay','event',1,1)",
            [],
        )
        .unwrap();
        assert_eq!(current(&tx, "owner", "relay").unwrap(), 2);
        tx.rollback().unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 1);
    }

    #[test]
    fn owner_scope_for_agent_metric_does_not_advance_observer_revision() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE archived_events (
               identity_pubkey TEXT, relay_url TEXT, id TEXT, kind INTEGER
             );
             CREATE TABLE archived_event_scopes (
               identity_pubkey TEXT, relay_url TEXT, id TEXT,
               scope_type TEXT, scope_value TEXT
             );
             CREATE TABLE observer_time_index (
               identity_pubkey TEXT, relay_url TEXT, id TEXT,
               observed_start_at INTEGER, observed_end_at INTEGER
             );",
        )
        .unwrap();
        ensure_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO archived_events VALUES ('owner','relay','metric',44200)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO archived_event_scopes VALUES ('owner','relay','metric','owner_p','owner')",
            [],
        )
        .unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 0);

        conn.execute("DELETE FROM archived_event_scopes WHERE id = 'metric'", [])
            .unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 0);

        conn.execute(
            "INSERT INTO archived_events VALUES ('owner','relay','observer',24200)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO archived_event_scopes VALUES ('owner','relay','observer','owner_p','owner')",
            [],
        )
        .unwrap();
        assert_eq!(current(&conn, "owner", "relay").unwrap(), 2);
    }
}
