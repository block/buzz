//! Durable provider calls and returned handles, scoped to one relay owner.
//!
//! A provider call is an external side effect.  This journal is written before
//! invoking it and records a returned handle before any relay/disk settlement,
//! so a crash or a concurrent relay edit cannot make that resource invisible.

use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSettlement {
    pub owner: String,
    pub agent: String,
    pub invocation_id: String,
    pub authority_event_id: String,
    pub provider_id: String,
    pub config_fingerprint: String,
    pub backend_agent_id: Option<String>,
}

pub(super) fn initialize(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS managed_agent_provider_settlements (
            owner TEXT NOT NULL,
            agent TEXT NOT NULL,
            invocation_id TEXT NOT NULL,
            authority_event_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            config_fingerprint TEXT NOT NULL,
            backend_agent_id TEXT,
            PRIMARY KEY (owner, agent)
        );",
    )
    .map_err(|error| format!("failed to initialize provider settlement journal: {error}"))
}

pub(crate) fn begin(conn: &Connection, row: &ProviderSettlement) -> Result<(), String> {
    if pending(conn, &row.owner, &row.agent)?.is_some() {
        return Err("an unresolved provider settlement already exists for this agent".into());
    }
    conn.execute(
        "INSERT INTO managed_agent_provider_settlements
         (owner, agent, invocation_id, authority_event_id, provider_id, config_fingerprint, backend_agent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
        params![row.owner, row.agent, row.invocation_id, row.authority_event_id,
                row.provider_id, row.config_fingerprint],
    )
    .map_err(|error| format!("failed to begin provider settlement: {error}"))?;
    Ok(())
}

pub(crate) fn record_handle(
    conn: &Connection,
    owner: &str,
    agent: &str,
    invocation_id: &str,
    backend_agent_id: &str,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE managed_agent_provider_settlements SET backend_agent_id = ?4
             WHERE owner = ?1 AND agent = ?2 AND invocation_id = ?3",
            params![owner, agent, invocation_id, backend_agent_id],
        )
        .map_err(|error| format!("failed to record provider handle: {error}"))?;
    if changed != 1 {
        return Err("provider settlement invocation is no longer current".into());
    }
    Ok(())
}

pub(crate) fn pending(
    conn: &Connection,
    owner: &str,
    agent: &str,
) -> Result<Option<ProviderSettlement>, String> {
    conn.query_row(
        "SELECT invocation_id, authority_event_id, provider_id, config_fingerprint, backend_agent_id
         FROM managed_agent_provider_settlements WHERE owner = ?1 AND agent = ?2",
        params![owner, agent],
        |row| {
            Ok(ProviderSettlement {
                owner: owner.to_string(),
                agent: agent.to_string(),
                invocation_id: row.get(0)?,
                authority_event_id: row.get(1)?,
                provider_id: row.get(2)?,
                config_fingerprint: row.get(3)?,
                backend_agent_id: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("failed to read provider settlement: {error}"))
}

pub(crate) fn finish(
    conn: &Connection,
    owner: &str,
    agent: &str,
    invocation_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM managed_agent_provider_settlements
         WHERE owner = ?1 AND agent = ?2 AND invocation_id = ?3",
        params![owner, agent, invocation_id],
    )
    .map_err(|error| format!("failed to finish provider settlement: {error}"))?;
    Ok(())
}

/// Explicit operator escape after accepting that the provider protocol has no
/// undeploy operation. Ordinary deletion must never call this implicitly.
pub(crate) fn abandon(conn: &Connection, owner: &str, agent: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM managed_agent_provider_settlements WHERE owner = ?1 AND agent = ?2",
        params![owner, agent],
    )
    .map_err(|error| format!("failed to abandon provider settlement: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::retention::open_retention_db;

    fn row() -> ProviderSettlement {
        ProviderSettlement {
            owner: "owner".into(),
            agent: "agent".into(),
            invocation_id: "call-1".into(),
            authority_event_id: "head-a".into(),
            provider_id: "provider-a".into(),
            config_fingerprint: "fingerprint-a".into(),
            backend_agent_id: None,
        }
    }

    #[test]
    fn returned_handle_survives_restart_until_exact_invocation_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.db");
        {
            let conn = open_retention_db(&path).unwrap();
            begin(&conn, &row()).unwrap();
            record_handle(&conn, "owner", "agent", "call-1", "handle-a").unwrap();
        }
        let conn = open_retention_db(&path).unwrap();
        let restored = pending(&conn, "owner", "agent").unwrap().unwrap();
        assert_eq!(restored.backend_agent_id.as_deref(), Some("handle-a"));
        assert!(finish(&conn, "owner", "agent", "wrong-call").is_ok());
        assert!(pending(&conn, "owner", "agent").unwrap().is_some());
        finish(&conn, "owner", "agent", "call-1").unwrap();
        assert!(pending(&conn, "owner", "agent").unwrap().is_none());
    }

    #[test]
    fn unresolved_call_blocks_a_second_provider_side_effect() {
        let conn = open_retention_db(std::path::Path::new(":memory:")).unwrap();
        begin(&conn, &row()).unwrap();
        assert!(begin(&conn, &row()).unwrap_err().contains("unresolved"));
    }
}
