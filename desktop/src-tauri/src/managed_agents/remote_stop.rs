//! Durable Stop admission. Compact outcomes never compact the per-agent fence.
use buzz_core_pkg::desktop_stop::{StopOutcome, StopResult, StopTarget};
use nostr::{Event, JsonUtil, Keys};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tauri::{AppHandle, Manager};

use super::retention::{open_retention_db, scoped_retention_db_path};
use super::ManagedAgentRuntimeKey;

const HISTORY_LIMIT: i64 = 256;

fn schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_stop_fence (
        agent TEXT PRIMARY KEY, stamp INTEGER NOT NULL, event_id TEXT NOT NULL, blocked INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS desktop_stop_results (
        id TEXT PRIMARY KEY, raw TEXT NOT NULL);")
        .map_err(|e| e.to_string())
}

/// Persist admission before effect; duplicates/interruption never repeat Stop.
pub(crate) fn admit(
    conn: &mut Connection,
    request: &Event,
    target: &StopTarget,
) -> Result<bool, String> {
    schema(conn)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let previous: Option<(u64, String)> = tx
        .query_row(
            "SELECT stamp, event_id FROM desktop_stop_fence WHERE agent = ?1",
            [&target.agent],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let id = request.id.to_hex();
    let stamp = request.created_at.as_secs();
    if previous.is_some_and(|(time, key)| time > stamp || (time == stamp && key <= id)) {
        return Ok(false);
    }
    tx.execute("INSERT INTO desktop_stop_fence VALUES (?1, ?2, ?3, 1)
        ON CONFLICT(agent) DO UPDATE SET stamp=excluded.stamp, event_id=excluded.event_id, blocked=1",
        params![target.agent, stamp, id]).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(true)
}

pub(crate) fn saved_result(conn: &Connection, id: &str) -> Result<Option<String>, String> {
    schema(conn)?;
    conn.query_row(
        "SELECT raw FROM desktop_stop_results WHERE id=?1",
        [id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub(crate) fn save_result(conn: &mut Connection, id: &str, raw: &str) -> Result<(), String> {
    schema(conn)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO desktop_stop_results VALUES (?1, ?2)",
        params![id, raw],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM desktop_stop_results WHERE rowid NOT IN
        (SELECT rowid FROM desktop_stop_results ORDER BY rowid DESC LIMIT ?1)",
        [HISTORY_LIMIT],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// Authenticate and durably consume a live request before invoking ordinary Stop.
/// The caller holds the runtime transition lock across this entire operation.
pub(crate) fn receive(
    conn: &mut Connection,
    request: &Event,
    keys: &Keys,
    community: &str,
    desktop: &str,
    owned: bool,
    stop: impl FnOnce(&StopTarget) -> Result<(), String>,
) -> Result<Option<Event>, String> {
    let target = StopTarget::read(request, keys, community)?;
    if target.desktop != desktop {
        return Ok(None);
    }
    let id = request.id.to_hex();
    if let Some(raw) = saved_result(conn, &id)? {
        let result = Event::from_json(raw).map_err(|e| e.to_string())?;
        StopResult::read(&result, keys, request, community)?;
        return Ok(Some(result));
    }
    let outcome = if !owned {
        StopOutcome::Failed
    } else if admit(conn, request, &target)? {
        outcome(stop(&target))
    } else {
        StopOutcome::Unknown
    };
    let result = StopResult {
        target,
        request: id.clone(),
        outcome,
    }
    .sign(keys)?;
    save_result(conn, &id, &result.as_json())?;
    Ok(Some(result))
}

/// Explicit local Start captures the Stop fence before its asynchronous preflight.
/// Automatic starts and Restart continuations never receive this permission.
pub(crate) struct ResumeTicket {
    previous: Option<String>,
}

pub(crate) fn capture_resume(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: &str,
) -> Result<ResumeTicket, String> {
    let conn = connection(app, key, owner)?;
    schema(&conn)?;
    let previous = conn
        .query_row(
            "SELECT event_id FROM desktop_stop_fence WHERE agent=?1",
            [&key.pubkey],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(ResumeTicket { previous })
}

fn connection(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: &str,
) -> Result<Connection, String> {
    let path =
        scoped_retention_db_path(&super::managed_agents_base_dir(app)?, &key.relay_url, owner);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    open_retention_db(&path)
}

/// Every ordinary spawn passes here, including restore/config/reconcile.
/// Caller holds the existing transition lock through child registration.
pub(crate) fn check_launch(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: Option<&str>,
    resume: Option<&ResumeTicket>,
) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let current_owner = state.signing_keys()?.public_key().to_hex();
    if owner != Some(current_owner.as_str()) {
        return Err("Desktop launch owner changed".into());
    }
    let conn = connection(app, key, &current_owner)?;
    schema(&conn)?;
    let scope = super::retention::RetentionScope {
        db_path: scoped_retention_db_path(
            &super::managed_agents_base_dir(app)?,
            &key.relay_url,
            &current_owner,
        ),
        relay_url: key.relay_url.clone(),
        owner_keys: state.signing_keys()?,
    };
    let mut local_conn = connection(app, key, &current_owner)?;
    let host = crate::commands::desktop_stop::local_id(&mut local_conn, &scope)?;
    if super::placement::blocked(&conn, &key.pubkey, &host)?
        && (resume.is_none() || super::placement::has_start(&conn, &key.pubkey)?)
    {
        return Err("This Desktop is no longer the selected running destination. Use explicit Start on this Desktop.".into());
    }
    let row: Option<(String, bool)> = conn
        .query_row(
            "SELECT event_id, blocked FROM desktop_stop_fence WHERE agent=?1",
            [&key.pubkey],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    allow_launch(row.as_ref(), resume)
}

fn allow_launch(row: Option<&(String, bool)>, resume: Option<&ResumeTicket>) -> Result<(), String> {
    if let Some(ticket) = resume {
        if ticket.previous.as_ref() != row.map(|(id, _)| id) {
            return Err("A newer Stop interrupted this Start".into());
        }
    } else if row.is_some_and(|(_, blocked)| *blocked) {
        return Err(
            "Stopped from another Desktop. Use Start agent to start it again explicitly.".into(),
        );
    }
    Ok(())
}

/// A failed spawn must not unblock config/restore. Commit only after the child
/// has its ordinary receipt and tracked handle, still under the transition lock.
pub(crate) fn finish_resume(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: Option<&str>,
    ticket: Option<&ResumeTicket>,
) -> Result<(), String> {
    if ticket.is_none() {
        return Ok(());
    }
    let owner = owner.ok_or("Desktop launch owner unavailable")?;
    check_launch(app, key, Some(owner), ticket)?;
    connection(app, key, owner)?
        .execute(
            "UPDATE desktop_stop_fence SET blocked=0 WHERE agent=?1",
            [&key.pubkey],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Expose outcomes without mistaking a missing record for success.
pub(crate) fn outcome(stopped: Result<(), String>) -> StopOutcome {
    if stopped.is_ok() {
        StopOutcome::Stopped
    } else {
        StopOutcome::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Timestamp};
    fn request(keys: &Keys, target: &StopTarget, time: u64) -> Event {
        let e = target.sign(keys).unwrap();
        EventBuilder::new(e.kind, e.content)
            .tags(e.tags.to_vec())
            .custom_created_at(Timestamp::from(time))
            .sign_with_keys(keys)
            .unwrap()
    }
    #[test]
    fn launch_fence_requires_explicit_start_and_rejects_delayed_preflight() {
        let stopped = ("stop-a".to_owned(), true);
        let resumed = ("stop-a".to_owned(), false);
        let new_stop = ("stop-b".to_owned(), true);
        assert!(allow_launch(None, None).is_ok());
        assert!(allow_launch(Some(&stopped), None).is_err());
        assert!(allow_launch(Some(&resumed), None).is_ok());
        let ticket = ResumeTicket {
            previous: Some("stop-a".to_owned()),
        };
        assert!(allow_launch(Some(&stopped), Some(&ticket)).is_ok());
        assert!(allow_launch(Some(&new_stop), Some(&ticket)).is_err());
        let before_any_stop = ResumeTicket { previous: None };
        assert!(allow_launch(Some(&stopped), Some(&before_any_stop)).is_err());
        assert!(allow_launch(None, Some(&before_any_stop)).is_ok());
    }

    #[test]
    fn saved_result_is_immutable_and_duplicate_after_interruption_is_unknown() {
        let mut conn = Connection::open_in_memory().unwrap();
        let keys = Keys::generate();
        let target = StopTarget {
            v: 1,
            community: "wss://one.example".into(),
            desktop: "a".repeat(32),
            agent: Keys::generate().public_key().to_hex(),
        };
        let event = request(&keys, &target, 100);
        assert!(admit(&mut conn, &event, &target).unwrap());
        // A crash between admission and recording an outcome never reexecutes.
        assert!(saved_result(&conn, &event.id.to_hex()).unwrap().is_none());
        assert!(!admit(&mut conn, &event, &target).unwrap());
        save_result(&mut conn, &event.id.to_hex(), "original bytes").unwrap();
        save_result(&mut conn, &event.id.to_hex(), "replacement").unwrap();
        assert_eq!(
            saved_result(&conn, &event.id.to_hex()).unwrap().as_deref(),
            Some("original bytes")
        );
        assert_eq!(
            outcome(Err("ordinary Stop failed".into())),
            StopOutcome::Failed
        );
        assert_eq!(outcome(Ok(())), StopOutcome::Stopped);
    }

    #[test]
    fn receiver_authenticates_routes_and_returns_exact_saved_result_without_effect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receiver.db");
        let mut conn = open_retention_db(&path).unwrap();
        let keys = Keys::generate();
        let target = StopTarget {
            v: 1,
            community: "wss://one.example".into(),
            desktop: "a".repeat(32),
            agent: Keys::generate().public_key().to_hex(),
        };
        let event = request(&keys, &target, 100);
        let no_effect = |_: &StopTarget| panic!("must not invoke ordinary Stop");
        let foreign = Keys::generate();
        for (signer, community, host, rejected) in [
            (
                &foreign,
                target.community.as_str(),
                target.desktop.as_str(),
                true,
            ),
            (&keys, "wss://other.example", target.desktop.as_str(), true),
            (
                &keys,
                target.community.as_str(),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                false,
            ),
        ] {
            let result = receive(&mut conn, &event, signer, community, host, true, no_effect);
            if rejected {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_none());
            }
        }
        let receive_owned = |conn: &mut Connection, event: &Event, owned, stop| {
            receive(
                conn,
                event,
                &keys,
                &target.community,
                &target.desktop,
                owned,
                stop,
            )
            .unwrap()
            .unwrap()
        };
        let fail: fn(&StopTarget) -> Result<(), String> = |_| Err("ordinary Stop error".into());
        let no_effect: fn(&StopTarget) -> Result<(), String> = no_effect;
        // The first effect succeeds. The reopened retry must return its exact
        // signed bytes without invoking the callback at all.
        let mut effects = 0;
        let result = receive(
            &mut conn,
            &event,
            &keys,
            &target.community,
            &target.desktop,
            true,
            |actual| {
                assert_eq!(actual, &target);
                effects += 1;
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(effects, 1);
        let assert_outcome = |result: &Event, request: &Event, expected| {
            assert_eq!(
                StopResult::read(result, &keys, request, &target.community)
                    .unwrap()
                    .outcome,
                expected
            );
        };
        assert_outcome(&result, &event, StopOutcome::Stopped);
        drop(conn);
        let mut conn = open_retention_db(&path).unwrap();
        let duplicate = receive_owned(&mut conn, &event, true, no_effect);
        assert_eq!(result.as_json(), duplicate.as_json());
        let next = request(&keys, &target, 101);
        let failed = receive_owned(&mut conn, &next, true, fail);
        assert_outcome(&failed, &next, StopOutcome::Failed);
        assert_eq!(
            failed.as_json(),
            receive_owned(&mut conn, &next, true, no_effect).as_json()
        );
        let unowned = request(&keys, &target, 102);
        let denied = receive_owned(&mut conn, &unowned, false, no_effect);
        assert_outcome(&denied, &unowned, StopOutcome::Failed);
        assert_eq!(
            conn.query_row(
                "SELECT event_id FROM desktop_stop_fence WHERE agent=?1",
                [&target.agent],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            next.id.to_hex()
        );
        let interrupted = request(&keys, &target, 103);
        assert!(admit(&mut conn, &interrupted, &target).unwrap());
        let unknown = receive_owned(&mut conn, &interrupted, true, no_effect);
        assert_outcome(&unknown, &interrupted, StopOutcome::Unknown);
    }

    #[test]
    fn durable_fence_survives_outcome_eviction_and_accepts_fresh_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stop.db");
        let mut conn = open_retention_db(&path).unwrap();
        let keys = Keys::generate();
        let target = StopTarget {
            v: 1,
            community: "wss://one.example".into(),
            desktop: "a".repeat(32),
            agent: Keys::generate().public_key().to_hex(),
        };
        let first = request(&keys, &target, 100);
        assert!(admit(&mut conn, &first, &target).unwrap());
        assert!(!admit(&mut conn, &first, &target).unwrap());
        for i in 0..HISTORY_LIMIT + 2 {
            save_result(&mut conn, &format!("{i}"), "result").unwrap();
        }
        drop(conn);
        let mut conn = open_retention_db(&path).unwrap();
        assert!(!admit(&mut conn, &first, &target).unwrap());
        assert!(admit(&mut conn, &request(&keys, &target, 101), &target).unwrap());
        assert!(!admit(&mut conn, &request(&keys, &target, 99), &target).unwrap());
        let a = request(&keys, &target, 102);
        let b = request(&keys, &target, 102);
        let (low, high) = if a.id < b.id { (a, b) } else { (b, a) };
        assert!(admit(&mut conn, &high, &target).unwrap());
        assert!(admit(&mut conn, &low, &target).unwrap());
        assert!(!admit(&mut conn, &high, &target).unwrap());
    }
}
