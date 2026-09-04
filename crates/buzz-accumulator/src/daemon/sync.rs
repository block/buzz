//! Relay synchronization: discovery → backfill → live tail, with reconnect.
//!
//! Protocol constraints honored here (all verified against the relay source):
//! - One REQ, one sub id, one `#h`, one channel — multi-`#h` unions degrade
//!   the subscription to global scope and let unrelated channels eat the
//!   LIMIT.
//! - WS REQs have no `before_id` tiebreak, so backfill pages on `until` alone
//!   with the cursor set to the OLDEST timestamp of the page (not `-1`) and
//!   dedupes by event id; a page that makes no progress decrements the cursor
//!   by one second as a last resort.
//! - REQ frames are paced (125ms) to stay under the relay's burst admission
//!   window on multi-channel backfills and reconnects.
//! - Archived channels are dropped at discovery: re-offering one draws a
//!   `CLOSED restricted` loop.
//! - The relay acks a client CLOSE with an empty-message CLOSED. Every
//!   one-shot REQ therefore gets a connection-unique sub id (a late ack for a
//!   reused id would kill the next request), and the live loop ignores CLOSED
//!   frames for subscriptions it does not own. Observed live 2026-08-31.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use buzz_ws_client::{build_auth_event, NostrWsConnection, RelayMessage};
use nostr::{Keys, Tag};
use serde_json::{json, Value};
use tracing::{info, warn};

use super::status::StatusRegistry;
use super::store::{Store, StoredEvent};

/// Events per backfill page (relay clamps to 1000).
const PAGE_LIMIT: usize = 500;
/// Pause between REQ frames — stays well under the relay burst budget.
const REQ_PACING: Duration = Duration::from_millis(125);
/// Deadline for a single backfill page (REQ → EOSE).
const PAGE_TIMEOUT: Duration = Duration::from_secs(60);
/// Idle tick for the live loop; timeouts are quiet periods, not errors.
const LIVE_TICK: Duration = Duration::from_secs(30);
/// Overlap subtracted from `since` on every (re)subscribe, covering the gap
/// between the newest stored event and the moment the subscription opens.
const SINCE_SKEW_SECS: i64 = 5;
/// Reconnect backoff ceiling.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Exact CLOSED reasons that mean "drop this channel, keep the socket".
/// Never match on a `restricted:` prefix — that would swallow connection-level
/// scope failures where reconnecting is the right move.
const CHANNEL_ACCESS_DENIED: &[&str] = &[
    "restricted: not a channel member",
    "restricted: channel access revoked",
];

/// Sync loop configuration.
pub struct SyncConfig {
    /// Relay websocket URL.
    pub relay_url: String,
    /// Identity to authenticate and read as.
    pub keys: Keys,
    /// Optional NIP-OA ownership tag (agent identities only).
    pub auth_tag: Option<Tag>,
    /// Nudged by the HTTP layer (e.g. an exclusion toggle) to request an
    /// immediate clean resync instead of waiting for relay traffic.
    pub resync: Arc<Notify>,
}

/// Runs the sync loop forever, reconnecting with capped exponential backoff.
pub async fn run_sync(cfg: SyncConfig, store: Store, registry: StatusRegistry) {
    let mut attempt: u32 = 0;
    loop {
        registry.connecting();
        match sync_once(&cfg, &store, &registry).await {
            Ok(()) => {
                // sync_once only returns to request a clean resync
                // (e.g. membership changed); reconnect without backoff.
                attempt = 0;
            }
            Err(e) => {
                attempt = attempt.saturating_add(1);
                registry.backoff(&e.to_string());
                let delay = backoff_delay(attempt);
                warn!(error = %e, attempt, ?delay, "connection lost; backing off");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Capped exponential backoff: 1s, 2s, 4s, … 60s.
pub(crate) fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.saturating_sub(1).min(6);
    Duration::from_secs(secs).min(BACKOFF_MAX)
}

/// One connection lifetime: connect, auth, discover, backfill, tail.
///
/// `Ok(())` requests an immediate resync; `Err` requests backoff + reconnect.
async fn sync_once(
    cfg: &SyncConfig,
    store: &Store,
    registry: &StatusRegistry,
) -> anyhow::Result<()> {
    let mut conn =
        NostrWsConnection::connect_authenticated(&cfg.relay_url, &cfg.keys, cfg.auth_tag.as_ref())
            .await?;
    let now = chrono::Utc::now().timestamp();
    registry.connected(now);
    info!(relay = %cfg.relay_url, "connected and authenticated");

    // Absorb any resync nudge issued before this point: its store write
    // happened before the notify, so the reads below already reflect it. A
    // nudge arriving after this line stores a permit the live loop picks up.
    let _ = tokio::time::timeout(Duration::ZERO, cfg.resync.notified()).await;

    discover_channels(&mut conn, cfg, store, registry).await?;
    // Discovery just read the authoritative rosters, so membership is known
    // current as of `now` (captured before the roster query). Advancing the
    // watermark here is what lets the live-membership subscription anchor at
    // it instead of "now": a 44100/44101 emitted while this connection was
    // still backfilling — or while the daemon was disconnected — replays on
    // subscribe and triggers the resync it would otherwise have missed.
    store
        .set_meta(MEMBERSHIP_WATERMARK_KEY, &now.to_string())
        .await?;
    backfill(&mut conn, store, registry).await?;
    backfill_profiles(&mut conn, store).await?;
    live_tail(&mut conn, cfg, store, registry).await
}

/// Meta key holding the unix-seconds timestamp up to which membership
/// notifications are known to be reflected in the channel table.
const MEMBERSHIP_WATERMARK_KEY: &str = "membership_watermark";

/// `since` anchor for the live membership subscription: the persisted
/// watermark when one exists (replaying anything missed while unsubscribed),
/// otherwise "now" — discovery has just run either way, the watermark simply
/// wasn't persisted yet on first boot. Skew keeps the boundary inclusive.
pub(crate) fn membership_since(watermark: Option<i64>, now: i64) -> i64 {
    (watermark.unwrap_or(now) - SINCE_SKEW_SECS).max(0)
}

/// Discovers every channel the key is a member of (39002 → 39000) and
/// registers the non-archived ones.
async fn discover_channels(
    conn: &mut NostrWsConnection,
    cfg: &SyncConfig,
    store: &Store,
    registry: &StatusRegistry,
) -> anyhow::Result<()> {
    let me = cfg.keys.public_key().to_hex();
    let members = request_until_eose(
        conn,
        "disc-members",
        json!({ "kinds": [buzz_core::kind::KIND_NIP29_GROUP_MEMBERS], "#p": [me] }),
        PAGE_TIMEOUT,
    )
    .await?
    .settled_or_bail("membership discovery")?;
    let channel_ids: BTreeSet<String> = members
        .iter()
        .filter_map(|ev| tag_value(ev, "d").map(str::to_string))
        .collect();
    info!(channels = channel_ids.len(), "membership discovered");
    if channel_ids.is_empty() {
        return Ok(());
    }

    // Metadata lookups are chunked to stay under the 128-explicit-value cap.
    let mut meta = Vec::new();
    let ids: Vec<String> = channel_ids.iter().cloned().collect();
    for (i, chunk) in ids.chunks(100).enumerate() {
        tokio::time::sleep(REQ_PACING).await;
        meta.extend(
            request_until_eose(
                conn,
                &format!("disc-meta-{i}"),
                json!({ "kinds": [buzz_core::kind::KIND_NIP29_GROUP_METADATA], "#d": chunk }),
                PAGE_TIMEOUT,
            )
            .await?
            .settled_or_bail("channel metadata discovery")?,
        );
    }

    let now = chrono::Utc::now().timestamp();
    for id in &channel_ids {
        let m = meta
            .iter()
            .find(|ev| tag_value(ev, "d") == Some(id.as_str()));
        let archived = m
            .map(|ev| tag_value(ev, "archived") == Some("true"))
            .unwrap_or(false);
        if archived {
            continue;
        }
        let name = m.and_then(|ev| tag_value(ev, "name")).map(str::to_string);
        let channel_type = m.map(channel_type_from_meta).unwrap_or("unknown");
        store
            .upsert_channel(id, name.as_deref(), channel_type, now)
            .await?;
        registry.channel(id, |c| {
            c.name = name.clone();
            c.channel_type = channel_type.to_string();
            if c.backfill.is_empty() {
                c.backfill = "pending".into();
            }
        });
    }
    Ok(())
}

/// Derives `stream` | `private` | `dm` from a kind-39000 metadata event.
fn channel_type_from_meta(ev: &nostr::Event) -> &'static str {
    let has = |name: &str| {
        ev.tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some(name))
    };
    let t_tag = tag_value(ev, "t");
    if has("hidden") || t_tag == Some("dm") {
        "dm"
    } else if has("private") || t_tag == Some("private") {
        "private"
    } else {
        "stream"
    }
}

/// Whether the sync loop backfills and tails a channel: it must be active
/// (access not revoked) and not excluded by the person. Excluded channels
/// stay in the mirror, listable and queryable, but receive no traffic.
pub(crate) fn sync_eligible(c: &super::store::ChannelRow) -> bool {
    c.active && !c.excluded
}

/// Pages every not-yet-complete channel from its persisted cursor down to the
/// beginning of history. Progress survives restarts: the cursor is persisted
/// after every page.
async fn backfill(
    conn: &mut NostrWsConnection,
    store: &Store,
    registry: &StatusRegistry,
) -> anyhow::Result<()> {
    let channels = store.channels().await?;
    for ch in channels.iter().filter(|c| c.active) {
        if ch.excluded {
            registry.channel(&ch.id, |c| {
                c.backfill = "excluded".into();
                c.live = false;
            });
            continue;
        }
        if ch.backfill_done {
            // Already complete in a prior run; reflect that in status.
            registry.channel(&ch.id, |c| c.backfill = "done".into());
            continue;
        }
        let mut cursor = ch
            .backfill_cursor
            .unwrap_or_else(|| chrono::Utc::now().timestamp());
        registry.channel(&ch.id, |c| c.backfill = "paging".into());
        let mut page_no: u64 = 0;
        loop {
            tokio::time::sleep(REQ_PACING).await;
            let sub = format!("bf-{page_no}-{}", ch.id);
            page_no += 1;
            let page = match request_until_eose(
                conn,
                &sub,
                json!({ "#h": [ch.id], "until": cursor, "limit": PAGE_LIMIT }),
                PAGE_TIMEOUT,
            )
            .await?
            {
                ReqOutcome::Settled(events) => events,
                ReqOutcome::Denied(message) => {
                    // Revoked mid-backfill; bailing would hard-loop the
                    // reconnect, so drop the channel and move on.
                    warn!(channel = %ch.id, %message, "channel access revoked during backfill");
                    store.deactivate_channel(&ch.id).await?;
                    registry.channel_revoked(&ch.id);
                    break;
                }
            };
            let stored = verify_and_map(page).await;
            let inserted = store.upsert_events(&stored).await?;
            let newest = stored.iter().map(|e| e.created_at).max();
            registry.channel(&ch.id, |c| {
                c.pages += 1;
                if let Some(ts) = newest {
                    c.newest_ts = Some(c.newest_ts.unwrap_or(ts).max(ts));
                }
            });
            let done = stored.len() < PAGE_LIMIT;
            if done {
                store.set_backfill(&ch.id, None, true).await?;
                registry.channel(&ch.id, |c| c.backfill = "done".into());
                info!(channel = %ch.id, "backfill complete");
                break;
            }
            cursor = advance_cursor(cursor, oldest_ts(&stored), inserted);
            store.set_backfill(&ch.id, Some(cursor), false).await?;
        }
    }
    Ok(())
}

/// Next `until` cursor for a full page.
///
/// The cursor moves to the oldest timestamp in the page (inclusive `until`
/// re-fetches that second; id-dedupe absorbs the overlap). A page that makes
/// no progress — everything already stored and the oldest timestamp equals
/// the cursor — steps back one second so a same-second stratum wider than one
/// page cannot wedge the loop. That step can skip unseen same-second events;
/// the WS surface has no id-tiebreak cursor, so this is logged, not hidden.
pub(crate) fn advance_cursor(cursor: i64, page_oldest: Option<i64>, inserted: u64) -> i64 {
    match page_oldest {
        Some(oldest) if oldest < cursor => oldest,
        Some(oldest) => {
            if inserted == 0 {
                warn!(
                    cursor,
                    "same-second stratum exceeded one page with no progress; stepping past it"
                );
                oldest - 1
            } else {
                oldest
            }
        }
        None => cursor - 1,
    }
}

fn oldest_ts(page: &[StoredEvent]) -> Option<i64> {
    page.iter().map(|e| e.created_at).min()
}

/// Mirrors kind-0 profiles so transcripts can show names. Cosmetic: failures
/// are logged and skipped, never fatal.
///
/// Pages to the beginning (bounded): a single newest-first page lets recent
/// ephemeral agent profiles crowd out the long-standing human ones, which is
/// exactly backwards for display names.
async fn backfill_profiles(conn: &mut NostrWsConnection, store: &Store) -> anyhow::Result<()> {
    const MAX_PROFILE_PAGES: usize = 40;
    let mut cursor = chrono::Utc::now().timestamp() + 1;
    let mut total = 0usize;
    for page_no in 0..MAX_PROFILE_PAGES {
        tokio::time::sleep(REQ_PACING).await;
        // Sub ids must be connection-unique (the relay acks CLOSE late).
        let page = match request_until_eose(
            conn,
            &format!("profiles-backfill-{page_no}"),
            json!({ "kinds": [0], "until": cursor, "limit": PAGE_LIMIT }),
            PAGE_TIMEOUT,
        )
        .await
        {
            Ok(ReqOutcome::Settled(events)) => events,
            Ok(ReqOutcome::Denied(_)) | Err(_) => Vec::new(),
        };
        let count = page.len();
        total += count;
        let mut oldest: Option<i64> = None;
        for ev in &page {
            let ts = ev.created_at.as_secs() as i64;
            oldest = Some(oldest.map_or(ts, |o| o.min(ts)));
            let name = profile_name(&ev.content);
            store
                .upsert_profile(&ev.pubkey.to_hex(), name.as_deref(), ts)
                .await?;
        }
        if count < PAGE_LIMIT {
            break;
        }
        // Inclusive `until` re-fetches the boundary second (upsert absorbs
        // it); never let the cursor stall on a same-second stratum.
        let next = oldest.unwrap_or(cursor - 1);
        cursor = if next >= cursor { cursor - 1 } else { next };
    }
    info!(profiles = total, "profile mirror refreshed");
    Ok(())
}

/// Extracts a display name from kind-0 content (display_name, then name).
pub(crate) fn profile_name(content: &str) -> Option<String> {
    let v: Value = serde_json::from_str(content).ok()?;
    let obj = v.as_object()?;
    for key in ["display_name", "name"] {
        if let Some(name) = obj.get(key).and_then(Value::as_str) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Opens live subscriptions (one per channel + profiles + membership) and
/// routes incoming events into the store until the connection drops or the
/// membership set changes (which returns `Ok(())` to request a resync).
async fn live_tail(
    conn: &mut NostrWsConnection,
    cfg: &SyncConfig,
    store: &Store,
    registry: &StatusRegistry,
) -> anyhow::Result<()> {
    let me = cfg.keys.public_key().to_hex();
    let now = chrono::Utc::now().timestamp();
    let channels = store.channels().await?;
    for ch in channels.iter().filter(|c| sync_eligible(c)) {
        tokio::time::sleep(REQ_PACING).await;
        let newest = store.newest_ts(&ch.id).await?;
        let since = newest
            .unwrap_or(ch.discovered_at)
            .saturating_sub(SINCE_SKEW_SECS);
        conn.send_raw(&json!([
            "REQ",
            format!("live-{}", ch.id),
            { "#h": [ch.id], "since": since }
        ]))
        .await?;
        registry.channel(&ch.id, |c| {
            c.live = true;
            // Seed from the mirror so a resumed run shows real recency
            // before the first live event lands.
            if let Some(ts) = newest {
                c.newest_ts = Some(c.newest_ts.unwrap_or(ts).max(ts));
            }
        });
    }
    tokio::time::sleep(REQ_PACING).await;
    conn.send_raw(&json!([
        "REQ", "live-profiles",
        { "kinds": [0], "since": now - SINCE_SKEW_SECS }
    ]))
    .await?;
    tokio::time::sleep(REQ_PACING).await;
    // Membership notifications are persisted and p-gated, so anchoring at the
    // watermark (not "now") replays anything emitted while this daemon was
    // backfilling, reconnecting, or down since the last discovery — e.g. a
    // channel the person created during startup. Each replayed or live event
    // triggers the same clean resync, and the resync's discovery advances the
    // watermark, so replays terminate.
    let watermark = store
        .get_meta(MEMBERSHIP_WATERMARK_KEY)
        .await?
        .and_then(|v| v.parse::<i64>().ok());
    conn.send_raw(&json!([
        "REQ", "live-membership",
        {
            "kinds": [
                buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION,
                buzz_core::kind::KIND_MEMBER_REMOVED_NOTIFICATION
            ],
            "#p": [me],
            "since": membership_since(watermark, now)
        }
    ]))
    .await?;
    info!("live tail active");

    loop {
        // A dropped ws frame at this cancellation point is repaired by the
        // resync itself (fresh connection, discovery, gap-fill).
        let next = tokio::select! {
            _ = cfg.resync.notified() => {
                info!("resync requested (exclusion change)");
                return Ok(());
            }
            next = conn.next_event(LIVE_TICK) => next,
        };
        let msg = match next {
            Ok(msg) => msg,
            Err(buzz_ws_client::WsClientError::Timeout) => continue, // quiet period
            Err(e) => return Err(e.into()),
        };
        match msg {
            RelayMessage::Event {
                subscription_id,
                event,
            } => {
                registry.saw_event(chrono::Utc::now().timestamp());
                if subscription_id == "live-membership" {
                    // Membership changed; the clean move is a full resync.
                    // Advance the watermark past this notification first —
                    // the resync's discovery reads the authoritative rosters
                    // regardless, and this keeps a notification stamped ahead
                    // of our clock from replaying on every reconnect.
                    let advance = (event.created_at.as_secs() as i64)
                        .saturating_add(1)
                        .max(now);
                    store
                        .set_meta(MEMBERSHIP_WATERMARK_KEY, &advance.to_string())
                        .await?;
                    info!(kind = %event.kind, "membership changed; resyncing");
                    return Ok(());
                }
                if subscription_id == "live-profiles" {
                    let name = profile_name(&event.content);
                    store
                        .upsert_profile(
                            &event.pubkey.to_hex(),
                            name.as_deref(),
                            event.created_at.as_secs() as i64,
                        )
                        .await?;
                    continue;
                }
                if let Some(channel_id) = subscription_id.strip_prefix("live-") {
                    let channel_id = channel_id.to_string();
                    let stored = verify_and_map(vec![*event]).await;
                    store.upsert_events(&stored).await?;
                    if let Some(ts) = oldest_ts(&stored) {
                        registry.channel(&channel_id, |c| {
                            c.newest_ts = Some(c.newest_ts.unwrap_or(ts).max(ts));
                        });
                    }
                }
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                if let Some(channel_id) = subscription_id.strip_prefix("live-") {
                    if CHANNEL_ACCESS_DENIED.contains(&message.as_str()) {
                        warn!(channel = %channel_id, %message, "channel access revoked");
                        store.deactivate_channel(channel_id).await?;
                        registry.channel_revoked(channel_id);
                        continue;
                    }
                    anyhow::bail!("subscription {subscription_id} closed: {message}");
                }
                // Straggling ack for an already-CLOSEd one-shot sub
                // (discovery/backfill); not ours to act on.
                warn!(%subscription_id, %message, "ignoring CLOSED for non-live subscription");
            }
            RelayMessage::Auth { challenge } => {
                // Mid-stream re-auth: answer it and carry on.
                let auth =
                    build_auth_event(&challenge, &cfg.relay_url, &cfg.keys, cfg.auth_tag.as_ref())?;
                conn.send_raw(&json!(["AUTH", auth])).await?;
            }
            RelayMessage::Notice { message } => warn!(%message, "relay notice"),
            _ => {}
        }
    }
}

/// How a one-shot REQ ended.
enum ReqOutcome {
    /// EOSE reached (or the relay settled the sub); here is everything.
    Settled(Vec<nostr::Event>),
    /// The relay refused with a channel-access CLOSED.
    Denied(String),
}

impl ReqOutcome {
    /// Unwraps `Settled`, turning `Denied` into an error — for requests where
    /// an access refusal means the whole connection is mis-scoped.
    fn settled_or_bail(self, what: &str) -> anyhow::Result<Vec<nostr::Event>> {
        match self {
            ReqOutcome::Settled(events) => Ok(events),
            ReqOutcome::Denied(message) => anyhow::bail!("{what} denied: {message}"),
        }
    }
}

/// Sends one REQ, collects its events until EOSE, then CLOSEs the sub.
///
/// `sub_id` must be unique for the lifetime of the connection: the relay acks
/// CLOSE with an empty-message CLOSED, and a late ack for a reused id would
/// be indistinguishable from a refusal of the current request.
async fn request_until_eose(
    conn: &mut NostrWsConnection,
    sub_id: &str,
    filter: Value,
    timeout: Duration,
) -> anyhow::Result<ReqOutcome> {
    conn.send_raw(&json!(["REQ", sub_id, filter])).await?;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut events = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            anyhow::bail!("timed out waiting for EOSE on {sub_id}");
        }
        match conn.next_event(remaining).await? {
            RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == sub_id => {
                events.push(*event);
            }
            RelayMessage::Eose { subscription_id } if subscription_id == sub_id => {
                conn.send_raw(&json!(["CLOSE", sub_id])).await?;
                return Ok(ReqOutcome::Settled(events));
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == sub_id => {
                if CHANNEL_ACCESS_DENIED.contains(&message.as_str()) {
                    return Ok(ReqOutcome::Denied(message));
                }
                if message.is_empty() {
                    // Relay-side settle of an active sub; whatever arrived is
                    // everything it will send.
                    warn!(%sub_id, "relay settled subscription with empty CLOSED");
                    return Ok(ReqOutcome::Settled(events));
                }
                anyhow::bail!("subscription {sub_id} closed: {message}");
            }
            _ => {}
        }
    }
}

/// Verifies a batch of events (Schnorr is CPU-bound, so this runs on the
/// blocking pool) and maps the valid ones to storage rows. Invalid events are
/// dropped with a warning — the mirror only holds verified signatures.
async fn verify_and_map(events: Vec<nostr::Event>) -> Vec<StoredEvent> {
    tokio::task::spawn_blocking(move || {
        events
            .into_iter()
            .filter_map(|ev| match buzz_core::verification::verify_event(&ev) {
                Ok(()) => Some(event_to_stored(&ev)),
                Err(e) => {
                    warn!(id = %ev.id, error = %e, "dropping event that failed verification");
                    None
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Distills a verified relay event into its storage row.
pub(crate) fn event_to_stored(ev: &nostr::Event) -> StoredEvent {
    let tag_rows: Vec<Vec<String>> = ev.tags.iter().map(|t| t.as_slice().to_vec()).collect();
    StoredEvent {
        id: ev.id.to_hex(),
        channel: tag_value(ev, "h").map(str::to_string),
        pubkey: ev.pubkey.to_hex(),
        kind: ev.kind.as_u16() as u32,
        created_at: ev.created_at.as_secs() as i64,
        content: ev.content.clone(),
        raw: serde_json::to_string(ev).unwrap_or_default(),
        parent: super::store::parent_from_tag_rows(&tag_rows),
    }
}

/// First value of the named tag, if present.
fn tag_value<'a>(ev: &'a nostr::Event, name: &str) -> Option<&'a str> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_advances_to_page_oldest() {
        assert_eq!(advance_cursor(1000, Some(900), 10), 900);
    }

    #[test]
    fn excluded_or_revoked_channels_are_not_sync_eligible() {
        let row = |active: bool, excluded: bool| super::super::store::ChannelRow {
            id: "cha".into(),
            name: None,
            channel_type: "stream".into(),
            backfill_cursor: None,
            backfill_done: false,
            active,
            excluded,
            discovered_at: 0,
        };
        assert!(sync_eligible(&row(true, false)));
        assert!(!sync_eligible(&row(true, true)));
        assert!(!sync_eligible(&row(false, false)));
        assert!(!sync_eligible(&row(false, true)));
    }

    #[test]
    fn cursor_steps_past_wedged_same_second_stratum() {
        // Full page, nothing new, oldest == cursor: without the step the loop
        // would refetch the identical page forever.
        assert_eq!(advance_cursor(1000, Some(1000), 0), 999);
    }

    #[test]
    fn cursor_stays_when_same_second_page_made_progress() {
        assert_eq!(advance_cursor(1000, Some(1000), 42), 1000);
    }

    #[test]
    fn cursor_backs_off_on_empty_page() {
        assert_eq!(advance_cursor(1000, None, 0), 999);
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(3), Duration::from_secs(4));
        assert_eq!(backoff_delay(99), Duration::from_secs(60));
    }

    #[test]
    fn membership_anchor_replays_from_the_watermark() {
        // With a persisted watermark, the subscription reaches back to it —
        // notifications emitted while unsubscribed (startup backfill window,
        // disconnect, downtime) replay instead of vanishing.
        assert_eq!(
            membership_since(Some(1_000), 5_000),
            1_000 - SINCE_SKEW_SECS
        );
        // First boot (no watermark yet): discovery just ran, anchor at now.
        assert_eq!(membership_since(None, 5_000), 5_000 - SINCE_SKEW_SECS);
        // Never underflows.
        assert_eq!(membership_since(Some(2), 5), 0);
    }

    #[test]
    fn profile_name_prefers_display_name() {
        assert_eq!(
            profile_name(r#"{"name":"riley","display_name":"Riley Crane"}"#).as_deref(),
            Some("Riley Crane")
        );
        assert_eq!(profile_name(r#"{"name":"  "}"#), None);
        assert_eq!(profile_name("not json"), None);
    }
}
