//! Rebuildable inner-time index for encrypted observer envelopes.

use chrono::DateTime;
use nostr::{Event, JsonUtil, Keys};
use rusqlite::{params, Connection};

#[cfg(test)]
use super::store;

const BACKFILL_BATCH_LIMIT: i64 = 500;

fn timestamp_seconds(value: &serde_json::Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.get("timestamp")?.as_str()?)
        .ok()
        .map(|timestamp| timestamp.timestamp())
}

fn collect_bounds(value: &serde_json::Value, start: &mut Option<i64>, end: &mut Option<i64>) {
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("batch") {
        if let Some(events) = value
            .get("payload")
            .and_then(|payload| payload.get("events"))
            .and_then(serde_json::Value::as_array)
        {
            for event in events {
                collect_bounds(event, start, end);
            }
        }
        return;
    }
    let Some(timestamp) = timestamp_seconds(value) else {
        return;
    };
    *start = Some(start.map_or(timestamp, |current| current.min(timestamp)));
    *end = Some(end.map_or(timestamp, |current| current.max(timestamp)));
}

pub(super) fn bounds(value: &serde_json::Value) -> (Option<i64>, Option<i64>) {
    let mut start = None;
    let mut end = None;
    collect_bounds(value, &mut start, &mut end);
    (start, end)
}

/// Record inclusive inner timestamp bounds. NULL bounds are the durable
/// processed marker for malformed or undecryptable envelopes.
pub(super) fn upsert(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    event_id: &str,
    observed_start_at: Option<i64>,
    observed_end_at: Option<i64>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO observer_time_index
             (identity_pubkey, relay_url, id, observed_start_at, observed_end_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            identity_pubkey,
            relay_url,
            event_id,
            observed_start_at,
            observed_end_at
        ],
    )
    .map_err(|error| format!("failed to upsert observer_time_index: {error}"))?;
    Ok(())
}

fn read_missing(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT ae.id, ae.raw_json
               FROM archived_events ae
               INNER JOIN archived_event_scopes aes
                 ON aes.identity_pubkey = ae.identity_pubkey
                AND aes.relay_url = ae.relay_url
                AND aes.id = ae.id
              WHERE ae.identity_pubkey = ?1
                AND ae.relay_url = ?2
                AND ae.kind = 24200
                AND aes.scope_type = 'owner_p'
                AND aes.scope_value = ?1
                AND NOT EXISTS (
                    SELECT 1 FROM observer_time_index oti
                     WHERE oti.identity_pubkey = ae.identity_pubkey
                       AND oti.relay_url = ae.relay_url
                       AND oti.id = ae.id
                )
              ORDER BY ae.created_at DESC, ae.id DESC
              LIMIT ?3",
        )
        .map_err(|error| format!("prepare observer time backfill: {error}"))?;
    let rows = stmt
        .query_map(
            params![identity_pubkey, relay_url, BACKFILL_BATCH_LIMIT + 1],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| format!("query observer time backfill: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read observer time backfill: {error}"))
}

/// Lazily migrate one bounded batch of historical signed envelopes. Every
/// examined row receives an index record, including failures, so repeated
/// reads make deterministic progress without monopolizing the archive actor.
pub(super) fn backfill_missing(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    owner_keys: &Keys,
) -> Result<bool, String> {
    let mut rows = read_missing(conn, identity_pubkey, relay_url)?;
    let complete = rows.len() <= BACKFILL_BATCH_LIMIT as usize;
    rows.truncate(BACKFILL_BATCH_LIMIT as usize);
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("begin observer time backfill: {error}"))?;
    for (event_id, raw_json) in rows {
        let observed_bounds = Event::from_json(&raw_json)
            .ok()
            .filter(|event| event.id.to_hex() == event_id && event.verify().is_ok())
            .and_then(|event| {
                buzz_core_pkg::observer::decrypt_observer_payload::<serde_json::Value>(
                    owner_keys, &event,
                )
                .ok()
            })
            .map(|value| bounds(&value))
            .unwrap_or((None, None));
        upsert(
            &tx,
            identity_pubkey,
            relay_url,
            &event_id,
            observed_bounds.0,
            observed_bounds.1,
        )?;
    }
    tx.commit()
        .map_err(|error| format!("commit observer time backfill: {error}"))?;
    Ok(complete)
}

/// Read owner-scoped observer events whose decrypted inner timestamps overlap
/// a half-open time range. The compound outer cursor remains stable for paging.
#[allow(clippy::too_many_arguments)]
pub(super) fn read_archived_observer_events_for_range(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start_created_at: i64,
    end_created_at: i64,
    agent_pubkey: Option<&str>,
    channel_id: Option<&str>,
    before_created_at: Option<i64>,
    before_id: Option<&str>,
    limit: i64,
) -> Result<Vec<String>, String> {
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(identity_pubkey.to_owned()),
        Box::new(relay_url.to_owned()),
        Box::new(identity_pubkey.to_owned()),
        Box::new(start_created_at),
        Box::new(end_created_at),
    ];
    let mut clauses = String::new();
    if let Some(agent) = agent_pubkey {
        values.push(Box::new(agent.to_owned()));
        clauses.push_str(&format!(" AND ae.pubkey = ?{}", values.len()));
    }
    if let Some(channel) = channel_id {
        values.push(Box::new(channel.to_owned()));
        clauses.push_str(&format!(
            " AND EXISTS (
                SELECT 1 FROM observer_channel_index oci
                 WHERE oci.identity_pubkey = ae.identity_pubkey
                   AND oci.relay_url = ae.relay_url
                   AND oci.id = ae.id
                   AND oci.channel_id = ?{})",
            values.len()
        ));
    }
    if let (Some(created_at), Some(id)) = (before_created_at, before_id) {
        values.push(Box::new(created_at));
        let created_at_slot = values.len();
        values.push(Box::new(id.to_owned()));
        let id_slot = values.len();
        clauses.push_str(&format!(
            " AND (ae.created_at < ?{created_at_slot}
               OR (ae.created_at = ?{created_at_slot} AND ae.id < ?{id_slot}))"
        ));
    }
    values.push(Box::new(limit));
    let limit_slot = values.len();
    let sql = format!(
        "SELECT ae.raw_json
           FROM archived_events ae
           JOIN archived_event_scopes aes USING (identity_pubkey, relay_url, id)
           JOIN observer_time_index oti USING (identity_pubkey, relay_url, id)
          WHERE ae.identity_pubkey = ?1 AND ae.relay_url = ?2
            AND aes.scope_type = 'owner_p' AND aes.scope_value = ?3
            AND ae.kind = 24200 AND oti.observed_start_at IS NOT NULL
            AND oti.observed_end_at >= ?4 AND oti.observed_start_at < ?5
            {clauses}
          ORDER BY ae.created_at DESC, ae.id DESC LIMIT ?{limit_slot}"
    );
    let refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|value| value.as_ref()).collect();
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|error| format!("prepare observer inner-time range: {error}"))?;
    let rows = stmt
        .query_map(refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|error| format!("query observer inner-time range: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read observer inner-time range row: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    const RELAY: &str = "wss://relay.example";

    fn signed_observer(owner: &Keys, agent: &Keys, timestamp: &str) -> Event {
        let payload = serde_json::json!({
            "seq": 1,
            "timestamp": timestamp,
            "kind": "turn_started",
            "channelId": "channel-1",
            "turnId": "turn-1",
            "payload": {}
        });
        let ciphertext =
            buzz_core_pkg::observer::encrypt_observer_payload(agent, &owner.public_key(), &payload)
                .unwrap();
        EventBuilder::new(Kind::Custom(24200), ciphertext)
            .tags([
                Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent.public_key().to_hex()]).unwrap(),
                Tag::parse(["frame", "telemetry"]).unwrap(),
            ])
            .sign_with_keys(agent)
            .unwrap()
    }

    fn archive(conn: &Connection, owner: &Keys, event: &Event) {
        let owner_pubkey = owner.public_key().to_hex();
        store::upsert_archived_event(
            conn,
            &owner_pubkey,
            RELAY,
            &event.id.to_hex(),
            24200,
            &event.pubkey.to_hex(),
            10_000,
            &event.as_json(),
            10_000,
        )
        .unwrap();
        store::upsert_event_scope(
            conn,
            &owner_pubkey,
            RELAY,
            &event.id.to_hex(),
            "owner_p",
            &owner_pubkey,
            10_000,
        )
        .unwrap();
    }

    #[test]
    fn batch_bounds_follow_inner_events_not_outer_publication_time() {
        let value = serde_json::json!({
            "kind": "batch",
            "timestamp": "2026-08-22T10:00:00Z",
            "payload": { "events": [
                { "kind": "turn_started", "timestamp": "2026-08-21T23:59:58Z" },
                { "kind": "turn_completed", "timestamp": "2026-08-22T00:00:02Z" }
            ] }
        });
        assert_eq!(bounds(&value), (Some(1_787_356_798), Some(1_787_356_802)));
    }

    #[test]
    fn historical_backfill_is_durable_idempotent_and_uses_inner_time() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        let owner = Keys::generate();
        let agent = Keys::generate();
        let owner_pubkey = owner.public_key().to_hex();
        let event = signed_observer(&owner, &agent, "2026-08-21T23:59:59Z");
        archive(&conn, &owner, &event);

        assert!(backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
        assert!(backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
        let rows = read_archived_observer_events_for_range(
            &conn,
            &owner_pubkey,
            RELAY,
            1_787_356_799,
            1_787_356_800,
            None,
            None,
            None,
            None,
            10,
        )
        .unwrap();
        assert_eq!(rows, [event.as_json()]);
    }

    #[test]
    fn invalid_historical_row_gets_durable_null_marker() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        let owner = Keys::generate();
        let owner_pubkey = owner.public_key().to_hex();
        store::upsert_archived_event(
            &conn,
            &owner_pubkey,
            RELAY,
            "bad",
            24200,
            "agent",
            10_000,
            "{}",
            10_000,
        )
        .unwrap();
        store::upsert_event_scope(
            &conn,
            &owner_pubkey,
            RELAY,
            "bad",
            "owner_p",
            &owner_pubkey,
            10_000,
        )
        .unwrap();

        assert!(backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
        assert!(backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
        let bounds: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT observed_start_at, observed_end_at FROM observer_time_index WHERE id='bad'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(bounds, (None, None));
    }

    #[test]
    fn historical_backfill_is_bounded_and_resumable() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        let owner = Keys::generate();
        let owner_pubkey = owner.public_key().to_hex();
        for index in 0..=BACKFILL_BATCH_LIMIT {
            let id = format!("bad-{index:04}");
            store::upsert_archived_event(
                &conn,
                &owner_pubkey,
                RELAY,
                &id,
                24200,
                "agent",
                index,
                "{}",
                index,
            )
            .unwrap();
            store::upsert_event_scope(
                &conn,
                &owner_pubkey,
                RELAY,
                &id,
                "owner_p",
                &owner_pubkey,
                index,
            )
            .unwrap();
        }

        assert!(!backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
        assert!(backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
        assert!(backfill_missing(&conn, &owner_pubkey, RELAY, &owner).unwrap());
    }
}
