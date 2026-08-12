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

use buzz_auth::{FinalizedAuthContext, RouteCapability};
use buzz_core::kind::{
    KIND_MODERATION_BAN, KIND_MODERATION_RESOLVE_REPORT, KIND_MODERATION_TIMEOUT,
    KIND_MODERATION_UNBAN, KIND_MODERATION_UNTIMEOUT,
};
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_db::authorization_admission::{
    AdmissionApplicationContext, AdmissionApplicationEffect, AdmissionApplicationOutcome,
    AdmissionApplicationResult, AdmissionApplicationResultBinding,
    AdmissionApplicationResultSchema, AdmissionCommitError, AdmissionCommitOutcome,
    AdmissionCommitRequest, AdmissionObject, AdmissionObjectKind, CanonicalAdmissionCommitter,
};
use buzz_db::moderation::NewAction;
use chrono::{DateTime, TimeZone, Utc};
use nostr::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use tracing::info;
use uuid::Uuid;

use crate::handlers::ingest::{IngestAuth, ModerationTransportEvidence};
use crate::handlers::moderation_authz::{
    authorize_moderation_action_tx, ModerationAction, ModerationTarget,
};
use crate::handlers::moderation_notices::{send_moderation_notice, ModerationNotice};
use crate::state::AppState;

const MAX_COMMAND_SKEW_SECS: i64 = 120;

/// Execute one moderation command under the configured admission mode.
pub async fn handle_moderation_command(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    auth: &IngestAuth,
) -> Result<(), String> {
    execute_moderation_command(
        tenant,
        &state.db,
        state.config.nip_fi_mode,
        &state.config.relay_url,
        event,
        auth,
        |action| async move {
            dispatch_moderation_postcommit(tenant, state, action).await;
        },
    )
    .await
}

async fn execute_moderation_command<F, Fut>(
    tenant: &TenantContext,
    db: &buzz_db::Db,
    mode: buzz_auth::NipFiMode,
    relay_url: &str,
    event: &Event,
    auth: &IngestAuth,
    dispatch: F,
) -> Result<(), String>
where
    F: FnOnce(ModerationPostCommitAction) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    match mode {
        buzz_auth::NipFiMode::Off => {
            let mut effect = prepare_moderation_application_effect(tenant, event)?;
            let mut transaction = db.begin_transaction().await.map_err(database_error)?;
            let action = effect.apply_in_transaction(&mut transaction).await?;
            transaction.commit().await.map_err(database_error)?;
            dispatch(action).await;
            Ok(())
        }
        buzz_auth::NipFiMode::DenyProtected => {
            record_moderation_denial(
                db,
                tenant.community(),
                buzz_db::authorization_events::ProtectedDenialReason::ModeDenied,
                buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
            )
            .await?;
            Err(moderation_unavailable())
        }
        buzz_auth::NipFiMode::Enforce => {
            let effect = match prepare_moderation_application_effect(tenant, event) {
                Ok(effect) => effect,
                Err(_) => {
                    record_moderation_denial(
                        db,
                        tenant.community(),
                        buzz_db::authorization_events::ProtectedDenialReason::InvalidProof,
                        buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                    )
                    .await?;
                    return Err(moderation_denied());
                }
            };
            let object = effect.admission_object();
            let now = Utc::now();
            let proof = match auth.moderation_evidence() {
                Some(ModerationTransportEvidence::Nip98 {
                    authorization_event,
                    body,
                }) => {
                    let expected_url =
                        crate::api::bridge::nip98_expected_url(relay_url, tenant, "/events");
                    let coordinates = match buzz_auth::Nip98ModerationCommandCoordinates::new(
                        tenant.community(),
                        *object.key(),
                        &expected_url,
                        body,
                        event,
                    ) {
                        Ok(coordinates) => coordinates,
                        Err(_) => {
                            record_moderation_denial(
                                db,
                                tenant.community(),
                                buzz_db::authorization_events::ProtectedDenialReason::InvalidProof,
                                buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                            )
                            .await?;
                            return Err(moderation_denied());
                        }
                    };
                    match buzz_auth::verify_nip98_moderation_command_proof(
                        authorization_event,
                        &coordinates,
                        body,
                        now,
                    ) {
                        Ok(proof) => proof,
                        Err(_) => {
                            record_moderation_denial(
                                db,
                                tenant.community(),
                                buzz_db::authorization_events::ProtectedDenialReason::InvalidProof,
                                buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                            )
                            .await?;
                            return Err(moderation_denied());
                        }
                    }
                }
                Some(ModerationTransportEvidence::Nip42 { relay_url }) => {
                    let coordinates = match buzz_auth::Nip42ModerationCommandCoordinates::new(
                        tenant.community(),
                        *object.key(),
                        relay_url,
                        event,
                    ) {
                        Ok(coordinates) => coordinates,
                        Err(_) => {
                            record_moderation_denial(
                                db,
                                tenant.community(),
                                buzz_db::authorization_events::ProtectedDenialReason::InvalidProof,
                                buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                            )
                            .await?;
                            return Err(moderation_denied());
                        }
                    };
                    match buzz_auth::verify_nip42_moderation_command_proof(event, &coordinates, now)
                    {
                        Ok(proof) => proof,
                        Err(_) => {
                            record_moderation_denial(
                                db,
                                tenant.community(),
                                buzz_db::authorization_events::ProtectedDenialReason::InvalidProof,
                                buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                            )
                            .await?;
                            return Err(moderation_denied());
                        }
                    }
                }
                None => {
                    record_moderation_denial(
                        db,
                        tenant.community(),
                        buzz_db::authorization_events::ProtectedDenialReason::MissingProof,
                        buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                    )
                    .await?;
                    return Err(moderation_denied());
                }
            };
            let request = match db.prepare_canonical_moderation_request(proof, object).await {
                Ok(request) => request,
                Err(commit_error) => {
                    record_moderation_denial(
                        db,
                        tenant.community(),
                        protected_denial_reason(commit_error),
                        buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                    )
                    .await?;
                    return Err(admission_error(commit_error));
                }
            };
            let request = match install_moderation_application_effect(request, effect) {
                Ok(request) => request,
                Err(commit_error) => {
                    record_moderation_denial(
                        db,
                        tenant.community(),
                        protected_denial_reason(commit_error),
                        buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                    )
                    .await?;
                    return Err(admission_error(commit_error));
                }
            };
            let outcome = match db.canonical_moderation_committer().commit(request).await {
                Ok(outcome) => outcome,
                Err(commit_error) => {
                    let fallback_reason = match commit_error {
                        AdmissionCommitError::InvalidRequest => Some(
                            buzz_db::authorization_events::ProtectedDenialReason::InvalidProof,
                        ),
                        AdmissionCommitError::AuthorizationDenied => Some(
                            buzz_db::authorization_events::ProtectedDenialReason::AuthorizationDenied,
                        ),
                        AdmissionCommitError::IntentConflict
                        | AdmissionCommitError::ReplayRejected => Some(
                            buzz_db::authorization_events::ProtectedDenialReason::ReplayConflict,
                        ),
                        AdmissionCommitError::AuditUnavailable
                        | AdmissionCommitError::DependencyUnavailable => Some(
                            buzz_db::authorization_events::ProtectedDenialReason::DependencyUnavailable,
                        ),
                        _ => None,
                    };
                    if let Some(reason) = fallback_reason {
                        record_moderation_denial(
                            db,
                            tenant.community(),
                            reason,
                            buzz_db::authorization_events::ProtectedDenialAction::ModerationCommand,
                        )
                        .await?;
                    }
                    return Err(admission_error(commit_error));
                }
            };
            if let Err(commit_error) =
                dispatch_committed_moderation_outcome(tenant, outcome, dispatch).await
            {
                return Err(admission_error(commit_error));
            }
            Ok(())
        }
    }
}

fn protected_denial_reason(
    commit_error: AdmissionCommitError,
) -> buzz_db::authorization_events::ProtectedDenialReason {
    use buzz_db::authorization_events::ProtectedDenialReason;
    match commit_error {
        AdmissionCommitError::InvalidRequest | AdmissionCommitError::RecordedInvalidRequest => {
            ProtectedDenialReason::InvalidProof
        }
        AdmissionCommitError::AuthorizationDenied
        | AdmissionCommitError::RecordedAuthorizationDenied => {
            ProtectedDenialReason::AuthorizationDenied
        }
        AdmissionCommitError::IntentConflict
        | AdmissionCommitError::ReplayRejected
        | AdmissionCommitError::RecordedIntentConflict
        | AdmissionCommitError::RecordedReplayRejected => ProtectedDenialReason::ReplayConflict,
        AdmissionCommitError::AuditUnavailable
        | AdmissionCommitError::RecordedAuditUnavailable
        | AdmissionCommitError::DependencyUnavailable => {
            ProtectedDenialReason::DependencyUnavailable
        }
    }
}

async fn record_moderation_denial(
    db: &buzz_db::Db,
    community: CommunityId,
    reason: buzz_db::authorization_events::ProtectedDenialReason,
    action: buzz_db::authorization_events::ProtectedDenialAction,
) -> Result<(), String> {
    if db
        .record_protected_denial_bucket(
            community,
            buzz_db::authorization_events::ProtectedDenialSurface::Moderation,
            reason,
            action,
        )
        .await
        .is_err()
    {
        let _ = db
            .latch_authorization_event_failure(
                community,
                buzz_db::authorization_events::AuthorizationAuditFailureCode::StorageUnavailable,
            )
            .await;
        return Err(moderation_unavailable());
    }
    Ok(())
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

#[derive(Clone, Copy)]
enum ModerationApplicationTarget<'a> {
    Pubkey(&'a [u8]),
    ReportEvent(&'a [u8]),
}

impl<'a> ModerationApplicationTarget<'a> {
    fn parts(self) -> Option<(&'static [u8], &'a [u8])> {
        let (kind, target) = match self {
            Self::Pubkey(target) => (b"pubkey".as_slice(), target),
            Self::ReportEvent(target) => (b"report".as_slice(), target),
        };
        (target.len() == 32).then_some((kind, target))
    }
}

impl PreparedModerationCommand {
    fn application_target(&self) -> ModerationApplicationTarget<'_> {
        match self {
            Self::Ban { target, .. }
            | Self::Unban { target }
            | Self::Timeout { target, .. }
            | Self::Untimeout { target } => ModerationApplicationTarget::Pubkey(target),
            Self::ResolveReport {
                report_event_id, ..
            } => ModerationApplicationTarget::ReportEvent(report_event_id),
        }
    }
}

/// Opaque socket/notice work returned by an applied effect.
///
/// The admission owner must retain this value until its transaction commits.
#[must_use = "dispatch this action only after the transaction commits"]
pub struct ModerationPostCommitAction(ModerationPostCommitKind);

#[derive(Clone, Serialize, Deserialize)]
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
        report_event_id: Vec<u8>,
        report_id: Uuid,
        reporter_pubkey: Vec<u8>,
        status: String,
        action: String,
        summary: String,
    },
}

impl ModerationPostCommitKind {
    fn application_target(&self) -> Result<ModerationApplicationTarget<'_>, AdmissionCommitError> {
        let target = match self {
            Self::Ban { target, .. }
            | Self::Unban { target }
            | Self::Timeout { target, .. }
            | Self::Untimeout { target } => ModerationApplicationTarget::Pubkey(target),
            Self::ResolveReport {
                report_event_id, ..
            } => ModerationApplicationTarget::ReportEvent(report_event_id),
        };
        target
            .parts()
            .map(|_| target)
            .ok_or(AdmissionCommitError::IntentConflict)
    }
}

#[derive(Serialize, Deserialize)]
struct ModerationApplicationResultPayload {
    object_key: [u8; 32],
    intent_digest: [u8; 32],
    action: ModerationPostCommitKind,
}

#[derive(Clone, Copy)]
struct ModerationResultBinding {
    authorization_domain: CommunityId,
    object: AdmissionObject,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: [u8; 32],
}

impl From<AdmissionApplicationResultBinding> for ModerationResultBinding {
    fn from(binding: AdmissionApplicationResultBinding) -> Self {
        Self {
            authorization_domain: binding.authorization_domain(),
            object: binding.object(),
            semantic_fingerprint: *binding.semantic_fingerprint(),
            application_intent_digest: *binding.application_intent_digest(),
        }
    }
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
    let object = moderation_object(tenant.community(), command.application_target())
        .ok_or_else(|| invalid("moderation target could not be bound"))?;
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
                        report_event_id,
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
            let action_object = moderation_object(
                context.authorization_domain(),
                action.0.application_target()?,
            )
            .ok_or(AdmissionCommitError::IntentConflict)?;
            if action_object != self.object {
                return Err(AdmissionCommitError::IntentConflict);
            }
            let result = moderation_application_result(self.object, self.intent_digest, action.0)?;
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
async fn dispatch_committed_moderation_outcome<F, Fut>(
    tenant: &TenantContext,
    outcome: AdmissionCommitOutcome,
    dispatch: F,
) -> Result<bool, AdmissionCommitError>
where
    F: FnOnce(ModerationPostCommitAction) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(action) = committed_moderation_action(tenant, outcome)? else {
        return Ok(false);
    };
    dispatch(action).await;
    Ok(true)
}

fn committed_moderation_action(
    tenant: &TenantContext,
    outcome: AdmissionCommitOutcome,
) -> Result<Option<ModerationPostCommitAction>, AdmissionCommitError> {
    let (receipt, application_result, binding) = match outcome {
        AdmissionCommitOutcome::Committed {
            authorization,
            receipt,
            application_result,
            application_result_binding,
        } => {
            validate_moderation_dispatch_binding(&authorization, tenant, receipt)?;
            (
                receipt,
                application_result.ok_or(AdmissionCommitError::IntentConflict)?,
                application_result_binding
                    .ok_or(AdmissionCommitError::IntentConflict)?
                    .into(),
            )
        }
        AdmissionCommitOutcome::ExactReplay { .. } => return Ok(None),
    };
    validate_committed_moderation_result(tenant, binding, receipt, &application_result).map(Some)
}

fn validate_moderation_dispatch_binding(
    authorization: &FinalizedAuthContext,
    tenant: &TenantContext,
    receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
) -> Result<(), AdmissionCommitError> {
    validate_moderation_dispatch_coordinates(
        authorization.authorization_domain(),
        authorization.capability(),
        authorization.request_fingerprint(),
        authorization.lease().request_binding().1,
        tenant,
        receipt,
    )
}

fn validate_moderation_dispatch_coordinates(
    authorization_domain: CommunityId,
    capability: RouteCapability,
    request_fingerprint: &[u8; 32],
    target_fingerprint: &[u8; 32],
    tenant: &TenantContext,
    receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
) -> Result<(), AdmissionCommitError> {
    if authorization_domain != tenant.community()
        || authorization_domain != receipt.authorization_domain()
        || capability != RouteCapability::Moderation
        || request_fingerprint != receipt.request_fingerprint()
        || target_fingerprint != receipt.object().key()
        || receipt.object().kind() != AdmissionObjectKind::ModerationTarget
    {
        Err(AdmissionCommitError::IntentConflict)
    } else {
        Ok(())
    }
}

fn validate_committed_moderation_result(
    tenant: &TenantContext,
    binding: ModerationResultBinding,
    receipt: buzz_db::authorization_admission::AdmissionCommitReceipt,
    application_result: &AdmissionApplicationResult,
) -> Result<ModerationPostCommitAction, AdmissionCommitError> {
    if application_result.schema() != moderation_result_schema() || application_result.code() != 1 {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let payload =
        serde_json::from_slice::<ModerationApplicationResultPayload>(application_result.payload())
            .map_err(|_| AdmissionCommitError::IntentConflict)?;
    let action_object = moderation_object(tenant.community(), payload.action.application_target()?)
        .ok_or(AdmissionCommitError::IntentConflict)?;
    if binding.authorization_domain != tenant.community()
        || binding.object != action_object
        || receipt.authorization_domain() != binding.authorization_domain
        || receipt.object() != action_object
        || receipt.semantic_fingerprint() != &binding.semantic_fingerprint
        || action_object.key() != &payload.object_key
        || payload.intent_digest != binding.application_intent_digest
    {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let expected_result = moderation_application_result(
        action_object,
        binding.application_intent_digest,
        payload.action.clone(),
    )?;
    if application_result != &expected_result {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let expected_digest = canonical_moderation_result_digest(
        binding.authorization_domain,
        action_object,
        binding.semantic_fingerprint,
        binding.application_intent_digest,
        &expected_result,
    )?;
    if receipt.application_result_digest() != Some(&expected_digest) {
        return Err(AdmissionCommitError::IntentConflict);
    }
    Ok(ModerationPostCommitAction(payload.action))
}

fn moderation_result_schema() -> AdmissionApplicationResultSchema {
    AdmissionApplicationResultSchema::moderation()
}

fn moderation_application_result(
    object: AdmissionObject,
    intent_digest: [u8; 32],
    action: ModerationPostCommitKind,
) -> Result<AdmissionApplicationResult, AdmissionCommitError> {
    let payload = serde_json::to_vec(&ModerationApplicationResultPayload {
        object_key: *object.key(),
        intent_digest,
        action,
    })
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    AdmissionApplicationResult::new(moderation_result_schema(), 1, payload)
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)
}

fn canonical_moderation_result_digest(
    authorization_domain: CommunityId,
    object: AdmissionObject,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: [u8; 32],
    result: &AdmissionApplicationResult,
) -> Result<[u8; 32], AdmissionCommitError> {
    if authorization_domain.as_uuid().is_nil()
        || semantic_fingerprint == [0; 32]
        || application_intent_digest == [0; 32]
    {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    let schema = result.schema();
    Ok(admission_framed_digest(
        b"buzz:canonical-application-result:v1",
        &[
            authorization_domain.as_uuid().as_bytes(),
            &object.kind().database_code().to_be_bytes(),
            object.key(),
            &semantic_fingerprint,
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
pub(crate) async fn dispatch_moderation_postcommit(
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
            report_event_id: _,
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
    target: ModerationApplicationTarget<'_>,
) -> Option<AdmissionObject> {
    let (target_kind, target) = target.parts()?;
    let key = framed_digest(
        b"buzz:nip-fi:moderation-target:v1",
        &[community.as_uuid().as_bytes(), target_kind, target],
    );
    AdmissionObject::new(AdmissionObjectKind::ModerationTarget, key)
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

fn admission_error(error_value: AdmissionCommitError) -> String {
    match error_value {
        AdmissionCommitError::InvalidRequest
        | AdmissionCommitError::RecordedInvalidRequest
        | AdmissionCommitError::AuthorizationDenied
        | AdmissionCommitError::RecordedAuthorizationDenied
        | AdmissionCommitError::IntentConflict
        | AdmissionCommitError::ReplayRejected
        | AdmissionCommitError::RecordedIntentConflict
        | AdmissionCommitError::RecordedReplayRejected => moderation_denied(),
        AdmissionCommitError::AuditUnavailable
        | AdmissionCommitError::RecordedAuditUnavailable
        | AdmissionCommitError::DependencyUnavailable => moderation_unavailable(),
    }
}

fn moderation_denied() -> String {
    invalid("moderation_admission_denied")
}

fn moderation_unavailable() -> String {
    error("moderation_admission_unavailable")
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
    use sqlx::PgPool;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    async fn install_moderation_authority(
        pool: &PgPool,
        domain: CommunityId,
        actor: nostr::PublicKey,
        with_audit_capacity: bool,
    ) {
        let operation_id = Uuid::new_v4();
        let history_id = Uuid::new_v4();
        let binding_id = Uuid::new_v4();
        let request_fingerprint = [31_u8; 32];
        let actor_bytes = actor.to_bytes();
        let mut transaction = pool.begin().await.expect("begin authority fixture");
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(domain.as_uuid())
            .bind(format!("moderation-{}.example", domain.as_uuid().simple()))
            .execute(&mut *transaction)
            .await
            .expect("insert authority community");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 1, 2, $2, transaction_timestamp() - interval '1 second')",
        )
        .bind(domain.as_uuid())
        .bind([32_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert authority policy");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id, current_generation) \
             VALUES ($1, 0)",
        )
        .bind(domain.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("insert invalidation domain");
        let max_events = if with_audit_capacity { 32_i64 } else { 1_i64 };
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, $2, 2097152, 16384)",
        )
        .bind(domain.as_uuid())
        .bind(max_events)
        .execute(&mut *transaction)
        .await
        .expect("insert audit capacity");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 1, $5)",
        )
        .bind(domain.as_uuid())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(actor_bytes.as_slice())
        .bind([33_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert binding receipt");
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, actor_kind, \
              actor_fingerprint, operation_id, request_fingerprint, correlation_id, attempt_id, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 1, 1, 1, 1, $3, $4, $5, $6, $7, \
                     transaction_timestamp(), $8, $9)",
        )
        .bind(domain.as_uuid())
        .bind(Uuid::new_v4())
        .bind(actor_bytes.as_slice())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind([1_u8].as_slice())
        .bind([37_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert binding audit event");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id, binding_id, issuer, subject, principal_fingerprint, \
              event_author_pubkey, binding_state, lifecycle_revision, binding_provenance, \
              policy_revision, enrollment_evidence_digest, birth_history_id, \
              creation_operation_id, creation_request_fingerprint) \
             VALUES ($1, $2, 'https://issuer.example', $3, $4, $5, 1, 1, 2, 1, $6, $7, $8, $9) \
             RETURNING binding_version",
        )
        .bind(domain.as_uuid())
        .bind(binding_id)
        .bind(format!("moderator-{binding_id}"))
        .bind([34_u8; 32].as_slice())
        .bind(actor_bytes.as_slice())
        .bind([35_u8; 32].as_slice())
        .bind(history_id)
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .fetch_one(&mut *transaction)
        .await
        .expect("insert active moderation binding");
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id, history_id, transition_kind, outcome_code, successor_binding_id, \
              successor_binding_version, successor_lifecycle_revision, successor_state, \
              operation_id, request_fingerprint, transition_digest) \
             VALUES ($1, $2, 1, 1, $3, $4, 1, 1, $5, $6, $7)",
        )
        .bind(domain.as_uuid())
        .bind(history_id)
        .bind(binding_id)
        .bind(binding_version)
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind([36_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert binding history");
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
             VALUES ($1, $2, 'owner', NULL)",
        )
        .bind(domain.as_uuid())
        .bind(hex::encode(actor_bytes))
        .execute(&mut *transaction)
        .await
        .expect("insert moderation owner");
        transaction
            .commit()
            .await
            .expect("commit authority fixture");
    }

    fn signed_ban(keys: &Keys, target: nostr::PublicKey) -> Event {
        let target_hex = target.to_hex();
        EventBuilder::new(Kind::from(KIND_MODERATION_BAN as u16), "")
            .tag(Tag::parse(["p", target_hex.as_str()]).expect("moderation target tag"))
            .custom_created_at(nostr::Timestamp::now())
            .sign_with_keys(keys)
            .expect("sign moderation command")
    }

    fn signed_unban(keys: &Keys, target: nostr::PublicKey, nonce: &str) -> Event {
        let target_hex = target.to_hex();
        EventBuilder::new(Kind::from(KIND_MODERATION_UNBAN as u16), "")
            .tags([
                Tag::parse(["p", target_hex.as_str()]).expect("moderation target tag"),
                Tag::parse(["nonce", nonce]).expect("test nonce tag"),
            ])
            .custom_created_at(nostr::Timestamp::now())
            .sign_with_keys(keys)
            .expect("sign moderation command")
    }

    fn websocket_moderation_proof(
        domain: CommunityId,
        object: AdmissionObject,
        event: &Event,
    ) -> buzz_auth::VerifiedModerationCommandProof {
        let coordinates = buzz_auth::Nip42ModerationCommandCoordinates::new(
            domain,
            *object.key(),
            "wss://relay.example",
            event,
        )
        .expect("moderation coordinates");
        buzz_auth::verify_nip42_moderation_command_proof(event, &coordinates, Utc::now())
            .expect("moderation proof")
    }

    fn websocket_ingest_auth(keys: &Keys, relay_url: &str) -> IngestAuth {
        IngestAuth::Nip42 {
            pubkey: keys.public_key(),
            scopes: Vec::new(),
            channel_ids: None,
            conn_id: Uuid::new_v4(),
            moderation_evidence: ModerationTransportEvidence::Nip42 {
                relay_url: relay_url.into(),
            },
        }
    }

    fn http_ingest_auth(event: &Event, keys: &Keys) -> IngestAuth {
        let url = "https://relay.example/events";
        let body = serde_json::to_vec(event).expect("serialize moderation command");
        let payload = hex::encode(Sha256::digest(&body));
        let authorization = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url]).expect("NIP-98 URL tag"),
                Tag::parse(["method", "POST"]).expect("NIP-98 method tag"),
                Tag::parse(["payload", payload.as_str()]).expect("NIP-98 payload tag"),
            ])
            .custom_created_at(nostr::Timestamp::now())
            .sign_with_keys(keys)
            .expect("sign NIP-98 authorization");
        IngestAuth::Http {
            pubkey: keys.public_key(),
            scopes: buzz_auth::Scope::all_known(),
            auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
            moderation_evidence: Some(ModerationTransportEvidence::Nip98 {
                authorization_event: serde_json::to_string(&authorization)
                    .expect("serialize NIP-98 authorization")
                    .into(),
                body: body.into(),
            }),
        }
    }

    fn receipt_with_application_result(
        tenant: &TenantContext,
        object: AdmissionObject,
        intent_digest: [u8; 32],
        result: &AdmissionApplicationResult,
    ) -> buzz_db::authorization_admission::AdmissionCommitReceipt {
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
        let application_result_digest = canonical_moderation_result_digest(
            tenant.community(),
            object,
            *provisional.semantic_fingerprint(),
            intent_digest,
            result,
        )
        .expect("canonical result digest");
        buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
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
        .expect("canonical receipt")
    }

    fn result_binding(
        tenant: &TenantContext,
        object: AdmissionObject,
        intent_digest: [u8; 32],
    ) -> ModerationResultBinding {
        ModerationResultBinding {
            authorization_domain: tenant.community(),
            object,
            semantic_fingerprint: [10; 32],
            application_intent_digest: intent_digest,
        }
    }

    fn application_result_digest(
        authorization_domain: CommunityId,
        object: AdmissionObject,
        semantic_fingerprint: [u8; 32],
        application_intent_digest: [u8; 32],
        result: &AdmissionApplicationResult,
    ) -> [u8; 32] {
        let schema = result.schema();
        let domain = b"buzz:canonical-application-result:v1";
        let object_kind = object.kind().database_code().to_be_bytes();
        let schema_version = schema.version().to_be_bytes();
        let result_code = result.code().to_be_bytes();
        let fields: [&[u8]; 9] = [
            authorization_domain.as_uuid().as_bytes(),
            &object_kind,
            object.key(),
            &semantic_fingerprint,
            &application_intent_digest,
            schema.type_key(),
            &schema_version,
            &result_code,
            result.payload(),
        ];
        let mut digest = Sha256::new();
        digest.update((domain.len() as u64).to_be_bytes());
        digest.update(domain);
        for field in fields {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field);
        }
        digest.finalize().into()
    }

    fn receipt_with_application_digest(
        tenant: &TenantContext,
        object: AdmissionObject,
        semantic_fingerprint: [u8; 32],
        application_result_digest: [u8; 32],
    ) -> buzz_db::authorization_admission::AdmissionCommitReceipt {
        buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
            tenant.community(),
            object,
            Uuid::new_v4(),
            [9; 32],
            semantic_fingerprint,
            buzz_db::authorization_admission::AdmissionCommitDigests::new(
                [11; 32],
                Some(application_result_digest),
            )
            .expect("receipt digests"),
            Uuid::new_v4(),
        )
        .expect("storage receipt")
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
    fn protected_moderation_denials_use_stable_public_tokens() {
        use buzz_db::authorization_admission::AdmissionCommitError;

        for denial in [
            AdmissionCommitError::InvalidRequest,
            AdmissionCommitError::AuthorizationDenied,
            AdmissionCommitError::IntentConflict,
            AdmissionCommitError::ReplayRejected,
            AdmissionCommitError::RecordedInvalidRequest,
            AdmissionCommitError::RecordedAuthorizationDenied,
            AdmissionCommitError::RecordedIntentConflict,
            AdmissionCommitError::RecordedReplayRejected,
        ] {
            assert_eq!(
                admission_error(denial),
                "invalid: moderation_admission_denied"
            );
        }
        for unavailable in [
            AdmissionCommitError::AuditUnavailable,
            AdmissionCommitError::RecordedAuditUnavailable,
            AdmissionCommitError::DependencyUnavailable,
        ] {
            assert_eq!(
                admission_error(unavailable),
                "error: moderation_admission_unavailable"
            );
        }
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
    fn moderation_exact_replay_never_yields_a_postcommit_action() {
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
            committed_moderation_action(&tenant, outcome),
            Ok(None)
        ));
    }

    async fn create_disposable_moderation_database() -> (String, PgPool, PgPool) {
        let base_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .expect("BUZZ_TEST_DATABASE_URL must name a PostgreSQL administrator database");
        let admin_url = base_url
            .rsplit_once('/')
            .map(|(prefix, _)| format!("{prefix}/postgres"))
            .expect("test database URL has a database name");
        let admin = PgPool::connect(&admin_url)
            .await
            .expect("connect PostgreSQL admin database");
        let database_name = format!("buzz_moderation_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE \"{database_name}\""
        )))
        .execute(&admin)
        .await
        .expect("create disposable moderation database");
        let database_url = base_url
            .rsplit_once('/')
            .map(|(prefix, _)| format!("{prefix}/{database_name}"))
            .expect("derive disposable moderation database URL");
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect disposable moderation database");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("run moderation migrations");
        (database_name, admin, pool)
    }

    async fn drop_disposable_moderation_database(database_name: &str, admin: PgPool, pool: PgPool) {
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE \"{database_name}\" WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop disposable moderation database");
        admin.close().await;
    }

    #[tokio::test]
    async fn production_commit_dispatches_only_one_fresh_moderation_outcome() {
        let (database_name, admin, pool) = create_disposable_moderation_database().await;
        let db = buzz_db::Db::from_pool(pool.clone());
        let keys = Keys::generate();

        let committed_domain = CommunityId::from_uuid(Uuid::new_v4());
        install_moderation_authority(&pool, committed_domain, keys.public_key(), true).await;
        let committed_tenant = TenantContext::resolved(committed_domain, "relay.example");
        let committed_target = Keys::generate().public_key();
        let committed_event = signed_ban(&keys, committed_target);
        let dispatches = AtomicUsize::new(0);
        execute_moderation_command(
            &committed_tenant,
            &db,
            buzz_auth::NipFiMode::Enforce,
            "wss://relay.example",
            &committed_event,
            &websocket_ingest_auth(&keys, "wss://relay.example"),
            |_| async {
                dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .expect("execute committed moderation command");
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);

        execute_moderation_command(
            &committed_tenant,
            &db,
            buzz_auth::NipFiMode::Enforce,
            "wss://relay.example",
            &committed_event,
            &http_ingest_auth(&committed_event, &keys),
            |_| async {
                dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .expect("execute cross-transport exact replay");
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);

        let rollback_domain = CommunityId::from_uuid(Uuid::new_v4());
        install_moderation_authority(&pool, rollback_domain, keys.public_key(), true).await;
        sqlx::query("UPDATE relay_members SET role='member' WHERE community_id=$1 AND pubkey=$2")
            .bind(rollback_domain.as_uuid())
            .bind(keys.public_key().to_hex())
            .execute(&pool)
            .await
            .expect("remove moderator role before protected admission");
        let rollback_tenant = TenantContext::resolved(rollback_domain, "rollback.example");
        let rollback_target = Keys::generate().public_key();
        let rollback_event = signed_ban(&keys, rollback_target);
        assert!(execute_moderation_command(
            &rollback_tenant,
            &db,
            buzz_auth::NipFiMode::Enforce,
            "wss://rollback.example",
            &rollback_event,
            &websocket_ingest_auth(&keys, "wss://rollback.example"),
            |_| async {
                dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .is_err());
        let rollback_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id=$1")
                .bind(rollback_domain.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count rolled-back actions");
        assert_eq!(rollback_rows, 0);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        let rollback_denials: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_events \
             WHERE community_id=$1 AND event_kind=11 AND outcome_code=2 AND reason_code=9",
        )
        .bind(rollback_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count durable rolled-back moderation denial");
        assert_eq!(rollback_denials, 1);
        let rollback_envelope: Vec<u8> = sqlx::query_scalar(
            "SELECT canonical_envelope FROM authorization_events \
             WHERE community_id=$1 AND event_kind=11 AND outcome_code=2",
        )
        .bind(rollback_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read redacted moderation denial envelope");
        for sensitive in [keys.public_key().to_bytes(), rollback_target.to_bytes()] {
            assert!(
                !rollback_envelope
                    .windows(sensitive.len())
                    .any(|window| window == sensitive),
                "canonical denial envelope retained a raw moderation coordinate"
            );
        }
        let rollback_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_kind=11 AND outcome_code=2",
        )
        .bind(rollback_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count durable denied moderation receipt");
        assert_eq!(rollback_receipts, 1);
        let rollback_buckets: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operator_denial_buckets \
             WHERE community_id=$1 AND surface_kind=3",
        )
        .bind(rollback_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count unresolved rollback buckets");
        assert_eq!(rollback_buckets, 0);
        assert!(execute_moderation_command(
            &rollback_tenant,
            &db,
            buzz_auth::NipFiMode::Enforce,
            "wss://rollback.example",
            &rollback_event,
            &websocket_ingest_auth(&keys, "wss://rollback.example"),
            |_| async {
                dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .is_err());
        let replayed_denials: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_events \
             WHERE community_id=$1 AND event_kind=11 AND outcome_code=2",
        )
        .bind(rollback_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count exact denied replay evidence");
        assert_eq!(replayed_denials, 1);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);

        let capacity_domain = CommunityId::from_uuid(Uuid::new_v4());
        install_moderation_authority(&pool, capacity_domain, keys.public_key(), false).await;
        let capacity_tenant = TenantContext::resolved(capacity_domain, "capacity.example");
        let capacity_event = signed_ban(&keys, Keys::generate().public_key());
        assert!(execute_moderation_command(
            &capacity_tenant,
            &db,
            buzz_auth::NipFiMode::Enforce,
            "wss://capacity.example",
            &capacity_event,
            &websocket_ingest_auth(&keys, "wss://capacity.example"),
            |_| async {
                dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .is_err());
        let capacity_state: (i64, i64, i64, i16, Option<i16>, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM moderation_actions WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_events \
                    WHERE community_id=$1 AND event_kind=11), \
                 (SELECT count(*) FROM authorization_operation_receipts \
                    WHERE community_id=$1 AND operation_kind=11), \
                 (SELECT health_state FROM authorization_event_capacity \
                    WHERE community_id=$1), \
                 (SELECT failure_code FROM authorization_event_capacity \
                    WHERE community_id=$1), \
                 (SELECT COALESCE(sum(lifetime_count),0)::BIGINT \
                    FROM authorization_operator_denial_buckets \
                    WHERE community_id=$1 AND surface_kind=3 AND denial_class=8)",
        )
        .bind(capacity_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read failed-closed moderation capacity state");
        assert_eq!(capacity_state, (0, 0, 0, 2, Some(1), 1));
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);

        let mode_dispatches = AtomicUsize::new(0);
        assert!(execute_moderation_command(
            &committed_tenant,
            &db,
            buzz_auth::NipFiMode::DenyProtected,
            "wss://relay.example",
            &signed_ban(&keys, Keys::generate().public_key()),
            &websocket_ingest_auth(&keys, "wss://relay.example"),
            |_| async {
                mode_dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .is_err());
        let missing_proof = IngestAuth::Http {
            pubkey: keys.public_key(),
            scopes: buzz_auth::Scope::all_known(),
            auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
            moderation_evidence: None,
        };
        assert!(execute_moderation_command(
            &committed_tenant,
            &db,
            buzz_auth::NipFiMode::Enforce,
            "wss://relay.example",
            &signed_ban(&keys, Keys::generate().public_key()),
            &missing_proof,
            |_| async {
                mode_dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .is_err());
        assert_eq!(mode_dispatches.load(Ordering::SeqCst), 0);
        let mode_denials: Vec<(i16, i64)> = sqlx::query_as(
            "SELECT denial_class,sum(denial_count)::BIGINT \
             FROM authorization_operator_denial_buckets \
             WHERE community_id=$1 AND surface_kind=3 AND action_kind=2 \
             GROUP BY denial_class ORDER BY denial_class",
        )
        .bind(committed_domain.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("read durable moderation mode denials");
        assert_eq!(mode_denials, vec![(1, 1), (3, 1)]);

        let off_domain = CommunityId::from_uuid(Uuid::new_v4());
        install_moderation_authority(&pool, off_domain, keys.public_key(), true).await;
        let off_tenant = TenantContext::resolved(off_domain, "off.example");
        let off_event = signed_ban(&keys, Keys::generate().public_key());
        execute_moderation_command(
            &off_tenant,
            &db,
            buzz_auth::NipFiMode::Off,
            "wss://off.example",
            &off_event,
            &websocket_ingest_auth(&keys, "wss://off.example"),
            |_| async {
                mode_dispatches.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await
        .expect("execute Off-mode moderation command");
        assert_eq!(mode_dispatches.load(Ordering::SeqCst), 1);
        let off_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id=$1")
                .bind(off_domain.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count Off-mode actions");
        assert_eq!(off_rows, 1);
        let off_denials: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operator_denial_buckets \
             WHERE community_id=$1 AND surface_kind=3",
        )
        .bind(off_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count Off-mode denial buckets");
        assert_eq!(off_denials, 0);

        let first_unban = signed_unban(&keys, committed_target, "first");
        let second_unban = signed_unban(&keys, committed_target, "second");
        let first_effect = prepare_moderation_application_effect(&committed_tenant, &first_unban)
            .expect("prepare first same-target effect");
        let second_effect = prepare_moderation_application_effect(&committed_tenant, &second_unban)
            .expect("prepare second same-target effect");
        let same_target_object = first_effect.admission_object();
        assert_eq!(same_target_object, second_effect.admission_object());
        let first_request = db
            .prepare_canonical_moderation_request(
                websocket_moderation_proof(committed_domain, same_target_object, &first_unban),
                same_target_object,
            )
            .await
            .expect("prepare first same-target request");
        let second_request = db
            .prepare_canonical_moderation_request(
                websocket_moderation_proof(committed_domain, same_target_object, &second_unban),
                same_target_object,
            )
            .await
            .expect("prepare second same-target request");
        let first_request = install_moderation_application_effect(first_request, first_effect)
            .expect("install first same-target effect");
        let second_request = install_moderation_application_effect(second_request, second_effect)
            .expect("install second same-target effect");
        let first_committer = db.canonical_moderation_committer();
        let second_committer = db.canonical_moderation_committer();
        let (first_result, second_result) = tokio::join!(
            first_committer.commit(first_request),
            second_committer.commit(second_request),
        );
        assert_eq!(
            usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
            1
        );
        let rejected = if first_result.is_err() {
            first_result
        } else {
            second_result
        };
        assert!(matches!(
            rejected,
            Err(AdmissionCommitError::RecordedAuthorizationDenied)
        ));
        let authority_epoch: i64 = sqlx::query_scalar(
            "SELECT authority_epoch FROM authorization_authority_epochs \
             WHERE community_id=$1 AND object_kind=$2 AND object_key=$3",
        )
        .bind(committed_domain.as_uuid())
        .bind(same_target_object.kind().database_code())
        .bind(same_target_object.key().as_slice())
        .fetch_one(&pool)
        .await
        .expect("read serialized moderation epoch");
        assert_eq!(authority_epoch, 2);

        let denied_domain = CommunityId::from_uuid(Uuid::new_v4());
        install_moderation_authority(&pool, denied_domain, keys.public_key(), true).await;
        let denied_tenant = TenantContext::resolved(denied_domain, "denied.example");
        let denied_event = signed_ban(&keys, Keys::generate().public_key());
        let denied_effect = prepare_moderation_application_effect(&denied_tenant, &denied_event)
            .expect("prepare denied effect");
        let denied_object = denied_effect.admission_object();
        let denied_proof = websocket_moderation_proof(denied_domain, denied_object, &denied_event);
        let denied_request = db
            .prepare_canonical_moderation_request(denied_proof, denied_object)
            .await
            .expect("prepare denied request");
        let denied_correlation_id = denied_request.correlation_id();
        let denied_attempt_id = denied_request.attempt_id();
        assert_ne!(denied_correlation_id, denied_attempt_id);
        let denied_request = install_moderation_application_effect(denied_request, denied_effect)
            .expect("install denied effect");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 2, 2, $2, transaction_timestamp())",
        )
        .bind(denied_domain.as_uuid())
        .bind([38_u8; 32].as_slice())
        .execute(&pool)
        .await
        .expect("supersede prepared authority policy");
        assert!(matches!(
            db.canonical_moderation_committer()
                .commit(denied_request)
                .await,
            Err(AdmissionCommitError::RecordedAuthorizationDenied)
        ));
        let denied_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id=$1")
                .bind(denied_domain.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count denied actions");
        assert_eq!(denied_rows, 0);
        let retained_denial_identity: (Uuid, Uuid) = sqlx::query_as(
            "SELECT correlation_id,attempt_id FROM authorization_events \
             WHERE community_id=$1 AND event_kind=11 AND outcome_code=2",
        )
        .bind(denied_domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read retained moderation denial correlation");
        assert_eq!(retained_denial_identity.0, denied_correlation_id);
        assert_eq!(retained_denial_identity.1, denied_attempt_id);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);

        drop(db);
        drop_disposable_moderation_database(&database_name, admin, pool).await;
    }

    #[test]
    fn moderation_committed_dispatch_rejects_authority_reused_for_another_receipt() {
        let tenant_context = tenant(10);
        let object = AdmissionObject::new(AdmissionObjectKind::ModerationTarget, [7; 32])
            .expect("moderation object");
        let receipt = buzz_db::authorization_admission::AdmissionCommitReceipt::from_storage(
            tenant_context.community(),
            object,
            Uuid::new_v4(),
            [9; 32],
            [10; 32],
            buzz_db::authorization_admission::AdmissionCommitDigests::new([11; 32], Some([12; 32]))
                .expect("digests"),
            Uuid::new_v4(),
        )
        .expect("receipt");

        assert!(validate_moderation_dispatch_coordinates(
            tenant_context.community(),
            RouteCapability::Moderation,
            &[9; 32],
            object.key(),
            &tenant_context,
            receipt,
        )
        .is_ok());
        for denied in [
            validate_moderation_dispatch_coordinates(
                tenant(11).community(),
                RouteCapability::Moderation,
                &[9; 32],
                object.key(),
                &tenant_context,
                receipt,
            ),
            validate_moderation_dispatch_coordinates(
                tenant_context.community(),
                RouteCapability::MediaWrite,
                &[9; 32],
                object.key(),
                &tenant_context,
                receipt,
            ),
            validate_moderation_dispatch_coordinates(
                tenant_context.community(),
                RouteCapability::Moderation,
                &[13; 32],
                object.key(),
                &tenant_context,
                receipt,
            ),
            validate_moderation_dispatch_coordinates(
                tenant_context.community(),
                RouteCapability::Moderation,
                &[9; 32],
                &[14; 32],
                &tenant_context,
                receipt,
            ),
        ] {
            assert!(matches!(denied, Err(AdmissionCommitError::IntentConflict)));
        }
    }

    #[test]
    fn committed_result_must_match_receipt_digest_and_moderation_object() {
        let tenant = tenant(10);
        let target = vec![8; 32];
        let object = moderation_object(
            tenant.community(),
            ModerationApplicationTarget::Pubkey(&target),
        )
        .expect("moderation object");
        let intent_digest = [13; 32];
        let result = AdmissionApplicationResult::new(
            moderation_result_schema(),
            1,
            serde_json::to_vec(&ModerationApplicationResultPayload {
                object_key: *object.key(),
                intent_digest,
                action: ModerationPostCommitKind::Unban {
                    target: target.clone(),
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
        let application_result_digest = canonical_moderation_result_digest(
            tenant.community(),
            object,
            *provisional.semantic_fingerprint(),
            intent_digest,
            &result,
        )
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
        let binding = result_binding(&tenant, object, intent_digest);
        let action = validate_committed_moderation_result(&tenant, binding, receipt, &result)
            .expect("matching receipt and result");
        assert!(matches!(
            action.0,
            ModerationPostCommitKind::Unban { target } if target == vec![8; 32]
        ));

        let other_target = vec![14; 32];
        let other_object = moderation_object(
            tenant.community(),
            ModerationApplicationTarget::Pubkey(&other_target),
        )
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
            validate_committed_moderation_result(&tenant, binding, other_receipt, &result),
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
            validate_committed_moderation_result(&tenant, binding, wrong_digest_receipt, &result),
            Err(AdmissionCommitError::IntentConflict)
        ));

        let substituted_result = AdmissionApplicationResult::new(
            moderation_result_schema(),
            1,
            serde_json::to_vec(&ModerationApplicationResultPayload {
                object_key: *object.key(),
                intent_digest,
                action: ModerationPostCommitKind::Unban {
                    target: vec![16; 32],
                },
            })
            .expect("serialize substituted decoded target"),
        )
        .expect("substituted decoded target result");
        let substituted_receipt =
            receipt_with_application_result(&tenant, object, intent_digest, &substituted_result);
        assert!(matches!(
            validate_committed_moderation_result(
                &tenant,
                binding,
                substituted_receipt,
                &substituted_result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        ));
    }

    #[test]
    fn moderation_result_binding_rejects_recomputed_substitutions() {
        let tenant = tenant(12);
        let target = vec![19; 32];
        let object = moderation_object(
            tenant.community(),
            ModerationApplicationTarget::Pubkey(&target),
        )
        .expect("moderation object");
        let intent_digest = [20; 32];
        let semantic_fingerprint = [10; 32];
        let binding = result_binding(&tenant, object, intent_digest);
        let action = ModerationPostCommitKind::Unban { target };
        let result = moderation_application_result(object, intent_digest, action.clone())
            .expect("canonical result");
        let result_digest = application_result_digest(
            tenant.community(),
            object,
            semantic_fingerprint,
            intent_digest,
            &result,
        );
        let receipt =
            receipt_with_application_digest(&tenant, object, semantic_fingerprint, result_digest);
        assert!(validate_committed_moderation_result(&tenant, binding, receipt, &result).is_ok());

        let substituted_semantic = [21; 32];
        let substituted_semantic_digest = application_result_digest(
            tenant.community(),
            object,
            substituted_semantic,
            intent_digest,
            &result,
        );
        let substituted_semantic_receipt = receipt_with_application_digest(
            &tenant,
            object,
            substituted_semantic,
            substituted_semantic_digest,
        );
        assert!(matches!(
            validate_committed_moderation_result(
                &tenant,
                binding,
                substituted_semantic_receipt,
                &result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        ));

        let substituted_intent = [22; 32];
        let substituted_intent_result =
            moderation_application_result(object, substituted_intent, action)
                .expect("substituted intent result");
        let substituted_intent_digest = application_result_digest(
            tenant.community(),
            object,
            semantic_fingerprint,
            substituted_intent,
            &substituted_intent_result,
        );
        let substituted_intent_receipt = receipt_with_application_digest(
            &tenant,
            object,
            semantic_fingerprint,
            substituted_intent_digest,
        );
        assert!(matches!(
            validate_committed_moderation_result(
                &tenant,
                binding,
                substituted_intent_receipt,
                &substituted_intent_result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        ));

        let mut payload: serde_json::Value =
            serde_json::from_slice(result.payload()).expect("canonical payload");
        payload
            .as_object_mut()
            .expect("application result object")
            .insert("ignored".to_owned(), serde_json::Value::Bool(true));
        let noncanonical = AdmissionApplicationResult::new(
            moderation_result_schema(),
            1,
            serde_json::to_vec(&payload).expect("noncanonical payload"),
        )
        .expect("bounded noncanonical result");
        let noncanonical_digest = application_result_digest(
            tenant.community(),
            object,
            semantic_fingerprint,
            intent_digest,
            &noncanonical,
        );
        let noncanonical_receipt = receipt_with_application_digest(
            &tenant,
            object,
            semantic_fingerprint,
            noncanonical_digest,
        );
        assert!(matches!(
            validate_committed_moderation_result(
                &tenant,
                binding,
                noncanonical_receipt,
                &noncanonical,
            ),
            Err(AdmissionCommitError::IntentConflict)
        ));
    }

    #[test]
    fn moderation_resolve_result_rejects_a_substituted_report_event_target() {
        let tenant = tenant(11);
        let report_event_id = vec![17; 32];
        let object = moderation_object(
            tenant.community(),
            ModerationApplicationTarget::ReportEvent(&report_event_id),
        )
        .expect("report moderation object");
        let intent_digest = [18; 32];
        let make_result = |decoded_report_event_id: Vec<u8>| {
            AdmissionApplicationResult::new(
                moderation_result_schema(),
                1,
                serde_json::to_vec(&ModerationApplicationResultPayload {
                    object_key: *object.key(),
                    intent_digest,
                    action: ModerationPostCommitKind::ResolveReport {
                        report_event_id: decoded_report_event_id,
                        report_id: Uuid::new_v4(),
                        reporter_pubkey: vec![19; 32],
                        status: "resolved".to_owned(),
                        action: "delete".to_owned(),
                        summary: "resolved".to_owned(),
                    },
                })
                .expect("serialize report result"),
            )
            .expect("typed report result")
        };

        let matching_result = make_result(report_event_id.clone());
        let matching_receipt =
            receipt_with_application_result(&tenant, object, intent_digest, &matching_result);
        let binding = result_binding(&tenant, object, intent_digest);
        assert!(validate_committed_moderation_result(
            &tenant,
            binding,
            matching_receipt,
            &matching_result,
        )
        .is_ok());

        let substituted_result = make_result(vec![20; 32]);
        let substituted_receipt =
            receipt_with_application_result(&tenant, object, intent_digest, &substituted_result);
        assert!(matches!(
            validate_committed_moderation_result(
                &tenant,
                binding,
                substituted_receipt,
                &substituted_result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        ));

        assert_ne!(
            moderation_object(
                tenant.community(),
                ModerationApplicationTarget::Pubkey(&report_event_id),
            ),
            Some(object),
            "target namespace must distinguish a report event from a pubkey"
        );
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
