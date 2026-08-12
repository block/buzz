//! Community moderation commands (kinds 9040–9044).
//!
//! A command is parsed into a tenant-bound
//! [`ModerationAdmissionApplicationEffect`].
//! The canonical admission owner supplies the database transaction that also
//! contains the receipt and replay claim. This module never begins or commits
//! a transaction. Socket disconnection and notices are represented by an
//! opaque [`ModerationPostCommitAction`] and run only through
//! [`dispatch_moderation_postcommit`] after the caller confirms commit.

use std::sync::Arc;

use buzz_auth::RouteCapability;
use buzz_core::kind::{
    KIND_MODERATION_BAN, KIND_MODERATION_RESOLVE_REPORT, KIND_MODERATION_TIMEOUT,
    KIND_MODERATION_UNBAN, KIND_MODERATION_UNTIMEOUT,
};
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_db::authorization_admission::{
    AdmissionApplicationContext, AdmissionApplicationEffect, AdmissionApplicationOutcome,
    AdmissionApplicationResult, AdmissionApplicationResultSchema, AdmissionCommitError,
    AdmissionCommitOutcome, AdmissionCommitRequest, AdmissionObject, AdmissionObjectKind,
};
use buzz_db::moderation::NewAction;
use chrono::{DateTime, TimeZone, Utc};
use nostr::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::handlers::moderation_authz::{
    authorize_moderation_action_tx, ModerationAction, ModerationTarget,
};
use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
use crate::state::AppState;

const MAX_COMMAND_SKEW_SECS: i64 = 120;

/// Legacy direct entry point.
///
/// It validates the command but fails closed because this signature has no
/// caller-owned transaction. Canonical admission must instead call
/// [`prepare_moderation_application_effect`], apply the returned effect on its
/// transaction, commit, and then dispatch the returned post-commit action.
pub async fn handle_moderation_command(
    tenant: &TenantContext,
    _state: &Arc<AppState>,
    event: &Event,
) -> Result<(), String> {
    let _effect = prepare_moderation_application_effect(tenant, event)?;
    Err(error(
        "moderation command requires the canonical caller-owned admission transaction",
    ))
}

/// A validated, tenant-bound mutation consumed by one canonical admission
/// transaction.
///
/// The effect owns no connection and cannot begin or commit a transaction.
#[must_use = "apply the effect in the canonical admission transaction"]
pub struct ModerationAdmissionApplicationEffect {
    tenant: TenantContext,
    actor: Vec<u8>,
    event_id: String,
    object: AdmissionObject,
    command: Option<PreparedModerationCommand>,
    intent_digest: [u8; 32],
}

enum PreparedModerationCommand {
    Ban {
        target: Vec<u8>,
        expires_at: Option<DateTime<Utc>>,
        reason: Option<String>,
    },
    Unban {
        target: Vec<u8>,
    },
    Timeout {
        target: Vec<u8>,
        muted_until: DateTime<Utc>,
        reason: Option<String>,
    },
    Untimeout {
        target: Vec<u8>,
    },
    ResolveReport {
        report_event_id: Vec<u8>,
        status: String,
        action: String,
        reason: Option<String>,
    },
}

/// Opaque socket/notice work returned by an applied effect.
///
/// The admission owner must retain this value until its transaction commits.
#[must_use = "dispatch this action only after the transaction commits"]
pub struct ModerationPostCommitAction(ModerationPostCommitKind);

#[derive(Serialize, Deserialize)]
enum ModerationPostCommitKind {
    Ban {
        target: Vec<u8>,
        event_id: String,
        action_id: Uuid,
        public_reason: String,
    },
    Unban {
        target: Vec<u8>,
    },
    Timeout {
        target: Vec<u8>,
        action_id: Uuid,
        public_reason: String,
    },
    Untimeout {
        target: Vec<u8>,
    },
    ResolveReport {
        report_id: Uuid,
        reporter_pubkey: Vec<u8>,
        status: String,
        action: String,
        summary: String,
    },
}

#[derive(Serialize, Deserialize)]
struct ModerationApplicationResultPayload {
    object_key: [u8; 32],
    intent_digest: [u8; 32],
    action: ModerationPostCommitKind,
}

/// Parse, validate, and bind a signed command without performing I/O.
pub fn prepare_moderation_application_effect(
    tenant: &TenantContext,
    event: &Event,
) -> Result<ModerationAdmissionApplicationEffect, String> {
    validate_freshness(event)?;

    let command = match event.kind.as_u16() as u32 {
        KIND_MODERATION_BAN => PreparedModerationCommand::Ban {
            target: required_pubkey(event)?,
            expires_at: extract_expiration(event)?,
            reason: extract_tag_value(event, "reason"),
        },
        KIND_MODERATION_UNBAN => PreparedModerationCommand::Unban {
            target: required_pubkey(event)?,
        },
        KIND_MODERATION_TIMEOUT => PreparedModerationCommand::Timeout {
            target: required_pubkey(event)?,
            muted_until: extract_expiration(event)?
                .ok_or_else(|| invalid("timeout requires an expiration tag"))?,
            reason: extract_tag_value(event, "reason"),
        },
        KIND_MODERATION_UNTIMEOUT => PreparedModerationCommand::Untimeout {
            target: required_pubkey(event)?,
        },
        KIND_MODERATION_RESOLVE_REPORT => prepare_resolve_report(event)?,
        other => {
            return Err(invalid(format!(
                "unexpected moderation command kind: {other}"
            )))
        }
    };

    let actor = event.pubkey.to_bytes().to_vec();
    let event_id = event.id.to_hex();
    let object = moderation_object(tenant.community(), &command)?;
    let intent_digest = framed_digest(
        b"buzz:nip-fi:moderation-effect-intent:v1",
        &[
            tenant.community().as_uuid().as_bytes(),
            &actor,
            event_id.as_bytes(),
            object.key(),
        ],
    );
    Ok(ModerationAdmissionApplicationEffect {
        tenant: tenant.clone(),
        actor,
        event_id,
        object,
        command: Some(command),
        intent_digest,
    })
}

impl ModerationAdmissionApplicationEffect {
    /// Exact server-derived protected target required on the admission request.
    pub const fn admission_object(&self) -> AdmissionObject {
        self.object
    }

    /// Recheck authorization and restrictions, then mutate state and audit on
    /// the caller-owned transaction.
    ///
    /// One-shot command extraction prevents accidental double application. No
    /// external effects happen here; the returned action is dispatched after
    /// commit.
    async fn apply_in_transaction(
        &mut self,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<ModerationPostCommitAction, String> {
        let tenant = &self.tenant;

        {
            let (target, action) = self.authorization_target()?;
            authorize_moderation_action_tx(transaction, tenant, &self.actor, None, target, action)
                .await
                .map_err(authz_denial)?;
        }
        let restriction =
            buzz_db::moderation::restriction_state_tx(transaction, tenant.community(), &self.actor)
                .await
                .map_err(|e| error(format!("database error checking restriction state: {e}")))?;
        ensure_actor_not_banned(&restriction)?;

        let command = self
            .command
            .take()
            .ok_or_else(|| error("moderation effect was already consumed"))?;

        match command {
            PreparedModerationCommand::Ban {
                target,
                expires_at,
                reason,
            } => {
                buzz_db::moderation::ban_member_tx(
                    transaction,
                    tenant.community(),
                    &target,
                    &self.actor,
                    reason.as_deref(),
                    expires_at,
                )
                .await
                .map_err(database_error)?;
                let action_id = insert_audit_tx(
                    transaction,
                    tenant,
                    &self.actor,
                    "ban",
                    Some(&target),
                    None,
                    reason.as_deref(),
                )
                .await?;
                Ok(ModerationPostCommitAction(ModerationPostCommitKind::Ban {
                    target,
                    event_id: self.event_id.clone(),
                    action_id,
                    public_reason: reason.unwrap_or_default(),
                }))
            }
            PreparedModerationCommand::Unban { target } => {
                let lifted = buzz_db::moderation::unban_member_tx(
                    transaction,
                    tenant.community(),
                    &target,
                    &self.actor,
                )
                .await
                .map_err(database_error)?;
                if !lifted {
                    return Err(invalid("member is not banned"));
                }
                insert_audit_tx(
                    transaction,
                    tenant,
                    &self.actor,
                    "unban",
                    Some(&target),
                    None,
                    None,
                )
                .await?;
                Ok(ModerationPostCommitAction(
                    ModerationPostCommitKind::Unban { target },
                ))
            }
            PreparedModerationCommand::Timeout {
                target,
                muted_until,
                reason,
            } => {
                buzz_db::moderation::timeout_member_tx(
                    transaction,
                    tenant.community(),
                    &target,
                    &self.actor,
                    muted_until,
                    reason.as_deref(),
                )
                .await
                .map_err(database_error)?;
                let action_id = insert_audit_tx(
                    transaction,
                    tenant,
                    &self.actor,
                    "timeout",
                    Some(&target),
                    None,
                    reason.as_deref(),
                )
                .await?;
                Ok(ModerationPostCommitAction(
                    ModerationPostCommitKind::Timeout {
                        target,
                        action_id,
                        public_reason: reason.unwrap_or_default(),
                    },
                ))
            }
            PreparedModerationCommand::Untimeout { target } => {
                let cleared = buzz_db::moderation::untimeout_member_tx(
                    transaction,
                    tenant.community(),
                    &target,
                    &self.actor,
                )
                .await
                .map_err(database_error)?;
                if !cleared {
                    return Err(invalid("member is not timed out"));
                }
                insert_audit_tx(
                    transaction,
                    tenant,
                    &self.actor,
                    "untimeout",
                    Some(&target),
                    None,
                    None,
                )
                .await?;
                Ok(ModerationPostCommitAction(
                    ModerationPostCommitKind::Untimeout { target },
                ))
            }
            PreparedModerationCommand::ResolveReport {
                report_event_id,
                status,
                action,
                reason,
            } => {
                let report = buzz_db::moderation::get_report_by_event_tx(
                    transaction,
                    tenant.community(),
                    &report_event_id,
                )
                .await
                .map_err(database_error)?
                .ok_or_else(|| invalid("report not found in this community"))?;
                if report.status != "open" {
                    return Err(invalid(
                        "report is not open (already resolved or dismissed)",
                    ));
                }

                let (target_pubkey, target_event_id) = match &report.target {
                    buzz_db::moderation::ReportTarget::Pubkey(pubkey) => {
                        (Some(pubkey.as_slice()), None)
                    }
                    buzz_db::moderation::ReportTarget::Event(event_id) => {
                        (None, Some(event_id.as_slice()))
                    }
                    buzz_db::moderation::ReportTarget::Blob(_) => (None, None),
                };
                let action_id = insert_audit_tx(
                    transaction,
                    tenant,
                    &self.actor,
                    resolution_audit_action(&action),
                    target_pubkey,
                    target_event_id,
                    reason.as_deref(),
                )
                .await?;
                let resolved = buzz_db::moderation::resolve_report_tx(
                    transaction,
                    tenant.community(),
                    report.id,
                    &status,
                    &self.actor,
                    Some(action_id),
                )
                .await
                .map_err(database_error)?;
                if !resolved {
                    return Err(invalid(
                        "report is not open (already resolved or dismissed)",
                    ));
                }
                let summary = reason.unwrap_or_else(|| match status.as_str() {
                    "dismissed" => "Your report was reviewed and dismissed.".to_string(),
                    _ => "Your report was reviewed and acted on.".to_string(),
                });
                Ok(ModerationPostCommitAction(
                    ModerationPostCommitKind::ResolveReport {
                        report_id: report.id,
                        reporter_pubkey: report.reporter_pubkey,
                        status,
                        action,
                        summary,
                    },
                ))
            }
        }
    }

    fn authorization_target(&self) -> Result<(ModerationTarget<'_>, ModerationAction), String> {
        let command = self
            .command
            .as_ref()
            .ok_or_else(|| error("moderation effect was already consumed"))?;
        Ok(match command {
            PreparedModerationCommand::Ban { target, .. } => {
                (ModerationTarget::Pubkey(target), ModerationAction::Ban)
            }
            PreparedModerationCommand::Unban { target } => {
                (ModerationTarget::Pubkey(target), ModerationAction::Unban)
            }
            PreparedModerationCommand::Timeout { target, .. } => {
                (ModerationTarget::Pubkey(target), ModerationAction::Timeout)
            }
            PreparedModerationCommand::Untimeout { target } => (
                ModerationTarget::Pubkey(target),
                ModerationAction::Untimeout,
            ),
            PreparedModerationCommand::ResolveReport {
                report_event_id, ..
            } => (
                ModerationTarget::Event(report_event_id),
                ModerationAction::ResolveReport,
            ),
        })
    }
}

impl AdmissionApplicationEffect for ModerationAdmissionApplicationEffect {
    fn intent_digest(&self) -> [u8; 32] {
        self.intent_digest
    }

    fn result_schema(&self) -> AdmissionApplicationResultSchema {
        moderation_result_schema()
    }

    fn apply<'a, 'transaction>(
        &'a mut self,
        transaction: &'a mut Transaction<'transaction, Postgres>,
        context: &'a AdmissionApplicationContext<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<AdmissionApplicationOutcome, AdmissionCommitError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if context.authorization_domain() != self.tenant.community()
                || context.object() != self.object
                || context.authorization().capability() != RouteCapability::Moderation
                || context.authorization().actor_pubkey().to_bytes().as_slice()
                    != self.actor.as_slice()
            {
                return Err(AdmissionCommitError::AuthorizationDenied);
            }
            let action = self
                .apply_in_transaction(transaction)
                .await
                .map_err(map_application_error)?;
            let payload = serde_json::to_vec(&ModerationApplicationResultPayload {
                object_key: *self.object.key(),
                intent_digest: self.intent_digest,
                action: action.0,
            })
            .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            let result = AdmissionApplicationResult::new(moderation_result_schema(), 1, payload)
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            let effect_digest = framed_digest(
                b"buzz:nip-fi:moderation-effect:v1",
                &[
                    context.authorization_domain().as_uuid().as_bytes(),
                    context.operation_id().as_bytes(),
                    context.request_fingerprint(),
                    self.intent_digest.as_slice(),
                    result.payload(),
                ],
            );
            AdmissionApplicationOutcome::new(result, effect_digest)
        })
    }
}

/// Attach one prepared moderation mutation to canonical final admission.
pub fn install_moderation_application_effect(
    request: AdmissionCommitRequest,
    effect: ModerationAdmissionApplicationEffect,
) -> Result<AdmissionCommitRequest, AdmissionCommitError> {
    if request.authorization_domain() != effect.tenant.community()
        || request.object() != effect.object
    {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    request.with_application_effect(Box::new(effect))
}

/// Dispatch external moderation work only for a newly committed mutation.
///
/// Exact replay returns the original typed result but deliberately performs no
/// socket disconnect or notice. A failed/rolled-back commit produces no
/// outcome, so it cannot reach this boundary either.
pub async fn dispatch_committed_moderation_outcome(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    outcome: &AdmissionCommitOutcome,
) -> Result<bool, AdmissionCommitError> {
    let Some(action) = committed_moderation_action(tenant, outcome)? else {
        return Ok(false);
    };
    dispatch_moderation_postcommit(tenant, state, action).await;
    Ok(true)
}

fn committed_moderation_action(
    tenant: &TenantContext,
    outcome: &AdmissionCommitOutcome,
) -> Result<Option<ModerationPostCommitAction>, AdmissionCommitError> {
    let (receipt, application_result) = match outcome {
        AdmissionCommitOutcome::Committed {
            receipt,
            application_result,
            ..
        } => {
            if receipt.authorization_domain() != tenant.community()
                || receipt.object().kind() != AdmissionObjectKind::ModerationTarget
            {
                return Err(AdmissionCommitError::IntentConflict);
            }
            (
                *receipt,
                application_result
                    .as_ref()
                    .ok_or(AdmissionCommitError::IntentConflict)?,
            )
        }
        AdmissionCommitOutcome::ExactReplay { .. } => return Ok(None),
    };
    validate_committed_moderation_result(receipt, application_result).map(Some)
}

fn validate_committed_moderation_result(
    receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
    application_result: &AdmissionApplicationResult,
) -> Result<ModerationPostCommitAction, AdmissionCommitError> {
    if application_result.schema() != moderation_result_schema() || application_result.code() != 1 {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let payload =
        serde_json::from_slice::<ModerationApplicationResultPayload>(application_result.payload())
            .map_err(|_| AdmissionCommitError::IntentConflict)?;
    if receipt.object().kind() != AdmissionObjectKind::ModerationTarget
        || receipt.object().key() != &payload.object_key
        || payload.intent_digest == [0; 32]
    {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let expected_digest =
        canonical_moderation_result_digest(receipt, payload.intent_digest, application_result)?;
    if receipt.application_result_digest() != Some(&expected_digest) {
        return Err(AdmissionCommitError::IntentConflict);
    }
    Ok(ModerationPostCommitAction(payload.action))
}

fn moderation_result_schema() -> AdmissionApplicationResultSchema {
    AdmissionApplicationResultSchema::moderation()
}

fn canonical_moderation_result_digest(
    receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
    application_intent_digest: [u8; 32],
    result: &AdmissionApplicationResult,
) -> Result<[u8; 32], AdmissionCommitError> {
    if application_intent_digest == [0; 32] {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    let object = receipt.object();
    let schema = result.schema();
    Ok(admission_framed_digest(
        b"buzz:canonical-application-result:v1",
        &[
            receipt.authorization_domain().as_uuid().as_bytes(),
            &object.kind().database_code().to_be_bytes(),
            object.key(),
            receipt.semantic_fingerprint(),
            &application_intent_digest,
            schema.type_key(),
            &schema.version().to_be_bytes(),
            &result.code().to_be_bytes(),
            result.payload(),
        ],
    ))
}

fn admission_framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

fn map_application_error(error: String) -> AdmissionCommitError {
    if error.starts_with("restricted:")
        || error.starts_with("blocked:")
        || error.starts_with("invalid:")
    {
        AdmissionCommitError::AuthorizationDenied
    } else {
        AdmissionCommitError::DependencyUnavailable
    }
}

/// Run the socket and notice work represented by `action` after commit.
pub async fn dispatch_moderation_postcommit(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    action: ModerationPostCommitAction,
) {
    match action.0 {
        ModerationPostCommitKind::Ban {
            target,
            event_id,
            action_id,
            public_reason,
        } => {
            state.disconnect_pubkey_clusterwide(
                tenant,
                &target,
                &event_id,
                "blocked: you are banned from this community",
            );
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
                info!(error = %e, "ban notice DM delivery failed (ban still enforced)");
            }
            info!(target = %hex::encode(&target), "community ban applied");
        }
        ModerationPostCommitKind::Unban { target } => {
            info!(target = %hex::encode(&target), "community ban lifted");
        }
        ModerationPostCommitKind::Timeout {
            target,
            action_id,
            public_reason,
        } => {
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
        }
        ModerationPostCommitKind::Untimeout { target } => {
            info!(target = %hex::encode(&target), "community timeout cleared");
        }
        ModerationPostCommitKind::ResolveReport {
            report_id,
            reporter_pubkey,
            status,
            action,
            summary,
        } => {
            if let Err(e) = send_moderation_notice(
                tenant,
                state,
                &reporter_pubkey,
                ModerationNotice::ReportResolved {
                    report_id,
                    status: status.clone(),
                    summary,
                },
            )
            .await
            {
                info!(error = %e, "report-resolution notice DM delivery failed (report still resolved)");
            }
            info!(report_id = %report_id, status = %status, action = %action, "report resolved");
        }
    }
}

fn moderation_object(
    community: CommunityId,
    command: &PreparedModerationCommand,
) -> Result<AdmissionObject, String> {
    let (target_kind, target) = match command {
        PreparedModerationCommand::Ban { target, .. }
        | PreparedModerationCommand::Unban { target }
        | PreparedModerationCommand::Timeout { target, .. }
        | PreparedModerationCommand::Untimeout { target } => {
            (b"pubkey".as_slice(), target.as_slice())
        }
        PreparedModerationCommand::ResolveReport {
            report_event_id, ..
        } => (b"report".as_slice(), report_event_id.as_slice()),
    };
    let key = framed_digest(
        b"buzz:nip-fi:moderation-target:v1",
        &[community.as_uuid().as_bytes(), target_kind, target],
    );
    AdmissionObject::new(AdmissionObjectKind::ModerationTarget, key)
        .ok_or_else(|| invalid("moderation target could not be bound"))
}

fn framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

fn validate_freshness(event: &Event) -> Result<(), String> {
    let event_ts = event.created_at.as_secs() as i64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|e| error(format!("system clock unavailable: {e}")))?;
    if (event_ts - now).abs() > MAX_COMMAND_SKEW_SECS {
        return Err(invalid(format!(
            "event timestamp out of range: created_at={event_ts}, now={now}, delta={}s (max ±{MAX_COMMAND_SKEW_SECS}s)",
            event_ts - now
        )));
    }
    Ok(())
}

fn required_pubkey(event: &Event) -> Result<Vec<u8>, String> {
    extract_p_tag_bytes(event).ok_or_else(|| invalid("missing or invalid p tag"))
}

fn prepare_resolve_report(event: &Event) -> Result<PreparedModerationCommand, String> {
    let report_event_id = extract_report_tag(event)
        .ok_or_else(|| invalid("missing or invalid report tag (expect 64-hex event id)"))?;
    let status = extract_tag_value(event, "status").ok_or_else(|| invalid("missing status tag"))?;
    let action = extract_tag_value(event, "action").ok_or_else(|| invalid("missing action tag"))?;
    let reason = extract_tag_value(event, "reason");

    if status != "resolved" && status != "dismissed" {
        return Err(invalid(format!(
            "invalid status: {status} (expect resolved|dismissed)"
        )));
    }
    if !matches!(
        action.as_str(),
        "delete" | "kick" | "ban" | "timeout" | "dismiss" | "escalate"
    ) {
        return Err(invalid(format!(
            "invalid action: {action} (expect delete|kick|ban|timeout|dismiss|escalate)"
        )));
    }
    if (action == "dismiss") != (status == "dismissed") {
        return Err(invalid(
            "action `dismiss` pairs only with status `dismissed`",
        ));
    }

    Ok(PreparedModerationCommand::ResolveReport {
        report_event_id,
        status,
        action,
        reason,
    })
}

fn ensure_actor_not_banned(
    restriction: &buzz_db::moderation::RestrictionState,
) -> Result<(), String> {
    if restriction.banned {
        return Err("blocked: you are banned from this community".to_string());
    }
    Ok(())
}

fn resolution_audit_action(action: &str) -> &'static str {
    match action {
        "dismiss" => "dismiss_report",
        "escalate" => "escalate",
        "delete" => "resolve:delete",
        "kick" => "resolve:kick",
        "ban" => "resolve:ban",
        "timeout" => "resolve:timeout",
        _ => "resolve:unknown",
    }
}

async fn insert_audit_tx(
    transaction: &mut Transaction<'_, Postgres>,
    tenant: &TenantContext,
    actor: &[u8],
    action: &str,
    target_pubkey: Option<&[u8]>,
    target_event_id: Option<&[u8]>,
    public_reason: Option<&str>,
) -> Result<Uuid, String> {
    buzz_db::moderation::insert_action_tx(
        transaction,
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
    .map_err(|e| error(format!("failed to write audit row: {e}")))
}

fn authz_denial(e: anyhow::Error) -> String {
    format!("restricted: {e}")
}

fn database_error(e: impl std::fmt::Display) -> String {
    error(format!("database error: {e}"))
}

fn invalid(message: impl Into<String>) -> String {
    format!("invalid: {}", message.into())
}

fn error(message: impl Into<String>) -> String {
    format!("error: {}", message.into())
}

fn extract_p_tag_bytes(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|value| value.as_str()) == Some("p") {
            if let Some(value) = parts.get(1).map(|value| value.as_str()) {
                if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
                {
                    return hex::decode(value).ok();
                }
            }
        }
    }
    None
}

fn extract_report_tag(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|value| value.as_str()) == Some("report") {
            if let Some(value) = parts.get(1).map(|value| value.as_str()) {
                if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
                {
                    return hex::decode(value).ok();
                }
            }
        }
    }
    None
}

fn extract_expiration(event: &Event) -> Result<Option<DateTime<Utc>>, String> {
    match extract_tag_value(event, "expiration") {
        None => Ok(None),
        Some(raw) => {
            let seconds: i64 = raw
                .parse()
                .map_err(|_| invalid(format!("invalid expiration tag: {raw}")))?;
            Utc.timestamp_opt(seconds, 0)
                .single()
                .map(Some)
                .ok_or_else(|| invalid(format!("expiration out of range: {seconds}")))
        }
    }
}

fn extract_tag_value(event: &Event, name: &str) -> Option<String> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|value| value.as_str()) == Some(name) {
            return parts.get(1).map(ToString::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn make_event(kind: u16, created_at_secs: u64, tags: Vec<Vec<String>>) -> Event {
        let keys = Keys::generate();
        let tags = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid tag"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(nostr::Timestamp::from_secs(created_at_secs))
            .sign_with_keys(&keys)
            .expect("signing failed")
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs()
    }

    fn tenant(id: u128) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), "relay.example")
    }

    #[test]
    fn banned_admin_cannot_reach_an_unban_command() {
        let banned = buzz_db::moderation::RestrictionState {
            banned: true,
            muted_until: None,
        };
        assert_eq!(
            ensure_actor_not_banned(&banned),
            Err("blocked: you are banned from this community".to_string())
        );

        let timed_out = buzz_db::moderation::RestrictionState {
            banned: false,
            muted_until: Some(Utc::now() + Duration::minutes(5)),
        };
        assert!(ensure_actor_not_banned(&timed_out).is_ok());
    }

    #[test]
    fn prepared_effect_is_bound_to_server_resolved_tenant() {
        let event = make_event(
            KIND_MODERATION_BAN as u16,
            now_secs(),
            vec![vec!["p".into(), "a".repeat(64)]],
        );
        let effect = prepare_moderation_application_effect(&tenant(7), &event).expect("effect");
        assert_eq!(effect.tenant.community(), tenant(7).community());
        assert_ne!(effect.tenant.community(), tenant(8).community());
        assert_eq!(
            effect.admission_object().kind(),
            AdmissionObjectKind::ModerationTarget
        );
    }

    #[test]
    fn exact_replay_never_yields_a_postcommit_action() {
        let tenant = tenant(9);
        let object = AdmissionObject::new(AdmissionObjectKind::ModerationTarget, [7; 32])
            .expect("moderation object");
        let result = AdmissionApplicationResult::new(
            moderation_result_schema(),
            1,
            serde_json::to_vec(&ModerationPostCommitKind::Unban {
                target: vec![8; 32],
            })
            .expect("serialize action"),
        )
        .expect("typed result");
        let receipt = buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
            tenant.community(),
            object,
            Uuid::new_v4(),
            [9; 32],
            [10; 32],
            buzz_db::authorization_admission::AdmissionCommitDigests::new([11; 32], Some([12; 32]))
                .expect("digests"),
            Uuid::new_v4(),
        )
        .expect("receipt");
        let outcome = AdmissionCommitOutcome::ExactReplay {
            receipt,
            application_result: Some(result),
        };
        assert!(matches!(
            committed_moderation_action(&tenant, &outcome),
            Ok(None)
        ));
    }

    #[test]
    fn committed_result_must_match_receipt_digest_and_moderation_object() {
        let tenant = tenant(10);
        let object = AdmissionObject::new(AdmissionObjectKind::ModerationTarget, [7; 32])
            .expect("moderation object");
        let intent_digest = [13; 32];
        let result = AdmissionApplicationResult::new(
            moderation_result_schema(),
            1,
            serde_json::to_vec(&ModerationApplicationResultPayload {
                object_key: *object.key(),
                intent_digest,
                action: ModerationPostCommitKind::Unban {
                    target: vec![8; 32],
                },
            })
            .expect("serialize application result"),
        )
        .expect("typed result");
        let provisional = buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
            tenant.community(),
            object,
            Uuid::new_v4(),
            [9; 32],
            [10; 32],
            buzz_db::authorization_admission::AdmissionCommitDigests::new([11; 32], Some([12; 32]))
                .expect("provisional digests"),
            Uuid::new_v4(),
        )
        .expect("provisional receipt");
        let application_result_digest =
            canonical_moderation_result_digest(provisional, intent_digest, &result)
                .expect("canonical result digest");
        let receipt = buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
            tenant.community(),
            object,
            provisional.operation_id(),
            *provisional.request_fingerprint(),
            *provisional.semantic_fingerprint(),
            buzz_db::authorization_admission::AdmissionCommitDigests::new(
                *provisional.result_digest(),
                Some(application_result_digest),
            )
            .expect("canonical digests"),
            provisional.audit_event_id(),
        )
        .expect("canonical receipt");
        let action = validate_committed_moderation_result(receipt, &result)
            .expect("matching receipt and result");
        assert!(matches!(
            action.0,
            ModerationPostCommitKind::Unban { target } if target == vec![8; 32]
        ));

        let other_object = AdmissionObject::new(AdmissionObjectKind::ModerationTarget, [14; 32])
            .expect("other moderation object");
        let other_receipt = buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
            tenant.community(),
            other_object,
            receipt.operation_id(),
            *receipt.request_fingerprint(),
            *receipt.semantic_fingerprint(),
            buzz_db::authorization_admission::AdmissionCommitDigests::new(
                *receipt.result_digest(),
                Some(application_result_digest),
            )
            .expect("other digests"),
            receipt.audit_event_id(),
        )
        .expect("other receipt");
        assert!(matches!(
            validate_committed_moderation_result(other_receipt, &result),
            Err(AdmissionCommitError::IntentConflict)
        ));

        let wrong_digest_receipt =
            buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
                tenant.community(),
                object,
                receipt.operation_id(),
                *receipt.request_fingerprint(),
                *receipt.semantic_fingerprint(),
                buzz_db::authorization_admission::AdmissionCommitDigests::new(
                    *receipt.result_digest(),
                    Some([15; 32]),
                )
                .expect("wrong digest"),
                receipt.audit_event_id(),
            )
            .expect("wrong-digest receipt");
        assert!(matches!(
            validate_committed_moderation_result(wrong_digest_receipt, &result),
            Err(AdmissionCommitError::IntentConflict)
        ));
    }

    #[test]
    fn stale_command_is_rejected_before_an_effect_exists() {
        let event = make_event(
            KIND_MODERATION_BAN as u16,
            now_secs() - (MAX_COMMAND_SKEW_SECS as u64 + 1),
            vec![vec!["p".into(), "a".repeat(64)]],
        );
        assert!(prepare_moderation_application_effect(&tenant(7), &event).is_err());
    }

    #[test]
    fn resolve_audit_actions_are_allowed_by_db_check_vocabulary() {
        for action in ["dismiss", "escalate", "delete", "kick", "ban", "timeout"] {
            let audit_action = resolution_audit_action(action);
            assert!(
                buzz_db::moderation::MODERATION_ACTION_CHECK_VOCAB.contains(&audit_action),
                "action={action} maps to disallowed {audit_action}"
            );
        }
    }

    #[test]
    fn command_error_prefix_helpers_preserve_machine_readable_token() {
        assert_eq!(
            authz_denial(anyhow::anyhow!("moderator access required")),
            "restricted: moderator access required"
        );
        assert_eq!(invalid("missing status tag"), "invalid: missing status tag");
        assert_eq!(
            error("database error: connection lost"),
            "error: database error: connection lost"
        );
    }

    #[test]
    fn extract_p_tag_bytes_validates_exact_hex_shape() {
        let valid = "a".repeat(64);
        let event = make_event(
            KIND_MODERATION_BAN as u16,
            now_secs(),
            vec![vec!["p".into(), valid.clone()]],
        );
        assert_eq!(extract_p_tag_bytes(&event), hex::decode(valid).ok());

        for invalid_value in ["abcd".to_string(), "g".repeat(64)] {
            let event = make_event(
                KIND_MODERATION_BAN as u16,
                now_secs(),
                vec![vec!["p".into(), invalid_value]],
            );
            assert_eq!(extract_p_tag_bytes(&event), None);
        }
    }

    #[test]
    fn report_tag_requires_64_hex() {
        let valid = "b".repeat(64);
        let event = make_event(
            KIND_MODERATION_RESOLVE_REPORT as u16,
            now_secs(),
            vec![vec!["report".into(), valid.clone()]],
        );
        assert_eq!(extract_report_tag(&event), hex::decode(valid).ok());
    }

    #[test]
    fn expiration_parses_and_rejects_malformed_values() {
        let absent = make_event(KIND_MODERATION_BAN as u16, now_secs(), vec![]);
        assert_eq!(extract_expiration(&absent).expect("absent"), None);

        let valid = make_event(
            KIND_MODERATION_BAN as u16,
            now_secs(),
            vec![vec!["expiration".into(), "1893456000".into()]],
        );
        assert_eq!(
            extract_expiration(&valid).expect("valid"),
            Utc.timestamp_opt(1_893_456_000, 0).single()
        );

        for invalid_value in ["not-a-number", "99999999999999"] {
            let event = make_event(
                KIND_MODERATION_BAN as u16,
                now_secs(),
                vec![vec!["expiration".into(), invalid_value.into()]],
            );
            assert!(extract_expiration(&event).is_err());
        }
    }
}
