//! NIP-AR lifecycle entry point.
use super::ingest::{IngestAuth, IngestError, IngestResult};
use crate::state::AppState;
use buzz_core::artifact::ArtifactOp;
use buzz_core::{StoredEvent, TenantContext};
use buzz_db::artifact::ArtifactOutcome;
use nostr::Event;
use std::sync::Arc;

/// Accept a revision whose home channel already passed ingest's kind-9 write
/// gates. A move must also be writable in its source channel.
pub(crate) async fn accept(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let env = buzz_core::artifact::validate(event)
        .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    verify_revision_auth(event)?;
    let source = if env.op == ArtifactOp::Move {
        let source = state
            .db
            .artifact_home(tenant.community(), env.id)
            .await
            .map_err(|e| IngestError::Internal(format!("artifact storage: {e}")))?
            .ok_or_else(|| {
                IngestError::CanvasConflict("conflict: artifact head unavailable".into())
            })?;
        super::ingest::check_channel_write(tenant, state, auth, source)
            .await
            .map_err(IngestError::Rejected)?;
        Some(source)
    } else {
        None
    };
    let outcome = state
        .db
        .accept_artifact(
            tenant.community(),
            event,
            &env,
            source,
            &state.relay_keypair,
        )
        .await
        .map_err(|e| IngestError::Internal(format!("artifact storage: {e}")))?;
    match outcome {
        ArtifactOutcome::Accepted(events) => {
            for stored in &events {
                publish(state, tenant, stored).await;
            }
        }
        ArtifactOutcome::Duplicate => {}
        ArtifactOutcome::Conflict(message) => {
            return Err(IngestError::CanvasConflict(format!("conflict: {message}")))
        }
        ArtifactOutcome::Rejected(message) => {
            return Err(IngestError::Rejected(format!("invalid: {message}")))
        }
    }
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: String::new(),
    })
}

/// Live delivery only; unlike kind 9 there are no audit, workflow, or thread
/// side effects. Missed deliveries are recovered by replaying stored events.
async fn publish(state: &Arc<AppState>, tenant: &TenantContext, stored: &StoredEvent) {
    let Some(channel) = stored.channel_id else {
        return;
    };
    state.mark_local_event(tenant.community(), &stored.event.id);
    if let Err(e) = state
        .pubsub
        .publish_event(
            tenant,
            buzz_pubsub::EventTopic::Channel(channel),
            &stored.event,
        )
        .await
    {
        state
            .local_event_ids
            .invalidate(&(tenant.community(), stored.event.id.to_bytes()));
        tracing::warn!(error=%e, "artifact Redis publish failed");
    }
    super::event::fan_out_event_to_local_subscribers(state, tenant.community(), stored).await;
}

fn verify_revision_auth(event: &Event) -> Result<(), IngestError> {
    for tag in event.tags.iter().filter(|t| t.as_slice()[0] == "auth") {
        let json = serde_json::to_string(tag.as_slice())
            .map_err(|e| IngestError::Internal(e.to_string()))?;
        buzz_sdk::nip_oa::verify_auth_tag_for_auth_event(
            &json,
            &event.pubkey,
            event.created_at.as_secs(),
        )
        .map_err(|_| IngestError::Rejected("invalid: unverified artifact auth tag".into()))?;
        // Unlike connection admission, an attestation carried by a revision
        // must satisfy its kind clauses against that revision.
        if tag.as_slice()[2].split('&').any(|clause| {
            clause
                .strip_prefix("kind=")
                .is_some_and(|value| value.parse::<u16>().ok() != Some(event.kind.as_u16()))
        }) {
            return Err(IngestError::Rejected(
                "invalid: artifact auth kind mismatch".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    #[test]
    fn revision_auth_checks_every_condition_not_just_admission() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        for (conditions, accepted) in [
            ("", true),
            ("kind=45010", true),
            ("kind=9", false),
            ("kind=45010&kind=9", false),
            ("created_at<1", false),
        ] {
            let json = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), conditions)
                .unwrap();
            let tag = buzz_sdk::nip_oa::parse_auth_tag(&json).unwrap();
            let event = EventBuilder::new(Kind::Custom(45010), "")
                .tags([tag])
                .sign_with_keys(&agent)
                .unwrap();
            assert_eq!(
                verify_revision_auth(&event).is_ok(),
                accepted,
                "{conditions}"
            );
        }
    }
}
