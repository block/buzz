//! Republishing relay-signed channel discovery events (kinds 39000–39002)
//! that a failed post-commit emission left missing.
//!
//! The relay publishes a channel's discovery events after its write commits,
//! best-effort. If that fails, the channel and its members are saved but
//! clients cannot read its metadata, admins or roster, and a 1.x client
//! refuses to confirm a DM it cannot see the roster of. Two paths repair
//! that: reopening a DM checks the one DM it returns, and a startup sweep
//! checks recently created channels.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use chrono::{DateTime, Utc};
use tracing::{info, warn};
use uuid::Uuid;

use super::side_effects::{emit_group_discovery_events, ReconcileSummary};
use crate::state::AppState;

/// Channels the startup sweep reads per statement.
const SWEEP_PAGE: i64 = 200;

/// Channels created this recently are skipped: their creation may still be
/// publishing.
const IN_FLIGHT_GRACE: chrono::Duration = chrono::Duration::minutes(1);

/// Publish `channel_id`'s discovery events again if any of them is missing.
/// Returns whether it published. Healthy channels cost one indexed read and
/// no writes. `path` names the caller in the republish metric.
pub async fn republish_missing_discovery(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    path: &'static str,
) -> anyhow::Result<bool> {
    let relay_pubkey = state.relay_keypair.public_key().to_bytes();
    if !state
        .db
        .channel_missing_discovery(tenant.community(), channel_id, &relay_pubkey)
        .await?
    {
        return Ok(false);
    }
    emit_group_discovery_events(tenant, state, channel_id).await?;
    metrics::counter!("buzz_channel_discovery_republished_total", "path" => path).increment(1);
    info!(channel = %channel_id, path, "republished missing channel discovery events");
    Ok(true)
}

/// Republish discovery events for live channels in active communities that
/// were created in the `lookback` before now and lack any of them.
///
/// The scan reads [`SWEEP_PAGE`] channels per statement and moves past a
/// channel whose repair fails, so it ends after one pass over the window.
pub async fn reconcile_missing_channel_discovery(
    state: &Arc<AppState>,
    lookback: chrono::Duration,
) -> anyhow::Result<ReconcileSummary> {
    reconcile_missing_channel_discovery_paged(state, lookback, SWEEP_PAGE).await
}

/// [`reconcile_missing_channel_discovery`], reading `page_size` channels per
/// statement.
pub(crate) async fn reconcile_missing_channel_discovery_paged(
    state: &Arc<AppState>,
    lookback: chrono::Duration,
    page_size: i64,
) -> anyhow::Result<ReconcileSummary> {
    let relay_pubkey = state.relay_keypair.public_key().to_bytes();
    let now = Utc::now();
    let created_since = now - lookback;
    let created_before = now - IN_FLIGHT_GRACE;
    let mut cursor: Option<(DateTime<Utc>, Uuid)> = None;
    let mut summary = ReconcileSummary::default();

    loop {
        let page = state
            .db
            .list_channels_missing_discovery(
                &relay_pubkey,
                created_since,
                created_before,
                cursor,
                page_size,
            )
            .await?;
        for channel in &page {
            let tenant = TenantContext::resolved(channel.community_id, channel.host.clone());
            match emit_group_discovery_events(&tenant, state, channel.channel_id).await {
                Ok(()) => summary.repaired += 1,
                Err(error) => {
                    summary.failed += 1;
                    warn!(
                        community_id = %channel.community_id,
                        channel_id = %channel.channel_id,
                        %error,
                        "missing channel discovery repair failed"
                    );
                }
            }
        }
        match page.last() {
            Some(last) if page.len() as i64 == page_size => {
                cursor = Some((last.created_at, last.channel_id));
            }
            _ => break,
        }
    }

    metrics::counter!("buzz_channel_discovery_republished_total", "path" => "sweep")
        .increment(summary.repaired as u64);
    if summary.failed > 0 {
        metrics::counter!("buzz_channel_discovery_repair_failures_total")
            .increment(summary.failed as u64);
    }
    Ok(summary)
}
