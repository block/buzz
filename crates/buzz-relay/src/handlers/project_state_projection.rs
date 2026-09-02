//! Relay signing and durable repair for NIP-PC Project State projections.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use buzz_core::event::StoredEvent;
use buzz_core::kind::KIND_PROJECT_STATE;
use buzz_core::tenant::TenantContext;
use buzz_db::project_state::{ProjectStateProjectionCandidate, ProjectStateProjectionCommitResult};
use nostr::{Event, EventBuilder, Timestamp};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

use super::event::dispatch_persistent_event;

const RECONCILE_BATCH_SIZE: i64 = 100;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);

fn projection_created_at(now: u64, previous: Option<u64>) -> anyhow::Result<u64> {
    let after_previous = previous
        .map(|value| {
            value
                .checked_add(1)
                .ok_or_else(|| anyhow!("Project projection timestamp overflow"))
        })
        .transpose()?;
    Ok(after_previous.map_or(now, |value| now.max(value)))
}

fn sign_candidate(
    candidate: &ProjectStateProjectionCandidate,
    relay_keypair: &nostr::Keys,
) -> anyhow::Result<Event> {
    let template = candidate.template();
    let created_at =
        projection_created_at(Timestamp::now().as_secs(), candidate.previous_created_at())?;
    EventBuilder::new(template.kind, template.content.clone())
        .tags(template.tags.clone())
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(relay_keypair)
        .context("sign Project State projection")
}

async fn publish_candidate(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    candidate: ProjectStateProjectionCandidate,
) -> anyhow::Result<bool> {
    if candidate.community_id() != tenant.community() {
        return Err(anyhow!("Project projection candidate crossed communities"));
    }
    let event = sign_candidate(&candidate, &state.relay_keypair)?;
    let result = state
        .db
        .commit_project_state_projection(&candidate, &event)
        .await
        .context("commit Project State projection")?;
    if result == ProjectStateProjectionCommitResult::Stale {
        return Ok(false);
    }

    let stored = StoredEvent::new(event, None);
    let relay_pubkey = state.relay_keypair.public_key().to_hex();
    dispatch_persistent_event(
        tenant,
        state,
        &stored,
        KIND_PROJECT_STATE,
        &relay_pubkey,
        None,
    )
    .await;
    Ok(true)
}

/// Publish the current projection for one accepted Project lifecycle event, if pending.
pub(crate) async fn publish_project_state_for_coordinate(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    project_owner: &[u8],
    project_d_tag: &str,
) -> anyhow::Result<bool> {
    let relay_pubkey = state.relay_keypair.public_key().to_bytes();
    let Some(candidate) = state
        .db
        .load_pending_project_state_projection(
            tenant.community(),
            project_owner,
            project_d_tag,
            &relay_pubkey,
        )
        .await
        .context("load pending Project State projection")?
    else {
        return Ok(false);
    };
    publish_candidate(tenant, state, candidate).await
}

/// Repair a bounded batch of durable Project State publication markers.
pub async fn reconcile_project_state_projections(state: &Arc<AppState>) -> anyhow::Result<usize> {
    let relay_pubkey = state.relay_keypair.public_key().to_bytes();
    let candidates = state
        .db
        .load_pending_project_state_projections(&relay_pubkey, RECONCILE_BATCH_SIZE)
        .await
        .context("load pending Project State projections")?;
    let mut committed = 0;
    for candidate in candidates {
        let community_id = candidate.community_id();
        let result = async {
            let host = state
                .db
                .lookup_community_host(community_id)
                .await?
                .ok_or_else(|| anyhow!("Project projection community has no active host"))?;
            let tenant = TenantContext::resolved(community_id, host);
            publish_candidate(&tenant, state, candidate).await
        }
        .await;
        match result {
            Ok(true) => committed += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%community_id, %error, "Project State projection repair failed")
            }
        }
    }
    Ok(committed)
}

/// Run periodic bounded repair until graceful shutdown is requested.
pub fn spawn_project_state_projection_reconciler(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match reconcile_project_state_projections(&state).await {
            Ok(count) if count > 0 => {
                tracing::info!(count, "Project State projections repaired on startup")
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "Project State startup reconciliation failed")
            }
        }

        let mut interval = tokio::time::interval(RECONCILE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    match reconcile_project_state_projections(&state).await {
                        Ok(count) if count > 0 => {
                            tracing::info!(count, "Project State projections repaired")
                        }
                        Ok(_) => {}
                        Err(error) => tracing::warn!(%error, "Project State projection reconciliation failed"),
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_timestamp_is_strictly_monotonic() {
        assert_eq!(projection_created_at(100, None).expect("timestamp"), 100);
        assert_eq!(
            projection_created_at(100, Some(50)).expect("timestamp"),
            100
        );
        assert_eq!(
            projection_created_at(100, Some(100)).expect("timestamp"),
            101
        );
        assert_eq!(
            projection_created_at(100, Some(200)).expect("timestamp"),
            201
        );
    }

    #[test]
    fn projection_timestamp_rejects_overflow() {
        assert!(projection_created_at(100, Some(u64::MAX)).is_err());
    }
}
