//! Compact intent, separate from one-shot execution. No history replay.
use buzz_core_pkg::{
    desktop_lifecycle::{Action, Outcome, Request, ResultMessage},
    desktop_stop::StopTarget,
    kind::KIND_DESKTOP_STOP,
};
use nostr::{Event, JsonUtil, Keys};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_placement (
        agent TEXT NOT NULL, slot TEXT NOT NULL, host TEXT NOT NULL, stamp INTEGER NOT NULL,
        id TEXT NOT NULL, PRIMARY KEY(agent,slot));
        CREATE TABLE IF NOT EXISTS desktop_lifecycle_admission (
        agent TEXT NOT NULL, host TEXT NOT NULL, action TEXT NOT NULL, stamp INTEGER NOT NULL,
        id TEXT NOT NULL, PRIMARY KEY(agent,host,action));
        CREATE TABLE IF NOT EXISTS desktop_lifecycle_results (
        id TEXT PRIMARY KEY, raw TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())
}

/// Start's shared max and each host's Stop max are sufficient statistics.
/// Stopping newest Start never falls back to an earlier host.
pub(crate) fn observe(
    conn: &Connection,
    event: &Event,
    keys: &Keys,
    community: &str,
) -> Result<String, String> {
    let (target, slot) = if event.kind.as_u16() as u32 == KIND_DESKTOP_STOP {
        let target = StopTarget::read(event, keys, community)?;
        let slot = format!("stop:{}", target.desktop);
        (target, slot)
    } else {
        let request = Request::read(event, keys, community)?;
        if request.action != Action::Start {
            return Ok(request.target.agent);
        }
        (request.target, "start".into())
    };
    schema(conn)?;
    conn.execute(
        "INSERT INTO desktop_placement VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(agent,slot) DO UPDATE SET host=excluded.host,stamp=excluded.stamp,id=excluded.id
        WHERE excluded.stamp > desktop_placement.stamp OR
        (excluded.stamp = desktop_placement.stamp AND excluded.id < desktop_placement.id)",
        params![
            target.agent,
            slot,
            target.desktop,
            event.created_at.as_secs(),
            event.id.to_hex()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(target.agent)
}

type Row = (String, u64, String);
fn row(conn: &Connection, agent: &str, slot: &str) -> Result<Option<Row>, String> {
    schema(conn)?;
    conn.query_row(
        "SELECT host,stamp,id FROM desktop_placement WHERE agent=?1 AND slot=?2",
        params![agent, slot],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(|e| e.to_string())
}
fn newer(a: &Row, b: &Row) -> bool {
    a.1 > b.1 || (a.1 == b.1 && a.2 < b.2)
}

/// Some Start remains desired, or none. Intent does not establish process state.
pub(crate) fn desired(conn: &Connection, agent: &str) -> Result<Option<(String, String)>, String> {
    let Some(start) = row(conn, agent, "start")? else {
        return Ok(None);
    };
    if row(conn, agent, &format!("stop:{}", start.0))?.is_some_and(|stop| newer(&stop, &start)) {
        return Ok(None);
    }
    Ok(Some((start.0, start.2)))
}

/// Unknown preserves existing local behavior; known supersession blocks every spawn.
pub(crate) fn blocked(conn: &Connection, agent: &str, host: &str) -> Result<bool, String> {
    let start = row(conn, agent, "start")?;
    let stop = row(conn, agent, &format!("stop:{host}"))?;
    Ok(match (start, stop) {
        (None, Some(_)) => true,
        (None, None) => false,
        (Some(start), Some(stop)) if newer(&stop, &start) => true,
        (Some(start), _) => start.0 != host,
    })
}

/// Durable high-water marks are never evicted with diagnostic/result history.
pub(crate) fn admit(conn: &Connection, event: &Event, request: &Request) -> Result<bool, String> {
    schema(conn)?;
    let action = match request.action {
        Action::Start => "start",
        Action::Restart => "restart",
        Action::Status => "status",
    };
    let changed = conn.execute("INSERT INTO desktop_lifecycle_admission VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(agent,host,action) DO UPDATE SET stamp=excluded.stamp,id=excluded.id
        WHERE excluded.stamp > desktop_lifecycle_admission.stamp OR
        (excluded.stamp = desktop_lifecycle_admission.stamp AND excluded.id < desktop_lifecycle_admission.id)",
        params![request.target.agent,request.target.desktop,action,event.created_at.as_secs(),event.id.to_hex()]).map_err(|e| e.to_string())?;
    Ok(changed == 1)
}
pub(crate) fn saved(conn: &Connection, id: &str) -> Result<Option<Event>, String> {
    schema(conn)?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT raw FROM desktop_lifecycle_results WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(|s| Event::from_json(s).map_err(|e| e.to_string()))
        .transpose()
}
pub(crate) fn save(conn: &mut Connection, id: &str, event: &Event) -> Result<(), String> {
    schema(conn)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO desktop_lifecycle_results VALUES (?1,?2)",
        params![id, event.as_json()],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM desktop_lifecycle_results WHERE rowid NOT IN (SELECT rowid FROM desktop_lifecycle_results ORDER BY rowid DESC LIMIT 256)", []).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;

pub(crate) fn has_start(conn: &Connection, agent: &str) -> Result<bool, String> {
    Ok(row(conn, agent, "start")?.is_some())
}
pub(crate) fn latest_start(
    conn: &Connection,
    agent: &str,
) -> Result<Option<(String, String)>, String> {
    Ok(row(conn, agent, "start")?.map(|(host, _, id)| (host, id)))
}

/// A newer local Stop invalidates stale Restart even after a subsequent Start.
pub(crate) fn stale_restart(
    conn: &Connection,
    event: &Event,
    request: &Request,
) -> Result<bool, String> {
    let command = (
        request.target.desktop.clone(),
        event.created_at.as_secs(),
        event.id.to_hex(),
    );
    Ok(row(
        conn,
        &request.target.agent,
        &format!("stop:{}", request.target.desktop),
    )?
    .is_some_and(|stop| newer(&stop, &command)))
}

/// Authenticate and consume before effects; crashes and evicted results never
/// turn an exact retry into a fresh launch or Restart.
pub(crate) fn receive(
    conn: &mut Connection,
    event: &Event,
    keys: &Keys,
    community: &str,
    desktop: &str,
    owned: bool,
    effect: impl FnOnce(&Connection, &Request) -> Result<Outcome, String>,
) -> Result<Option<Event>, String> {
    let request = Request::read(event, keys, community)?;
    observe(conn, event, keys, community)?;
    if request.target.desktop != desktop {
        return Ok(None);
    }
    if let Some(saved) = saved(conn, &event.id.to_hex())? {
        ResultMessage::read(&saved, keys, event, community)?;
        return Ok(Some(saved));
    }
    let outcome = if !owned {
        Outcome::Failed
    } else if !admit(conn, event, &request)?
        || (request.action != Action::Status
            && (blocked(conn, &request.target.agent, desktop)?
                || (request.action == Action::Restart && stale_restart(conn, event, &request)?)
                || (request.action == Action::Start
                    && desired(conn, &request.target.agent)?.map(|(_, id)| id)
                        != Some(event.id.to_hex()))))
    {
        Outcome::Unknown
    } else {
        effect(conn, &request).unwrap_or(if request.action == Action::Status {
            Outcome::Unknown
        } else {
            Outcome::Failed
        })
    };
    let result = ResultMessage {
        request,
        id: event.id.to_hex(),
        outcome,
    }
    .sign(keys)?;
    save(conn, &event.id.to_hex(), &result)?;
    Ok(Some(result))
}
