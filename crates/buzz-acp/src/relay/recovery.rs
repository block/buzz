//! Overflow recovery is an attempted replay, not an EOSE/consumer receipt.
//! Keep the existing IDs and cursor retirement rules; bound when work is sent.
use super::*;

pub(super) const RECOVERY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct RecoverySchedule {
    next_attempt: Option<tokio::time::Instant>,
    pub(super) last_attempt: HashMap<String, tokio::time::Instant>,
}

/// Attempt at most one affected subscription, with space for replay to arrive.
/// Failed writes retain the loss cursor and are paced too. No EOSE is interpreted
/// as completion: overlapping requests keep their existing stable wire IDs.
pub(super) async fn recover_one(
    ws: &mut WsStream,
    state: &mut BgState,
    event_tx: &mpsc::Sender<Option<BuzzEvent>>,
    agent_pubkey_hex: &str,
) {
    let now = tokio::time::Instant::now();
    if event_tx.is_closed()
        || event_tx.capacity() < event_tx.max_capacity().div_ceil(2)
        || state.recovery.next_attempt.is_some_and(|next| now < next)
        || state.check_rate_gate().is_some()
    {
        return;
    }

    // One record per active intent, not per loss or per request generation.
    // Least recently attempted prevents a repeatedly overflowing channel from
    // starving other channels or membership. Missing filters fail closed.
    let channel = state
        .channel_dropped_since
        .keys()
        .filter(|ch| {
            state.active_subscriptions.contains_key(ch)
                && state.active_filters.contains_key(ch)
                && !state.rate_limited_pending.contains_key(ch)
                && !state.resubscribe_retry.contains(ch)
        })
        .copied()
        .map(Some)
        .chain(
            (state.membership_sub_active
                && state.membership_dropped_since.is_some()
                && !state.membership_resub_needed)
                .then_some(None),
        )
        .min_by_key(|ch| {
            let sub = ch.map_or_else(|| MEMBERSHIP_NOTIF_SUB_ID.to_owned(), channel_sub_id);
            (state.recovery.last_attempt.get(&sub).copied(), sub)
        });
    let Some(channel) = channel else { return };
    let sub = channel.map_or_else(|| MEMBERSHIP_NOTIF_SUB_ID.to_owned(), channel_sub_id);
    state.recovery.last_attempt.insert(sub.clone(), now);
    info!(subscription = sub, "attempting targeted overflow replay");

    if let Some(ch) = channel {
        if let Some(filter) = state.active_filters.get(&ch).cloned() {
            let since = state.channel_since(&ch);
            if send_subscribe(ws, state, ch, agent_pubkey_hex, since, &filter).await {
                // Baseline retirement point: REQ write, NOT proven delivery.
                // New overflow after this attempt creates another pending cursor.
                state.channel_dropped_since.remove(&ch);
            }
        }
    } else {
        let since = match (state.membership_dropped_since, state.membership_last_seen) {
            (Some(d), Some(l)) => Some(d.min(l)),
            (Some(d), None) => Some(d),
            (None, Some(l)) => Some(l),
            (None, None) => state.startup_watermark,
        };
        if send_membership_subscribe(ws, agent_pubkey_hex, since).await {
            state.membership_dropped_since = None;
        }
    }
    // Pace from the end of a potentially backpressured write. No catch-up burst.
    // The existing bounded write timeout and read/ping owner detect socket loss.
    state.recovery.next_attempt = Some(tokio::time::Instant::now() + RECOVERY_INTERVAL);
}
