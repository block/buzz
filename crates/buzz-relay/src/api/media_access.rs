//! File-share rollout and the audited moderation exception.

use std::sync::Arc;

use buzz_audit::{AuditAction, NewAuditEntry};
use buzz_core::TenantContext;
use buzz_media::MediaError;

use crate::config::MediaReadMode;
use crate::handlers::moderation_authz::{
    authorize_moderation_action, ModerationAction, ModerationTarget,
};
use crate::state::AppState;

enum Access {
    Shared,
    Report(uuid::Uuid, Vec<u8>),
    Denied,
}

#[cfg(test)]
#[path = "media_access_postgres_tests.rs"]
mod postgres_tests;

/// Called only after authentication and community admission, for GET and HEAD.
pub(super) async fn enforce(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &[u8],
    sha256: &str,
) -> Result<(), MediaError> {
    let mode = state.config.media_read_mode;
    if mode == MediaReadMode::Off {
        return Ok(());
    }
    let mut decision = evaluate(state, tenant, actor, sha256).await;
    if mode == MediaReadMode::Enforce {
        if let Ok(Access::Report(report, event)) = &decision {
            if let Err(error) = audit_access(state, tenant, actor, sha256, *report, event).await {
                decision = Err(error);
            }
        }
    }
    let outcome = match &decision {
        Ok(Access::Shared) => "allow",
        Ok(Access::Report(..)) => "report",
        Ok(Access::Denied) => "deny",
        Err(_) => "error",
    };
    metrics::counter!("buzz_media_read_access_total", "mode" => mode.as_str(), "outcome" => outcome)
        .increment(1);
    if mode == MediaReadMode::LogOnly {
        // No identifiers in metrics; no report grant or audit write in shadow.
        return Ok(());
    }
    match decision? {
        Access::Shared | Access::Report(..) => Ok(()),
        Access::Denied => Err(MediaError::NotFound),
    }
}

async fn audit_access(
    state: &AppState,
    tenant: &TenantContext,
    actor: &[u8],
    sha256: &str,
    report: uuid::Uuid,
    event: &[u8],
) -> Result<(), MediaError> {
    let audit = state
        .audit
        .as_ref()
        .ok_or(MediaError::AuthorizationUnavailable)?;
    // Await durable audit, rather than the best-effort upload queue.
    audit
        .log(NewAuditEntry {
            community_id: tenant.community(),
            action: AuditAction::ModerationMediaRead,
            actor_pubkey: Some(actor.to_vec()),
            object_id: Some(sha256.to_owned()),
            detail: serde_json::json!({"report_id": report, "event_id": hex::encode(event)}),
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "Could not audit moderator media access");
            MediaError::AuthorizationUnavailable
        })?;
    Ok(())
}

async fn evaluate(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &[u8],
    sha256: &str,
) -> Result<Access, MediaError> {
    evaluate_inner(state, tenant, actor, sha256)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Media read authorization lookup failed");
            MediaError::AuthorizationUnavailable
        })
}

async fn evaluate_inner(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &[u8],
    sha256: &str,
) -> anyhow::Result<Access> {
    if state
        .db
        .can_read_blob(tenant.community(), sha256, actor)
        .await?
    {
        return Ok(Access::Shared);
    }
    let Some((report, event)) = state
        .db
        .open_media_report(tenant.community(), sha256)
        .await?
    else {
        return Ok(Access::Denied);
    };
    match authorize_moderation_action(
        tenant,
        state,
        actor,
        None,
        ModerationTarget::None,
        ModerationAction::ViewQueue,
    )
    .await
    {
        Ok(_) => {}
        Err(error) if error.downcast_ref::<buzz_db::DbError>().is_some() => return Err(error),
        Err(_) => return Ok(Access::Denied),
    }
    anyhow::ensure!(
        state.audit.is_some(),
        "moderator media access requires durable auditing"
    );
    Ok(Access::Report(report, event))
}

#[cfg(test)]
mod tests {
    use crate::config::MediaReadMode;

    #[test]
    fn rollout_modes_default_to_log_only_and_reject_unknown_values() {
        assert_eq!(MediaReadMode::default(), MediaReadMode::LogOnly);
        for (value, mode) in [
            ("", MediaReadMode::LogOnly),
            ("log-only", MediaReadMode::LogOnly),
            ("off", MediaReadMode::Off),
            ("enforce", MediaReadMode::Enforce),
        ] {
            assert_eq!(value.parse::<MediaReadMode>().unwrap(), mode);
        }
        for value in ["disabled", "Enforce", "log_only", "enforc"] {
            assert!(value.parse::<MediaReadMode>().is_err());
        }
    }
}
