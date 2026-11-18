//! Explicit community artifacts: transport authorization precedes receipts;
//! edit policy is checked only for a new write, in its accepting transaction.
use super::ingest::{IngestAuth, IngestError, IngestResult};
use crate::state::AppState;
use buzz_auth::Scope;
use buzz_core::{
    artifact::{self, ArtifactScope},
    TenantContext,
};
use buzz_db::community_artifact::{CommunityArtifactPrincipal, PolicyOutcome};
use nostr::Event;
use std::sync::Arc;

/// AuthContext records owners for revocation even when OA admission is off.
/// Only a verified FI agent credential or current OA policy permits fallback.
pub(crate) fn owner_admission(
    state: &AppState,
    conn: &crate::connection::ConnectionState,
    owner: Option<nostr::PublicKey>,
) -> bool {
    state.config.allow_nip_oa_auth
        || conn
            .nip_fi_assertion
            .as_ref()
            .and_then(|assertion| assertion.owner())
            .is_some_and(|verified| Some(verified) == owner)
}

pub(crate) fn principal(state: &AppState, auth: &IngestAuth) -> CommunityArtifactPrincipal {
    CommunityArtifactPrincipal {
        pubkey: auth.pubkey().to_bytes().to_vec(),
        verified_owner: auth.verified_owner().map(|p| p.to_bytes().to_vec()),
        owner_admission: auth.owner_admission(),
        require_membership: state.config.require_relay_membership,
    }
}
pub(crate) fn storage_error(error: buzz_db::DbError) -> IngestError {
    match error {
        buzz_db::DbError::AccessDenied(_) => {
            IngestError::AuthFailed("restricted: community artifact access denied".into())
        }
        buzz_db::DbError::NotFound(_) => {
            IngestError::Rejected("invalid: community artifact unavailable".into())
        }
        buzz_db::DbError::InvalidData(message) => {
            IngestError::Rejected(format!("invalid: {message}"))
        }
        _ => IngestError::Internal("error: community artifact operation unavailable".into()),
    }
}
pub(crate) async fn enabled(state: &AppState) -> Result<bool, buzz_db::DbError> {
    if !state.config.community_artifacts_enabled {
        return Ok(false);
    }
    state
        .community_artifact_schema
        .get_or_try_init(|| state.db.community_artifact_schema_ready())
        .await
        .copied()
}
fn invalid(reason: &str) -> IngestError {
    IngestError::Rejected(format!("invalid: {reason}"))
}
fn credential(auth: &IngestAuth, event: &Event, scope: Scope) -> Result<(), IngestError> {
    if !auth.signed_transport()
        || auth.pubkey() != &event.pubkey
        || auth.channel_ids().is_some()
        || !auth.scopes().contains(&scope)
    {
        return Err(IngestError::AuthFailed(
            "restricted: unrestricted authenticated community credential required".into(),
        ));
    }
    Ok(())
}
/// Route only explicitly scoped operations. Every other kind keeps its existing
/// pipeline. The shared ingest seam has already verified the event signature.
pub(crate) async fn ingest(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: &Event,
    auth: &IngestAuth,
) -> Option<Result<IngestResult, IngestError>> {
    let kind = event.kind.as_u16();
    let community_tag = event
        .tags
        .iter()
        .any(|t| t.as_slice().first().is_some_and(|n| n == "community"));
    if kind == 9005 && !community_tag {
        let h: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().is_some_and(|n| n == "h"))
            .collect();
        if h.len() != 1
            || h[0]
                .as_slice()
                .get(1)
                .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
        {
            return Some(Err(invalid(
                "9005 requires exactly one channel or community scope",
            )));
        }
        return None;
    }
    if kind != 9034 && !(community_tag && matches!(kind, 45010 | 9005)) {
        return None;
    }
    Some(execute(state, tenant, event, auth).await)
}
async fn execute(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let community = tenant.community();
    let kind = event.kind.as_u16();
    // Parsing and signed tenant binding always precede accepted-command state.
    let env = if kind == 45010 {
        let env = artifact::validate(event).map_err(invalid)?;
        if env.scope != ArtifactScope::Community || env.community != Some(*community.as_uuid()) {
            return Err(invalid("community scope does not match request tenant"));
        }
        Some(env)
    } else {
        None
    };
    let command = if kind == 9034 {
        let command = artifact::parse_policy_command(event).map_err(invalid)?;
        if command.community != *community.as_uuid() {
            return Err(invalid("community scope does not match request tenant"));
        }
        Some(command)
    } else {
        None
    };
    let target = if kind == 9005 {
        Some(moderation_target(event, tenant)?)
    } else {
        None
    };
    let scope = match kind {
        9034 => Scope::AdminUsers,
        9005 => Scope::MessagesWrite,
        _ => Scope::MessagesWrite,
    };
    credential(auth, event, scope)?;
    if !enabled(state).await.map_err(storage_error)? {
        return Err(invalid(
            "community artifacts are not enabled on a compatible schema",
        ));
    }
    let principal = principal(state, auth);
    if let Some(env) = env {
        // An accepted replay is acknowledgement only: no content, publish, edit
        // policy, timeout or freshness check. Admission still includes bans.
        if state
            .db
            .community_artifact_accepted(community, &principal, event)
            .await
            .map_err(storage_error)?
        {
            return Ok(ack(event));
        }
        let now = chrono::Utc::now().timestamp();
        if event.created_at.as_secs().abs_diff(now as u64) > 900 {
            return Err(invalid("event timestamp too far from server time"));
        }
        if event.content.len() > 256 * 1024 {
            return Err(invalid("event content too large"));
        }
        super::media_access::validate_media_references(tenant, state, event).await?;
        let base =
            crate::api::media::media_base_url_for_tenant(&state.config.relay_url, tenant.host());
        super::ingest::validate_event_imeta(tenant, state, event, &base).await?;
        super::artifact::verify_revision_auth(event)?;
        outcome(
            state,
            tenant,
            state
                .db
                .accept_community_artifact(community, &principal, event, &env)
                .await
                .map_err(storage_error)?,
        )
        .await?;
    } else if let Some(command) = command {
        super::ingest::enforce_write_restriction(
            state,
            tenant,
            buzz_core::kind::RELAY_ADMIN_SET_ARTIFACT_TYPE_POLICY,
            auth.pubkey(),
        )
        .await?;
        let mut tx =
            state.db.pool().begin().await.map_err(|_| {
                IngestError::Internal("error: policy transaction unavailable".into())
            })?;
        match buzz_db::community_artifact::set_policy_in_transaction(
            &mut tx, community, &principal, event, &command,
        )
        .await
        .map_err(storage_error)?
        {
            PolicyOutcome::Conflict(reason) => {
                return Err(IngestError::CanvasConflict(format!("conflict: {reason}")))
            }
            PolicyOutcome::Duplicate => {}
            PolicyOutcome::Applied => {
                buzz_audit::AuditService::log_in_transaction(&mut tx,buzz_audit::NewAuditEntry {
                    community_id:community,action:buzz_audit::AuditAction::EventCreated,actor_pubkey:Some(principal.pubkey),object_id:Some(event.id.to_hex()),
                    detail:serde_json::json!({"command":"set_artifact_type_policy","event_kind":9034,"type":command.artifact_type,"policy":command.policy,"prev":command.prev}),
                }).await.map_err(|_|IngestError::Internal("error: policy audit unavailable".into()))?;
            }
        }
        tx.commit()
            .await
            .map_err(|_| IngestError::Internal("error: policy commit unavailable".into()))?;
    } else if let Some(target) = target {
        if event.content.len() > 256 * 1024 {
            return Err(invalid("event content too large"));
        }
        let now = chrono::Utc::now().timestamp();
        if event.created_at.as_secs().abs_diff(now as u64) > 900 {
            return Err(invalid("event timestamp too far from server time"));
        }
        if let Some((d, artifact_type)) = state
            .db
            .redact_community_artifact(community, &principal, &target)
            .await
            .map_err(storage_error)?
        {
            signal(
                state,
                tenant,
                buzz_pubsub::artifact_signal::ArtifactSignal {
                    d,
                    artifact_type,
                    cause: buzz_pubsub::artifact_signal::ArtifactChange::Redacted,
                },
            )
            .await;
        }
    }
    Ok(ack(event))
}
fn ack(event: &Event) -> IngestResult {
    IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: String::new(),
    }
}
async fn outcome(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    outcome: buzz_db::artifact::ArtifactOutcome,
) -> Result<(), IngestError> {
    use buzz_db::artifact::ArtifactOutcome;
    match outcome {
        ArtifactOutcome::Accepted(events) => {
            for stored in events {
                let env = artifact::validate(&stored.event).map_err(invalid)?;
                signal(
                    state,
                    tenant,
                    buzz_pubsub::artifact_signal::ArtifactSignal {
                        d: env.id,
                        artifact_type: env.artifact_type,
                        cause: buzz_pubsub::artifact_signal::ArtifactChange::Changed,
                    },
                )
                .await;
            }
        }
        ArtifactOutcome::Duplicate => {}
        ArtifactOutcome::Conflict(reason) => {
            return Err(IngestError::CanvasConflict(format!("conflict: {reason}")))
        }
        ArtifactOutcome::Rejected(reason) => return Err(invalid(reason)),
    }
    Ok(())
}
/// Community moderation is discriminated by the signed scope, never by an
/// absent/nullable channel decoded from the target.
fn moderation_target(event: &Event, tenant: &TenantContext) -> Result<Vec<u8>, IngestError> {
    let mut community = None;
    let mut target = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(String::as_str) {
            Some("h") => return Err(invalid("mixed moderation scope")),
            Some("community") => {
                if parts.len() != 2 || community.is_some() {
                    return Err(invalid("exactly one community scope required"));
                }
                community = Some(artifact::canonical_uuid(&parts[1]).map_err(invalid)?);
            }
            Some("e") => {
                if parts.len() != 2 || target.is_some() {
                    return Err(invalid("exactly one target revision required"));
                }
                target = Some(artifact::event_id(&parts[1]).map_err(invalid)?);
            }
            _ => {}
        }
    }
    if community != Some(*tenant.community().as_uuid()) {
        return Err(invalid("community scope does not match request tenant"));
    }
    target.ok_or_else(|| invalid("one target revision required"))
}
/// Compact metadata reaches every node, including this one, through the same
/// confirmed Redis interest. Retry until success or shutdown; no content is fanned out.
async fn signal(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    signal: buzz_pubsub::artifact_signal::ArtifactSignal,
) {
    if state.pubsub.publish_artifact(tenant, &signal).await.is_ok() {
        return;
    }
    let state = Arc::clone(state);
    let tenant = tenant.clone();
    tokio::spawn(async move {
        let mut delay = std::time::Duration::from_millis(100);
        loop {
            tokio::select! {
                () = state.community_revalidator_cancel.cancelled() => return,
                () = tokio::time::sleep(delay) => {}
            }
            if state
                .pubsub
                .publish_artifact(&tenant, &signal)
                .await
                .is_ok()
            {
                return;
            }
            delay = (delay * 2).min(std::time::Duration::from_secs(5));
        }
    });
}
