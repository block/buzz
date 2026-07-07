//! Community moderation command handler (kinds 9040–9044, Phase 1 contract).
//!
//! Mirrors the NIP-43 relay-admin pattern (`relay_admin.rs`, 9030-series):
//! commands are validated + executed directly and are **never** stored as
//! regular events. Authorization goes through
//! [`crate::handlers::moderation_authz::authorize_moderation_action`] —
//! never inline role checks.
//!
//! | Kind | Operation      | Side effects (all mandatory)                       |
//! |------|----------------|----------------------------------------------------|
//! | 9040 | Ban            | `community_bans` upsert, audit row, live disconnect (L4 fanout), restriction notice DM (L5) |
//! | 9041 | Unban          | ban lift, audit row                                |
//! | 9042 | Timeout        | `muted_until` upsert, audit row, notice DM         |
//! | 9043 | Untimeout      | mute clear, audit row                              |
//! | 9044 | Resolve report | report status update, audit row, reporter notice DM; `delete`/`kick`/`ban` resolutions fan out through the existing 9005/9001 + 9040 paths |
//!
//! Targets (`p` tag pubkey, `report` tag row id) are resolved under the
//! request's `TenantContext` only.
//!
//! ## Routing (pinned — Wren contract review, 2026-07-07)
//! 9040–9044 are **community-global direct commands**, exactly like the
//! relay-admin 9030-series: route via
//! [`buzz_core::kind::is_moderation_command_kind`], list them in
//! `is_global_only_kind` so a stray `h` tag can never channel-scope them
//! (no channel membership/archive gates apply), require a fresh timestamp,
//! never store them, and reject channel-scoped API tokens.
//!
//! ## Tag vocabulary (pinned — CLI and relay must agree)
//! - 9040 ban: `["p", <hex pubkey>]` required; optional
//!   `["expiration", <unix secs>]` (absent ⇒ permanent), `["reason", <text>]`.
//! - 9041 unban: `["p", <hex pubkey>]`.
//! - 9042 timeout: `["p", <hex pubkey>]` + required `["expiration", <unix secs>]`;
//!   optional `["reason", <text>]`.
//! - 9043 untimeout: `["p", <hex pubkey>]`.
//! - 9044 resolve (pinned — thread event `86f46207`, 2026-07-07): required,
//!   exactly one each: `["report", <report event id hex>]` (the 1984 report
//!   being resolved; resolves under `tenant.community()` only),
//!   `["status", resolved|dismissed]`,
//!   `["action", delete|kick|ban|timeout|dismiss|escalate]` (`dismiss` pairs
//!   with status `dismissed`; everything else with `resolved`). Optional
//!   `["reason", <text>]` — audited into `moderation_actions.public_reason`
//!   and relayed in the notice DM (so it must be safe for the reporter's
//!   eyes; `private_reason` is mod-only and not fed by 9044 tags). Unknown
//!   extra tags are ignored, not rejected
//!   (forward-compat). `delete`/`kick`/`ban`/`timeout` actions fan out through
//!   the existing 9005/9001 paths and the 9040/9042 handlers — no second
//!   implementation.
//!
//! Lane ownership: L6 (Quinn) — plus `buzz-cli` `moderation` command group.
//! The `ingest.rs` routing entries (scope map + `is_global_only_kind` +
//! direct-processing dispatch) for 9040–9044 belong to L3 (Perci):
//! coordinate, don't edit ingest.rs.

use std::sync::Arc;

use buzz_core::kind::{
    KIND_MODERATION_BAN, KIND_MODERATION_RESOLVE_REPORT, KIND_MODERATION_TIMEOUT,
    KIND_MODERATION_UNBAN, KIND_MODERATION_UNTIMEOUT,
};
use buzz_core::tenant::TenantContext;
use chrono::{DateTime, TimeZone, Utc};
use nostr::Event;
use tracing::info;
use uuid::Uuid;

use crate::handlers::moderation_authz::{
    authorize_moderation_action, ModerationAction, ModerationTarget,
};
use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
use crate::state::AppState;
use buzz_db::moderation::NewAction;

/// Max clock skew for a freshly-signed command (mirrors `relay_admin.rs` and
/// the NIP-42 auth freshness window). Commands are never stored, so replay of a
/// captured command is the only threat and a tight window is the mitigation.
const MAX_COMMAND_SKEW_SECS: i64 = 120;

/// Validate and execute a moderation command (kinds 9040–9044).
///
/// Returns a client-safe error string for `OK false` on rejection.
///
/// Routing note: 9040–9044 are community-global direct commands (L3 lists them
/// in `is_global_only_kind`), so no `h`/channel context is consulted here; the
/// tenant is bound from the request. Authorization goes through
/// [`authorize_moderation_action`] — never inline role checks.
pub async fn handle_moderation_command(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(), String> {
    let kind = event.kind.as_u16() as u32;
    let actor = event.pubkey.to_bytes().to_vec();

    // Freshness: reject stale/replayed commands (they are never stored).
    let event_ts = event.created_at.as_secs() as i64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if (event_ts - now).abs() > MAX_COMMAND_SKEW_SECS {
        return Err(format!(
            "event timestamp out of range: created_at={event_ts}, now={now}, delta={}s (max ±{MAX_COMMAND_SKEW_SECS}s)",
            event_ts - now
        ));
    }

    match kind {
        KIND_MODERATION_BAN => handle_ban(tenant, state, event, &actor).await,
        KIND_MODERATION_UNBAN => handle_unban(tenant, state, event, &actor).await,
        KIND_MODERATION_TIMEOUT => handle_timeout(tenant, state, event, &actor).await,
        KIND_MODERATION_UNTIMEOUT => handle_untimeout(tenant, state, event, &actor).await,
        KIND_MODERATION_RESOLVE_REPORT => handle_resolve(tenant, state, event, &actor).await,
        other => Err(format!("unexpected moderation command kind: {other}")),
    }
}

// ── 9040: ban ───────────────────────────────────────────────────────────────

async fn handle_ban(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target =
        extract_p_tag_bytes(event).ok_or_else(|| "missing or invalid p tag".to_string())?;
    let expires_at = extract_expiration(event)?; // None ⇒ permanent
    let reason = extract_tag_value(event, "reason");

    authorize_moderation_action(
        tenant,
        state,
        actor,
        None,
        ModerationTarget::Pubkey(&target),
        ModerationAction::Ban,
    )
    .await
    .map_err(authz_denial)?;

    state
        .db
        .ban_community_member(
            tenant.community(),
            &target,
            actor,
            reason.as_deref(),
            expires_at,
        )
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let action_id = insert_audit(
        state,
        tenant,
        actor,
        "ban",
        Some(&target),
        None,
        reason.as_deref(),
    )
    .await?;

    // Live enforcement: close open sessions for the banned principal now —
    // this pod's sockets synchronously (fenced to this community) and every
    // other pod's via the fire-and-forget cross-pod fan-out. The paired helper
    // makes "close locally but forget the Redis publish" unrepresentable, so a
    // live ban takes effect immediately, everywhere (decision 4).
    state.disconnect_pubkey_clusterwide(
        tenant,
        &target,
        &event.id.to_hex(),
        "blocked: you are banned from this community",
    );

    // Notice DM: tell the banned user the terms of the restriction.
    let public_reason = reason.clone().unwrap_or_default();
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        &target,
        ModerationNotice::Restriction {
            action_id,
            kind: "ban".to_string(),
            public_reason,
        },
    )
    .await
    {
        // Notice delivery is best-effort; the ban itself has already landed and
        // been audited. Log and continue rather than fail the command.
        info!(error = %e, "ban notice DM delivery failed (ban still enforced)");
    }

    info!(target = %hex::encode(&target), "community ban applied");
    Ok(())
}

// ── 9041: unban ──────────────────────────────────────────────────────────────

async fn handle_unban(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target =
        extract_p_tag_bytes(event).ok_or_else(|| "missing or invalid p tag".to_string())?;

    authorize_moderation_action(
        tenant,
        state,
        actor,
        None,
        ModerationTarget::Pubkey(&target),
        ModerationAction::Unban,
    )
    .await
    .map_err(authz_denial)?;

    let lifted = state
        .db
        .unban_community_member(tenant.community(), &target, actor)
        .await
        .map_err(|e| format!("database error: {e}"))?;
    if !lifted {
        return Err("member is not banned".to_string());
    }

    insert_audit(state, tenant, actor, "unban", Some(&target), None, None).await?;

    info!(target = %hex::encode(&target), "community ban lifted");
    Ok(())
}

// ── 9042: timeout ────────────────────────────────────────────────────────────

async fn handle_timeout(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target =
        extract_p_tag_bytes(event).ok_or_else(|| "missing or invalid p tag".to_string())?;
    let muted_until = extract_expiration(event)?
        .ok_or_else(|| "timeout requires an expiration tag".to_string())?;
    let reason = extract_tag_value(event, "reason");

    authorize_moderation_action(
        tenant,
        state,
        actor,
        None,
        ModerationTarget::Pubkey(&target),
        ModerationAction::Timeout,
    )
    .await
    .map_err(authz_denial)?;

    state
        .db
        .timeout_community_member(
            tenant.community(),
            &target,
            actor,
            muted_until,
            reason.as_deref(),
        )
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let action_id = insert_audit(
        state,
        tenant,
        actor,
        "timeout",
        Some(&target),
        None,
        reason.as_deref(),
    )
    .await?;

    let public_reason = reason.clone().unwrap_or_default();
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        &target,
        ModerationNotice::Restriction {
            action_id,
            kind: "timeout".to_string(),
            public_reason,
        },
    )
    .await
    {
        info!(error = %e, "timeout notice DM delivery failed (timeout still enforced)");
    }

    info!(target = %hex::encode(&target), "community timeout applied");
    Ok(())
}

// ── 9043: untimeout ──────────────────────────────────────────────────────────

async fn handle_untimeout(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let target =
        extract_p_tag_bytes(event).ok_or_else(|| "missing or invalid p tag".to_string())?;

    authorize_moderation_action(
        tenant,
        state,
        actor,
        None,
        ModerationTarget::Pubkey(&target),
        ModerationAction::Untimeout,
    )
    .await
    .map_err(authz_denial)?;

    let cleared = state
        .db
        .untimeout_community_member(tenant.community(), &target, actor)
        .await
        .map_err(|e| format!("database error: {e}"))?;
    if !cleared {
        return Err("member is not timed out".to_string());
    }

    insert_audit(state, tenant, actor, "untimeout", Some(&target), None, None).await?;

    info!(target = %hex::encode(&target), "community timeout cleared");
    Ok(())
}

// ── 9044: resolve report ─────────────────────────────────────────────────────

async fn handle_resolve(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let report_event_id = extract_report_tag(event)
        .ok_or_else(|| "missing or invalid report tag (expect 64-hex event id)".to_string())?;
    let status =
        extract_tag_value(event, "status").ok_or_else(|| "missing status tag".to_string())?;
    let action =
        extract_tag_value(event, "action").ok_or_else(|| "missing action tag".to_string())?;
    let reason = extract_tag_value(event, "reason");

    // Vocab is validated at build time in the SDK, but the relay must not trust
    // the client: re-validate the pinned vocabulary here.
    if status != "resolved" && status != "dismissed" {
        return Err(format!(
            "invalid status: {status} (expect resolved|dismissed)"
        ));
    }
    if !matches!(
        action.as_str(),
        "delete" | "kick" | "ban" | "timeout" | "dismiss" | "escalate"
    ) {
        return Err(format!(
            "invalid action: {action} (expect delete|kick|ban|timeout|dismiss|escalate)"
        ));
    }
    if (action == "dismiss") != (status == "dismissed") {
        return Err("action `dismiss` pairs only with status `dismissed`".to_string());
    }

    authorize_moderation_action(
        tenant,
        state,
        actor,
        None,
        ModerationTarget::Event(&report_event_id),
        ModerationAction::ResolveReport,
    )
    .await
    .map_err(authz_denial)?;

    // Resolve the report row under this tenant only. The `report` tag carries
    // the signed 1984 event id (pinned contract); look the row up by it.
    let report = state
        .db
        .get_moderation_report_by_event(tenant.community(), &report_event_id)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| "report not found in this community".to_string())?;

    // Carry the report's own target into the audit row so `delete`/`kick`/`ban`
    // resolutions record what they acted on.
    let (target_pubkey, target_event_id) = match &report.target {
        buzz_db::moderation::ReportTarget::Pubkey(p) => (Some(p.as_slice()), None),
        buzz_db::moderation::ReportTarget::Event(e) => (None, Some(e.as_slice())),
        buzz_db::moderation::ReportTarget::Blob(_) => (None, None),
    };

    let audit_action = match action.as_str() {
        "dismiss" => "dismiss_report",
        "escalate" => "escalate",
        other => other, // delete | kick | ban | timeout — fan out via existing paths
    };
    let action_id = insert_audit(
        state,
        tenant,
        actor,
        audit_action,
        target_pubkey,
        target_event_id,
        reason.as_deref(),
    )
    .await?;

    let resolved = state
        .db
        .resolve_moderation_report(
            tenant.community(),
            report.id,
            &status,
            actor,
            Some(action_id),
        )
        .await
        .map_err(|e| format!("database error: {e}"))?;
    if !resolved {
        return Err("report is not open (already resolved or dismissed)".to_string());
    }

    // Close the loop: DM the reporter that their report was reviewed.
    let summary = reason.clone().unwrap_or_else(|| match status.as_str() {
        "dismissed" => "Your report was reviewed and dismissed.".to_string(),
        _ => "Your report was reviewed and acted on.".to_string(),
    });
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        &report.reporter_pubkey,
        ModerationNotice::ReportResolved {
            report_id: report.id,
            status: status.clone(),
            summary,
        },
    )
    .await
    {
        info!(error = %e, "report-resolution notice DM delivery failed (report still resolved)");
    }

    info!(report_id = %report.id, status = %status, action = %action, "report resolved");
    Ok(())
}

// ── shared helpers ────────────────────────────────────────────────────────────

/// Insert a moderation audit row for an accepted command. `matched_principal`
/// is left `None` here: that NIP-OA field records which principal an
/// *enforcement* check matched at the auth seam (L4), not who issued a command.
async fn insert_audit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &[u8],
    action: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    public_reason: Option<&str>,
) -> Result<Uuid, String> {
    state
        .db
        .insert_moderation_action(
            tenant.community(),
            NewAction {
                actor_pubkey: actor,
                action,
                target_pubkey,
                target_event_id,
                channel_id: None,
                reason_code: None,
                public_reason,
                private_reason: None,
                matched_principal: None,
            },
        )
        .await
        .map_err(|e| format!("failed to write audit row: {e}"))
}

/// Map an authorization error to a client-safe `restricted:`-prefixed denial.
fn authz_denial(e: anyhow::Error) -> String {
    format!("restricted: {e}")
}

/// Extract the first valid `p` tag as raw pubkey bytes (32 bytes).
fn extract_p_tag_bytes(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some("p") {
            if let Some(val) = parts.get(1).map(|s| s.as_str()) {
                if val.len() == 64 && val.chars().all(|c| c.is_ascii_hexdigit()) {
                    return hex::decode(val).ok();
                }
            }
        }
    }
    None
}

/// Extract the `report` tag as a 32-byte event id (the signed 1984 report).
fn extract_report_tag(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some("report") {
            if let Some(val) = parts.get(1).map(|s| s.as_str()) {
                if val.len() == 64 && val.chars().all(|c| c.is_ascii_hexdigit()) {
                    return hex::decode(val).ok();
                }
            }
        }
    }
    None
}

/// Parse an optional `expiration` tag (unix seconds) into a UTC timestamp.
/// Returns `Ok(None)` when absent, `Err` on a malformed value.
fn extract_expiration(event: &Event) -> Result<Option<DateTime<Utc>>, String> {
    match extract_tag_value(event, "expiration") {
        None => Ok(None),
        Some(raw) => {
            let secs: i64 = raw
                .parse()
                .map_err(|_| format!("invalid expiration tag: {raw}"))?;
            match Utc.timestamp_opt(secs, 0).single() {
                Some(ts) => Ok(Some(ts)),
                None => Err(format!("expiration out of range: {secs}")),
            }
        }
    }
}

/// Extract the value of the first tag with the given name.
fn extract_tag_value(event: &Event, name: &str) -> Option<String> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some(name) {
            return parts.get(1).map(|s| s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Build a signed event with the given kind, timestamp, and tags.
    fn make_event(kind: u16, created_at_secs: u64, tags: Vec<Vec<String>>) -> Event {
        let keys = Keys::generate();
        let nostr_tags: Vec<Tag> = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::from(kind), "")
            .tags(nostr_tags)
            .custom_created_at(nostr::Timestamp::from_secs(created_at_secs))
            .sign_with_keys(&keys)
            .expect("signing failed")
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn extract_p_tag_bytes_valid() {
        let hex = "a".repeat(64);
        let e = make_event(9040, now_secs(), vec![vec!["p".into(), hex.clone()]]);
        assert_eq!(extract_p_tag_bytes(&e), hex::decode(&hex).ok());
    }

    #[test]
    fn extract_p_tag_bytes_rejects_short_and_nonhex() {
        assert_eq!(
            extract_p_tag_bytes(&make_event(
                9040,
                now_secs(),
                vec![vec!["p".into(), "abcd".into()]]
            )),
            None
        );
        let bad = "g".repeat(64);
        assert_eq!(
            extract_p_tag_bytes(&make_event(9040, now_secs(), vec![vec!["p".into(), bad]])),
            None
        );
    }

    #[test]
    fn extract_report_tag_requires_64_hex() {
        let id = "b".repeat(64);
        let e = make_event(9044, now_secs(), vec![vec!["report".into(), id.clone()]]);
        assert_eq!(extract_report_tag(&e), hex::decode(&id).ok());
        // A UUID-shaped value (Wren's L5 lesson: never a UUID where an event id belongs).
        let uuid = make_event(
            9044,
            now_secs(),
            vec![vec![
                "report".into(),
                "550e8400-e29b-41d4-a716-446655440000".into(),
            ]],
        );
        assert_eq!(extract_report_tag(&uuid), None);
    }

    #[test]
    fn expiration_absent_is_none() {
        let e = make_event(9040, now_secs(), vec![]);
        assert_eq!(extract_expiration(&e).unwrap(), None);
    }

    #[test]
    fn expiration_valid_parses() {
        let e = make_event(
            9040,
            now_secs(),
            vec![vec!["expiration".into(), "1893456000".into()]],
        );
        assert_eq!(
            extract_expiration(&e).unwrap(),
            Utc.timestamp_opt(1_893_456_000, 0).single()
        );
    }

    #[test]
    fn expiration_malformed_errs() {
        let e = make_event(
            9040,
            now_secs(),
            vec![vec!["expiration".into(), "not-a-number".into()]],
        );
        assert!(extract_expiration(&e).is_err());
    }

    #[test]
    fn expiration_out_of_range_errs() {
        let e = make_event(
            9040,
            now_secs(),
            vec![vec!["expiration".into(), "99999999999999".into()]],
        );
        assert!(extract_expiration(&e).is_err());
    }
}
