//! DB-leased worker for `relay_admin_outbox` artifact delivery.
//!
//! Runs as a background `tokio::spawn` task. Each tick claims a batch of
//! pending outbox rows using `SELECT FOR UPDATE SKIP LOCKED`, processes them,
//! and marks each row delivered or failed. Multiple pods may run the worker
//! concurrently — the `held_by`/`lease_expires_at` lease prevents double-delivery.
//!
//! Task types driven by this worker:
//! - `tombstone`: publish an admin-deletion system message in the target channel.
//! - `system_message`: publish a kick notification system message.
//! - `reporter_notice`: send a moderation DM to the reporter.
//! - `affected_user_notice`: send a moderation DM to the actioned user (the
//!   author whose content was deleted, or the kicked/banned/timed-out user).

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

use buzz_db::relay_admin_actions::OutboxRecord;

use crate::state::AppState;

/// Lease duration: if the worker pod dies mid-delivery, another pod picks up
/// the row once the lease expires.
const LEASE_SECS: i64 = 30;
/// Rows claimed per tick.
const BATCH_SIZE: i64 = 16;

/// Run the admin outbox delivery worker. Never returns; intended for
/// `tokio::spawn`.
pub async fn run(state: Arc<AppState>) {
    let worker_id = format!("admin-outbox-{}", Uuid::new_v4());
    info!(worker_id = %worker_id, "Admin outbox worker started");

    let mut idle_delay = Duration::from_millis(500);

    loop {
        let lease_until = Utc::now() + chrono::Duration::seconds(LEASE_SECS);
        let batch = match state
            .db
            .claim_pending_admin_outbox_batch(&worker_id, lease_until, BATCH_SIZE)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                error!(worker_id = %worker_id, "Admin outbox claim failed: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        if batch.is_empty() {
            // Back off exponentially when idle, capped at 10 s.
            tokio::time::sleep(idle_delay).await;
            idle_delay = (idle_delay * 2).min(Duration::from_secs(10));
            continue;
        }

        idle_delay = Duration::from_millis(500);

        for row in batch {
            deliver_one(&state, &row).await;
        }
    }
}

/// Attempt to deliver one outbox row and update its state.
/// Made pub(crate) for integration tests.
pub(crate) async fn deliver_one(state: &Arc<AppState>, row: &OutboxRecord) {
    let result = match row.task_type.as_str() {
        "tombstone" => deliver_tombstone(state, row).await,
        "system_message" => deliver_system_message(state, row).await,
        "reporter_notice" => deliver_reporter_notice(state, row).await,
        "affected_user_notice" => deliver_affected_user_notice(state, row).await,
        other => Err(format!("unknown task_type: {other}")),
    };

    match result {
        Ok(()) => {
            match state
                .db
                .mark_admin_outbox_delivered(row.id, row.claim_token)
                .await
            {
                Ok(true) => {
                    info!(
                        outbox_id = %row.id,
                        action_id = %row.action_id,
                        task_type = %row.task_type,
                        "Outbox row delivered"
                    );
                }
                Ok(false) => {
                    // Ownership was lost before we could mark delivered (lease expired,
                    // another worker reclaimed and may have already completed this row).
                    // Stop processing — the row is in safe hands.
                    warn!(
                        outbox_id = %row.id,
                        "Outbox mark_delivered: ownership lost (stale worker), stopping"
                    );
                }
                Err(e) => {
                    warn!(outbox_id = %row.id, "mark_delivered failed: {e}");
                }
            }
        }
        Err(e) => {
            warn!(
                outbox_id = %row.id,
                action_id = %row.action_id,
                task_type = %row.task_type,
                error = %e,
                "Outbox delivery failed"
            );
            match state
                .db
                .fail_admin_outbox_row(row.id, row.claim_token, &e)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    // Ownership lost — another worker holds this row now. Don't
                    // double-record the failure.
                    warn!(outbox_id = %row.id, "fail_outbox_row: ownership lost (stale worker)");
                }
                Err(db_err) => {
                    error!(outbox_id = %row.id, "fail_outbox_row DB call failed: {db_err}");
                }
            }
        }
    }
}

/// Resolve community TenantContext by community_id.
async fn resolve_tenant(
    state: &AppState,
    community_id: buzz_core::CommunityId,
    task_type: &str,
) -> Result<buzz_core::tenant::TenantContext, String> {
    let host = state
        .db
        .lookup_community_host(community_id)
        .await
        .map_err(|e| format!("{task_type}: host lookup failed: {e}"))?
        .ok_or_else(|| format!("{task_type}: community not found"))?;
    Ok(buzz_core::tenant::TenantContext::resolved(
        community_id,
        host,
    ))
}

/// Deliver a tombstone: publish an admin-deletion system message in the channel.
///
/// The emitted system message matches the channel-moderation tombstone schema
/// (`side_effects.rs` NIP-29 DELETE_EVENT: `type: "message_deleted"` with
/// `actor`, `target_event_id`, and an optional public reason) so the room
/// renders it identically. Without `target_event_id` the room cannot tell which
/// message was removed, and without a reason it renders as a bare self-delete
/// rather than a moderator removal — `SystemMessageRow` keys "Removed by
/// community moderators" on `public_reason`.
///
/// The `reason_code`/`public_reason` fields are the operator's `reason` string
/// verbatim (an operator-authored public reason, not a sanitized derivative);
/// the resolve API documents that this text is public.
async fn deliver_tombstone(state: &Arc<AppState>, row: &OutboxRecord) -> Result<(), String> {
    let payload = &row.payload;
    let community_uuid: Uuid = payload["community_id"]
        .as_str()
        .ok_or("tombstone: missing community_id")?
        .parse()
        .map_err(|_| "tombstone: invalid community_id")?;
    let channel_id: Uuid = payload["channel_id"]
        .as_str()
        .ok_or("tombstone: missing channel_id")?
        .parse()
        .map_err(|_| "tombstone: invalid channel_id")?;
    let target_event_id = payload["target_event_id"]
        .as_str()
        .ok_or("tombstone: missing target_event_id")?;
    let actor = payload["actor"]
        .as_str()
        .ok_or("tombstone: missing actor")?;

    let community_id = buzz_core::CommunityId::from_uuid(community_uuid);
    let tenant = resolve_tenant(state, community_id, "tombstone").await?;

    // Match the established channel-moderation `message_deleted` schema
    // (`side_effects.rs`): `type`, `actor` (the acting operator's pubkey hex),
    // and `target_event_id`, plus the admin `action_id`. `reason_code` is the
    // operator's `reason` string (see `finalize_success`) — forwarded as the
    // room-facing public reason; the room renders the moderator-removal template
    // only when a non-empty reason is present.
    let mut content = serde_json::json!({
        "type": "message_deleted",
        "actor": actor,
        "target_event_id": target_event_id,
        "action_id": row.action_id.to_string(),
    });
    if let Some(reason_code) = payload["reason_code"].as_str().filter(|r| !r.is_empty()) {
        content["reason_code"] = serde_json::Value::String(reason_code.to_string());
        content["public_reason"] = serde_json::Value::String(reason_code.to_string());
    }

    crate::handlers::side_effects::emit_system_message(
        &tenant,
        state,
        channel_id,
        content,
        row.created_at,
    )
    .await
    .map_err(|e| format!("tombstone: system message failed: {e}"))
}

/// Deliver a system message for a kick action.
async fn deliver_system_message(state: &Arc<AppState>, row: &OutboxRecord) -> Result<(), String> {
    let payload = &row.payload;
    let community_uuid: Uuid = payload["community_id"]
        .as_str()
        .ok_or("system_message: missing community_id")?
        .parse()
        .map_err(|_| "system_message: invalid community_id")?;
    let channel_id: Uuid = payload["channel_id"]
        .as_str()
        .ok_or("system_message: missing channel_id")?
        .parse()
        .map_err(|_| "system_message: invalid channel_id")?;
    let target_hex = payload["target"]
        .as_str()
        .ok_or("system_message: missing target")?;

    let community_id = buzz_core::CommunityId::from_uuid(community_uuid);
    let tenant = resolve_tenant(state, community_id, "system_message").await?;

    crate::handlers::side_effects::emit_system_message(
        &tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": "admin_kick",
            "target": target_hex,
            "action_id": row.action_id.to_string(),
        }),
        row.created_at,
    )
    .await
    .map_err(|e| format!("system_message: failed: {e}"))
}

/// Deliver a reporter notice DM.
async fn deliver_reporter_notice(state: &Arc<AppState>, row: &OutboxRecord) -> Result<(), String> {
    let payload = &row.payload;
    let action_id: Uuid = payload["action_id"]
        .as_str()
        .ok_or("reporter_notice: missing action_id")?
        .parse()
        .map_err(|_| "reporter_notice: invalid action_id")?;
    let community_uuid: Uuid = payload["community_id"]
        .as_str()
        .ok_or("reporter_notice: missing community_id")?
        .parse()
        .map_err(|_| "reporter_notice: invalid community_id")?;
    let community_id = buzz_core::CommunityId::from_uuid(community_uuid);

    // Load the action record to find the report_id.
    let action = state
        .db
        .get_admin_action(action_id)
        .await
        .map_err(|e| format!("reporter_notice: action lookup failed: {e}"))?
        .ok_or_else(|| "reporter_notice: action not found".to_string())?;

    // Load the report to find reporter_pubkey (hex string in AdminReportDetail).
    let report = state
        .db
        .admin_get_report(action.report_id)
        .await
        .map_err(|e| format!("reporter_notice: report lookup failed: {e}"))?
        .ok_or_else(|| "reporter_notice: report not found".to_string())?;

    let tenant = resolve_tenant(state, community_id, "reporter_notice").await?;

    let reporter_bytes = hex::decode(&report.report.reporter_pubkey)
        .map_err(|_| "reporter_notice: invalid reporter_pubkey hex".to_string())?;

    let summary = payload["summary"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Your report was reviewed and acted on.".to_string());

    use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
    send_moderation_notice(
        &tenant,
        state,
        &reporter_bytes,
        ModerationNotice::ReportResolved {
            report_id: action.report_id,
            status: "resolved".to_string(),
            summary,
        },
        row.created_at,
    )
    .await
    .map_err(|e| format!("reporter_notice: send failed: {e}"))
}

/// Deliver a moderation DM to the actioned user. `delete`/`kick` map to the
/// `ContentActioned` notice, `ban`/`timeout` to `Restriction`. The recipient
/// pubkey and public reason travel in the payload (enqueued in the same
/// finalization transaction as the enforcement), so no extra DB lookup is
/// needed here.
async fn deliver_affected_user_notice(
    state: &Arc<AppState>,
    row: &OutboxRecord,
) -> Result<(), String> {
    let payload = &row.payload;
    let community_uuid: Uuid = payload["community_id"]
        .as_str()
        .ok_or("affected_user_notice: missing community_id")?
        .parse()
        .map_err(|_| "affected_user_notice: invalid community_id")?;
    let recipient = hex::decode(
        payload["recipient"]
            .as_str()
            .ok_or("affected_user_notice: missing recipient")?,
    )
    .map_err(|_| "affected_user_notice: invalid recipient hex")?;
    let notice_kind = payload["notice_kind"]
        .as_str()
        .ok_or("affected_user_notice: missing notice_kind")?;
    let public_reason = payload["public_reason"].as_str().unwrap_or("").to_string();

    use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
    let notice = match notice_kind {
        "content_actioned" => ModerationNotice::ContentActioned {
            action_id: row.action_id,
            public_reason,
        },
        "restriction" => ModerationNotice::Restriction {
            action_id: row.action_id,
            kind: payload["restriction_kind"]
                .as_str()
                .ok_or("affected_user_notice: missing restriction_kind")?
                .to_string(),
            public_reason,
            // Present for `timeout` (the expiry the notice renders); absent for
            // an indefinite `ban`. A malformed timestamp is treated as absent
            // rather than failing delivery — the notice is best-effort.
            timeout_until: payload["timeout_until"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc)),
        },
        other => {
            return Err(format!(
                "affected_user_notice: unknown notice_kind: {other}"
            ))
        }
    };

    let community_id = buzz_core::CommunityId::from_uuid(community_uuid);
    let tenant = resolve_tenant(state, community_id, "affected_user_notice").await?;

    send_moderation_notice(&tenant, state, &recipient, notice, row.created_at)
        .await
        .map_err(|e| format!("affected_user_notice: send failed: {e}"))
}
