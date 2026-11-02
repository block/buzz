//! Optional NIP-FI verified profiles. Errors here never change admission.

use crate::{nip_fi_config::CommunitySource, state::AppState};
use buzz_auth::VerifiedAssertion;
use buzz_core::{StoredEvent, TenantContext};
use buzz_db::verified_profiles::{ProfileAdmission, ProfilePolicy};
use std::sync::Arc;

/// Policies that may publish: none without a stable relay key or NIP-FI
/// enforcement, since a stale name is only harmless when every post needs a
/// fresh issuer assertion.
fn enabled_policies(state: &AppState) -> impl Iterator<Item = &ProfilePolicy> {
    let enabled = state.config.relay_private_key.is_some() && state.config.nip_fi.mode.enforces();
    state
        .config
        .nip_fi
        .profile_policies
        .iter()
        .filter(move |policy| enabled && policy.label.is_some())
}

/// Run only after a successful WebSocket admission; HTTP never calls this.
/// Agent tokens and missing or invalid claims publish nothing.
pub(crate) fn admitted(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    assertion: Option<&VerifiedAssertion>,
) {
    let Some(assertion) = assertion else {
        return;
    };
    let (Some(evidence), Some(key)) = (assertion.profile_assertion(), assertion.asserted_key())
    else {
        return;
    };
    if evidence.profile.is_none() {
        return;
    }
    let issuer = assertion.identity().issuer();
    let Some(policy) = enabled_policies(state).find(|p| p.issuer == issuer) else {
        return;
    };
    let state = Arc::clone(state);
    let tenant = tenant.clone();
    let evidence = evidence.clone();
    let policy = policy.clone();
    let pubkey = key.to_hex();
    let admitted_at = chrono::Utc::now();
    tokio::spawn(async move {
        let (Some(profile), Some(label)) = (evidence.profile.as_ref(), policy.label.as_deref())
        else {
            return;
        };
        let result = state
            .db
            .publish_verified_profile(
                ProfileAdmission {
                    community: tenant.community(),
                    issuer: &policy.issuer,
                    pubkey: &pubkey,
                    issued_at: evidence.issued_at,
                    admitted_at,
                    display_name: &profile.display_name,
                    username: &profile.username,
                    full_name: profile.full_name.as_deref(),
                    label,
                    refresh_seconds: policy.refresh_seconds,
                    membership_required: state.config.require_relay_membership,
                },
                &state.relay_keypair,
            )
            .await;
        // A failed publish is not retried; the next admission republishes.
        let event = match result {
            Ok(Some(event)) => event,
            Ok(None) => {
                // Unchanged, ineligible, or superseded by a newer verification.
                metrics::counter!("buzz_verified_profile_total", "outcome" => "not_written")
                    .increment(1);
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "verified profile publication failed");
                metrics::counter!("buzz_verified_profile_total", "outcome" => "publication_failed")
                    .increment(1);
                return;
            }
        };
        deliver(&state, &tenant, &event).await;
    });
}

/// Deliver once to local subscribers and other pods. Clients that miss it
/// read the stored event on their next fetch.
async fn deliver(state: &AppState, tenant: &TenantContext, event: &nostr::Event) {
    let community = tenant.community();
    state.mark_local_event(community, &event.id);
    let outcome = if state
        .pubsub
        .publish_event(tenant, buzz_pubsub::EventTopic::Global, event)
        .await
        .is_ok()
    {
        "published"
    } else {
        state
            .local_event_ids
            .invalidate(&(community, event.id.to_bytes()));
        "broadcast_failed"
    };
    let stored = StoredEvent::with_received_at(event.clone(), chrono::Utc::now(), None, true);
    crate::handlers::event::fan_out_event_to_local_subscribers(state, community, &stored).await;
    metrics::counter!("buzz_verified_profile_total", "outcome" => outcome).increment(1);
}

/// Community-authorized, stable-key capability. Failure omits discovery.
pub(crate) async fn discovery(
    state: &AppState,
    tenant: &TenantContext,
) -> Option<serde_json::Value> {
    let mut count = 0;
    for policy in enabled_policies(state) {
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
        if authorized {
            count += 1;
        }
    }
    (count > 0).then(|| serde_json::json!({"kind": 30382, "multiple_issuers": count > 1}))
}
