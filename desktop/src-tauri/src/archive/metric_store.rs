//! Parsed index of kind 44200 (NIP-AM agent turn metric) archive rows.
//!
//! `agent_metric_index` is a derived, rebuildable cache of parsed NIP-AM
//! payload fields, keyed by `(identity_pubkey, relay_url, id)` exactly like
//! `archived_events`. It exists so the accounting algorithm in
//! `agent_usage.rs` can query parsed columns instead of re-parsing JSON on
//! every render. The canonical source of truth remains
//! `archived_events.raw_json`; every row here is reproducible from it alone
//! via [`AgentMetricIndexRow::from_payload`].
//!
//! Kept in a sibling file (not `store.rs`) to keep that file under the
//! 1000-line gate, per the existing `pipeline.rs` precedent.

use rusqlite::{params, Connection, OptionalExtension};

use buzz_core_pkg::agent_turn_metric::AgentTurnMetricPayload;

// ── u64-safe sortable encoding ───────────────────────────────────────────────

/// Fixed-width digit count for the lexicographically order-preserving decimal
/// encoding of a `u64`. `u64::MAX` = 18446744073709551615 is 20 digits.
const U64_SORTABLE_WIDTH: usize = 20;

/// Encode a `u64` as a fixed-width zero-padded decimal string so SQLite TEXT
/// ordering matches numeric ordering, and so the full `u64` range survives
/// SQLite's signed-`i64` INTEGER column type (rusqlite has no unsigned
/// binding). Used for both token counters and `turn_seq`.
pub(super) fn encode_u64_sortable(value: u64) -> String {
    format!("{value:0U64_SORTABLE_WIDTH$}")
}

/// Decode a value written by [`encode_u64_sortable`]. Returns `None` if the
/// string is not a well-formed same-width decimal `u64` — defensive only;
/// every value written by this module is always well-formed.
pub(super) fn decode_u64_sortable(text: &str) -> Option<u64> {
    if text.len() != U64_SORTABLE_WIDTH {
        return None;
    }
    text.parse::<u64>().ok()
}

fn parse_rfc3339_secs(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp())
}

// ── Row type ──────────────────────────────────────────────────────────────

/// Parse status of a stored `agent_metric_index` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseStatus {
    Valid,
    Invalid,
}

impl ParseStatus {
    fn as_str(self) -> &'static str {
        match self {
            ParseStatus::Valid => "valid",
            ParseStatus::Invalid => "invalid",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "valid" => ParseStatus::Valid,
            _ => ParseStatus::Invalid,
        }
    }
}

/// One fully parsed `agent_metric_index` row.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AgentMetricIndexRow {
    pub id: String,
    pub agent_pubkey: String,
    pub event_created_at: i64,
    pub archived_at: i64,
    pub reported_at: Option<i64>,
    pub session_id: Option<String>,
    pub turn_seq: Option<u64>,
    pub model: Option<String>,
    pub delta_reliable: Option<bool>,
    pub turn_input_tokens: Option<u64>,
    pub turn_output_tokens: Option<u64>,
    pub turn_total_tokens: Option<u64>,
    pub turn_cost_usd: Option<f64>,
    pub cumulative_input_tokens: Option<u64>,
    pub cumulative_output_tokens: Option<u64>,
    pub cumulative_total_tokens: Option<u64>,
    pub cumulative_cost_usd: Option<f64>,
    pub parse_status: ParseStatus,
}

impl AgentMetricIndexRow {
    /// Parse a decrypted NIP-AM payload (the plaintext JSON already stored in
    /// `archived_events.raw_json` for kind 44200 rows) into an index row.
    ///
    /// New ingest and backfill share this single parser so validity rules
    /// cannot drift between the two paths (frozen plan requirement).
    ///
    /// "Invalid" per Rev 2 A5: the payload's JSON decodes but fails a
    /// semantic check this layer owns: unparseable RFC3339 `timestamp`, or
    /// `cumulative` present without both `sessionId` and `turnSeq` (NIP-AM
    /// REQUIREs both whenever `cumulative` is present — a row missing either
    /// cannot supply the complete `(agent, session, seq)` key needed to
    /// compete as a cumulative snapshot). The upstream fail-closed ingest
    /// path (`pipeline.rs::commit_archive`) has already decrypted,
    /// deserialized, and numeric-validated (non-negative/finite `costUsd`)
    /// before this row is ever produced — a raw-JSON parse failure here
    /// would indicate on-disk corruption, not a normal producer error, but
    /// is still handled fail-closed rather than panicking.
    pub(super) fn from_payload(
        raw_json: &str,
        id: &str,
        agent_pubkey: &str,
        event_created_at: i64,
        archived_at: i64,
    ) -> Self {
        let Ok(payload) = serde_json::from_str::<AgentTurnMetricPayload>(raw_json) else {
            return Self::invalid(id, agent_pubkey, event_created_at, archived_at);
        };

        let reported_at = parse_rfc3339_secs(&payload.timestamp);
        let cumulative_requires_session_seq = payload.cumulative.is_some();
        let has_session_and_seq = payload.session_id.is_some() && payload.turn_seq.is_some();

        if reported_at.is_none() || (cumulative_requires_session_seq && !has_session_and_seq) {
            return Self::invalid(id, agent_pubkey, event_created_at, archived_at);
        }

        let turn = payload.turn.as_ref();
        let cumulative = payload.cumulative.as_ref();

        Self {
            id: id.to_string(),
            agent_pubkey: agent_pubkey.to_string(),
            event_created_at,
            archived_at,
            reported_at,
            session_id: payload.session_id,
            turn_seq: payload.turn_seq,
            model: payload.model,
            delta_reliable: Some(payload.delta_reliable),
            turn_input_tokens: turn.and_then(|t| t.input_tokens),
            turn_output_tokens: turn.and_then(|t| t.output_tokens),
            turn_total_tokens: turn.and_then(|t| t.total_tokens),
            turn_cost_usd: turn.and_then(|t| t.cost_usd),
            cumulative_input_tokens: cumulative.and_then(|c| c.input_tokens),
            cumulative_output_tokens: cumulative.and_then(|c| c.output_tokens),
            cumulative_total_tokens: cumulative.and_then(|c| c.total_tokens),
            cumulative_cost_usd: cumulative.and_then(|c| c.cost_usd),
            parse_status: ParseStatus::Valid,
        }
    }

    fn invalid(id: &str, agent_pubkey: &str, event_created_at: i64, archived_at: i64) -> Self {
        Self {
            id: id.to_string(),
            agent_pubkey: agent_pubkey.to_string(),
            event_created_at,
            archived_at,
            reported_at: None,
            session_id: None,
            turn_seq: None,
            model: None,
            delta_reliable: None,
            turn_input_tokens: None,
            turn_output_tokens: None,
            turn_total_tokens: None,
            turn_cost_usd: None,
            cumulative_input_tokens: None,
            cumulative_output_tokens: None,
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            parse_status: ParseStatus::Invalid,
        }
    }

    /// The `(agent_pubkey, session_id, turn_seq)` cumulative-accounting key,
    /// or `None` if this row cannot participate in cumulative delta
    /// recomputation (missing session/sequence).
    pub(super) fn accounting_key(&self) -> Option<(String, String, u64)> {
        match (&self.session_id, self.turn_seq) {
            (Some(sid), Some(seq)) => Some((self.agent_pubkey.clone(), sid.clone(), seq)),
            _ => None,
        }
    }
}

fn row_from_sql(row: &rusqlite::Row) -> rusqlite::Result<AgentMetricIndexRow> {
    let turn_seq_text: Option<String> = row.get("turn_seq")?;
    let turn_input_text: Option<String> = row.get("turn_input_tokens")?;
    let turn_output_text: Option<String> = row.get("turn_output_tokens")?;
    let turn_total_text: Option<String> = row.get("turn_total_tokens")?;
    let cum_input_text: Option<String> = row.get("cumulative_input_tokens")?;
    let cum_output_text: Option<String> = row.get("cumulative_output_tokens")?;
    let cum_total_text: Option<String> = row.get("cumulative_total_tokens")?;
    let delta_reliable_int: Option<i64> = row.get("delta_reliable")?;
    let parse_status_str: String = row.get("parse_status")?;

    Ok(AgentMetricIndexRow {
        id: row.get("id")?,
        agent_pubkey: row.get("agent_pubkey")?,
        event_created_at: row.get("event_created_at")?,
        archived_at: row.get("archived_at")?,
        reported_at: row.get("reported_at")?,
        session_id: row.get("session_id")?,
        turn_seq: turn_seq_text.as_deref().and_then(decode_u64_sortable),
        model: row.get("model")?,
        delta_reliable: delta_reliable_int.map(|v| v != 0),
        turn_input_tokens: turn_input_text.as_deref().and_then(decode_u64_sortable),
        turn_output_tokens: turn_output_text.as_deref().and_then(decode_u64_sortable),
        turn_total_tokens: turn_total_text.as_deref().and_then(decode_u64_sortable),
        turn_cost_usd: row.get("turn_cost_usd")?,
        cumulative_input_tokens: cum_input_text.as_deref().and_then(decode_u64_sortable),
        cumulative_output_tokens: cum_output_text.as_deref().and_then(decode_u64_sortable),
        cumulative_total_tokens: cum_total_text.as_deref().and_then(decode_u64_sortable),
        cumulative_cost_usd: row.get("cumulative_cost_usd")?,
        parse_status: ParseStatus::from_str(&parse_status_str),
    })
}

const ROW_COLUMNS: &str = "id, agent_pubkey, event_created_at, archived_at, reported_at, \
     session_id, turn_seq, model, delta_reliable, turn_input_tokens, turn_output_tokens, \
     turn_total_tokens, turn_cost_usd, cumulative_input_tokens, cumulative_output_tokens, \
     cumulative_total_tokens, cumulative_cost_usd, parse_status";

// ── Write path ───────────────────────────────────────────────────────────────

/// Insert one metric index row inside the caller's transaction.
///
/// Called from `pipeline::commit_archive` ONLY when the corresponding
/// `archived_events` row was newly inserted (never for a duplicate), and
/// from the backfill driver for pre-existing unindexed rows. `INSERT OR
/// IGNORE` on the shared PK makes a second call for the same
/// `(identity, relay, id)` a safe no-op (defensive; callers already guard
/// against re-indexing).
pub(super) fn insert_metric_index_row(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    row: &AgentMetricIndexRow,
) -> Result<bool, String> {
    let turn_seq = row.turn_seq.map(encode_u64_sortable);
    let turn_input = row.turn_input_tokens.map(encode_u64_sortable);
    let turn_output = row.turn_output_tokens.map(encode_u64_sortable);
    let turn_total = row.turn_total_tokens.map(encode_u64_sortable);
    let cum_input = row.cumulative_input_tokens.map(encode_u64_sortable);
    let cum_output = row.cumulative_output_tokens.map(encode_u64_sortable);
    let cum_total = row.cumulative_total_tokens.map(encode_u64_sortable);
    let delta_reliable = row.delta_reliable.map(|b| b as i64);

    let affected = conn
        .execute(
            "INSERT INTO agent_metric_index
                 (identity_pubkey, relay_url, id, agent_pubkey, event_created_at,
                  archived_at, reported_at, session_id, turn_seq, model,
                  delta_reliable, turn_input_tokens, turn_output_tokens,
                  turn_total_tokens, turn_cost_usd, cumulative_input_tokens,
                  cumulative_output_tokens, cumulative_total_tokens,
                  cumulative_cost_usd, parse_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
             ON CONFLICT (identity_pubkey, relay_url, id) DO NOTHING",
            params![
                identity_pubkey,
                relay_url,
                row.id,
                row.agent_pubkey,
                row.event_created_at,
                row.archived_at,
                row.reported_at,
                row.session_id,
                turn_seq,
                row.model,
                delta_reliable,
                turn_input,
                turn_output,
                turn_total,
                row.turn_cost_usd,
                cum_input,
                cum_output,
                cum_total,
                row.cumulative_cost_usd,
                row.parse_status.as_str(),
            ],
        )
        .map_err(|e| format!("failed to insert agent_metric_index row: {e}"))?;
    Ok(affected > 0)
}

// ── Backfill ─────────────────────────────────────────────────────────────────

/// Backfill existing `archived_events` kind-44200 rows that have no matching
/// `agent_metric_index` row yet, for the given identity + relay.
///
/// Runs in bounded chunks (~500 rows per transaction) so a large existing
/// archive never holds one unbounded write lock; each chunk is independently
/// atomic and the whole backfill is idempotent (anti-join + index PK is the
/// source of truth) and restartable — interruption between chunks loses
/// nothing, and a later run simply resumes against the still-missing rows.
///
/// Returns the total number of newly indexed rows.
pub(super) fn backfill_agent_metric_index(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<usize, String> {
    const CHUNK_SIZE: i64 = 500;
    let mut total = 0usize;

    loop {
        let mut stmt = conn
            .prepare(
                "SELECT ae.id, ae.pubkey, ae.created_at, ae.archived_at, ae.raw_json
                 FROM archived_events ae
                 WHERE ae.identity_pubkey = ?1
                   AND ae.relay_url       = ?2
                   AND ae.kind            = 44200
                   AND ae.id NOT IN (
                       SELECT id FROM agent_metric_index
                       WHERE identity_pubkey = ?1
                         AND relay_url       = ?2
                   )
                 ORDER BY ae.created_at ASC, ae.id ASC
                 LIMIT ?3",
            )
            .map_err(|e| format!("prepare backfill_agent_metric_index select: {e}"))?;

        let chunk: Vec<(String, String, i64, i64, String)> = stmt
            .query_map(params![identity_pubkey, relay_url, CHUNK_SIZE], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| format!("query backfill_agent_metric_index select: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read backfill_agent_metric_index row: {e}"))?;
        drop(stmt);

        if chunk.is_empty() {
            break;
        }
        let chunk_len = chunk.len();

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("failed to begin backfill chunk transaction: {e}"))?;
        for (id, pubkey, created_at, archived_at, raw_json) in &chunk {
            let parsed =
                AgentMetricIndexRow::from_payload(raw_json, id, pubkey, *created_at, *archived_at);
            insert_metric_index_row(&tx, identity_pubkey, relay_url, &parsed)?;
        }
        tx.commit()
            .map_err(|e| format!("failed to commit backfill chunk: {e}"))?;

        total += chunk_len;
        if (chunk_len as i64) < CHUNK_SIZE {
            break;
        }
    }

    Ok(total)
}

// ── GC / orphan repair ───────────────────────────────────────────────────────

/// Delete `agent_metric_index` rows whose canonical `archived_events` row no
/// longer exists. Called from `store::gc_orphaned_events` inside the SAME
/// SQLite transaction as the canonical delete (A6) so the index can never
/// observe a canonical row as gone while its own row survives.
pub(super) fn delete_orphaned_metric_index_rows(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<usize, String> {
    let affected = conn
        .execute(
            "DELETE FROM agent_metric_index
             WHERE identity_pubkey = ?1
               AND relay_url       = ?2
               AND id NOT IN (
                   SELECT id FROM archived_events
                   WHERE identity_pubkey = ?1
                     AND relay_url       = ?2
               )",
            params![identity_pubkey, relay_url],
        )
        .map_err(|e| format!("failed to gc orphaned agent_metric_index rows: {e}"))?;
    Ok(affected)
}

/// Read-time orphan repair: same anti-join delete as
/// [`delete_orphaned_metric_index_rows`], run defensively before every read
/// so a planted/legacy orphan is self-healed even if a future deletion path
/// forgets to call the GC cascade. Defense in depth, not an atomicity
/// substitute for A6.
pub(super) fn repair_orphaned_metric_index_rows(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<usize, String> {
    delete_orphaned_metric_index_rows(conn, identity_pubkey, relay_url)
}

// ── Read path ────────────────────────────────────────────────────────────────

/// Load all VALID rows whose `reported_at` falls in `[start, end)`, optionally
/// filtered to one agent author. This is the exact set of rows that may ever
/// be counted into a bucket (A11 step 1).
pub(super) fn load_window_valid_rows(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start: i64,
    end: i64,
    agent_pubkey: Option<&str>,
) -> Result<Vec<AgentMetricIndexRow>, String> {
    let sql = format!(
        "SELECT {ROW_COLUMNS} FROM agent_metric_index
         WHERE identity_pubkey = ?1 AND relay_url = ?2 AND parse_status = 'valid'
           AND reported_at >= ?3 AND reported_at < ?4
           AND (?5 IS NULL OR agent_pubkey = ?5)
         ORDER BY reported_at ASC, id ASC"
    );
    let mut stmt = stmt_prepare(conn, &sql)?;
    let rows = stmt
        .query_map(
            params![identity_pubkey, relay_url, start, end, agent_pubkey],
            row_from_sql,
        )
        .map_err(|e| format!("query load_window_valid_rows: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read load_window_valid_rows row: {e}"))
}

/// Count INVALID rows whose `event_created_at` falls in `[start, end)`
/// (invalid rows have no trustworthy `reported_at`, so coarse signed
/// `created_at` is the only available time signal), optionally filtered to
/// one agent author.
pub(super) fn count_invalid_rows_in_window(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    start: i64,
    end: i64,
    agent_pubkey: Option<&str>,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM agent_metric_index
         WHERE identity_pubkey = ?1 AND relay_url = ?2 AND parse_status = 'invalid'
           AND event_created_at >= ?3 AND event_created_at < ?4
           AND (?5 IS NULL OR agent_pubkey = ?5)",
        params![identity_pubkey, relay_url, start, end, agent_pubkey],
        |row| row.get(0),
    )
    .map_err(|e| format!("count_invalid_rows_in_window: {e}"))
}

/// For the given set of exact `(agent_pubkey, session_id, turn_seq)` keys,
/// load ALL valid rows matching those exact keys with NO `reported_at`
/// restriction (A11). Used both for duplicate-cardinality checks at a
/// sequence and for exact-predecessor baseline lookups — a single probe
/// covers both, since the predecessor key is included in the request set
/// alongside each window row's own key.
///
/// Keys are grouped by `(agent_pubkey, session_id)` and queried with a
/// `turn_seq IN (...)` clause per group (typically few groups per window),
/// served by `idx_agent_metric_session`.
pub(super) fn load_rows_at_exact_keys(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    keys: &std::collections::HashSet<(String, String, u64)>,
) -> Result<Vec<AgentMetricIndexRow>, String> {
    use std::collections::HashMap;

    // Group by (agent, session) so each group becomes one IN-list query.
    let mut groups: HashMap<(String, String), Vec<u64>> = HashMap::new();
    for (agent, session, seq) in keys {
        groups
            .entry((agent.clone(), session.clone()))
            .or_default()
            .push(*seq);
    }

    let mut out = Vec::new();
    for ((agent, session), seqs) in groups {
        let encoded: Vec<String> = seqs.into_iter().map(encode_u64_sortable).collect();
        let sql = format!(
            "SELECT {ROW_COLUMNS} FROM agent_metric_index
             WHERE identity_pubkey = ?1 AND relay_url = ?2 AND parse_status = 'valid'
               AND agent_pubkey = ?3 AND session_id = ?4
               AND turn_seq IN ({})",
            encoded
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 5))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut stmt = stmt_prepare(conn, &sql)?;
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(identity_pubkey.to_owned()),
            Box::new(relay_url.to_owned()),
            Box::new(agent.clone()),
            Box::new(session.clone()),
        ];
        for e in &encoded {
            bound.push(Box::new(e.clone()));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), row_from_sql)
            .map_err(|e| format!("query load_rows_at_exact_keys: {e}"))?;
        for r in rows {
            out.push(r.map_err(|e| format!("read load_rows_at_exact_keys row: {e}"))?);
        }
    }

    Ok(out)
}

/// `hasArchivedEvidence` (A13): does at least one surviving `agent_metric_index`
/// row (either `parse_status`) exist for this author under the active
/// identity+relay, with NO bucket-boundary restriction? Computed after
/// backfill + orphan repair by the caller.
pub(super) fn has_archived_evidence(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    agent_pubkey: &str,
) -> Result<bool, String> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM agent_metric_index
             WHERE identity_pubkey = ?1 AND relay_url = ?2 AND agent_pubkey = ?3
             LIMIT 1",
            params![identity_pubkey, relay_url, agent_pubkey],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("has_archived_evidence: {e}"))?;
    Ok(exists.is_some())
}

fn stmt_prepare<'a>(conn: &'a Connection, sql: &str) -> Result<rusqlite::Statement<'a>, String> {
    conn.prepare(sql)
        .map_err(|e| format!("prepare failed: {e} — sql: {sql}"))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "metric_store_tests.rs"]
mod metric_store_tests;
