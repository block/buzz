//! Relay-provisioned system channels.
//!
//! For each label in `BUZZ_PROVISIONED_SYSTEM_CHANNELS`, the relay keeps one
//! open system channel in every active community, created by the relay key.
//! Clients find it with `{kinds:[39000], #t:[label], #P:[relay pubkey]}`.
//! Other keys may not create or label a channel with a provisioned label, and
//! in these channels only an artifact's author may change it (NIP-AR).
//!
//! A community gets its channels when an operator provisions it, and a
//! periodic pass creates any that are missing, so a community created any
//! other way, or whose channel was archived, gets one within an interval. The
//! startup pass also repairs a channel whose relay-signed kind:39000 is
//! missing, as when its emission failed after the channel committed.

use std::sync::Arc;
use std::time::Duration;

use buzz_core::{CommunityId, TenantContext};
use tracing::{error, info, warn};

use crate::state::AppState;

/// Communities read per page of a reconciliation pass.
const PAGE: i64 = 100;

/// Whether a non-relay `pubkey` sets a provisioned label among `labels`; the
/// first such label.
pub(crate) fn reserved_label<'a>(
    state: &AppState,
    pubkey: &nostr::PublicKey,
    labels: &'a [String],
) -> Option<&'a str> {
    if *pubkey == state.relay_keypair.public_key() {
        return None;
    }
    labels
        .iter()
        .find(|label| state.config.provisioned_system_channels.contains(label))
        .map(String::as_str)
}

/// Ensure every provisioned channel exists in `tenant`'s community, and emit
/// its group-state events when it was just created or has no kind:39000.
pub async fn ensure_for_community(
    state: &Arc<AppState>,
    tenant: &TenantContext,
) -> anyhow::Result<()> {
    let relay = state.relay_keypair.public_key().to_bytes();
    for label in &state.config.provisioned_system_channels {
        let provisioned = state
            .db
            .ensure_provisioned_system_channel(tenant.community(), label, &relay)
            .await?;
        if provisioned.created {
            state.invalidate_membership(tenant, provisioned.channel.id, &relay);
            state.invalidate_all_accessible_channels(tenant);
            metrics::counter!("buzz_provisioned_channels_created_total").increment(1);
            info!(
                community = %tenant.community(),
                channel = %provisioned.channel.id,
                label = %label,
                "provisioned system channel created"
            );
        }
        if provisioned.created || !provisioned.has_metadata {
            super::side_effects::emit_group_discovery_events(tenant, state, provisioned.channel.id)
                .await?;
        }
    }
    Ok(())
}

/// One pass over every community missing a provisioned channel. With
/// `require_metadata`, a channel without a relay-signed kind:39000 counts as
/// missing. Returns how many communities failed.
async fn reconcile(state: &Arc<AppState>, require_metadata: bool) -> usize {
    let relay = state.relay_keypair.public_key().to_bytes();
    let mut failed = 0;
    for label in &state.config.provisioned_system_channels {
        let mut after: Option<CommunityId> = None;
        loop {
            let page = match state
                .db
                .communities_missing_provisioned_channel(
                    label,
                    &relay,
                    require_metadata,
                    after,
                    PAGE,
                )
                .await
            {
                Ok(page) => page,
                Err(e) => {
                    error!(label = %label, error = %e, "provisioned channel pass failed to list communities");
                    failed += 1;
                    break;
                }
            };
            let full = i64::try_from(page.len()).is_ok_and(|len| len == PAGE);
            for (community, host) in page {
                after = Some(community);
                let tenant = TenantContext::resolved(community, host);
                if let Err(e) = ensure_for_community(state, &tenant).await {
                    failed += 1;
                    metrics::counter!("buzz_provisioned_channels_failures_total").increment(1);
                    warn!(community = %community, error = %e, "provisioned channel ensure failed; next pass retries");
                }
            }
            if !full {
                break;
            }
        }
    }
    failed
}

/// Start the reconciler: a startup pass that also repairs missing group
/// state, then a channel-only pass every configured interval with up to 20%
/// jitter, so pods don't pass in lockstep.
pub fn spawn_reconciler(state: Arc<AppState>) {
    if state.config.provisioned_system_channels.is_empty() {
        return;
    }
    let interval = state.config.provisioned_system_channels_interval_secs;
    tokio::spawn(async move {
        info!(
            labels = ?state.config.provisioned_system_channels,
            interval_secs = interval,
            "provisioned system channel reconciler started"
        );
        let mut require_metadata = true;
        loop {
            let failed = reconcile(&state, require_metadata).await;
            require_metadata = require_metadata && failed > 0;
            let jitter = rand::random::<u64>() % (interval / 5 + 1);
            tokio::time::sleep(Duration::from_secs(interval + jitter)).await;
        }
    });
}
