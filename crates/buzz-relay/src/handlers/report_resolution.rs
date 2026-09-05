//! Report-resolution orchestrations for the HTTP admin API (Phase 2).
//!
//! Two transport-independent orchestration functions:
//!
//! - [`resolve_report_decision_only`] — HTTP `dismiss`/`escalate` and 9044
//!   community-moderation. Atomically CASes report to terminal status with
//!   a linked decision audit row in **one transaction**.
//!
//! - [`resolve_report_with_enforcement`] — HTTP `delete`/`kick`/`ban`/`timeout`.
//!   Claims the report (`open → processing`) in one transaction, acquires an
//!   action lease to prevent concurrent double-mutation, executes the durable
//!   enforcement mutation and step marker in **one atomic DB transaction**
//!   (via `execute_*_with_marker`), then finalizes (action → succeeded, report →
//!   resolved, outbox rows enqueued) in a third transaction.  Delivery is driven
//!   by the outbox worker ([`crate::handlers::admin_outbox_worker`]) and the
//!   action recovery worker ([`crate::handlers::admin_action_worker`]) — **never
//!   from this request path**.
//!
//! ## Crash safety
//!
//! Each enforcement mutation and its `step_marker = 'mutation_committed'` are
//! written in a single PG transaction (`execute_*_with_marker`). A crash between
//! claim and the mutation transaction leaves the action in `pending`/`enforcing`
//! with no step marker — the action recovery worker re-drives it via
//! `claim_stranded_action_batch`. A crash after `mutation_committed` re-drives
//! directly to `finalize_success` (marker already set → skip mutation). A crash
//! after finalization leaves outbox rows pending for the outbox worker.
//!
//! ## Action lease
//!
//! An action lease token prevents two concurrent drivers (two HTTP retries with
//! the same `request_id`) from both running the mutation branch. The loser of
//! `acquire_action_lease` gets `Contended`, reloads, and loops — seeing the
//! updated step state rather than re-running the mutation.
//!
//! ## Action matrix (frozen per Plan v3/v4 §7)
//!
//! | target_kind | actions |
//! |-------------|---------|
//! | event  | delete, kick, ban, timeout, dismiss, escalate |
//! | pubkey | ban, timeout, dismiss, escalate |
//! | blob   | dismiss, escalate |

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tracing::{info, warn};
use uuid::Uuid;

use buzz_core::tenant::TenantContext;
use buzz_db::admin_moderation::AdminReportDetail;
use buzz_db::relay_admin_actions::{AdminActionRecord, ClaimResult};

use crate::state::AppState;

/// Error returned by the resolution orchestrations.
#[derive(Debug)]
pub enum ResolutionError {
    /// The report was not found globally.
    NotFound,
    /// The report is not in `open` status. Includes current status.
    NotOpen(String),
    /// The action is not valid for this report's target kind.
    InvalidAction(String),
    /// Enforcement failed (durable mutation did not commit). Action record is
    /// left in `failed` state.
    EnforcementFailed {
        /// UUID of the action record.
        action_id: Uuid,
        /// Human-readable error from the failed enforcement step.
        error: String,
    },
    /// Internal database or infrastructure error.
    Internal(String),
}

impl From<buzz_db::DbError> for ResolutionError {
    fn from(e: buzz_db::DbError) -> Self {
        ResolutionError::Internal(e.to_string())
    }
}

/// Successful outcome of a decision-only resolution.
#[derive(Debug)]
pub struct DecisionResolved {
    /// The terminal status applied.
    pub status: String,
}

/// Successful outcome of an enforcement resolution.
#[derive(Debug)]
pub struct EnforcementResolved {
    /// The action record for the completed enforcement.
    pub action_id: Uuid,
}

/// Validate the action/target matrix and derive HTTP terminal status.
///
/// Returns `Ok(status)` where status is `"dismissed"`, `"escalated"`, or
/// `"resolved"`. Returns `Err` with a human-readable message if the combination
/// is invalid per the frozen action matrix.
pub fn http_validate_and_derive_status(
    action: &str,
    target_kind: &str,
    channel_id: Option<Uuid>,
    timeout_until: Option<DateTime<Utc>>,
) -> Result<String, String> {
    // Validate action/target matrix.
    let valid = matches!(
        (action, target_kind),
        (
            "delete" | "kick" | "ban" | "timeout" | "dismiss" | "escalate",
            "event"
        ) | ("ban" | "timeout" | "dismiss" | "escalate", "pubkey")
            | ("dismiss" | "escalate", "blob")
    );
    if !valid {
        return Err(format!(
            "action `{action}` is not valid for `{target_kind}` reports"
        ));
    }

    // kick requires channel_id from the report row.
    if action == "kick" && channel_id.is_none() {
        return Err("action `kick` requires the report to have an associated channel".to_string());
    }

    // timeout requires expiration; other actions reject it.
    if action == "timeout" && timeout_until.is_none() {
        return Err("`expiration_secs` is required for `timeout`".to_string());
    }
    if action != "timeout" && timeout_until.is_some() {
        return Err(format!(
            "`expiration_secs` is only valid for `timeout`, got `{action}`"
        ));
    }

    // Derive HTTP terminal status.
    Ok(match action {
        "dismiss" => "dismissed",
        "escalate" => "escalated",
        _ => "resolved",
    }
    .to_string())
}

/// Map enforcement action → decision audit row action string.
pub fn enforcement_audit_action(action: &str) -> &'static str {
    match action {
        "delete" => "resolve:delete",
        "kick" => "resolve:kick",
        "ban" => "resolve:ban",
        "timeout" => "resolve:timeout",
        "dismiss" => "dismiss_report",
        "escalate" => "escalate",
        _ => "resolve:delete",
    }
}

/// Atomically resolve a report without server-side enforcement.
///
/// Used by:
/// - HTTP `dismiss` and `escalate`.
/// - The 9044 community-moderation adapter (caller passes the event's signed
///   `status`; `actor_authority` = `"community"`).
///
/// Performs the CAS `open→terminal` AND the decision audit row insert in one
/// transaction via `db.resolve_report_decision_atomic`. A concurrent close
/// rolls back both — no orphan audit row. Reporter notice is best-effort
/// after commit.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_report_decision_only(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    report_id: Uuid,
    terminal_status: &str,
    audit_action: &str,
    actor_pubkey: &[u8],
    actor_authority: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    channel_id: Option<Uuid>,
    reason: Option<&str>,
    reporter_pubkey: &[u8],
) -> Result<DecisionResolved, ResolutionError> {
    let community_id = tenant.community();

    // Single-transaction CAS + audit — no orphan row on concurrent close.
    let resolved = state
        .db
        .resolve_report_decision_atomic(
            community_id,
            report_id,
            terminal_status,
            audit_action,
            actor_pubkey,
            actor_authority,
            target_pubkey,
            target_event_id,
            channel_id,
            reason,
        )
        .await
        .map_err(ResolutionError::from)?;

    if !resolved {
        return Err(ResolutionError::NotOpen("concurrent_close".to_string()));
    }

    // Best-effort reporter notice after commit.
    use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
    let summary = reason
        .map(|r| r.to_string())
        .unwrap_or_else(|| match terminal_status {
            "dismissed" => "Your report was reviewed and dismissed.".to_string(),
            "escalated" => "Your report has been escalated for further review.".to_string(),
            _ => "Your report was reviewed and acted on.".to_string(),
        });
    if let Err(e) = send_moderation_notice(
        tenant,
        state,
        reporter_pubkey,
        ModerationNotice::ReportResolved {
            report_id,
            status: terminal_status.to_string(),
            summary,
        },
        chrono::Utc::now(),
    )
    .await
    {
        warn!(error = %e, report_id = %report_id, "reporter notice delivery failed");
    }

    info!(report_id = %report_id, status = %terminal_status, "report resolved (decision-only)");
    Ok(DecisionResolved {
        status: terminal_status.to_string(),
    })
}

/// Resolve a report with server-side enforcement.
///
/// Claims report via CAS (`open → processing`) in one transaction, acquires an
/// action lease, runs the durable enforcement mutation + step marker in one atomic
/// DB transaction, then finalizes. Delivery never runs from this path.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_report_with_enforcement(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    report: &AdminReportDetail,
    action: &str,
    reason: Option<&str>,
    timeout_until: Option<DateTime<Utc>>,
    request_id: Uuid,
    actor_pubkey: &[u8],
    actor_role: &str,
    actor_authority: &str,
) -> Result<EnforcementResolved, ResolutionError> {
    let community_id = tenant.community();
    let report_id = report.report.id;
    let channel_id = report.report.channel_id;

    let (target_pubkey_opt, target_event_id_opt) = derive_enforcement_target(report)?;

    // Pre-claim guard: person-directed enforcement on an `event` report needs a
    // resolvable target user. A report row never stores the reported event's
    // author (the reporter `p` tag is validation-shape only), so it is derived
    // from the stored event row. When that row is missing — the event was purged
    // (or never accepted) before its author could be determined — reject BEFORE
    // claiming, so the report is never dirtied: it stays `open` with no failed
    // action to cancel-and-reopen. `delete` needs only the event id and is exempt
    // (a purged event is an idempotent no-op delete). A soft-deleted event still
    // carries a real author, so this rejects only a wholly absent event row.
    if matches!(action, "kick" | "ban" | "timeout") && target_pubkey_opt.is_none() {
        return Err(ResolutionError::InvalidAction(format!(
            "action `{action}` requires a resolvable target user, but the reported event is missing \
             or was deleted before its author could be determined"
        )));
    }

    let audit_action = enforcement_audit_action(action);

    // Claim: one transaction — audit row + action record + report CAS open→processing.
    let action_record = match state
        .db
        .claim_report_for_enforcement(
            community_id,
            report_id,
            request_id,
            actor_pubkey,
            actor_role,
            action,
            reason,
            timeout_until,
            audit_action,
            actor_authority,
            target_pubkey_opt.as_deref(),
            target_event_id_opt.as_deref(),
            channel_id,
        )
        .await
        .map_err(ResolutionError::from)?
    {
        ClaimResult::Claimed(a) => a,
        ClaimResult::AlreadyClaimed(a) => {
            // Idempotent retry: the report was already claimed under this
            // request_id. A retry that changes `action`/`reason`/`timeout_until`/
            // actor must NOT execute or finalize the new values — that would drive
            // an action the persisted audit record does not describe. Log the
            // divergence and drive exclusively from the persisted record below
            // (single source of truth: retries converge to the first outcome).
            if a.action != action
                || a.reason.as_deref() != reason
                || a.timeout_until != timeout_until
                || a.actor_pubkey.as_slice() != actor_pubkey
            {
                warn!(
                    action_id = %a.id,
                    request_id = %request_id,
                    persisted_action = %a.action,
                    retry_action = %action,
                    "idempotent retry body differs from the persisted claim; driving from the persisted record"
                );
            }
            a
        }
        ClaimResult::NotOpen(status) => return Err(ResolutionError::NotOpen(status)),
        ClaimResult::NotFound => return Err(ResolutionError::NotFound),
    };

    // Drive and finalize from the persisted record's fields — the single source
    // of truth for this action. For a fresh `Claimed`, these equal the request
    // values; for an `AlreadyClaimed` retry, they are the first claim's values,
    // so a changed retry body can never diverge the executed mutation, the outbox
    // payloads, or the audit record from the first claim.
    drive_enforcement(
        state,
        tenant,
        community_id,
        report_id,
        &action_record.action,
        action_record.reason.as_deref(),
        action_record.timeout_until,
        &action_record.actor_pubkey,
        target_pubkey_opt.as_deref(),
        target_event_id_opt.as_deref(),
        channel_id,
        &action_record,
        None, // HTTP path: no pre-held lease
    )
    .await
}

/// Context for the enforcement mutation — reduces argument count.
struct EnforcementCtx<'a> {
    community_id: buzz_core::tenant::CommunityId,
    action: &'a str,
    reason: Option<&'a str>,
    timeout_until: Option<DateTime<Utc>>,
    actor_pubkey: &'a [u8],
    target_pubkey: Option<&'a [u8]>,
    target_event_id: Option<&'a [u8]>,
    channel_id: Option<Uuid>,
}

/// Drive the enforcement state machine from the given action record forward to
/// completion.
///
/// Uses a loop (not recursion) to advance through CAS contention and lease
/// contention without boxing async futures. The loop terminates because each
/// iteration either returns or advances the action to a strictly later state
/// (pending → enforcing → mutation_committed → succeeded/failed).
///
/// Each enforcement mutation and its `step_marker = 'mutation_committed'` are
/// committed in a **single DB transaction** (`execute_*_with_marker`), guarded
/// by an action lease to prevent two concurrent drivers from both running the
/// mutation. Delivery rows are created atomically in `finalize_success` — never
/// before enforcement succeeds.
#[allow(clippy::too_many_arguments)]
async fn drive_enforcement(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    community_id: buzz_core::tenant::CommunityId,
    report_id: Uuid,
    action: &str,
    reason: Option<&str>,
    timeout_until: Option<DateTime<Utc>>,
    actor_pubkey: &[u8],
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    channel_id: Option<Uuid>,
    initial_record: &AdminActionRecord,
    // Pre-held lease token from a batch claim (e.g. stranded action worker).
    // When present, skip the acquire_action_lease call — the caller already
    // holds an exclusive lease on this action row.
    held_lease: Option<Uuid>,
) -> Result<EnforcementResolved, ResolutionError> {
    // Work on an owned copy so we can replace it when reloading.
    let mut rec = initial_record.clone();
    let action_id = rec.id;
    // Maximum iterations while waiting for lease contention to resolve (HTTP path only).
    // 30 × 100 ms = 3 s. Once exceeded, return a retryable error and let the recovery
    // worker converge the action asynchronously.
    let mut contention_attempts: u32 = 0;
    const MAX_CONTENTION_ATTEMPTS: u32 = 30;

    loop {
        // Already finalized — idempotent success.
        if rec.state == "succeeded" {
            return Ok(EnforcementResolved { action_id });
        }

        // Pre-mutation failure — surface error; caller retries with a new request_id.
        if rec.state == "failed" {
            return Err(ResolutionError::EnforcementFailed {
                action_id,
                error: rec.error_message.clone().unwrap_or_default(),
            });
        }

        // Advance to enforcing if still pending. False CAS = another driver won;
        // reload and loop — the reloaded state will be enforcing/succeeded/failed.
        if rec.state == "pending" {
            let advanced = state
                .db
                .begin_enforcing_action(action_id)
                .await
                .map_err(ResolutionError::from)?;
            if !advanced {
                rec = state
                    .db
                    .get_admin_action(action_id)
                    .await
                    .map_err(ResolutionError::from)?
                    .ok_or_else(|| {
                        ResolutionError::Internal("action disappeared after claim".to_string())
                    })?;
                continue;
            }
            // Re-read the updated record so step_marker check below is correct.
            rec = state
                .db
                .get_admin_action(action_id)
                .await
                .map_err(ResolutionError::from)?
                .ok_or_else(|| {
                    ResolutionError::Internal(
                        "action disappeared after begin_enforcing".to_string(),
                    )
                })?;
        }

        // Run mutation only if step marker is not yet committed.
        if rec.step_marker.is_none() {
            // Acquire exclusive action lease before running the mutation. Two
            // concurrent HTTP retries with the same request_id would both reach
            // this branch; the lease ensures only one runs the mutation.
            // When the caller already holds a lease (e.g. the stranded-action
            // recovery worker after a batch claim), skip re-acquisition.
            let lease_token = if let Some(token) = held_lease {
                token
            } else {
                let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
                let lease = state
                    .db
                    .acquire_admin_action_lease(action_id, lease_until)
                    .await
                    .map_err(ResolutionError::from)?;

                match lease {
                    buzz_db::relay_admin_actions::LeaseResult::Acquired(token) => token,
                    buzz_db::relay_admin_actions::LeaseResult::Contended => {
                        // Another driver holds the lease. Wait briefly, reload, and loop.
                        // Bounded: after MAX_CONTENTION_ATTEMPTS (≈3 s), return a retryable
                        // error so the HTTP request is not held indefinitely. The recovery
                        // worker will converge the action once the lease expires.
                        contention_attempts += 1;
                        if contention_attempts >= MAX_CONTENTION_ATTEMPTS {
                            return Err(ResolutionError::Internal(format!(
                                "action {action_id} lease contention unresolved after {contention_attempts} attempts; \
                                 recovery worker will complete"
                            )));
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        rec = state
                            .db
                            .get_admin_action(action_id)
                            .await
                            .map_err(ResolutionError::from)?
                            .ok_or_else(|| {
                                ResolutionError::Internal(
                                    "action disappeared while waiting for lease".to_string(),
                                )
                            })?;
                        continue;
                    }
                    buzz_db::relay_admin_actions::LeaseResult::NotLeasable => {
                        // Action reached a terminal state concurrently. Reload.
                        rec = state
                            .db
                            .get_admin_action(action_id)
                            .await
                            .map_err(ResolutionError::from)?
                            .ok_or_else(|| {
                                ResolutionError::Internal(
                                    "action disappeared (not leasable)".to_string(),
                                )
                            })?;
                        continue;
                    }
                }
            };

            // We hold the lease — run the atomic mutation + marker.
            let ctx = EnforcementCtx {
                community_id,
                action,
                reason,
                timeout_until,
                actor_pubkey,
                target_pubkey,
                target_event_id,
                channel_id,
            };
            let mutation_result = run_atomic_mutation(state, action_id, lease_token, &ctx).await;

            // On enforcement error, record the failure while we STILL hold the
            // lease — `record_action_failure` is fenced on the live token, so it
            // must run before the release below. A `false` return means the lease
            // was lost (our lease expired and another pod reclaimed the action);
            // that is not a terminal failure — the new owner will converge it, so
            // we surface a retryable error rather than marking the report failed.
            let mut failure_lease_lost = false;
            if let Err(e) = &mutation_result {
                match state
                    .db
                    .record_action_failure(action_id, lease_token, &e.to_string())
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        failure_lease_lost = true;
                        warn!(
                            action_id = %action_id,
                            "enforcement failed but action lease was lost; \
                             recovery worker will converge"
                        );
                    }
                    Err(db_err) => {
                        warn!(action_id = %action_id, error = %db_err, "record_action_failure failed");
                    }
                }
            }

            // Release lease regardless of outcome so the action worker
            // can pick up a failed action. Skip if we were given the lease
            // from a batch claim (caller manages its own lease lifecycle).
            if held_lease.is_none() {
                let _ = state
                    .db
                    .release_admin_action_lease(action_id, lease_token)
                    .await;
            }

            match mutation_result {
                Ok(MutationOutcome::AlreadyCommitted) => {
                    // step_marker already set by a concurrent driver. Reload and
                    // advance to finalization; live side effects fire at the
                    // convergence point below (after the is_none block).
                    rec = state
                        .db
                        .get_admin_action(action_id)
                        .await
                        .map_err(ResolutionError::from)?
                        .ok_or_else(|| {
                            ResolutionError::Internal(
                                "action disappeared after mutation".to_string(),
                            )
                        })?;
                    continue;
                }
                Ok(MutationOutcome::LeaseLost) => {
                    // This driver's lease has expired. Another pod has reclaimed
                    // (or will reclaim) the action. Do NOT loop with the same
                    // expired token — that would spin the recovery worker in a
                    // tight DB loop forever on a single-pod deployment. Return a
                    // retryable error; the recovery worker owns convergence.
                    return Err(ResolutionError::Internal(format!(
                        "action {action_id} lease lost mid-mutation; recovery worker will complete"
                    )));
                }
                Ok(MutationOutcome::Committed) => {
                    // Marker committed. Fall through to the convergence point
                    // below for live side effects and finalization.
                }
                Err(e) => {
                    if failure_lease_lost {
                        // The failure could not be recorded because the lease was
                        // lost; the reclaiming owner drives the action. Retryable,
                        // not terminal.
                        return Err(ResolutionError::Internal(format!(
                            "action {action_id} failed but lease lost; recovery worker will complete"
                        )));
                    }
                    return Err(ResolutionError::EnforcementFailed {
                        action_id,
                        error: e.to_string(),
                    });
                }
            }
        }
        // ── Convergence point ────────────────────────────────────────────────
        // Reached on every path where the step marker is (or was just) committed:
        // the fresh HTTP path (Committed above), a concurrent-driver path
        // (AlreadyCommitted → reload → loop reaches here with marker set), and
        // the crash-recovery path (process died after DB commit but before live
        // effects; recovery worker re-enters here directly with marker set).
        //
        // Live side effects for kick use the target context persisted at claim
        // time (enforcement_target_pubkey / enforcement_channel_id) rather than
        // the function parameters, which on the recovery path are re-derived from
        // mutable sources that may have changed or been purged since the kick
        // committed.  Missing persisted context is an invariant failure: the
        // INSERT that claimed the action required both values and stored them; if
        // they are absent the row is corrupt and we must not silently succeed.
        //
        // Eviction and workflow-disable are fenced behind membership_removal_fence
        // (which holds the per-channel advisory lock through both effects) so a
        // kick-commit → re-add → re-drive race does not revoke a legitimately
        // restored membership. Cache invalidation is unconditional because
        // stale-positive is always safe to drop. The fence applies on every path
        // (fresh and recovery) for a single consistent ordering guarantee.
        if action == "kick" {
            match (
                rec.enforcement_target_pubkey.as_deref(),
                rec.enforcement_channel_id,
            ) {
                (Some(target), Some(ch)) => {
                    crate::handlers::side_effects::apply_kick_live_side_effects(
                        tenant, state, ch, target,
                    )
                    .await
                    .map_err(|e| {
                        ResolutionError::Internal(format!(
                            "kick action {action_id} live side effects failed \
                             (mutation_committed marker is recoverable; worker will retry): {e}"
                        ))
                    })?;
                }
                _ => {
                    return Err(ResolutionError::Internal(format!(
                        "kick action {action_id} reached convergence with missing \
                         enforcement_target_pubkey or enforcement_channel_id — \
                         action row is corrupt; refusing to finalize as succeeded"
                    )));
                }
            }
        }
        // Finalize: action → succeeded, report → resolved, outbox rows created.
        // Requires step_marker = 'mutation_committed' AND active_action_id = this action.
        let finalized = state
            .db
            .finalize_action_success(
                action_id,
                community_id,
                report_id,
                "resolved",
                actor_pubkey,
                action,
                target_pubkey,
                target_event_id,
                channel_id,
                reason,
                timeout_until,
            )
            .await
            .map_err(ResolutionError::from)?;

        if !finalized {
            rec = state
                .db
                .get_admin_action(action_id)
                .await
                .map_err(ResolutionError::from)?
                .ok_or_else(|| {
                    ResolutionError::Internal("action disappeared during finalization".to_string())
                })?;
            if rec.state == "succeeded" {
                return Ok(EnforcementResolved { action_id });
            }
            return Err(ResolutionError::Internal(format!(
                "finalize_success failed (state={}, step={:?})",
                rec.state, rec.step_marker
            )));
        }

        info!(action_id = %action_id, report_id = %report_id, action = %action, "enforcement resolved");
        return Ok(EnforcementResolved { action_id });
    }
}

/// Outcome of an atomic mutation attempt.
enum MutationOutcome {
    /// This driver committed the domain mutation and the step marker.
    Committed,
    /// Another driver already set the step marker; no domain writes occurred.
    AlreadyCommitted,
    /// The caller's lease token is expired or no longer owned by this driver.
    /// The caller must stop driving this action — the recovery worker will pick
    /// it up once the new owner's lease expires.
    LeaseLost,
}

/// Execute the enforcement mutation AND commit `step_marker = 'mutation_committed'`
/// in a single DB transaction, fenced by `action_id` AND `lease_token`.
///
/// Returns:
/// - [`MutationOutcome::Committed`] — this driver committed the marker.
/// - [`MutationOutcome::AlreadyCommitted`] — another driver set the marker first.
/// - [`MutationOutcome::LeaseLost`] — the caller's lease has expired; the caller
///   must stop and let the recovery worker take over.
/// - `Err` — the mutation itself failed (DB or validation error).
async fn run_atomic_mutation(
    state: &Arc<AppState>,
    action_id: Uuid,
    lease_token: Uuid,
    ctx: &EnforcementCtx<'_>,
) -> anyhow::Result<MutationOutcome> {
    // Returns Ok(true) if this driver set the marker, Ok(false) if the lease
    // ownership fence rejected the transaction (lease lost or marker already set
    // by a concurrent driver). We classify Ok(false) by reloading the row.
    let raw: anyhow::Result<bool> = match ctx.action {
        "ban" => {
            let target = ctx
                .target_pubkey
                .ok_or_else(|| anyhow::anyhow!("ban requires target_pubkey"))?;
            state
                .db
                .execute_ban_with_marker(
                    action_id,
                    lease_token,
                    ctx.community_id,
                    target,
                    ctx.actor_pubkey,
                    ctx.reason,
                )
                .await
                .map_err(|e| anyhow::anyhow!("ban failed: {e}"))
        }
        "timeout" => {
            let target = ctx
                .target_pubkey
                .ok_or_else(|| anyhow::anyhow!("timeout requires target_pubkey"))?;
            let until = ctx
                .timeout_until
                .ok_or_else(|| anyhow::anyhow!("timeout requires timeout_until"))?;
            state
                .db
                .execute_timeout_with_marker(
                    action_id,
                    lease_token,
                    ctx.community_id,
                    target,
                    ctx.actor_pubkey,
                    until,
                    ctx.reason,
                )
                .await
                .map_err(|e| anyhow::anyhow!("timeout failed: {e}"))
        }
        "kick" => {
            let target = ctx
                .target_pubkey
                .ok_or_else(|| anyhow::anyhow!("kick requires target_pubkey"))?;
            let ch = ctx
                .channel_id
                .ok_or_else(|| anyhow::anyhow!("kick requires channel_id"))?;
            match state
                .db
                .execute_kick_with_marker(
                    action_id,
                    lease_token,
                    ctx.community_id,
                    ch,
                    target,
                    ctx.actor_pubkey,
                )
                .await
                .map_err(|e| anyhow::anyhow!("kick failed: {e}"))?
            {
                buzz_db::relay_admin_actions::KickWithMarkerResult::Removed => Ok(true),
                buzz_db::relay_admin_actions::KickWithMarkerResult::AlreadyMarked => Ok(false),
                buzz_db::relay_admin_actions::KickWithMarkerResult::AlreadyGone => Err(
                    anyhow::anyhow!("kick target was already absent before this action"),
                ),
            }
        }
        "delete" => {
            let target = ctx
                .target_event_id
                .ok_or_else(|| anyhow::anyhow!("delete requires target_event_id"))?;
            let meta = state
                .db
                .get_thread_metadata_by_event(ctx.community_id, target)
                .await
                .map_err(|e| anyhow::anyhow!("thread metadata lookup failed: {e}"))?;
            let parent_id = meta.as_ref().and_then(|m| m.parent_event_id.clone());
            let root_id = meta.as_ref().and_then(|m| m.root_event_id.clone());
            state
                .db
                .execute_delete_with_marker(
                    action_id,
                    lease_token,
                    ctx.community_id,
                    target,
                    parent_id.as_deref(),
                    root_id.as_deref(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("delete failed: {e}"))
        }
        other => Err(anyhow::anyhow!("unexpected enforcement action: {other}")),
    };

    match raw? {
        true => Ok(MutationOutcome::Committed),
        false => {
            // Reload to distinguish "step_marker already set by another driver"
            // (AlreadyCommitted — safe to proceed to finalization) from "this
            // driver's lease expired" (LeaseLost — must stop, recovery worker
            // will take over after expiry).
            let rec = state
                .db
                .get_admin_action(action_id)
                .await
                .map_err(|e| anyhow::anyhow!("classify mutation result: {e}"))?;
            match rec {
                Some(r) if r.step_marker.is_some() => Ok(MutationOutcome::AlreadyCommitted),
                _ => Ok(MutationOutcome::LeaseLost),
            }
        }
    }
}

/// Decode the report target hex into binary (public for the action recovery worker).
pub type TargetPair = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Derive the enforcement target from a full report detail.
///
/// This is the single source of truth for "who/what does enforcement act on",
/// shared by the HTTP driver ([`resolve_report_with_enforcement`]) and the action
/// recovery worker (via [`derive_enforcement_target_pub`]). Because both paths
/// derive from the same immutable report row + stored event row — and the action
/// record persists no target columns of its own — a stranded action always
/// re-derives against the **same** target it originally claimed.
///
/// Beyond [`decode_report_target`]'s `(kind, hex)` decode it overlays the reported
/// event's **author** onto `event`-kind reports. The report row never stores that
/// author (the reporter-supplied `p` tag is validation-shape only, never
/// inserted — see `handlers/report.rs`), so person-directed enforcement
/// (`ban`/`timeout`/`kick`) on an event report would otherwise have no target
/// pubkey. The author is server-owned truth read from the stored event row
/// (`message.author_pubkey`), never the reporter's claim.
///
/// A soft-deleted event (`deleted_at` set) still has a real author, so its author
/// is still surfaced here — the offense does not vanish with the message. When the
/// event row is entirely absent (purged, or never accepted) `message` is `None`
/// and the pubkey stays `None`; callers decide the failure semantics.
pub fn derive_enforcement_target(
    report: &AdminReportDetail,
) -> Result<TargetPair, ResolutionError> {
    let (target_pubkey, target_event_id) =
        decode_report_target(&report.report.target_kind, &report.report.target)?;

    if report.report.target_kind == "event" {
        let author = report
            .message
            .as_ref()
            .map(|m| hex::decode(&m.author_pubkey))
            .transpose()
            .map_err(|_| {
                ResolutionError::Internal("invalid stored event author hex".to_string())
            })?;
        return Ok((author, target_event_id));
    }

    Ok((target_pubkey, target_event_id))
}

/// Re-derive the enforcement target from a persisted report detail (used by the
/// action recovery worker on re-drive). Identical derivation to the HTTP claim
/// path, so a stranded action converges against the same target it claimed.
pub fn derive_enforcement_target_pub(
    report: &AdminReportDetail,
) -> Result<TargetPair, ResolutionError> {
    derive_enforcement_target(report)
}

/// Re-drive an enforcement action from a persisted record (used by the action
/// recovery worker). Equivalent to calling `drive_enforcement` from the persisted
/// step state rather than from a fresh HTTP claim.
#[allow(clippy::too_many_arguments)]
pub async fn drive_enforcement_pub(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    community_id: buzz_core::tenant::CommunityId,
    report_id: Uuid,
    action: &str,
    reason: Option<&str>,
    timeout_until: Option<DateTime<Utc>>,
    actor_pubkey: &[u8],
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    channel_id: Option<Uuid>,
    initial_record: &AdminActionRecord,
    // Pre-held lease token from a batch claim.  Pass `None` when re-driving
    // from the HTTP path (the driver will acquire its own lease).
    held_lease: Option<Uuid>,
) -> Result<EnforcementResolved, ResolutionError> {
    drive_enforcement(
        state,
        tenant,
        community_id,
        report_id,
        action,
        reason,
        timeout_until,
        actor_pubkey,
        target_pubkey,
        target_event_id,
        channel_id,
        initial_record,
        held_lease,
    )
    .await
}

fn decode_report_target(
    target_kind: &str,
    target_hex: &str,
) -> Result<TargetPair, ResolutionError> {
    match target_kind {
        "event" => {
            let bytes = hex::decode(target_hex)
                .map_err(|_| ResolutionError::Internal("invalid event target hex".to_string()))?;
            Ok((None, Some(bytes)))
        }
        "pubkey" => {
            let bytes = hex::decode(target_hex)
                .map_err(|_| ResolutionError::Internal("invalid pubkey target hex".to_string()))?;
            Ok((Some(bytes), None))
        }
        "blob" => Ok((None, None)),
        other => Err(ResolutionError::Internal(format!(
            "unknown target_kind: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_db::admin_moderation::{AdminReport, AdminReportedMessage};

    fn report(target_kind: &str, target: &str) -> AdminReport {
        AdminReport {
            id: Uuid::nil(),
            community_id: Uuid::nil(),
            community_host: "e2e.example".to_string(),
            report_event_id: "0".repeat(64),
            reporter_pubkey: "0".repeat(64),
            target_kind: target_kind.to_string(),
            target: target.to_string(),
            channel_id: None,
            report_type: "spam".to_string(),
            note: None,
            status: "open".to_string(),
            resolved_by: None,
            resolved_at: None,
            action_id: None,
            created_at: Utc::now(),
        }
    }

    fn message(author_hex: &str) -> AdminReportedMessage {
        AdminReportedMessage {
            author_pubkey: author_hex.to_string(),
            content: "reported".to_string(),
            created_at: Utc::now(),
            deleted_at: None,
        }
    }

    fn detail(report: AdminReport, message: Option<AdminReportedMessage>) -> AdminReportDetail {
        AdminReportDetail {
            report,
            message,
            active_action: None,
        }
    }

    #[test]
    fn event_report_overlays_stored_author_as_target_pubkey() {
        // The report row carries only the event id; the enforcement target
        // pubkey is the stored event's author, not the report's `target` hex.
        let event_hex = "ab".repeat(32);
        let author_hex = "cd".repeat(32);
        let d = detail(report("event", &event_hex), Some(message(&author_hex)));

        let (pubkey, event_id) = derive_enforcement_target(&d).expect("derive");
        assert_eq!(
            pubkey,
            Some(hex::decode(&author_hex).unwrap()),
            "event target pubkey must be the stored author, enabling kick/ban/timeout"
        );
        assert_eq!(event_id, Some(hex::decode(&event_hex).unwrap()));
    }

    #[test]
    fn event_report_with_soft_deleted_author_still_resolves_target() {
        // A soft-deleted event still has a real author: enforcement against that
        // author remains valid — the offense doesn't vanish with the message.
        let event_hex = "11".repeat(32);
        let author_hex = "22".repeat(32);
        let mut msg = message(&author_hex);
        msg.deleted_at = Some(Utc::now());
        let d = detail(report("event", &event_hex), Some(msg));

        let (pubkey, _event_id) = derive_enforcement_target(&d).expect("derive");
        assert_eq!(pubkey, Some(hex::decode(&author_hex).unwrap()));
    }

    #[test]
    fn event_report_with_missing_event_row_yields_no_target_pubkey() {
        // Event purged (or never accepted): no stored row → no author. The pair
        // keeps the event id (delete stays valid) but leaves the pubkey None, so
        // the person-directed pre-claim guard rejects deterministically.
        let event_hex = "33".repeat(32);
        let d = detail(report("event", &event_hex), None);

        let (pubkey, event_id) = derive_enforcement_target(&d).expect("derive");
        assert_eq!(
            pubkey, None,
            "missing event row must not fabricate a target"
        );
        assert_eq!(event_id, Some(hex::decode(&event_hex).unwrap()));
    }

    #[test]
    fn pubkey_report_target_is_unchanged_by_derivation() {
        // A pubkey report carries the target user directly; no event row exists,
        // so derivation must pass the decoded pubkey through untouched.
        let pubkey_hex = "44".repeat(32);
        let d = detail(report("pubkey", &pubkey_hex), None);

        let (pubkey, event_id) = derive_enforcement_target(&d).expect("derive");
        assert_eq!(pubkey, Some(hex::decode(&pubkey_hex).unwrap()));
        assert_eq!(event_id, None);
    }

    #[test]
    fn worker_and_http_derivations_are_identical() {
        // Convergence guarantee: the recovery worker's derivation must equal the
        // HTTP claim's for the same report row, since neither persists the target.
        let event_hex = "55".repeat(32);
        let author_hex = "66".repeat(32);
        let d = detail(report("event", &event_hex), Some(message(&author_hex)));

        assert_eq!(
            derive_enforcement_target(&d).unwrap(),
            derive_enforcement_target_pub(&d).unwrap(),
            "worker re-derive must match the HTTP claim derivation exactly"
        );
    }

    /// Verify that `apply_kick_live_side_effects` drops the membership cache
    /// entry and evicts the live channel subscription for the kicked user.
    ///
    /// Setup:
    ///   1. Seed the membership cache with `true` so the cache claims the target
    ///      is still a member.
    ///   2. Register a connection authenticated as the target pubkey and add a
    ///      channel-scoped subscription for them.
    ///   3. Call `apply_kick_live_side_effects`.
    ///
    /// Assertions:
    ///   - The membership cache entry is gone (cache returns `None`).
    ///   - The channel subscription index no longer lists the connection.
    ///
    /// Redis-dependent work inside the helper (cross-pod cache invalidation
    /// publish, pubsub topic release) hits an intentionally unreachable endpoint
    /// and is silently dropped — this mirrors the production "best-effort"
    /// contract and does not affect the in-process assertions.
    #[tokio::test]
    async fn kick_live_side_effects_clears_membership_cache_and_evicts_subscription() {
        use buzz_core::tenant::CommunityId;
        use std::sync::atomic::AtomicU8;
        use std::sync::Arc;
        use tokio::sync::Mutex;
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        let state = crate::state::tests::test_state().await;

        let community_id = CommunityId::from_uuid(Uuid::from_u128(0xCAFE_BABE));
        let channel_id = Uuid::from_u128(0x1234_5678);
        let target_pubkey: Vec<u8> = vec![0xABu8; 32];
        let tenant = buzz_core::tenant::TenantContext::resolved(community_id, "kick-test.example");

        // 1. Seed the membership cache — simulates a cache hit that would keep
        //    the kicked user appearing as a member after the DB write.
        state
            .membership_cache
            .insert((community_id, channel_id, target_pubkey.clone()), true);

        // Confirm the entry is visible before the side effects run.
        assert!(
            state
                .membership_cache
                .get(&(community_id, channel_id, target_pubkey.clone()))
                .is_some(),
            "pre-condition: membership cache entry must exist before kick"
        );

        // 2. Register a connection authenticated as the target pubkey and add a
        //    channel-scoped subscription so eviction has something to remove.
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = tokio::sync::mpsc::channel(1);
        state.conn_manager.register(
            conn_id,
            tx,
            ctrl_tx,
            None,
            CancellationToken::new(),
            community_id,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            3,
        );
        state
            .conn_manager
            .set_authenticated_pubkey(conn_id, target_pubkey.clone());

        let sub_id = "kick-test-sub".to_string();
        state.sub_registry.register_channels_scoped(
            community_id,
            conn_id,
            sub_id,
            // One unconstrained filter (no `kinds`) hits the wildcard index,
            // making the subscription visible in channel_subscriber_conns_scoped.
            vec![nostr::Filter::new()],
            vec![channel_id],
        );

        // Confirm subscription is visible before side effects run.
        assert!(
            state
                .sub_registry
                .channel_subscriber_conns_scoped(community_id, channel_id)
                .contains(&conn_id),
            "pre-condition: subscription must be registered before kick"
        );

        // 3. Fire kick live side effects.
        crate::handlers::side_effects::apply_kick_live_side_effects(
            &tenant,
            &state,
            channel_id,
            &target_pubkey,
        )
        .await
        .expect("kick live side effects must succeed in test");

        // Assert: membership cache entry is gone.
        assert!(
            state
                .membership_cache
                .get(&(community_id, channel_id, target_pubkey.clone()))
                .is_none(),
            "membership cache must not contain a stale entry after kick side effects"
        );

        // Assert: channel subscription is no longer indexed for this connection.
        assert!(
            !state
                .sub_registry
                .channel_subscriber_conns_scoped(community_id, channel_id)
                .contains(&conn_id),
            "kicked user's channel subscription must be evicted after kick side effects"
        );
    }
}
