//! Direct governance commands: one signed outcome and audit-chain append.
use super::ingest::IngestError;
use crate::state::AppState;
use buzz_audit::{AuditAction, AuditService, NewAuditEntry};
use buzz_core::{governance::Command, TenantContext};
use nostr::Event;

fn db_error(error: buzz_db::DbError) -> IngestError {
    match error {
        buzz_db::DbError::AccessDenied(message) => IngestError::AuthFailed(message),
        buzz_db::DbError::InvalidData(message) => IngestError::Rejected(message),
        error => {
            tracing::error!(%error, "governance transaction failed");
            IngestError::Internal("error: governance unavailable".into())
        }
    }
}

/// The Git grant an event carries, as a Git door proves one: a tag that
/// verifies as neither kind supplies no grant and no owner, and the signer's
/// own membership still decides.
async fn grant(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
) -> Result<Option<buzz_core::governance::GitGrant>, IngestError> {
    use crate::api::relay_members::{link_agent_owner, OwnerLink, OWNER_MISMATCH};
    let (owner, grant) = crate::api::relay_members::git_owner(
        &event.pubkey,
        super::auth::extract_auth_tag_json(event).as_deref(),
        Some(event.created_at.as_secs()),
        None,
    )
    .map_err(|refusal| IngestError::AuthFailed(refusal.into()))?;
    // Callers validate the signed community first. As at the Git door, a
    // verified first owner is recorded before repository authorization and
    // remains recorded even if that command or evidence is later refused.
    // Otherwise a refused agent approval could leave the key looking human.
    if let Some(owner) = owner {
        match link_agent_owner(state, tenant, &event.pubkey, &owner).await {
            OwnerLink::Linked => {}
            OwnerLink::Conflict => return Err(IngestError::AuthFailed(OWNER_MISMATCH.into())),
            OwnerLink::Unavailable => {
                return Err(IngestError::Internal(
                    "error: owner link unavailable".into(),
                ))
            }
        }
    }
    Ok(grant)
}

pub(super) async fn apply(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
) -> Result<(), IngestError> {
    let command = Command::parse(event).map_err(IngestError::Rejected)?;
    if command.community != *tenant.community().as_uuid() {
        return Err(IngestError::AuthFailed(
            "restricted: command community mismatch".into(),
        ));
    }
    let grant = grant(tenant, state, event).await?;
    let mut tx = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .map_err(db_error)?;
    buzz_db::governance::fence_authority_write(&mut tx, &command.document)
        .await
        .map_err(db_error)?;
    let prepared = state
        .db
        .prepare_governance_command(
            &mut tx,
            event,
            grant.as_ref(),
            &state.relay_keypair,
            !state.config.require_relay_membership,
        )
        .await
        .map_err(db_error)?;
    if !prepared.replay {
        AuditService::log_in_transaction_before(&mut tx, NewAuditEntry {
            community_id: tenant.community(),
            action: AuditAction::GovernanceChanged,
            actor_pubkey: Some(event.pubkey.to_bytes().to_vec()),
            object_id: Some(event.id.to_hex()),
            detail: serde_json::json!({"document": command.document.to_string(), "command_id": event.id.to_hex(), "document_body": command.body, "previous": prepared.previous, "state": prepared.state}),
        }, prepared.valid_until).await.map_err(|error| match error {
            buzz_audit::AuditError::Admission(error) => db_error(error),
            error => {
                tracing::error!(%error, "governance audit failed");
                IngestError::Internal("error: governance unavailable".into())
            }
        })?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(())
}

/// Evidence is private audit data, never an ordinary stored/fanned-out event.
pub(super) async fn evidence(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
) -> Result<(), IngestError> {
    let evidence = buzz_core::governance::Evidence::parse(event).map_err(IngestError::Rejected)?;
    if evidence.community != *tenant.community().as_uuid() {
        return Err(IngestError::AuthFailed(
            "restricted: evidence community mismatch".into(),
        ));
    }
    let grant = grant(tenant, state, event).await?;
    let mut tx = state
        .db
        .begin_event_write_transaction(tenant.community())
        .await
        .map_err(db_error)?;
    let (evidence, replay) = buzz_db::git_evidence::prepare(
        &mut tx,
        event,
        grant.as_ref(),
        !state.config.require_relay_membership,
    )
    .await
    .map_err(db_error)?;
    if replay {
        tx.rollback().await.map_err(db_error)?;
        return Ok(());
    }
    AuditService::log_in_transaction(
        &mut tx,
        NewAuditEntry {
            community_id: tenant.community(),
            action: AuditAction::GitEvidence,
            actor_pubkey: Some(event.pubkey.to_bytes().to_vec()),
            object_id: Some(event.id.to_hex()),
            detail: serde_json::json!({"evidence": evidence, "event_id": event.id.to_hex()}),
        },
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "evidence audit failed");
        IngestError::Internal("error: evidence unavailable".into())
    })?;
    tx.commit().await.map_err(db_error)?;
    Ok(())
}
