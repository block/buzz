//! Optional NIP-FI projection lifecycle. Errors here never change admission.

use crate::{nip_fi_config::CommunitySource, state::AppState};
use buzz_auth::VerifiedAssertion;
use buzz_core::{CommunityId, StoredEvent, TenantContext};
use buzz_db::verified_profiles::{ProfileAdmission, ProfileEpoch, ProfilePolicy};
use std::sync::Arc;

fn policies(state: &AppState) -> Vec<ProfilePolicy> {
    let mut policies = state.config.nip_fi.profile_policies.clone();
    if state.config.relay_private_key.is_none() || !state.config.nip_fi.mode.enforces() {
        for policy in &mut policies {
            policy.label = None;
        }
    }
    policies
}

/// Capture before admission completes so delayed publication cannot cross a
/// removal/restoration. Dependency failure suppresses projection work only.
pub(crate) async fn capture(
    state: &AppState,
    community: CommunityId,
    assertion: Option<&VerifiedAssertion>,
) -> Option<ProfileEpoch> {
    let assertion = assertion?;
    assertion.profile_assertion()?;
    let policy = policies(state)
        .into_iter()
        .find(|p| p.issuer == assertion.identity().issuer() && p.label.is_some())?;
    match tokio::time::timeout(
        std::time::Duration::from_millis(200),
        state
            .db
            .profile_epoch(community, &assertion.asserted_key()?.to_hex(), &policy),
    )
    .await
    {
        Ok(Ok(epoch)) => epoch,
        _ => {
            metrics::counter!("buzz_verified_profile_total", "outcome" => "capture_failed")
                .increment(1);
            None
        }
    }
}

/// Run only after a successful WebSocket admission; HTTP never calls this.
pub(crate) fn admitted(
    state: &Arc<AppState>,
    community: CommunityId,
    assertion: Option<&VerifiedAssertion>,
    epoch: Option<ProfileEpoch>,
    control: &crate::state::CommunityConnectionControl,
) {
    let (Some(assertion), Some(epoch)) = (assertion, epoch) else {
        return;
    };
    if assertion.profile_assertion().is_none() || assertion.asserted_key().is_none() {
        return;
    }
    let state = Arc::clone(state);
    let assertion = assertion.clone();
    let control = control.clone();
    let admitted_at = chrono::Utc::now();
    tokio::spawn(async move {
        let (Some(evidence), Some(key)) = (assertion.profile_assertion(), assertion.asserted_key())
        else {
            return;
        };
        let pubkey = key.to_hex();
        let mut attempts = 0;
        let affected_event = loop {
            attempts += 1;
            let result = state
                .db
                .apply_profile_admission(
                    ProfileAdmission {
                        community,
                        issuer: assertion.identity().issuer(),
                        pubkey: &pubkey,
                        issued_at: evidence.issued_at,
                        admitted_at,
                        profile: evidence.profile.as_ref().map(|p| {
                            (
                                p.display_name.as_str(),
                                p.username.as_str(),
                                p.full_name.as_deref(),
                            )
                        }),
                        epoch,
                        database_binding: state.config.nip_fi.source == CommunitySource::Database,
                        membership_required: state.config.require_relay_membership,
                    },
                    &state.relay_keypair,
                )
                .await;
            metrics::counter!("buzz_verified_profile_total", "outcome" => if result.is_ok() { "processed" } else { "publication_failed" }).increment(1);
            if let Ok(event_id) = result {
                break event_id;
            }
            // Optional publication and cleanup never hold admission open.
            if evidence.profile.is_some() || attempts >= 3 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(attempts * attempts)).await;
        };
        let Some(event_id) = affected_event else {
            return;
        };
        let cancellation = if let Some(triggered_at) = control.profile_disconnected_at() {
            Some((triggered_at, None))
        } else if state.nip_fi_deny_map.as_deref().is_some_and(|map| {
            crate::nip_fi_core::assertion_denied(map, &assertion, chrono::Utc::now())
        }) {
            // The socket may already have left the close-scan registry. A deny
            // entry supplies no command timestamp: cancel only this observed
            // event, never widen the command's boundary to the time of this check.
            Some((admitted_at.timestamp(), Some(event_id.as_str())))
        } else {
            None
        };
        if let Some((triggered_at, event_id)) = cancellation {
            retract(
                &state,
                community,
                assertion.identity().issuer(),
                &pubkey,
                triggered_at,
                event_id,
            )
            .await;
        }
    });
}

/// Start bounded optional housekeeping. It never gates normal service or access
/// control and checks the stored version against current eligibility.
pub fn start(state: Arc<AppState>) {
    if state.config.nip_fi.mode.observes_only() {
        return;
    }
    tokio::spawn(async move {
        let mut configured = false;
        let mut cursor = None;
        let mut delay = 1;
        loop {
            tokio::select! {
                _ = state.push_cancel.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
            }
            let configuration = async {
                if !configured {
                    let revision = std::env::var("BUZZ_NIP_FI_PROFILE_POLICY_REVISION")
                        .unwrap_or_else(|_| "0".into())
                        .parse::<i64>()?;
                    if revision < 0 {
                        anyhow::bail!("invalid profile policy revision");
                    }
                    state
                        .db
                        .configure_profile_policies(&policies(&state), revision)
                        .await?;
                    configured = true;
                }
                Ok::<(), anyhow::Error>(())
            }
            .await;
            // Recovery uses durable policy even when this process has a stale
            // configuration. Reconciliation failure must never stop retractions.
            let recovery = process_pending(&state, &mut cursor).await;
            let result = configuration.and(recovery);
            if result.is_err() {
                metrics::counter!("buzz_verified_profile_total", "outcome" => "worker_failed")
                    .increment(1);
                delay = (delay * 2).min(60);
            } else {
                delay = 1;
            }
        }
    });
}

async fn process_pending(
    state: &Arc<AppState>,
    cursor: &mut Option<(CommunityId, String)>,
) -> anyhow::Result<()> {
    if state.config.relay_private_key.is_none() {
        return Ok(());
    }
    let pending = state.db.pending_profile_work(cursor.clone()).await?;
    *cursor = pending.last().cloned();
    metrics::gauge!("buzz_verified_profile_pending_batch").set(pending.len() as f64);
    // A failure on one subject does not starve the rest of this bounded batch.
    let mut failed = false;
    for (community, pubkey) in pending {
        let result = async {
            let Some(event) = state
                .db
                .process_profile_work(
                    community,
                    &pubkey,
                    &state.relay_keypair,
                    state.config.require_relay_membership,
                )
                .await?
            else {
                return Ok::<(), anyhow::Error>(());
            };
            let host = state
                .db
                .lookup_community_host(community)
                .await?
                .ok_or_else(|| anyhow::anyhow!("profile community unavailable"))?;
            let tenant = TenantContext::resolved(community, host);
            state.mark_local_event(community, &event.id);
            if let Err(error) = state
                .pubsub
                .publish_event(&tenant, buzz_pubsub::EventTopic::Global, &event)
                .await
            {
                state
                    .local_event_ids
                    .invalidate(&(community, event.id.to_bytes()));
                return Err(error.into());
            }
            let stored =
                StoredEvent::with_received_at(event.clone(), chrono::Utc::now(), None, true);
            crate::handlers::event::fan_out_event_to_local_subscribers(state, community, &stored)
                .await;
            state
                .db
                .acknowledge_profile_broadcast(community, &pubkey, &event.id.to_hex())
                .await?;
            metrics::counter!("buzz_verified_profile_total", "outcome" => "broadcast").increment(1);
            Ok(())
        }
        .await;
        failed |= result.is_err();
    }
    if failed {
        anyhow::bail!("profile delivery pending");
    }
    Ok(())
}

/// Community-authorized, stable-key capability. Failure omits discovery.
pub(crate) async fn discovery(
    state: &AppState,
    tenant: &TenantContext,
) -> Option<serde_json::Value> {
    let mut count = 0;
    for policy in policies(state).into_iter().filter(|p| p.label.is_some()) {
        let authorized = match state.config.nip_fi.source {
            CommunitySource::Env => state
                .config
                .nip_fi
                .communities
                .resolve(tenant.host())
                .is_some_and(|binding| binding.authorizes(&policy.issuer)),
            CommunitySource::Database => state
                .db
                .lookup_nip_fi_binding_by_host(tenant.host())
                .await
                .ok()
                .flatten()
                .is_some_and(|b| b.active && b.issuers.contains(&policy.issuer)),
        };
        if authorized
            && state
                .db
                .profile_epoch(tenant.community(), "", &policy)
                .await
                .ok()
                .flatten()
                .is_some()
        {
            count += 1;
        }
    }
    (count > 0).then(|| serde_json::json!({"kind": 30382, "multiple_issuers": count > 1}))
}

async fn retract(
    state: &AppState,
    community: CommunityId,
    issuer: &str,
    pubkey: &str,
    triggered_at: i64,
    event_id: Option<&str>,
) {
    for delay in [0, 1, 4] {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        let result = state
            .db
            .retract_profile_admission(community, issuer, pubkey, triggered_at, event_id)
            .await;
        metrics::counter!("buzz_verified_profile_total", "outcome" => if result.is_ok() { "cleanup_scheduled" } else { "cleanup_failed" }).increment(1);
        if result.is_ok() {
            break;
        }
    }
}

/// Every accepted command initiates cleanup after deny insertion, close scans,
/// and propagation dispatch. Never await profile work on the response path.
pub(crate) fn disconnected(state: &AppState, issuer: &str, pubkey: &str, triggered_at: i64) {
    state
        .db
        .schedule_profile_cleanup(issuer, pubkey, triggered_at);
}
