//! Signed, durable authority records for Activity Ledger journals.
//!
//! Observer frames are evidence, not authority. These records let the active
//! owner explicitly override a journal summary or independently attest that a
//! receipt verifies a journal. The complete Nostr event is stored locally and
//! its id, signature, signer, schema, tags, and content bindings are validated
//! both before insertion and on every read.

use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const KIND_JOURNAL_AUTHORITY: u16 = 24201;
const ARTIFACT_SCHEMA: &str = "buzz.activity-journal-authority/v3";
const ARTIFACT_MARKER: &str = "buzz-activity-journal";
const MAX_RELAY_URL_BYTES: usize = 2_048;
const MAX_JOURNAL_ID_CHARS: usize = 512;
const MAX_CORRELATION_ID_CHARS: usize = 512;
const MAX_TEXT_CHARS: usize = 20_000;
const MAX_RECEIPT_REF_CHARS: usize = 2_048;
const MAX_SOURCE_EVENTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalAuthorityArtifactType {
    OwnerOverride,
    Verification,
}

impl JournalAuthorityArtifactType {
    fn as_str(self) -> &'static str {
        match self {
            Self::OwnerOverride => "owner_override",
            Self::Verification => "verification",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedArtifactContent {
    schema: String,
    relay_url: String,
    agent_pubkey: String,
    artifact_type: JournalAuthorityArtifactType,
    journal_id: String,
    correlation_id: String,
    revision: i64,
    summary: Option<String>,
    note: Option<String>,
    receipt_ref: Option<String>,
    source_event_ids: Vec<String>,
}

/// Validated wire response. Secret key material is never serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalAuthorityArtifact {
    pub owner_pubkey: String,
    pub relay_url: String,
    pub agent_pubkey: String,
    pub event_id: String,
    pub signature: String,
    pub created_at: i64,
    pub artifact_type: JournalAuthorityArtifactType,
    pub journal_id: String,
    pub correlation_id: String,
    pub revision: i64,
    pub summary: Option<String>,
    pub note: Option<String>,
    pub receipt_ref: Option<String>,
    pub source_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerJournalOverrideInput {
    pub agent_pubkey: String,
    pub journal_id: String,
    pub correlation_id: String,
    pub summary: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalVerificationInput {
    pub agent_pubkey: String,
    pub journal_id: String,
    pub correlation_id: String,
    pub receipt_ref: String,
    pub source_event_ids: Vec<String>,
}

#[derive(Debug)]
struct StoredArtifactRow {
    identity_pubkey: String,
    relay_url: String,
    agent_pubkey: String,
    journal_id: String,
    artifact_type: String,
    event_id: String,
    created_at: i64,
    revision: i64,
    raw_json: String,
}

pub fn normalize_relay_scope(relay_url: &str) -> Result<String, String> {
    if relay_url.is_empty() || relay_url.len() > MAX_RELAY_URL_BYTES {
        return Err(format!(
            "journal authority relay URL must contain between 1 and {MAX_RELAY_URL_BYTES} bytes"
        ));
    }
    buzz_core_pkg::relay::normalize_relay_url(relay_url)
        .map_err(|error| format!("journal authority relay URL is invalid: {error}"))
}

pub fn normalize_agent_scope(agent_pubkey: &str) -> Result<String, String> {
    PublicKey::from_hex(agent_pubkey.trim())
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| format!("journal authority managed agent pubkey is invalid: {error}"))
}

fn checked_nonempty(value: &str, label: &str, max_chars: usize) -> Result<String, String> {
    let value = value.trim();
    let len = value.chars().count();
    if len == 0 || len > max_chars {
        return Err(format!(
            "{label} must contain between 1 and {max_chars} characters"
        ));
    }
    Ok(value.to_owned())
}

fn checked_optional_text(
    value: Option<&str>,
    label: &str,
    max_chars: usize,
) -> Result<Option<String>, String> {
    value
        .map(|text| checked_nonempty(text, label, max_chars))
        .transpose()
}

fn normalize_source_event_ids(values: &[String]) -> Result<Vec<String>, String> {
    if values.is_empty() || values.len() > MAX_SOURCE_EVENTS {
        return Err(format!(
            "verification must bind between 1 and {MAX_SOURCE_EVENTS} source event IDs"
        ));
    }
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim().to_ascii_lowercase();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("source event IDs must be 64-character hexadecimal Nostr IDs".into());
        }
        normalized.push(value);
    }
    normalized.sort();
    normalized.dedup();
    if normalized.len() != values.len() {
        return Err("verification source event IDs must be unique".into());
    }
    Ok(normalized)
}

fn single_tag(event: &Event, name: &str) -> Result<String, String> {
    let values = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.len() == 2 && parts[0] == name).then(|| parts[1].clone())
        })
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(format!(
            "journal authority event must contain exactly one {name:?} tag"
        ));
    }
    Ok(values[0].clone())
}

fn repeated_tags(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.len() == 2 && parts[0] == name).then(|| parts[1].clone())
        })
        .collect()
}

fn validate_content(content: &SignedArtifactContent) -> Result<(), String> {
    if content.schema != ARTIFACT_SCHEMA {
        return Err("unsupported journal authority artifact schema".into());
    }
    if normalize_relay_scope(&content.relay_url)? != content.relay_url {
        return Err("journal authority relay URL must be canonical".into());
    }
    if normalize_agent_scope(&content.agent_pubkey)? != content.agent_pubkey {
        return Err("journal authority managed agent pubkey must be canonical".into());
    }
    checked_nonempty(&content.journal_id, "journalId", MAX_JOURNAL_ID_CHARS)?;
    checked_nonempty(
        &content.correlation_id,
        "correlationId",
        MAX_CORRELATION_ID_CHARS,
    )?;
    if content.correlation_id != content.journal_id {
        return Err("journal authority correlationId must equal the stable journalId".into());
    }
    if content.revision < 1 {
        return Err("journal authority revision must be positive".into());
    }
    match content.artifact_type {
        JournalAuthorityArtifactType::OwnerOverride => {
            let summary = content
                .summary
                .as_deref()
                .ok_or_else(|| "owner override is missing summary".to_string())?;
            checked_nonempty(summary, "summary", MAX_TEXT_CHARS)?;
            checked_optional_text(content.note.as_deref(), "note", MAX_TEXT_CHARS)?;
            if content.receipt_ref.is_some() || !content.source_event_ids.is_empty() {
                return Err("owner override cannot contain verification evidence".into());
            }
        }
        JournalAuthorityArtifactType::Verification => {
            if content.summary.is_some() || content.note.is_some() {
                return Err("verification artifact cannot override owner text".into());
            }
            let receipt_ref = content
                .receipt_ref
                .as_deref()
                .ok_or_else(|| "verification artifact is missing receiptRef".to_string())?;
            checked_nonempty(receipt_ref, "receiptRef", MAX_RECEIPT_REF_CHARS)?;
            let normalized = normalize_source_event_ids(&content.source_event_ids)?;
            if normalized != content.source_event_ids {
                return Err("verification source event IDs must be sorted and normalized".into());
            }
        }
    }
    Ok(())
}

fn artifact_from_event(event: &Event, content: SignedArtifactContent) -> JournalAuthorityArtifact {
    JournalAuthorityArtifact {
        owner_pubkey: event.pubkey.to_hex(),
        relay_url: content.relay_url,
        agent_pubkey: content.agent_pubkey,
        event_id: event.id.to_hex(),
        signature: event.sig.to_string(),
        created_at: event.created_at.as_secs() as i64,
        artifact_type: content.artifact_type,
        journal_id: content.journal_id,
        correlation_id: content.correlation_id,
        revision: content.revision,
        summary: content.summary,
        note: content.note,
        receipt_ref: content.receipt_ref,
        source_event_ids: content.source_event_ids,
    }
}

/// Parse and verify every signed field. This is called on insert and read.
pub fn validate_signed_artifact(
    raw_json: &str,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    expected_agent_pubkey: &str,
) -> Result<JournalAuthorityArtifact, String> {
    let expected_relay_url = normalize_relay_scope(expected_relay_url)?;
    let expected_agent_pubkey = normalize_agent_scope(expected_agent_pubkey)?;
    let event = Event::from_json(raw_json)
        .map_err(|error| format!("parse journal authority event: {error}"))?;
    event
        .verify()
        .map_err(|error| format!("journal authority signature verification failed: {error}"))?;
    if event.kind.as_u16() != KIND_JOURNAL_AUTHORITY {
        return Err(format!(
            "journal authority event must use kind {KIND_JOURNAL_AUTHORITY}"
        ));
    }
    if event.pubkey.to_hex() != expected_owner_pubkey {
        return Err("journal authority event signer is not the active owner identity".into());
    }

    let content: SignedArtifactContent = serde_json::from_str(&event.content)
        .map_err(|error| format!("parse journal authority content: {error}"))?;
    validate_content(&content)?;

    if content.relay_url != expected_relay_url {
        return Err("journal authority event relay is not the active relay".into());
    }
    if content.agent_pubkey != expected_agent_pubkey {
        return Err("journal authority event managed agent is not the requested agent".into());
    }

    if single_tag(&event, "t")? != ARTIFACT_MARKER
        || single_tag(&event, "relay_url")? != content.relay_url
        || single_tag(&event, "agent_pubkey")? != content.agent_pubkey
        || single_tag(&event, "artifact_type")? != content.artifact_type.as_str()
        || single_tag(&event, "journal_id")? != content.journal_id
        || single_tag(&event, "correlation_id")? != content.correlation_id
        || single_tag(&event, "revision")? != content.revision.to_string()
    {
        return Err("journal authority tags do not match signed content".into());
    }

    match content.artifact_type {
        JournalAuthorityArtifactType::OwnerOverride => {
            if !repeated_tags(&event, "receipt_ref").is_empty()
                || !repeated_tags(&event, "source_event").is_empty()
            {
                return Err("owner override contains verification-only tags".into());
            }
        }
        JournalAuthorityArtifactType::Verification => {
            if single_tag(&event, "receipt_ref")? != content.receipt_ref.as_deref().unwrap_or("") {
                return Err("verification receipt tag does not match signed content".into());
            }
            let mut tagged_sources = repeated_tags(&event, "source_event");
            tagged_sources.sort();
            if tagged_sources != content.source_event_ids {
                return Err("verification source-event tags do not match signed content".into());
            }
        }
    }

    Ok(artifact_from_event(&event, content))
}

fn tag(name: &str, value: &str) -> Result<Tag, String> {
    Tag::parse([name, value]).map_err(|error| format!("build {name} tag: {error}"))
}

fn build_signed_artifact(keys: &Keys, content: SignedArtifactContent) -> Result<String, String> {
    validate_content(&content)?;
    let mut tags = vec![
        tag("t", ARTIFACT_MARKER)?,
        tag("relay_url", &content.relay_url)?,
        tag("agent_pubkey", &content.agent_pubkey)?,
        tag("artifact_type", content.artifact_type.as_str())?,
        tag("journal_id", &content.journal_id)?,
        tag("correlation_id", &content.correlation_id)?,
        tag("revision", &content.revision.to_string())?,
    ];
    if let Some(receipt_ref) = &content.receipt_ref {
        tags.push(tag("receipt_ref", receipt_ref)?);
    }
    for source_event_id in &content.source_event_ids {
        tags.push(tag("source_event", source_event_id)?);
    }
    let content_json = serde_json::to_string(&content)
        .map_err(|error| format!("serialize journal authority content: {error}"))?;
    EventBuilder::new(Kind::Custom(KIND_JOURNAL_AUTHORITY), content_json)
        .tags(tags)
        .sign_with_keys(keys)
        .map(|event| event.as_json())
        .map_err(|error| format!("sign journal authority event: {error}"))
}

pub fn build_owner_override_event(
    keys: &Keys,
    relay_url: &str,
    input: &OwnerJournalOverrideInput,
    revision: i64,
) -> Result<String, String> {
    let content = SignedArtifactContent {
        schema: ARTIFACT_SCHEMA.to_string(),
        relay_url: normalize_relay_scope(relay_url)?,
        agent_pubkey: normalize_agent_scope(&input.agent_pubkey)?,
        artifact_type: JournalAuthorityArtifactType::OwnerOverride,
        journal_id: checked_nonempty(&input.journal_id, "journalId", MAX_JOURNAL_ID_CHARS)?,
        correlation_id: checked_nonempty(
            &input.correlation_id,
            "correlationId",
            MAX_CORRELATION_ID_CHARS,
        )?,
        revision,
        summary: Some(checked_nonempty(&input.summary, "summary", MAX_TEXT_CHARS)?),
        note: checked_optional_text(input.note.as_deref(), "note", MAX_TEXT_CHARS)?,
        receipt_ref: None,
        source_event_ids: Vec::new(),
    };
    build_signed_artifact(keys, content)
}

pub fn build_verification_event(
    keys: &Keys,
    relay_url: &str,
    input: &JournalVerificationInput,
    revision: i64,
) -> Result<String, String> {
    let content = SignedArtifactContent {
        schema: ARTIFACT_SCHEMA.to_string(),
        relay_url: normalize_relay_scope(relay_url)?,
        agent_pubkey: normalize_agent_scope(&input.agent_pubkey)?,
        artifact_type: JournalAuthorityArtifactType::Verification,
        journal_id: checked_nonempty(&input.journal_id, "journalId", MAX_JOURNAL_ID_CHARS)?,
        correlation_id: checked_nonempty(
            &input.correlation_id,
            "correlationId",
            MAX_CORRELATION_ID_CHARS,
        )?,
        revision,
        summary: None,
        note: None,
        receipt_ref: Some(checked_nonempty(
            &input.receipt_ref,
            "receiptRef",
            MAX_RECEIPT_REF_CHARS,
        )?),
        source_event_ids: normalize_source_event_ids(&input.source_event_ids)?,
    };
    build_signed_artifact(keys, content)
}

pub fn next_revision(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    agent_pubkey: &str,
    journal_id: &str,
    artifact_type: JournalAuthorityArtifactType,
) -> Result<i64, String> {
    let current = conn
        .query_row(
            "SELECT revision FROM journal_authority_artifacts
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND agent_pubkey = ?3 AND journal_id = ?4 AND artifact_type = ?5",
            params![
                identity_pubkey,
                relay_url,
                agent_pubkey,
                journal_id,
                artifact_type.as_str()
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("read journal authority revision: {error}"))?;
    current
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "journal authority revision overflow".to_string())
}

fn rollback(conn: &Connection) {
    let _ = conn.execute_batch("ROLLBACK");
}

/// Insert a first revision or replace the current row with exactly revision+1.
/// Re-inserting the exact same signed event is an idempotent success; a stale
/// but valid event is rejected so it cannot replay over newer authority state.
pub fn upsert_signed_artifact(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    agent_pubkey: &str,
    raw_json: &str,
    stored_at: i64,
) -> Result<JournalAuthorityArtifact, String> {
    let relay_url = normalize_relay_scope(relay_url)?;
    let agent_pubkey = normalize_agent_scope(agent_pubkey)?;
    let artifact = validate_signed_artifact(raw_json, identity_pubkey, &relay_url, &agent_pubkey)?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| format!("begin journal authority upsert: {error}"))?;
    let result = (|| -> Result<JournalAuthorityArtifact, String> {
        let current = conn
            .query_row(
                "SELECT event_id, revision FROM journal_authority_artifacts
                 WHERE identity_pubkey = ?1 AND relay_url = ?2
                   AND agent_pubkey = ?3 AND journal_id = ?4 AND artifact_type = ?5",
                params![
                    identity_pubkey,
                    relay_url,
                    agent_pubkey,
                    artifact.journal_id,
                    artifact.artifact_type.as_str()
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| format!("read current journal authority artifact: {error}"))?;

        if let Some((current_event_id, current_revision)) = current {
            if current_event_id == artifact.event_id {
                conn.execute(
                    "UPDATE journal_authority_artifacts SET raw_json = ?1, stored_at = ?2
                     WHERE identity_pubkey = ?3 AND relay_url = ?4
                       AND agent_pubkey = ?5 AND journal_id = ?6 AND artifact_type = ?7",
                    params![
                        raw_json,
                        stored_at,
                        identity_pubkey,
                        relay_url,
                        agent_pubkey,
                        artifact.journal_id,
                        artifact.artifact_type.as_str()
                    ],
                )
                .map_err(|error| format!("refresh journal authority artifact: {error}"))?;
                return Ok(artifact.clone());
            }
            let expected = current_revision
                .checked_add(1)
                .ok_or_else(|| "journal authority revision overflow".to_string())?;
            if artifact.revision != expected {
                return Err(format!(
                    "stale journal authority replay: expected revision {expected}, got {}",
                    artifact.revision
                ));
            }
        } else if artifact.revision != 1 {
            return Err(format!(
                "first journal authority revision must be 1, got {}",
                artifact.revision
            ));
        }

        conn.execute(
            "INSERT INTO journal_authority_artifacts
                 (identity_pubkey, relay_url, agent_pubkey, journal_id, artifact_type,
                  event_id, created_at, revision, raw_json, stored_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (identity_pubkey, relay_url, agent_pubkey, journal_id, artifact_type) DO UPDATE SET
                 event_id = excluded.event_id,
                 created_at = excluded.created_at,
                 revision = excluded.revision,
                 raw_json = excluded.raw_json,
                 stored_at = excluded.stored_at",
            params![
                identity_pubkey,
                relay_url,
                agent_pubkey,
                artifact.journal_id,
                artifact.artifact_type.as_str(),
                artifact.event_id,
                artifact.created_at,
                artifact.revision,
                raw_json,
                stored_at
            ],
        )
        .map_err(|error| format!("persist journal authority artifact: {error}"))?;
        Ok(artifact.clone())
    })();

    match result {
        Ok(artifact) => {
            if let Err(error) = conn.execute_batch("COMMIT") {
                rollback(conn);
                return Err(format!("commit journal authority artifact: {error}"));
            }
            Ok(artifact)
        }
        Err(error) => {
            rollback(conn);
            Err(error)
        }
    }
}

fn validate_stored_row(
    row: StoredArtifactRow,
    expected_identity: &str,
    expected_relay_url: &str,
    expected_agent_pubkey: &str,
) -> Result<JournalAuthorityArtifact, String> {
    let artifact = validate_signed_artifact(
        &row.raw_json,
        expected_identity,
        expected_relay_url,
        expected_agent_pubkey,
    )?;
    if row.identity_pubkey != expected_identity
        || row.relay_url != expected_relay_url
        || row.relay_url != artifact.relay_url
        || row.agent_pubkey != expected_agent_pubkey
        || row.agent_pubkey != artifact.agent_pubkey
        || row.journal_id != artifact.journal_id
        || row.artifact_type != artifact.artifact_type.as_str()
        || row.event_id != artifact.event_id
        || row.created_at != artifact.created_at
        || row.revision != artifact.revision
    {
        return Err("stored journal authority columns do not match signed event".into());
    }
    Ok(artifact)
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredArtifactRow> {
    Ok(StoredArtifactRow {
        identity_pubkey: row.get(0)?,
        relay_url: row.get(1)?,
        agent_pubkey: row.get(2)?,
        journal_id: row.get(3)?,
        artifact_type: row.get(4)?,
        event_id: row.get(5)?,
        created_at: row.get(6)?,
        revision: row.get(7)?,
        raw_json: row.get(8)?,
    })
}

pub fn get_journal_authority_artifacts(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    agent_pubkey: &str,
    journal_id: &str,
) -> Result<Vec<JournalAuthorityArtifact>, String> {
    let relay_url = normalize_relay_scope(relay_url)?;
    let agent_pubkey = normalize_agent_scope(agent_pubkey)?;
    let mut stmt = conn
        .prepare(
            "SELECT identity_pubkey, relay_url, agent_pubkey, journal_id, artifact_type, event_id,
                    created_at, revision, raw_json
             FROM journal_authority_artifacts
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND agent_pubkey = ?3 AND journal_id = ?4
             ORDER BY artifact_type ASC",
        )
        .map_err(|error| format!("prepare journal authority read: {error}"))?;
    let rows = stmt
        .query_map(
            params![identity_pubkey, relay_url, agent_pubkey, journal_id],
            row_from_sql,
        )
        .map_err(|error| format!("query journal authority artifacts: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read journal authority artifact row: {error}"))?
        .into_iter()
        .map(|row| validate_stored_row(row, identity_pubkey, &relay_url, &agent_pubkey))
        .collect()
}

/// Bounded range query suitable for the owner Today surface. It returns only
/// decoded public fields after revalidating every signed event; no secret keys
/// or raw key material cross the Tauri boundary.
pub fn query_journal_authority_artifacts(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    agent_pubkey: &str,
    start_created_at: i64,
    end_created_at: i64,
    limit: i64,
) -> Result<Vec<JournalAuthorityArtifact>, String> {
    let relay_url = normalize_relay_scope(relay_url)?;
    let agent_pubkey = normalize_agent_scope(agent_pubkey)?;
    if start_created_at >= end_created_at {
        return Err("journal authority range must be half-open and non-empty".into());
    }
    if !(1..=500).contains(&limit) {
        return Err("journal authority query limit must be between 1 and 500".into());
    }
    let mut stmt = conn
        .prepare(
            "SELECT identity_pubkey, relay_url, agent_pubkey, journal_id, artifact_type, event_id,
                    created_at, revision, raw_json
             FROM journal_authority_artifacts
             WHERE identity_pubkey = ?1 AND relay_url = ?2
               AND agent_pubkey = ?3 AND created_at >= ?4 AND created_at < ?5
             ORDER BY created_at DESC, event_id DESC
             LIMIT ?6",
        )
        .map_err(|error| format!("prepare journal authority range query: {error}"))?;
    let rows = stmt
        .query_map(
            params![
                identity_pubkey,
                relay_url,
                agent_pubkey,
                start_created_at,
                end_created_at,
                limit
            ],
            row_from_sql,
        )
        .map_err(|error| format!("query journal authority range: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read journal authority range row: {error}"))?
        .into_iter()
        .map(|row| validate_stored_row(row, identity_pubkey, &relay_url, &agent_pubkey))
        .collect()
}

fn validate_archived_observer_frame(event: &Event, identity_pubkey: &str) -> Result<(), String> {
    if event.kind.as_u16() != super::KIND_AGENT_OBSERVER_FRAME {
        return Err("archived observer event kind does not match".into());
    }
    if !event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.len() >= 2 && values[0] == "p" && values[1] == identity_pubkey
    }) {
        return Err("observer frame #p does not match the archived owner".into());
    }
    let tag_value = |name: &str| {
        event.tags.iter().find_map(|tag| {
            let values = tag.as_slice();
            (values.len() >= 2 && values[0] == name).then(|| values[1].clone())
        })
    };
    let agent_pubkey =
        tag_value("agent").ok_or_else(|| "observer frame missing `agent` tag".to_string())?;
    if event.pubkey.to_hex() != agent_pubkey {
        return Err("observer frame author does not match agent tag".into());
    }
    let frame =
        tag_value("frame").ok_or_else(|| "observer frame missing `frame` tag".to_string())?;
    if frame != super::OBSERVER_FRAME_TELEMETRY {
        return Err(format!("expected frame=telemetry, got {frame:?}"));
    }
    Ok(())
}

/// Revalidate every source event referenced by a verification artifact against
/// the active owner's immutable archive. This prevents an otherwise well-signed owner
/// artifact from yielding VERIFIED when it cites absent, cross-identity, or
/// tampered observer evidence. Current collection preferences are deliberately
/// irrelevant after the event was accepted into an owner-scoped archive row.
pub fn validate_archived_verification_sources(
    conn: &Connection,
    owner_keys: &Keys,
    artifact: &JournalAuthorityArtifact,
) -> Result<(), String> {
    if artifact.artifact_type != JournalAuthorityArtifactType::Verification {
        return Ok(());
    }
    let identity_pubkey = owner_keys.public_key().to_hex();
    if artifact.owner_pubkey != identity_pubkey {
        return Err("verification artifact owner does not match the active identity".into());
    }
    let relay_url = normalize_relay_scope(&artifact.relay_url)?;
    let mut correlation_bound = artifact.correlation_id == artifact.journal_id;
    for source_event_id in &artifact.source_event_ids {
        let mut stmt = conn
            .prepare(
                "SELECT ae.relay_url, ae.kind, ae.raw_json
                   FROM archived_events ae
                   INNER JOIN archived_event_scopes aes
                     ON aes.identity_pubkey = ae.identity_pubkey
                    AND aes.relay_url = ae.relay_url
                    AND aes.id = ae.id
                  WHERE ae.identity_pubkey = ?1
                    AND ae.id = ?2
                    AND aes.scope_type = 'owner_p'
                    AND aes.scope_value = ?1",
            )
            .map_err(|error| format!("prepare verification source read: {error}"))?;
        let rows = stmt
            .query_map(params![identity_pubkey, source_event_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| format!("read verification source event: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read verification source row: {error}"))?;
        if rows.is_empty() {
            return Err(format!(
                "verification source event {source_event_id} is not archived"
            ));
        }
        let mut validation_errors = Vec::new();
        let mut valid = false;
        for (stored_relay_url, kind, raw_json) in rows {
            match normalize_relay_scope(&stored_relay_url) {
                Ok(stored_scope) if stored_scope == relay_url => {}
                Ok(_) => {
                    validation_errors.push("source is archived under another relay".into());
                    continue;
                }
                Err(error) => {
                    validation_errors.push(format!("stored relay is invalid: {error}"));
                    continue;
                }
            }
            if kind != 24200 {
                validation_errors.push("not an observer event".to_string());
                continue;
            }
            let event = match Event::from_json(&raw_json) {
                Ok(event) if event.id.to_hex() == *source_event_id && event.verify().is_ok() => {
                    event
                }
                Ok(_) => {
                    validation_errors.push("signed ID or signature mismatch".to_string());
                    continue;
                }
                Err(error) => {
                    validation_errors.push(format!("parse failed: {error}"));
                    continue;
                }
            };
            if event.pubkey.to_hex() != artifact.agent_pubkey {
                validation_errors.push("observer source belongs to another managed agent".into());
                continue;
            }
            if let Err(error) = validate_archived_observer_frame(&event, &identity_pubkey) {
                validation_errors.push(format!("observer authorization failed: {error}"));
                continue;
            }
            let decoded = match buzz_core_pkg::observer::decrypt_observer_payload::<serde_json::Value>(
                owner_keys, &event,
            ) {
                Ok(decoded) => decoded,
                Err(error) => {
                    validation_errors.push(format!("observer decrypt failed: {error}"));
                    continue;
                }
            };
            let leaves: Vec<&serde_json::Value> =
                if decoded.get("kind").and_then(serde_json::Value::as_str) == Some("batch") {
                    decoded
                        .get("payload")
                        .and_then(|payload| payload.get("events"))
                        .and_then(serde_json::Value::as_array)
                        .map(|events| events.iter().collect())
                        .unwrap_or_default()
                } else {
                    vec![&decoded]
                };
            let matching_leaves = leaves
                .into_iter()
                .filter(|leaf| {
                    ["journalKey", "turnId", "sessionId", "channelId"]
                        .into_iter()
                        .filter_map(|field| leaf.get(field).and_then(serde_json::Value::as_str))
                        .any(|value| value == artifact.journal_id)
                })
                .collect::<Vec<_>>();
            if matching_leaves.is_empty() {
                validation_errors.push("observer payload does not bind the journal".to_string());
                continue;
            }
            correlation_bound |= matching_leaves.iter().any(|leaf| {
                let payload = leaf.get("payload");
                let triggering_matches = payload
                    .and_then(|value| value.get("triggeringEventIds"))
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_str() == Some(artifact.correlation_id.as_str()))
                    });
                let update = payload
                    .and_then(|value| value.get("params"))
                    .and_then(|value| value.get("update"));
                let update_matches = ["toolCallId", "messageId"]
                    .into_iter()
                    .filter_map(|field| {
                        update
                            .and_then(|value| value.get(field))
                            .and_then(serde_json::Value::as_str)
                    })
                    .any(|value| value == artifact.correlation_id);
                triggering_matches || update_matches
            });
            valid = true;
            break;
        }
        if !valid {
            return Err(format!(
                "verification source event {source_event_id} failed validation: {}",
                validation_errors.join("; ")
            ));
        }
    }
    if !correlation_bound {
        return Err(format!(
            "verification sources do not bind correlation {:?}",
            artifact.correlation_id
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "journal_authority_tests.rs"]
mod tests;
