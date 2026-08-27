//! Command executor — transactional event processing for command kinds.
//!
//! Command kinds (41010–41012, 30620, 46020, 46030–46031) are processed
//! transactionally: validate → begin tx → insert event → execute mutations → commit.
//!
//! SECURITY: This module is only reachable AFTER the ingest pipeline has verified:
//! 1. Event signature (verify_event)
//! 2. Timestamp freshness (±15 min)
//! 3. Pubkey/auth identity match
//! 4. Per-kind scope authorization

use std::sync::Arc;

use chrono::Utc;
use nostr::Event;
use sha2::{Digest, Sha256};
use tracing::warn;
use uuid::Uuid;

use buzz_core::kind::*;
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_datastore_tracing::datastore_span;
use buzz_db::workflow::{ApprovalStatus, RunStatus};
use buzz_db::DbError;
use buzz_workflow::executor::TriggerContext;

use crate::state::AppState;
use crate::webhook_secret;

use super::ingest::{extract_channel_id, IngestAuth, IngestError, IngestResult};
use super::side_effects::{
    emit_group_discovery_events, emit_membership_notification, emit_system_message,
    publish_dm_visibility_snapshot,
};

/// Route a command-kind event to the appropriate handler.
pub async fn handle_command(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: Event,
    auth: IngestAuth,
) -> Result<IngestResult, IngestError> {
    // Ensure the authenticated user exists in the users table (foreign key requirement).
    // The old REST handlers did this via extract_auth_context; command executor must do it explicitly.
    let pubkey_bytes = auth.pubkey().to_bytes().to_vec();
    match state
        .db
        .ensure_user(tenant.community(), &pubkey_bytes)
        .await
    {
        Ok(true) => {
            metrics::counter!(
                "buzz_users_created_total",
                "community" => tenant.host().to_owned()
            )
            .increment(1);
        }
        Ok(false) => {}
        Err(e) => {
            tracing::warn!("command_executor: ensure_user failed: {e}");
        }
    }

    let kind = event.kind.as_u16() as u32;
    match kind {
        KIND_DM_OPEN => handle_dm_open(tenant, state, &event, &auth).await,
        KIND_DM_ADD_MEMBER => handle_dm_add_member(tenant, state, &event, &auth).await,
        KIND_DM_HIDE => handle_dm_hide(tenant, state, &event, &auth).await,
        KIND_WORKFLOW_DEF => handle_workflow_def(tenant, state, &event, &auth).await,
        KIND_WORKFLOW_TRIGGER => handle_workflow_trigger(tenant, state, &event, &auth).await,
        KIND_APPROVAL_GRANT => handle_approval_grant(tenant, state, &event, &auth).await,
        KIND_APPROVAL_DENY => handle_approval_deny(tenant, state, &event, &auth).await,
        _ => Err(IngestError::Rejected(format!(
            "unknown command kind: {kind}"
        ))),
    }
}

/// Result of persisting a command event: either a duplicate (already processed)
/// or an open transaction that the handler must commit after executing mutations.
enum PersistResult {
    /// Event was already processed — return idempotent success.
    Duplicate,
    /// Event inserted — transaction is open, handler must commit after mutations.
    Inserted(sqlx::Transaction<'static, sqlx::Postgres>),
}

/// Persist a command event inside a transaction. Returns the OPEN transaction
/// as an idempotency guard — if the event was already stored, `Duplicate` is
/// returned and the handler skips execution.
///
/// If the event is a duplicate (ON CONFLICT DO NOTHING), the transaction is
/// rolled back and `PersistResult::Duplicate` is returned — no mutations needed.
///
/// NOTE: Most domain mutations still execute on the connection pool rather
/// than in this transaction. Workflow-definition ingestion is the exception:
/// its materialized workflow row and exact signed revision are written through
/// this transaction so the event and revision binding commit atomically.
/// Other operations remain idempotent but not strictly atomic.
#[datastore_span(name = "persist_command_event", system = "postgresql")]
async fn persist_command_event(
    db: &buzz_db::Db,
    tenant: &TenantContext,
    event: &Event,
    channel_id_override: Option<Uuid>,
) -> Result<PersistResult, IngestError> {
    use buzz_db::replaceable::{ParameterizedReplacePrecondition, ParameterizedReplaceStatus};

    let channel_id = channel_id_override.or_else(|| extract_channel_id(event));
    let mut tx = db
        .begin_transaction()
        .await
        .map_err(|e| IngestError::Internal(format!("error: begin transaction: {e}")))?;
    buzz_deletion::store(db)
        .guard_transaction(&mut tx, tenant.community())
        .await
        .map_err(|error| {
            IngestError::Rejected(format!("restricted: community writes are fenced: {error}"))
        })?;

    let d_tag = buzz_db::event::extract_d_tag(event);
    if let Some(d_tag) = d_tag.as_deref() {
        if d_tag.len() > buzz_db::event::D_TAG_MAX_LEN {
            return Err(IngestError::Rejected(format!(
                "invalid: d tag too long ({} bytes, max {})",
                d_tag.len(),
                buzz_db::event::D_TAG_MAX_LEN,
            )));
        }

        let kind = event.kind.as_u16() as i32;
        let (expected_revision, revision_error) = match parse_expected_workflow_revision(
            kind,
            extract_tag(event, "expected-revision").as_deref(),
        ) {
            Ok(expected_revision) => (expected_revision, None),
            Err(error) => (None, Some(error)),
        };
        let precondition = if revision_error.is_some() {
            ParameterizedReplacePrecondition::ExactReplayOnly
        } else if let Some(expected_revision) = expected_revision.as_deref() {
            ParameterizedReplacePrecondition::ExpectedRevision(expected_revision)
        } else {
            ParameterizedReplacePrecondition::Unconditional
        };
        let result = db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                tenant.community(),
                event,
                d_tag,
                channel_id,
                precondition,
            )
            .await
            .map_err(|e| {
                IngestError::Internal(format!("error: replace parameterized event: {e}"))
            })?;

        return match result.status {
            ParameterizedReplaceStatus::Inserted => Ok(PersistResult::Inserted(tx)),
            ParameterizedReplaceStatus::Duplicate => Ok(PersistResult::Duplicate),
            ParameterizedReplaceStatus::Superseded
                if kind == KIND_WORKFLOW_DEF as i32 && expected_revision.is_some() =>
            {
                Err(IngestError::Rejected(
                    "conflict: workflow update was superseded; refresh and try again".into(),
                ))
            }
            ParameterizedReplaceStatus::Superseded => Ok(PersistResult::Duplicate),
            ParameterizedReplaceStatus::RevisionMissing => Err(IngestError::Rejected(
                "conflict: workflow revision does not exist".into(),
            )),
            ParameterizedReplaceStatus::RevisionMismatch => Err(IngestError::Rejected(
                "conflict: workflow changed since it was loaded".into(),
            )),
            ParameterizedReplaceStatus::ReplayOnlyMiss => match revision_error {
                Some(error) => Err(error),
                None => Err(IngestError::Internal(
                    "error: replay-only replacement lacked a revision error".into(),
                )),
            },
        };
    }

    let (_, was_inserted) =
        buzz_db::event::insert_event_in_transaction(&mut tx, tenant.community(), event, channel_id)
            .await
            .map_err(|e| IngestError::Internal(format!("error: insert event: {e}")))?;
    if was_inserted {
        Ok(PersistResult::Inserted(tx))
    } else {
        Ok(PersistResult::Duplicate)
    }
}

fn parse_expected_workflow_revision(
    kind: i32,
    expected_revision: Option<&str>,
) -> Result<Option<Vec<u8>>, IngestError> {
    if kind != KIND_WORKFLOW_DEF as i32 {
        return Ok(None);
    }

    expected_revision
        .map(|expected| {
            let id = hex::decode(expected).map_err(|_| {
                IngestError::Rejected("invalid: bad expected workflow revision".into())
            })?;
            if id.len() != 32 {
                return Err(IngestError::Rejected(
                    "invalid: bad expected workflow revision".into(),
                ));
            }
            Ok(id)
        })
        .transpose()
}

/// Extract all `p` tag values (hex pubkeys) from an event.
fn extract_p_tags(event: &Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|t| {
            if t.kind().to_string() == "p" {
                t.content().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Extract the first `h` tag value (channel UUID) from an event.
fn extract_h_tag(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|t| {
        if t.kind().to_string() == "h" {
            t.content().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Extract the first `d` tag value from an event.
fn extract_d_tag(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|t| {
        if t.kind().to_string() == "d" {
            t.content().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Extract the first `e` tag value from an event.
fn extract_e_tag(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|t| {
        if t.kind().to_string() == "e" {
            t.content().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Extract a tag value by name.
fn extract_tag(event: &Event, tag_name: &str) -> Option<String> {
    event.tags.iter().find_map(|t| {
        if t.kind().to_string() == tag_name {
            t.content().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Decode a hex pubkey string to 32 bytes.
fn decode_pubkey(hex_str: &str) -> Result<Vec<u8>, IngestError> {
    let bytes = hex::decode(hex_str)
        .map_err(|_| IngestError::Rejected(format!("invalid: bad pubkey hex: {hex_str}")))?;
    if bytes.len() != 32 {
        return Err(IngestError::Rejected(format!(
            "invalid: pubkey must be 32 bytes: {hex_str}"
        )));
    }
    Ok(bytes)
}

/// Compute SHA-256 hash of a string, returning raw bytes.
fn compute_definition_hash(json_str: &str) -> Vec<u8> {
    Sha256::digest(json_str.as_bytes()).to_vec()
}

async fn handle_dm_open(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();
    let self_hex = hex::encode(&self_bytes);

    // 1. Extract participant pubkeys from `p` tags
    let p_tags = extract_p_tags(event);

    // 2. Validate: at least 1 other participant, max 8 others (9 total)
    if p_tags.is_empty() {
        return Err(IngestError::Rejected(
            "invalid: pubkeys must contain at least 1 other participant".into(),
        ));
    }
    if p_tags.len() > 8 {
        return Err(IngestError::Rejected(
            "invalid: pubkeys may contain at most 8 other participants (9 total)".into(),
        ));
    }

    // Decode all provided pubkeys
    let mut other_bytes: Vec<Vec<u8>> = Vec::with_capacity(p_tags.len());
    for hex_str in &p_tags {
        other_bytes.push(decode_pubkey(hex_str)?);
    }

    // 3. Build full participant set (self + others, deduplicated)
    let mut all_bytes: Vec<Vec<u8>> = vec![self_bytes.clone()];
    for ob in &other_bytes {
        if !all_bytes.iter().any(|b| b == ob) {
            all_bytes.push(ob.clone());
        }
    }

    // Persist the command event (idempotency) — returns open transaction
    let tx = match persist_command_event(&state.db, tenant, event, None).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // 4. Execute: open_dm
    let all_refs: Vec<&[u8]> = all_bytes.iter().map(|b| b.as_slice()).collect();
    let (channel, was_created) = state
        .db
        .open_dm(tenant.community(), &all_refs, &self_bytes)
        .await
        .map_err(|e| IngestError::Internal(format!("error: db open_dm: {e}")))?;

    // Finalize the idempotency record after the separate mutation succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // 5. Side effects if newly created (post-commit, best-effort)
    if was_created {
        metrics::counter!(
            "buzz_channels_created_total",
            "community" => tenant.host().to_owned(),
            "type" => "dm"
        )
        .increment(1);

        // Invalidate caches for all participants
        for pk in &all_bytes {
            state.invalidate_membership(tenant, channel.id, pk);
        }

        let participant_hexes: Vec<String> = all_bytes.iter().map(hex::encode).collect();
        if let Err(e) = emit_system_message(
            tenant,
            state,
            channel.id,
            serde_json::json!({
                "type": "dm_created",
                "actor": self_hex,
                "participants": participant_hexes,
            }),
        )
        .await
        {
            warn!("DM open: system message failed: {e}");
        }

        if let Err(e) = emit_group_discovery_events(tenant, state, channel.id).await {
            warn!(channel = %channel.id, "DM open: discovery emission failed: {e}");
        }

        for participant in &all_bytes {
            if let Err(e) = emit_membership_notification(
                tenant,
                state,
                channel.id,
                participant,
                &self_bytes,
                KIND_MEMBER_ADDED_NOTIFICATION,
            )
            .await
            {
                warn!("DM open: membership notification failed: {e}");
            }
        }
    } else {
        // Re-open of an existing DM cleared the caller's hidden_at; refresh
        // their NIP-DV snapshot so the DM reappears in the sidebar.
        if let Err(e) = publish_dm_visibility_snapshot(tenant, state, &self_bytes).await {
            warn!("DM re-open: visibility snapshot failed: {e}");
        }
    }

    // 6. Return response
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!(
            "response:{}",
            serde_json::json!({
                "channel_id": channel.id.to_string(),
                "created": was_created,
            })
        ),
    })
}

async fn handle_dm_add_member(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();

    // 1. Extract target channel from `h` tag, new member pubkeys from `p` tags
    let channel_id_str = extract_h_tag(event)
        .ok_or_else(|| IngestError::Rejected("invalid: missing h tag (channel_id)".into()))?;
    let channel_id = Uuid::parse_str(&channel_id_str)
        .map_err(|_| IngestError::Rejected("invalid: bad channel_id format".into()))?;

    let p_tags = extract_p_tags(event);
    if p_tags.is_empty() {
        return Err(IngestError::Rejected(
            "invalid: must specify at least 1 new participant in p tags".into(),
        ));
    }

    // 2. Validate caller is member of existing DM
    let is_member = state
        .is_member_cached(tenant.community(), channel_id, &self_bytes)
        .await
        .map_err(|e| IngestError::Internal(format!("error: membership check: {e}")))?;
    if !is_member {
        return Err(IngestError::Rejected(
            "forbidden: not a member of this DM".into(),
        ));
    }

    // 3. Validate channel is type "dm"
    let existing_channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|_| IngestError::Rejected("invalid: DM not found".into()))?;
    if existing_channel.channel_type != "dm" {
        return Err(IngestError::Rejected("invalid: channel is not a DM".into()));
    }

    // 4. Get existing members, merge with new
    let existing_members = state
        .db
        .get_members(tenant.community(), channel_id)
        .await
        .map_err(|e| IngestError::Internal(format!("error: get members: {e}")))?;

    let mut all_bytes: Vec<Vec<u8>> = existing_members.into_iter().map(|m| m.pubkey).collect();

    // Decode and merge new pubkeys
    for hex_str in &p_tags {
        let bytes = decode_pubkey(hex_str)?;
        if !all_bytes.iter().any(|b| b == &bytes) {
            all_bytes.push(bytes);
        }
    }

    // 5. Enforce max 9 participants
    if all_bytes.len() > 9 {
        return Err(IngestError::Rejected(
            "invalid: DM supports at most 9 participants".into(),
        ));
    }

    // Persist the command event — returns open transaction
    let tx = match persist_command_event(&state.db, tenant, event, None).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // 6. Execute: open_dm with expanded set (creates NEW DM — DM sets are immutable)
    let all_refs: Vec<&[u8]> = all_bytes.iter().map(|b| b.as_slice()).collect();
    let (new_channel, was_created) = state
        .db
        .open_dm(tenant.community(), &all_refs, &self_bytes)
        .await
        .map_err(|e| IngestError::Internal(format!("error: db open_dm: {e}")))?;

    // Finalize the idempotency record after the separate mutation succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // 7. Cache invalidation + notifications for new DM (post-commit, best-effort)
    if was_created {
        metrics::counter!(
            "buzz_channels_created_total",
            "community" => tenant.host().to_owned(),
            "type" => "dm"
        )
        .increment(1);

        for pk in &all_bytes {
            state.invalidate_membership(tenant, new_channel.id, pk);
        }

        if let Err(e) = emit_group_discovery_events(tenant, state, new_channel.id).await {
            warn!(channel = %new_channel.id, "DM add_member: discovery emission failed: {e}");
        }

        for participant_bytes in &all_bytes {
            if let Err(e) = emit_membership_notification(
                tenant,
                state,
                new_channel.id,
                participant_bytes,
                &self_bytes,
                KIND_MEMBER_ADDED_NOTIFICATION,
            )
            .await
            {
                warn!("DM add_member: membership notification failed: {e}");
            }
        }
    }

    // 8. Return response
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!(
            "response:{}",
            serde_json::json!({
                "channel_id": new_channel.id.to_string(),
            })
        ),
    })
}

async fn handle_dm_hide(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();

    // 1. Extract channel from `h` tag
    let channel_id_str = extract_h_tag(event)
        .ok_or_else(|| IngestError::Rejected("invalid: missing h tag (channel_id)".into()))?;
    let channel_id = Uuid::parse_str(&channel_id_str)
        .map_err(|_| IngestError::Rejected("invalid: bad channel_id format".into()))?;

    // 2. Validate caller is member of the DM
    let is_member = state
        .is_member_cached(tenant.community(), channel_id, &self_bytes)
        .await
        .map_err(|e| IngestError::Internal(format!("error: membership check: {e}")))?;
    if !is_member {
        return Err(IngestError::Rejected(
            "forbidden: not a member of this DM".into(),
        ));
    }

    // 3. Validate channel is type "dm"
    let channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|_| IngestError::Rejected("invalid: DM not found".into()))?;
    if channel.channel_type != "dm" {
        return Err(IngestError::Rejected("invalid: channel is not a DM".into()));
    }

    // Persist the command event — returns open transaction
    let tx = match persist_command_event(&state.db, tenant, event, None).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // 4. Execute: hide_dm
    state
        .db
        .hide_dm(tenant.community(), channel_id, &self_bytes)
        .await
        .map_err(|e| IngestError::Internal(format!("error: db hide_dm: {e}")))?;

    // Finalize the idempotency record after the separate mutation succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // 5. Side effect (post-commit, best-effort): refresh the caller's NIP-DV
    // visibility snapshot so clients can filter this DM out of the sidebar.
    if let Err(e) = publish_dm_visibility_snapshot(tenant, state, &self_bytes).await {
        warn!("DM hide: visibility snapshot failed: {e}");
    }

    // 6. Return response
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: "{}".into(),
    })
}

async fn handle_workflow_def(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();

    // 1. Extract channel and the canonical workflow UUID from the NIP-33 d-tag.
    let channel_id_str = extract_h_tag(event)
        .ok_or_else(|| IngestError::Rejected("invalid: missing h tag (channel_id)".into()))?;
    let channel_id = Uuid::parse_str(&channel_id_str)
        .map_err(|_| IngestError::Rejected("invalid: bad channel_id format".into()))?;

    let workflow_id_str = extract_d_tag(event)
        .ok_or_else(|| IngestError::Rejected("invalid: missing d tag (workflow_id)".into()))?;
    let workflow_id = Uuid::parse_str(&workflow_id_str)
        .map_err(|_| IngestError::Rejected("invalid: bad workflow_id format".into()))?;

    // 2. Validate caller has channel access (minimum: is a member)
    let is_member = state
        .is_member_cached(tenant.community(), channel_id, &self_bytes)
        .await
        .map_err(|e| IngestError::Internal(format!("error: membership check: {e}")))?;
    if !is_member {
        return Err(IngestError::Rejected(
            "forbidden: not a member of this channel".into(),
        ));
    }

    // 3. Parse YAML from event.content
    let (def, definition_json_str) = buzz_workflow::WorkflowEngine::parse_yaml(&event.content)
        .map_err(|e| IngestError::Rejected(format!("invalid: workflow YAML parse error: {e}")))?;
    let workflow_name = extract_tag(event, "name").unwrap_or_else(|| def.name.clone());

    // SEC-006: definitions with exfiltration-capable actions (call_webhook)
    // require elevated channel authority to save — plain membership is not
    // enough, because the workflow will forward channel content outward with
    // the owner's standing authority. Fail-closed on lookup errors.
    if def.requires_elevated_authority() {
        let role = state
            .db
            .get_member_role(tenant.community(), channel_id, &self_bytes)
            .await
            .map_err(|e| IngestError::Internal(format!("error: role check: {e}")))?;
        if !matches!(role.as_deref(), Some("owner") | Some("admin")) {
            return Err(IngestError::Rejected(
                "forbidden: workflows with call_webhook actions require the owner or admin role"
                    .into(),
            ));
        }
    }

    let mut definition_json: serde_json::Value = serde_json::from_str(&definition_json_str)
        .map_err(|e| IngestError::Internal(format!("error: json parse of definition: {e}")))?;

    let existing_workflow = match state.db.get_workflow(tenant.community(), workflow_id).await {
        Ok(workflow) => {
            if workflow.owner_pubkey != self_bytes || workflow.channel_id != Some(channel_id) {
                return Err(IngestError::Rejected(
                    "forbidden: workflow belongs to a different owner or channel".into(),
                ));
            }
            Some(workflow)
        }
        Err(DbError::NotFound(_)) => None,
        Err(e) => {
            return Err(IngestError::Internal(format!(
                "error: db get_workflow: {e}"
            )));
        }
    };

    // Preserve the existing webhook secret across updates. A new secret is
    // returned only when the workflow first gains a webhook trigger.
    let webhook_secret = if matches!(def.trigger, buzz_workflow::TriggerDef::Webhook) {
        let existing_secret = existing_workflow
            .as_ref()
            .and_then(|workflow| webhook_secret::extract_secret(&workflow.definition));
        let secret = existing_secret.unwrap_or_else(webhook_secret::generate_webhook_secret);
        webhook_secret::inject_secret(&mut definition_json, &secret);
        if existing_workflow
            .as_ref()
            .and_then(|workflow| webhook_secret::extract_secret(&workflow.definition))
            .is_none()
        {
            Some(secret)
        } else {
            None
        }
    } else {
        None
    };

    // Compute hash AFTER secret injection
    let definition_json_final = serde_json::to_string(&definition_json)
        .map_err(|e| IngestError::Internal(format!("error: json serialize: {e}")))?;
    let hash = compute_definition_hash(&definition_json_final);

    // Persist the command event — returns the transaction that will also own
    // the materialized workflow revision update.
    let mut tx = match persist_command_event(&state.db, tenant, event, Some(channel_id)).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // 4. Execute: upsert by the NIP-33 d-tag UUID. A retry updates the same
    // row instead of creating another enabled workflow that would fan out on
    // every matching event. The workflow's community is the request's
    // server-bound tenant — never re-derived from the (client-supplied) channel
    // id. `community_of_channel(channel_id)` is ambiguous when the same channel
    // UUID exists in two communities and could mint the workflow under the wrong
    // tenant; `tenant.community()` is the authoritative owner. We then verify the
    // channel actually exists *inside that community* (scoped `get_channel`),
    // which fails closed if the client named a channel that belongs to a
    // different community — the same guarantee the `(community_id, channel_id)`
    // composite FK enforces on insert, surfaced here as a clean rejection.
    let community_id = tenant.community();
    state
        .db
        .get_channel(community_id, channel_id)
        .await
        .map_err(|_| IngestError::Rejected("invalid: workflow channel not found".into()))?;

    state
        .db
        .upsert_workflow(
            &mut tx,
            community_id,
            workflow_id,
            Some(channel_id),
            &self_bytes,
            &workflow_name,
            &definition_json_final,
            &hash,
            event.id.as_bytes(),
        )
        .await
        .map_err(|e| match e {
            DbError::AccessDenied(_) => IngestError::Rejected(
                "forbidden: workflow belongs to a different owner or channel".into(),
            ),
            other => IngestError::Internal(format!("error: db upsert_workflow: {other}")),
        })?;

    // Commit the event transaction after the idempotent workflow upsert succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // Invalidate only after commit. Invalidating while the new row is still
    // invisible lets a concurrent trigger refill the cache with the old
    // definition and retain it until TTL expiry.
    state
        .workflow_engine
        .invalidate_channel_workflows(community_id, channel_id);

    // 5. Return response
    let mut resp = serde_json::json!({
        "workflow_id": workflow_id.to_string(),
    });
    if let Some(secret) = webhook_secret {
        resp["webhook_secret"] = serde_json::Value::String(secret);
    }

    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!("response:{}", resp),
    })
}

async fn caller_controls_workflow(
    state: &Arc<AppState>,
    community_id: CommunityId,
    workflow_owner: &[u8],
    caller: &[u8],
) -> Result<bool, IngestError> {
    if workflow_owner == caller {
        return Ok(true);
    }

    state
        .db
        .is_agent_owner(community_id, workflow_owner, caller)
        .await
        .map_err(|e| IngestError::Internal(format!("error: workflow owner check: {e}")))
}

fn exact_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut values = event.tags.iter().filter_map(|tag| {
        (tag.kind().to_string() == name)
            .then(|| tag.content())
            .flatten()
    });
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

async fn verify_workflow_revision(
    state: &Arc<AppState>,
    mut tx: Option<&mut sqlx::Transaction<'_, sqlx::Postgres>>,
    community_id: CommunityId,
    workflow: &buzz_db::workflow::WorkflowRecord,
    requested_revision: &[u8],
) -> Result<(), IngestError> {
    let Some(persisted_revision) = workflow.definition_event_id.as_deref() else {
        return Err(IngestError::Rejected(
            "invalid: owner-signed workflow revision is unavailable".into(),
        ));
    };
    if persisted_revision != requested_revision {
        return Err(IngestError::Rejected(
            "conflict: workflow revision does not match current definition".into(),
        ));
    }

    let stored = match tx.as_mut() {
        Some(tx) => {
            state
                .db
                .get_event_by_id_in_transaction(tx, community_id, persisted_revision)
                .await
        }
        None => {
            state
                .db
                .get_event_by_id(community_id, persisted_revision)
                .await
        }
    }
    .map_err(|e| IngestError::Internal(format!("error: workflow revision lookup: {e}")))?
    .ok_or_else(|| IngestError::Rejected("invalid: signed workflow revision not found".into()))?;
    let definition_event = &stored.event;
    let workflow_id = workflow.id.to_string();
    let workflow_channel_id = workflow.channel_id.map(|id| id.to_string());
    if definition_event.id.as_bytes() != persisted_revision
        || !definition_event.verify_id()
        || !definition_event.verify_signature()
        || definition_event.kind.as_u16() as u32 != KIND_WORKFLOW_DEF
        || definition_event.pubkey.to_bytes().as_slice() != workflow.owner_pubkey
        || exact_tag_value(definition_event, "d") != Some(workflow_id.as_str())
        || workflow_channel_id.is_none()
        || exact_tag_value(definition_event, "h") != workflow_channel_id.as_deref()
        || stored.channel_id != workflow.channel_id
    {
        return Err(IngestError::Rejected(
            "invalid: signed workflow revision binding mismatch".into(),
        ));
    }

    let (_, signed_json) = buzz_workflow::WorkflowEngine::parse_yaml(&definition_event.content)
        .map_err(|_| {
            IngestError::Rejected("invalid: signed workflow revision is malformed".into())
        })?;
    let signed_definition: serde_json::Value =
        serde_json::from_str(&signed_json).map_err(|_| {
            IngestError::Rejected("invalid: signed workflow revision is malformed".into())
        })?;
    if signed_definition != webhook_secret::strip_secret(&workflow.definition) {
        return Err(IngestError::Rejected(
            "invalid: signed workflow revision differs from materialized definition".into(),
        ));
    }
    Ok(())
}

async fn handle_workflow_trigger(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();

    // 1. Bind the command to both the workflow UUID and one exact signed revision.
    let workflow_id_str = exact_tag_value(event, "d").ok_or_else(|| {
        IngestError::Rejected("invalid: expected exactly one workflow d tag".into())
    })?;
    let revision_hex = exact_tag_value(event, "e").ok_or_else(|| {
        IngestError::Rejected("invalid: expected exactly one workflow revision e tag".into())
    })?;
    let requested_revision = hex::decode(revision_hex)
        .map_err(|_| IngestError::Rejected("invalid: bad workflow revision event id".into()))?;
    if requested_revision.len() != 32 {
        return Err(IngestError::Rejected(
            "invalid: bad workflow revision event id".into(),
        ));
    }
    let workflow_id = Uuid::parse_str(workflow_id_str)
        .map_err(|_| IngestError::Rejected("invalid: bad workflow_id format".into()))?;

    // 2. Validate workflow exists — scoped to the caller's community. The same
    // workflow UUID can exist in another community; a bare-id lookup could load
    // B's workflow and then satisfy the membership check below against B's
    // colliding channel, letting B trigger A's workflow.
    let community_id = tenant.community();
    let workflow = state
        .db
        .get_workflow(community_id, workflow_id)
        .await
        .map_err(|_| IngestError::Rejected("invalid: workflow not found".into()))?;

    // 3. Manual triggers execute with the workflow owner's authority. Permit
    // that principal and, for a managed agent, its immutable human owner.
    // Channel membership alone remains insufficient.
    if !caller_controls_workflow(state, community_id, &workflow.owner_pubkey, &self_bytes).await? {
        return Err(IngestError::Rejected(
            "forbidden: not authorized to trigger this workflow".into(),
        ));
    }
    // Managed-agent ownership is immutable. Carry the authorized workflow
    // principal across the transaction boundary so no pool-backed ownership
    // lookup is attempted while the command transaction holds its connection.
    let authorized_workflow_owner = workflow.owner_pubkey.clone();

    verify_workflow_revision(state, None, community_id, &workflow, &requested_revision).await?;

    // SEC-006: manual triggers must honor the workflow's lifecycle state and
    // recheck the owner's *current* channel authority before creating a run.
    // Without this, a disabled workflow — including one disabled because its
    // owner was removed from the channel — could still be fired by the owner.
    if !workflow.enabled || workflow.status != buzz_db::workflow::WorkflowStatus::Active {
        return Err(IngestError::Rejected(
            "forbidden: workflow is disabled or inactive".into(),
        ));
    }
    let def: buzz_workflow::WorkflowDef = serde_json::from_value(workflow.definition.clone())
        .map_err(|e| IngestError::Internal(format!("error: corrupt workflow definition: {e}")))?;
    let Some(wf_channel_id) = workflow.channel_id else {
        // No channel scope means no channel authority to verify — fail closed.
        return Err(IngestError::Rejected(
            "forbidden: workflow has no channel scope".into(),
        ));
    };
    state
        .workflow_engine
        .check_owner_authority(community_id, wf_channel_id, &workflow.owner_pubkey, &def)
        .await
        .map_err(|_| {
            IngestError::Rejected("forbidden: not authorized to trigger this workflow".into())
        })?;

    // Persist the command event under the workflow channel even though the
    // trigger event itself only carries the workflow UUID. Storing channel
    // triggers as global events leaks workflow IDs to unrelated relay members.
    let mut tx = match persist_command_event(&state.db, tenant, event, workflow.channel_id).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // Serialize the final authority check and run commit with channel
    // membership writers. If revocation commits first we observe no role; if
    // this lock wins, revocation cannot commit until this run is durable.
    buzz_db::channel::acquire_channel_membership_lock(&mut tx, community_id, wf_channel_id)
        .await
        .map_err(|e| IngestError::Internal(format!("error: membership lock: {e}")))?;

    // Re-read the workflow under a row lock on the same transaction that will
    // commit the trigger event and run. Definition replacement updates this row,
    // so it cannot commit between this exact-revision check and our commit. A
    // replacement that won first is observed here and rejected as stale.
    let workflow = state
        .db
        .get_workflow_for_share_in_transaction(&mut tx, community_id, workflow_id)
        .await
        .map_err(|_| IngestError::Rejected("invalid: workflow not found".into()))?;
    if workflow.owner_pubkey != authorized_workflow_owner {
        return Err(IngestError::Rejected(
            "conflict: workflow owner changed while trigger was being processed".into(),
        ));
    }
    verify_workflow_revision(
        state,
        Some(&mut tx),
        community_id,
        &workflow,
        &requested_revision,
    )
    .await?;
    if !workflow.enabled || workflow.status != buzz_db::workflow::WorkflowStatus::Active {
        return Err(IngestError::Rejected(
            "forbidden: workflow is disabled or inactive".into(),
        ));
    }
    let role = buzz_db::channel::get_member_role_in_transaction(
        &mut tx,
        community_id,
        wf_channel_id,
        &workflow.owner_pubkey,
    )
    .await
    .map_err(|e| IngestError::Internal(format!("error: owner authority lookup: {e}")))?;
    if !matches!(
        (role.as_deref(), def.requires_elevated_authority()),
        (Some(_), false) | (Some("owner" | "admin"), true)
    ) {
        return Err(IngestError::Rejected(
            "forbidden: not authorized to trigger this workflow".into(),
        ));
    }

    // 4. Execute: create workflow run
    let mut trigger_ctx = TriggerContext {
        channel_id: workflow
            .channel_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        author: hex::encode(&self_bytes),
        definition_event_id: revision_hex.to_owned(),
        ..Default::default()
    };
    if !event.content.is_empty() {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&event.content) {
            for (k, v) in map {
                let val_str = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                trigger_ctx.webhook_fields.insert(k, val_str);
            }
        }
    }
    let trigger_ctx_json = serde_json::to_value(&trigger_ctx).ok();

    let event_id_bytes = event.id.as_bytes().to_vec();
    let run_id = state
        .db
        .create_workflow_run_in_transaction(
            &mut tx,
            community_id,
            workflow_id,
            &requested_revision,
            Some(&event_id_bytes),
            trigger_ctx_json.as_ref(),
        )
        .await
        .map_err(|e| IngestError::Internal(format!("error: db create_workflow_run: {e}")))?;

    // Finalize the idempotency record after the separate run creation succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // 5. Spawn workflow execution
    let engine = Arc::clone(&state.workflow_engine);
    let trigger_ctx_clone = trigger_ctx.clone();
    tokio::spawn(async move {
        let result = match engine.load_run_definition(community_id, run_id).await {
            Ok((_, definition)) => {
                buzz_workflow::executor::execute_from_step(
                    &engine,
                    community_id,
                    run_id,
                    &definition,
                    &trigger_ctx_clone,
                    0,
                    None,
                )
                .await
            }
            Err(error) => Err((error, buzz_workflow::error::PartialProgress::default())),
        };
        engine
            .finalize_run(community_id, run_id, result, None)
            .await;
    });

    // 6. Return response
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!(
            "response:{}",
            serde_json::json!({
                "run_id": run_id.to_string(),
            })
        ),
    })
}

/// Enforce the approver_spec field against the requesting pubkey.
///
/// Accepted specs:
/// - `""` or `"any"` — any authenticated user may approve.
/// - 64-char lowercase hex string — only that exact pubkey may approve.
///
/// All other formats are rejected (fail-closed).
fn check_approver_spec(approver_spec: &str, requester_hex: &str) -> Result<(), IngestError> {
    let spec = approver_spec.trim();

    // Empty or "any" — anyone may approve
    if spec.is_empty() || spec == "any" {
        return Ok(());
    }

    // Exact pubkey match (64-char hex, case-insensitive)
    if spec.len() == 64 && spec.chars().all(|c| c.is_ascii_hexdigit()) {
        if requester_hex.to_lowercase() == spec.to_lowercase() {
            return Ok(());
        }
        return Err(IngestError::Rejected(
            "forbidden: not the designated approver for this request".into(),
        ));
    }

    // Role-based or unrecognised — fail closed
    Err(IngestError::Rejected(format!(
        "forbidden: approver spec '{}' is not yet supported",
        spec
    )))
}

async fn handle_approval_grant(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();
    let self_hex = hex::encode(&self_bytes);

    // 1. Extract approval reference from `e` tag (references the approval-requested event)
    //    or `d` tag (contains the token hash hex)
    let token_hash_hex = extract_d_tag(event)
        .or_else(|| extract_e_tag(event))
        .ok_or_else(|| {
            IngestError::Rejected("invalid: missing approval reference (d or e tag)".into())
        })?;

    let token_hash = hex::decode(&token_hash_hex)
        .map_err(|_| IngestError::Rejected("invalid: bad approval token hash hex".into()))?;

    // 2. Look up the approval record
    let approval = state
        .db
        .get_approval_by_stored_hash(tenant.community(), &token_hash)
        .await
        .map_err(|_| IngestError::Rejected("invalid: approval not found".into()))?;

    // 3. Validate approval is pending and not expired
    if approval.status != ApprovalStatus::Pending {
        return Err(IngestError::Rejected(format!(
            "invalid: approval already {}",
            approval.status
        )));
    }
    if Utc::now() > approval.expires_at {
        return Err(IngestError::Rejected(
            "invalid: approval token has expired".into(),
        ));
    }

    // 4. Validate caller is authorized approver
    check_approver_spec(&approval.approver_spec, &self_hex)?;

    // Persist the command event — returns open transaction
    let tx = match persist_command_event(&state.db, tenant, event, None).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // 5. Execute: update approval status to granted
    let note = if event.content.is_empty() {
        None
    } else {
        Some(event.content.as_str())
    };

    let updated = state
        .db
        .update_approval_by_stored_hash(
            tenant.community(),
            &token_hash,
            ApprovalStatus::Granted,
            Some(&self_bytes),
            note,
        )
        .await
        .map_err(|e| IngestError::Internal(format!("error: db update_approval: {e}")))?;

    if !updated {
        return Err(IngestError::Rejected(
            "invalid: approval already acted on (race)".into(),
        ));
    }

    // Finalize the idempotency record after the separate approval update succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // 6. Resume workflow execution (post-commit, async)
    let community_id = tenant.community();
    let run_id = approval.run_id;
    let workflow_id = approval.workflow_id;
    let resume_index = approval.step_index as usize + 1;
    let engine = Arc::clone(&state.workflow_engine);
    let db = state.db.clone();

    tokio::spawn(async move {
        resume_workflow_after_approval(engine, db, community_id, run_id, workflow_id, resume_index)
            .await;
    });

    // 7. Return response
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!(
            "response:{}",
            serde_json::json!({
                "status": "granted",
                "run_id": run_id.to_string(),
            })
        ),
    })
}

async fn handle_approval_deny(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let self_bytes = auth.pubkey().to_bytes().to_vec();
    let self_hex = hex::encode(&self_bytes);

    // 1. Extract approval reference
    let token_hash_hex = extract_d_tag(event)
        .or_else(|| extract_e_tag(event))
        .ok_or_else(|| {
            IngestError::Rejected("invalid: missing approval reference (d or e tag)".into())
        })?;

    let token_hash = hex::decode(&token_hash_hex)
        .map_err(|_| IngestError::Rejected("invalid: bad approval token hash hex".into()))?;

    // 2. Look up the approval record
    let approval = state
        .db
        .get_approval_by_stored_hash(tenant.community(), &token_hash)
        .await
        .map_err(|_| IngestError::Rejected("invalid: approval not found".into()))?;

    // 3. Validate approval is pending and not expired
    if approval.status != ApprovalStatus::Pending {
        return Err(IngestError::Rejected(format!(
            "invalid: approval already {}",
            approval.status
        )));
    }
    if Utc::now() > approval.expires_at {
        return Err(IngestError::Rejected(
            "invalid: approval token has expired".into(),
        ));
    }

    // 4. Validate caller is authorized approver
    check_approver_spec(&approval.approver_spec, &self_hex)?;

    // Persist the command event — returns open transaction
    let tx = match persist_command_event(&state.db, tenant, event, None).await? {
        PersistResult::Duplicate => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        PersistResult::Inserted(tx) => tx,
    };

    // 5. Execute: update approval status to denied
    let note = if event.content.is_empty() {
        None
    } else {
        Some(event.content.as_str())
    };

    let updated = state
        .db
        .update_approval_by_stored_hash(
            tenant.community(),
            &token_hash,
            ApprovalStatus::Denied,
            Some(&self_bytes),
            note,
        )
        .await
        .map_err(|e| IngestError::Internal(format!("error: db update_approval: {e}")))?;

    if !updated {
        return Err(IngestError::Rejected(
            "invalid: approval already acted on (race)".into(),
        ));
    }

    // Finalize the idempotency record after the separate approval denial succeeds.
    tx.commit()
        .await
        .map_err(|e| IngestError::Internal(format!("error: commit transaction: {e}")))?;

    // 6. Cancel the workflow run (post-commit, async)
    let community_id = tenant.community();
    let run_id = approval.run_id;
    let pubkey_hex = self_hex.clone();
    let db = state.db.clone();

    tokio::spawn(async move {
        let run = match db.get_workflow_run(community_id, run_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("approval_deny: failed to fetch run {run_id}: {e}");
                return;
            }
        };

        if run.status != RunStatus::WaitingApproval {
            tracing::warn!(
                "approval_deny: run {run_id} has status '{}', expected 'waiting_approval'",
                run.status
            );
            return;
        }

        let cancel_msg = format!("workflow cancelled: approval denied by {pubkey_hex}");
        if let Err(e) = db
            .update_workflow_run(
                community_id,
                run_id,
                RunStatus::Cancelled,
                run.current_step,
                &run.execution_trace,
                Some(buzz_db::workflow::WorkflowRunFailure {
                    code: "approval_denied",
                    message: &cancel_msg,
                }),
            )
            .await
        {
            tracing::error!("approval_deny: failed to cancel run {run_id}: {e}");
        }
    });

    // 7. Return response
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!(
            "response:{}",
            serde_json::json!({
                "status": "denied",
                "run_id": run_id.to_string(),
            })
        ),
    })
}

/// Resume a suspended workflow run after an approval gate has been granted.
async fn resume_workflow_after_approval(
    engine: Arc<buzz_workflow::WorkflowEngine>,
    db: buzz_db::Db,
    community_id: CommunityId,
    run_id: Uuid,
    workflow_id: Uuid,
    resume_index: usize,
) {
    let (run, def) = match engine.load_run_definition(community_id, run_id).await {
        Ok(bound) => bound,
        Err(e) => {
            tracing::error!(
                "resume_workflow: failed to load run-bound definition for {run_id}: {e}"
            );
            if let Ok(run) = db.get_workflow_run(community_id, run_id).await {
                let _ = db
                    .update_workflow_run(
                        community_id,
                        run_id,
                        RunStatus::Failed,
                        run.current_step,
                        &run.execution_trace,
                        Some(buzz_db::workflow::WorkflowRunFailure {
                            code: "invalid_definition_revision",
                            message: &e.to_string(),
                        }),
                    )
                    .await;
            }
            return;
        }
    };

    if run.workflow_id != workflow_id {
        tracing::error!(
            "resume_workflow: approval workflow {workflow_id} does not match run {}",
            run.workflow_id
        );
        return;
    }
    if run.status != RunStatus::WaitingApproval {
        tracing::warn!(
            "resume_workflow: run {run_id} has status '{}', expected 'waiting_approval'",
            run.status
        );
        return;
    }

    let mut initial_outputs: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    if let Some(trace_arr) = run.execution_trace.as_array() {
        for entry in trace_arr {
            if let (Some(step_id), Some(output)) = (
                entry.get("step_id").and_then(|v| v.as_str()),
                entry.get("output"),
            ) {
                initial_outputs.insert(step_id.to_string(), output.clone());
            }
        }
    }

    let trigger_ctx: TriggerContext = run
        .trigger_context
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let existing_trace = run.execution_trace.as_array().cloned();
    let result = buzz_workflow::executor::execute_from_step(
        &engine,
        community_id,
        run_id,
        &def,
        &trigger_ctx,
        resume_index,
        Some(initial_outputs),
    )
    .await;
    engine
        .finalize_run(community_id, run_id, result, existing_trace)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    async fn persistence_test_context() -> (buzz_db::Db, TenantContext) {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(&url)
            .await
            .expect("connect workflow persistence test database");
        let db = buzz_db::Db::from_pool(pool);
        db.migrate()
            .await
            .expect("migrate workflow persistence test database");
        let host = format!("workflow-cas-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("create workflow persistence test community")
            .id;
        (db, TenantContext::resolved(community, host))
    }

    async fn manual_trigger_test_context() -> (Arc<AppState>, TenantContext, Keys, Uuid, Event) {
        use buzz_core::channel::{ChannelType, ChannelVisibility};

        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        let setup_pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect workflow trigger setup database");
        let setup_db = buzz_db::Db::from_pool(setup_pool.clone());
        setup_db
            .migrate()
            .await
            .expect("migrate workflow trigger test database");

        let host = format!("workflow-trigger-{}.example", Uuid::new_v4().simple());
        let community = setup_db
            .ensure_configured_community(&host)
            .await
            .expect("create workflow trigger test community")
            .id;
        let tenant = TenantContext::resolved(community, host.clone());
        let human = Keys::generate();
        let agent = Keys::generate();
        let human_bytes = human.public_key().to_bytes();
        let agent_bytes = agent.public_key().to_bytes();
        setup_db
            .ensure_user(community, &human_bytes)
            .await
            .expect("ensure human owner");
        setup_db
            .ensure_user(community, &agent_bytes)
            .await
            .expect("ensure managed agent");
        assert!(setup_db
            .set_agent_owner(community, &agent_bytes, &human_bytes)
            .await
            .expect("set immutable agent owner"));
        let channel = setup_db
            .create_channel(
                community,
                "manual-trigger-pool",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &agent_bytes,
                None,
            )
            .await
            .expect("create workflow channel");
        let workflow_id = Uuid::new_v4();
        let definition = EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_DEF as u16),
            concat!(
                "name: manual-trigger-pool\n",
                "trigger:\n  on: message_posted\n",
                "steps:\n  - id: send\n    action: send_message\n    text: done\n",
            ),
        )
        .tags(vec![
            Tag::parse(["d", workflow_id.to_string().as_str()]).expect("d tag"),
            Tag::parse(["h", channel.id.to_string().as_str()]).expect("h tag"),
        ])
        .sign_with_keys(&agent)
        .expect("sign workflow definition");
        let (_, definition_json) = buzz_workflow::WorkflowEngine::parse_yaml(&definition.content)
            .expect("parse signed workflow definition");
        let definition_hash = compute_definition_hash(&definition_json);
        let mut tx = setup_db
            .begin_transaction()
            .await
            .expect("begin workflow seed");
        buzz_db::event::insert_event_in_transaction(
            &mut tx,
            community,
            &definition,
            Some(channel.id),
        )
        .await
        .expect("persist signed workflow definition");
        setup_db
            .upsert_workflow(
                &mut tx,
                community,
                workflow_id,
                Some(channel.id),
                &agent_bytes,
                "manual-trigger-pool",
                &definition_json,
                &definition_hash,
                definition.id.as_bytes(),
            )
            .await
            .expect("materialize signed workflow");
        tx.commit().await.expect("commit signed workflow");

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect(&url)
            .await
            .expect("connect one-connection workflow trigger pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let mut config = crate::config::Config::from_env().expect("config from env");
        config.database_url = url;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_url = format!("wss://{host}");
        config.require_relay_membership = false;
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool config");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool);
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        setup_pool.close().await;
        (Arc::new(state), tenant, human, workflow_id, definition)
    }

    fn workflow_trigger_event(keys: &Keys, workflow_id: Uuid, revision: &Event) -> Event {
        EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER as u16), "")
            .tags(vec![
                Tag::parse(["d", workflow_id.to_string().as_str()]).expect("d tag"),
                Tag::parse(["e", revision.id.to_hex().as_str()]).expect("revision tag"),
            ])
            .sign_with_keys(keys)
            .expect("sign workflow trigger")
    }

    fn http_auth(keys: &Keys) -> IngestAuth {
        IngestAuth::Http {
            pubkey: keys.public_key(),
            scopes: vec![buzz_auth::Scope::MessagesWrite],
            auth_method: super::super::ingest::HttpAuthMethod::Nip98,
        }
    }

    fn workflow_event(
        keys: &Keys,
        workflow_id: Uuid,
        created_at: u64,
        expected_revision: Option<&str>,
        name: &str,
    ) -> Event {
        let workflow_id = workflow_id.to_string();
        let channel_id = Uuid::new_v4().to_string();
        let mut tags = vec![
            Tag::parse(["d", workflow_id.as_str()]).expect("d tag"),
            Tag::parse(["h", channel_id.as_str()]).expect("h tag"),
        ];
        if let Some(revision) = expected_revision {
            tags.push(Tag::parse(["expected-revision", revision]).expect("revision tag"));
        }
        EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_DEF as u16),
            format!("name: {name}\ntrigger:\n  on: message_posted\nsteps: []\n"),
        )
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("workflow event")
    }

    fn rejection_message(result: Result<Option<Vec<u8>>, IngestError>) -> String {
        match result {
            Err(IngestError::Rejected(message)) => message,
            Err(IngestError::AuthFailed(message)) => panic!("unexpected auth failure: {message}"),
            Err(IngestError::Internal(message)) => panic!("unexpected internal failure: {message}"),
            Ok(_) => panic!("expected revision parsing to fail"),
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn human_owner_manual_trigger_completes_with_one_connection() {
        let (state, tenant, human, workflow_id, revision) = manual_trigger_test_context().await;
        let trigger = workflow_trigger_event(&human, workflow_id, &revision);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handle_workflow_trigger(&tenant, &state, &trigger, &http_auth(&human)),
        )
        .await
        .expect("human-owner trigger must not wait for a second pool connection")
        .expect("human-owner trigger must succeed");
        assert!(result.message.contains("\"run_id\""));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_human_owner_manual_triggers_do_not_starve_one_connection_pool() {
        let (state, tenant, human, workflow_id, revision) = manual_trigger_test_context().await;
        let triggers = (0..8)
            .map(|_| workflow_trigger_event(&human, workflow_id, &revision))
            .collect::<Vec<_>>();

        let results = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            let mut tasks = tokio::task::JoinSet::new();
            for trigger in triggers {
                let state = Arc::clone(&state);
                let tenant = tenant.clone();
                let auth = http_auth(&human);
                tasks.spawn(async move {
                    handle_workflow_trigger(&tenant, &state, &trigger, &auth).await
                });
            }
            let mut results = Vec::new();
            while let Some(result) = tasks.join_next().await {
                results.push(result.expect("trigger task must not panic"));
            }
            results
        })
        .await
        .expect("concurrent triggers must drain rather than pool-starve");

        assert_eq!(results.len(), 8);
        for result in results {
            assert!(
                result
                    .expect("concurrent human-owner trigger must succeed")
                    .accepted,
                "manual trigger should be accepted"
            );
        }
    }

    #[test]
    fn workflow_revision_parser_accepts_create_and_valid_update() {
        let revision = [0x42; 32];
        assert_eq!(
            parse_expected_workflow_revision(KIND_WORKFLOW_DEF as i32, None)
                .expect("tagless workflow"),
            None
        );
        assert_eq!(
            parse_expected_workflow_revision(
                KIND_WORKFLOW_DEF as i32,
                Some(&hex::encode(revision)),
            )
            .expect("valid revision"),
            Some(revision.to_vec())
        );
    }

    #[test]
    fn workflow_revision_parser_rejects_malformed_values() {
        for malformed in ["not-hex", "42"] {
            assert_eq!(
                rejection_message(parse_expected_workflow_revision(
                    KIND_WORKFLOW_DEF as i32,
                    Some(malformed),
                )),
                "invalid: bad expected workflow revision",
            );
        }
    }

    #[test]
    fn revision_tag_does_not_change_other_command_kinds() {
        assert_eq!(
            parse_expected_workflow_revision(KIND_DM_OPEN as i32, Some("not-hex"))
                .expect("non-workflow revision tag"),
            None
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_persistence_preserves_replays_and_rejects_dominated_cas_updates() {
        let (db, tenant) = persistence_test_context().await;
        let keys = Keys::generate();
        let workflow_id = Uuid::new_v4();
        let created_at = Timestamp::now().as_secs();
        let create = workflow_event(&keys, workflow_id, created_at, None, "create");
        let channel_id = Uuid::parse_str(
            exact_tag_value(&create, "h").expect("workflow definition channel tag"),
        )
        .expect("workflow definition channel UUID");

        let missing_revision = hex::encode([0x24; 32]);
        let missing_revision_update = workflow_event(
            &keys,
            Uuid::new_v4(),
            created_at,
            Some(&missing_revision),
            "missing-revision",
        );
        let error = match persist_command_event(&db, &tenant, &missing_revision_update, None).await
        {
            Err(error) => error,
            Ok(_) => panic!("missing revision must not create a workflow"),
        };
        assert!(matches!(
            error,
            IngestError::Rejected(ref message)
                if message == "conflict: workflow revision does not exist"
        ));

        let PersistResult::Inserted(tx) =
            persist_command_event(&db, &tenant, &create, Some(channel_id))
                .await
                .expect("persist create")
        else {
            panic!("first create must insert");
        };
        tx.commit().await.expect("commit create");
        let stored_create = db
            .get_event_by_id(tenant.community(), create.id.as_bytes())
            .await
            .expect("load persisted workflow definition")
            .expect("persisted workflow definition");
        assert_eq!(stored_create.channel_id, Some(channel_id));
        assert!(matches!(
            persist_command_event(&db, &tenant, &create, Some(channel_id))
                .await
                .expect("replay create"),
            PersistResult::Duplicate
        ));

        let create_revision = create.id.to_hex();
        let mut updates = (0..64).map(|index| {
            workflow_event(
                &keys,
                workflow_id,
                created_at,
                Some(&create_revision),
                &format!("update-{index}"),
            )
        });
        let update = updates
            .find(|candidate| candidate.id.as_bytes() < create.id.as_bytes())
            .expect("find same-second update that wins NIP-33 ordering");
        let dominated_update = (64..256)
            .map(|index| {
                workflow_event(
                    &keys,
                    workflow_id,
                    created_at,
                    Some(&update.id.to_hex()),
                    &format!("update-{index}"),
                )
            })
            .find(|candidate| candidate.id.as_bytes() > update.id.as_bytes())
            .expect("find same-second CAS-matching update dominated by current head");

        let PersistResult::Inserted(tx) = persist_command_event(&db, &tenant, &update, None)
            .await
            .expect("persist update")
        else {
            panic!("matching update must insert");
        };
        tx.commit().await.expect("commit update");
        assert!(matches!(
            persist_command_event(&db, &tenant, &update, None)
                .await
                .expect("replay update"),
            PersistResult::Duplicate
        ));

        let stale_revision_update = workflow_event(
            &keys,
            workflow_id,
            created_at + 1,
            Some(&create_revision),
            "stale-revision",
        );
        let error = match persist_command_event(&db, &tenant, &stale_revision_update, None).await {
            Err(error) => error,
            Ok(_) => panic!("stale revision must not replace the current workflow"),
        };
        assert!(matches!(
            error,
            IngestError::Rejected(ref message)
                if message == "conflict: workflow changed since it was loaded"
        ));

        let error = match persist_command_event(&db, &tenant, &dominated_update, None).await {
            Err(error) => error,
            Ok(_) => panic!("distinct dominated CAS update must not report duplicate success"),
        };
        assert!(matches!(
            error,
            IngestError::Rejected(ref message)
                if message == "conflict: workflow update was superseded; refresh and try again"
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_persistence_replays_legacy_malformed_revision_before_validation() {
        let (db, tenant) = persistence_test_context().await;
        let keys = Keys::generate();
        let workflow_id = Uuid::new_v4();
        let created_at = Timestamp::now().as_secs();
        let legacy = workflow_event(
            &keys,
            workflow_id,
            created_at,
            Some("not-hex"),
            "legacy-malformed",
        );

        let mut tx = db.begin_transaction().await.expect("begin legacy seed");
        let (_, was_inserted) = buzz_db::event::insert_event_in_transaction(
            &mut tx,
            tenant.community(),
            &legacy,
            extract_channel_id(&legacy),
        )
        .await
        .expect("seed legacy workflow event");
        assert!(was_inserted);
        tx.commit().await.expect("commit legacy seed");

        assert!(matches!(
            persist_command_event(&db, &tenant, &legacy, None)
                .await
                .expect("exact legacy replay must remain idempotent"),
            PersistResult::Duplicate
        ));

        let distinct = workflow_event(
            &keys,
            workflow_id,
            created_at + 1,
            Some("not-hex"),
            "distinct-malformed",
        );
        let error = match persist_command_event(&db, &tenant, &distinct, None).await {
            Err(error) => error,
            Ok(_) => panic!("distinct malformed revision must remain rejected"),
        };
        assert!(matches!(
            error,
            IngestError::Rejected(ref message)
                if message == "invalid: bad expected workflow revision"
        ));
    }
}
