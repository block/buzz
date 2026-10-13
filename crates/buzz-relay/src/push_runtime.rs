//! Durable NIP-PL event matcher and gateway delivery worker.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use buzz_core::filter::{filters_match, reader_authorized_for_event};
use chrono::{TimeDelta, Utc};
use nostr::Filter;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

use crate::{handlers::push_lease::Subscription, nip98::nip98_header, state::AppState};

const CLAIM_SECS: i64 = 30;
const EVENT_USEFUL_SECS: i64 = 3600;
const MAX_ATTEMPTS: i32 = 8;
/// Upper bound on one claimed matcher batch. Bounded well under
/// `get_events_by_ids`' 500-id batch-fetch contract.
const MATCH_BATCH_LIMIT: i64 = 64;
/// Idle poll floor and ceiling. An idle matcher previously issued a claim
/// transaction every 250ms forever; backing off to the ceiling folds that
/// steady-state cost while keeping first-message latency at the floor.
const IDLE_POLL_FLOOR: Duration = Duration::from_millis(250);
const IDLE_POLL_CEILING: Duration = Duration::from_secs(2);
/// Cadence of the poison-job sweep, which lives off the claim path: its scan
/// is not served by the due partial index, so running it inside every claim
/// made claims slower exactly when a backlog needed them fastest.
const REAP_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Serialize)]
struct DeliveryRequest<'a> {
    v: u8,
    endpoint_grant: &'a str,
    request_id: uuid::Uuid,
    expires_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<WebPushHint>,
}

/// Content-free deep-link hint, attached only for `buzz-web` leases so APNs
/// deliveries stay blind. Never carries message content or author.
#[derive(Serialize, Debug, PartialEq)]
struct WebPushHint {
    class: String,
    event_id: String,
    channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_root_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prefetch: Option<crate::push_prefetch::PrefetchHint>,
}

const WEB_APP_PROFILE: &str = "buzz-web";

fn web_hint(wake: &buzz_db::push::ClaimedWake, thread_root: Option<&[u8]>) -> Option<WebPushHint> {
    (wake.app_profile.as_deref() == Some(WEB_APP_PROFILE)).then(|| WebPushHint {
        class: wake.class.clone(),
        event_id: hex::encode(&wake.event_id),
        channel_id: wake.channel_id.map(|c| c.to_string()),
        thread_root_id: thread_root.map(hex::encode),
        prefetch: crate::push_prefetch::hint_for(wake, thread_root),
    })
}

/// Thread root for a web wake's event so the tap opens the thread. Best-effort:
/// a lookup failure only drops the deep link, never the notification.
async fn web_thread_root(state: &AppState, wake: &buzz_db::push::ClaimedWake) -> Option<Vec<u8>> {
    if wake.app_profile.as_deref() != Some(WEB_APP_PROFILE) {
        return None;
    }
    state
        .db
        .push_thread_roots(wake.community, std::slice::from_ref(&wake.event_id))
        .await
        .ok()?
        .into_iter()
        .next()
        .map(|(_, root)| root)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
enum DeliveryResponse {
    Accepted,
    InvalidEndpoint {
        generation: i64,
        invalid_at: Option<i64>,
    },
    Retry {
        retry_after_seconds: Option<i64>,
    },
}

/// Continuously claim accepted events in per-community batches and match
/// them against active leases (T2b). One batch costs one claim statement,
/// one event load, one lease scan, one membership scan, and one wake-enqueue
/// transaction — plus one complete/retry statement each — regardless of
/// batch size or how many (event, lease) pairs match.
pub async fn run_matcher(state: Arc<AppState>) {
    let mut idle_delay = IDLE_POLL_FLOOR;
    let mut last_reap = tokio::time::Instant::now();
    loop {
        if last_reap.elapsed() >= REAP_INTERVAL {
            match state.db.reap_exhausted_push_matches().await {
                Ok(reaped) if reaped > 0 => warn!(reaped, "reaped exhausted push match jobs"),
                Ok(_) => {}
                Err(e) => error!("push match reap failed: {e}"),
            }
            last_reap = tokio::time::Instant::now();
        }
        let until = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
        match state
            .db
            .claim_due_push_match_batch(MATCH_BATCH_LIMIT, until)
            .await
        {
            Ok(Some(batch)) => {
                idle_delay = IDLE_POLL_FLOOR;
                process_match_batch(&state, batch).await;
            }
            Ok(None) => {
                tokio::time::sleep(idle_delay).await;
                idle_delay = (idle_delay * 2).min(IDLE_POLL_CEILING);
            }
            Err(e) => {
                error!("push matcher claim failed: {e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// Per-batch state shared by every job: the community's active leases and
/// the exact (channel, lease author) membership pairs the jobs can consult.
struct MatchContext {
    leases: Vec<buzz_db::push::MatchLease>,
    memberships: std::collections::HashSet<(uuid::Uuid, Vec<u8>)>,
    /// Batch channels that are DM channels (for `scope: "dm"`).
    dm_channels: std::collections::HashSet<uuid::Uuid>,
    /// Reply event id -> thread root id (for `scope: "thread"`).
    thread_roots: std::collections::HashMap<Vec<u8>, Vec<u8>>,
    /// (thread root id, lease author) participation pairs.
    thread_participants: std::collections::HashSet<(Vec<u8>, Vec<u8>)>,
    /// Lease author -> synced notification prefs (`buzz:push-prefs`).
    member_prefs: std::collections::HashMap<Vec<u8>, MemberPushPrefs>,
}

/// Per-member notification switches + mutes, published by the clients as a
/// kind-30078 `d=buzz:push-prefs` event NIP-44-encrypted to the relay's push
/// executor key. Absent or undecodable prefs mean "everything on".
#[derive(Debug, Default, Clone, serde::Deserialize)]
struct MemberPushPrefs {
    /// Push classes the member turned off (`dm`, `thread_reply`, `mention`, `needs_action`).
    #[serde(default)]
    off: std::collections::HashSet<String>,
    /// Muted channel / DM ids (UUID strings). Silences `dm` and `thread_reply`; mentions still notify.
    #[serde(default)]
    muted_channels: std::collections::HashSet<String>,
    /// Muted thread root ids (hex). Silences `thread_reply`.
    #[serde(default)]
    muted_threads: std::collections::HashSet<String>,
}

fn decode_member_prefs(
    keys: &nostr::Keys,
    author: &[u8],
    content: &str,
) -> Option<MemberPushPrefs> {
    let pubkey = nostr::PublicKey::from_slice(author).ok()?;
    let plaintext = nostr::nips::nip44::decrypt(keys.secret_key(), &pubkey, content).ok()?;
    if plaintext.len() > 65_536 {
        return None;
    }
    serde_json::from_str(&plaintext).ok()
}

/// True when the member's synced prefs silence this wake.
fn prefs_silence(
    prefs: Option<&MemberPushPrefs>,
    class: &str,
    job: &buzz_db::push::BatchedMatch,
    context: &MatchContext,
) -> bool {
    let Some(prefs) = prefs else { return false };
    if prefs.off.contains(class) {
        return true;
    }
    if class == "dm" || class == "thread_reply" {
        if let Some(channel) = job.event.channel_id {
            if prefs.muted_channels.contains(&channel.to_string()) {
                return true;
            }
        }
    }
    if class == "thread_reply" {
        if let Some(root) = context
            .thread_roots
            .get(job.event.event.id.as_bytes().as_slice())
        {
            if prefs.muted_threads.contains(&hex::encode(root)) {
                return true;
            }
        }
    }
    false
}

async fn load_match_context(
    state: &AppState,
    batch: &buzz_db::push::ClaimedMatchBatch,
) -> anyhow::Result<MatchContext> {
    let leases = state.db.active_push_match_leases(batch.community).await?;
    let mut channels: Vec<uuid::Uuid> = batch
        .jobs
        .iter()
        .filter_map(|job| job.event.channel_id)
        .collect();
    channels.sort_unstable();
    channels.dedup();
    let mut authors: Vec<Vec<u8>> = leases.iter().map(|lease| lease.author.clone()).collect();
    authors.sort_unstable();
    authors.dedup();
    let memberships = state
        .db
        .membership_pairs(batch.community, &channels, &authors)
        .await?
        .into_iter()
        .collect();
    // Participation context is only loaded when some lease asks for it, so
    // APNs-only communities pay nothing extra.
    let wants_scope = |scope: &str| {
        leases.iter().any(|lease| {
            lease.subscriptions.as_array().is_some_and(|subs| {
                subs.iter()
                    .any(|s| s.get("scope").and_then(|v| v.as_str()) == Some(scope))
            })
        })
    };
    let dm_channels = if wants_scope("dm") {
        state
            .db
            .push_dm_channels(batch.community, &channels)
            .await?
            .into_iter()
            .collect()
    } else {
        Default::default()
    };
    let (thread_roots, thread_participants) = if wants_scope("thread") {
        let ids: Vec<Vec<u8>> = batch
            .jobs
            .iter()
            .map(|j| j.event.event.id.as_bytes().to_vec())
            .collect();
        let roots: std::collections::HashMap<Vec<u8>, Vec<u8>> = state
            .db
            .push_thread_roots(batch.community, &ids)
            .await?
            .into_iter()
            .collect();
        let mut root_ids: Vec<Vec<u8>> = roots.values().cloned().collect();
        root_ids.sort_unstable();
        root_ids.dedup();
        let participants = state
            .db
            .push_thread_participants(batch.community, &root_ids, &authors)
            .await?
            .into_iter()
            .collect();
        (roots, participants)
    } else {
        Default::default()
    };
    let member_prefs = state
        .db
        .push_member_prefs(batch.community, &authors)
        .await
        .unwrap_or_else(|e| {
            // Prefs are advisory: a failed load must not block delivery.
            warn!(community=%batch.community, "push member prefs load failed: {e}");
            Vec::new()
        })
        .into_iter()
        .filter_map(|(author, content)| {
            decode_member_prefs(&state.relay_keypair, &author, &content).map(|p| (author, p))
        })
        .collect();
    Ok(MatchContext {
        leases,
        memberships,
        dm_channels,
        thread_roots,
        thread_participants,
        member_prefs,
    })
}

async fn process_match_batch(state: &AppState, batch: buzz_db::push::ClaimedMatchBatch) {
    let community = batch.community;
    let context = match load_match_context(state, &batch).await {
        Ok(context) => context,
        Err(e) => {
            // Shared context failed to load, so no job was evaluated: release
            // the whole batch for retry. Jobs that keep failing are reaped by
            // the periodic sweep once their attempts are exhausted.
            warn!(%community, "push match context load failed: {e}");
            metrics::counter!("buzz_push_match_jobs_total", "result" => "context_error")
                .increment(batch.jobs.len() as u64);
            let ids: Vec<Vec<u8>> = batch
                .jobs
                .iter()
                .map(|job| job.event.event.id.as_bytes().to_vec())
                .collect();
            if let Err(e) = state
                .db
                .retry_push_match_batch(
                    community,
                    batch.claim_id,
                    &ids,
                    Utc::now() + TimeDelta::seconds(2),
                )
                .await
            {
                warn!(%community, "push match batch retry failed: {e}");
            }
            return;
        }
    };
    let mut completed = Vec::new();
    let mut retry = Vec::new();
    // Jobs whose wakes are pending the set-wise enqueue below: their
    // completion is decided by the flush, not by match evaluation.
    let mut pending = Vec::new();
    let mut wakes: Vec<buzz_db::push::WakeRequest> = Vec::new();
    for job in &batch.jobs {
        let match_queue_seconds = Utc::now()
            .signed_duration_since(job.event.received_at)
            .num_milliseconds()
            .max(0) as f64
            / 1_000.0;
        metrics::histogram!("buzz_push_match_queue_seconds").record(match_queue_seconds);
        let event_id = job.event.event.id.as_bytes().to_vec();
        match match_job(job, &context) {
            Ok(job_wakes) if job_wakes.is_empty() => {
                metrics::counter!("buzz_push_match_jobs_total", "result" => "unmatched")
                    .increment(1);
                completed.push(event_id);
            }
            Ok(job_wakes) => {
                metrics::counter!("buzz_push_match_jobs_total", "result" => "matched").increment(1);
                pending.push((event_id, job.attempt));
                wakes.extend(job_wakes);
            }
            Err(e) => {
                metrics::counter!("buzz_push_match_jobs_total", "result" => "error").increment(1);
                warn!(event_id=%job.event.event.id, attempt=job.attempt, "push match failed: {e}");
                if job.attempt >= buzz_db::push::MAX_MATCH_ATTEMPTS {
                    // A poison event/lease must not retry forever or pin
                    // delivered outbox retention through the rematch guard.
                    completed.push(event_id);
                } else {
                    retry.push(event_id);
                }
            }
        }
    }
    // One transaction for every wake in the batch (T2b). InactiveLease and
    // Duplicate outcomes are per-request non-errors; only a failed enqueue
    // transaction sends the contributing jobs back for an idempotent rematch
    // (the outbox dedup key absorbs any wakes that did commit elsewhere).
    match state.db.enqueue_push_wakes(community, &wakes).await {
        Ok(outcomes) => {
            for outcome in outcomes {
                let result = match outcome {
                    buzz_db::push::EnqueueWakeOutcome::Enqueued(_) => "enqueued",
                    buzz_db::push::EnqueueWakeOutcome::Duplicate(_) => "duplicate",
                    buzz_db::push::EnqueueWakeOutcome::InactiveLease => "inactive_lease",
                };
                metrics::counter!("buzz_push_wakes_total", "result" => result).increment(1);
            }
            completed.extend(pending.into_iter().map(|(event_id, _)| event_id));
        }
        Err(e) => {
            metrics::counter!("buzz_push_wake_enqueue_errors_total").increment(1);
            warn!(%community, "push wake batch enqueue failed: {e}");
            for (event_id, attempt) in pending {
                if attempt >= buzz_db::push::MAX_MATCH_ATTEMPTS {
                    completed.push(event_id);
                } else {
                    retry.push(event_id);
                }
            }
        }
    }
    if let Err(e) = state
        .db
        .complete_push_match_batch(community, batch.claim_id, &completed)
        .await
    {
        warn!(%community, "push match batch completion failed: {e}");
    }
    if let Err(e) = state
        .db
        .retry_push_match_batch(
            community,
            batch.claim_id,
            &retry,
            Utc::now() + TimeDelta::seconds(2),
        )
        .await
    {
        warn!(%community, "push match batch retry failed: {e}");
    }
}

/// Pure match evaluation: no DB access. Returns the wake requests this job
/// owes, which the caller flushes set-wise for the whole batch.
fn match_job(
    job: &buzz_db::push::BatchedMatch,
    context: &MatchContext,
) -> anyhow::Result<Vec<buzz_db::push::WakeRequest>> {
    let mut wakes = Vec::new();
    for lease in &context.leases {
        let author_hex = hex::encode(&lease.author);
        if !reader_authorized_for_event(&job.event.event, &author_hex) {
            continue;
        }
        if let Some(channel) = job.event.channel_id {
            if !context
                .memberships
                .contains(&(channel, lease.author.clone()))
            {
                continue;
            }
        }
        let subscriptions: Vec<Subscription> = serde_json::from_value(lease.subscriptions.clone())?;
        let mut class: Option<&str> = None;
        for sub in &subscriptions {
            let filter: Filter =
                serde_json::from_value(serde_json::Value::Object(sub.filter.clone()))?;
            if !push_filter_authorized_for_event(&filter, &job.event.event, &author_hex)
                || !filters_match(std::slice::from_ref(&filter), &job.event)
            {
                continue;
            }
            if let Some(scope) = sub.scope.as_deref() {
                if !scope_matches(scope, job, &lease.author, context) {
                    continue;
                }
            }
            let ignored = sub.ignore.iter().any(|raw| {
                serde_json::from_value::<Filter>(serde_json::Value::Object(raw.clone()))
                    .is_ok_and(|f| filters_match(&[f], &job.event))
            });
            let p_count = job
                .event
                .event
                .tags
                .iter()
                .filter(|t| t.kind().to_string() == "p")
                .count() as u64;
            if ignored
                || sub
                    .suppress
                    .as_ref()
                    .is_some_and(|s| p_count > s.p_tags_max)
            {
                continue;
            }
            if class.is_none_or(|old| class_rank(&sub.class) > class_rank(old)) {
                class = Some(&sub.class);
            }
        }
        let Some(class) = class else { continue };
        if prefs_silence(context.member_prefs.get(&lease.author), class, job, context) {
            continue;
        }
        let event_deadline = job.event.event.created_at.as_secs() as i64 + EVENT_USEFUL_SECS;
        let expires_at = lease.expires_at.min(event_deadline);
        if expires_at <= Utc::now().timestamp() {
            continue;
        }
        wakes.push(buzz_db::push::WakeRequest {
            author: lease.author.clone(),
            installation_id: lease.installation_id.clone(),
            lease_generation: lease.generation,
            event_id: job.event.event.id.as_bytes().to_vec(),
            class: class.to_string(),
            expires_at,
        });
    }
    Ok(wakes)
}

/// Relay-evaluated participation scope. Never matches the author's own events.
fn scope_matches(
    scope: &str,
    job: &buzz_db::push::BatchedMatch,
    lease_author: &[u8],
    context: &MatchContext,
) -> bool {
    if job.event.event.pubkey.to_bytes().as_slice() == lease_author {
        return false;
    }
    match scope {
        "dm" => job
            .event
            .channel_id
            .is_some_and(|c| context.dm_channels.contains(&c)),
        "thread" => context
            .thread_roots
            .get(job.event.event.id.as_bytes().as_slice())
            .is_some_and(|root| {
                context
                    .thread_participants
                    .contains(&(root.clone(), lease_author.to_vec()))
            }),
        _ => false,
    }
}

/// Match-time counterpart of REQ's filter-level `#p` authorization gate.
/// Kind 1059 is globally stored and leaks recipient activity through wake
/// timing, so a lease may only match gift wraps addressed to its own author.
fn push_filter_authorized_for_event(
    filter: &Filter,
    event: &nostr::Event,
    lease_author_hex: &str,
) -> bool {
    if buzz_core::kind::event_kind_u32(event) != buzz_core::kind::KIND_GIFT_WRAP {
        return true;
    }
    let p = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
    filter.generic_tags.get(&p).is_some_and(|values| {
        !values.is_empty()
            && values.iter().all(|value| value == lease_author_hex)
            && event
                .tags
                .filter(nostr::TagKind::SingleLetter(p))
                .any(|tag| tag.content() == Some(lease_author_hex))
    })
}

/// Continuously claim due wakes and deliver them through the push gateway.
pub async fn run_delivery_worker(state: Arc<AppState>) {
    let http = match reqwest::Client::builder()
        .timeout(state.config.push_gateway_timeout)
        .build()
    {
        Ok(http) => http,
        Err(error) => {
            error!(%error, "push HTTP client initialization failed");
            record_delivery("configuration_error");
            return;
        }
    };
    let mut idle_delay = Duration::from_millis(500);
    loop {
        let mut found = false;
        match state.db.usage_community_hosts().await {
            Ok(communities) => {
                for community in communities {
                    let community = buzz_core::CommunityId::from_uuid(community.id);
                    let until = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
                    match state.db.claim_due_push_wakes(community, 16, until).await {
                        Ok(wakes) => {
                            for wake in wakes {
                                found = true;
                                deliver_one(&state, &http, wake).await;
                            }
                        }
                        Err(e) => warn!(%community, "push wake claim failed: {e}"),
                    }
                }
            }
            Err(e) => error!("push worker community scan failed: {e}"),
        }
        if found {
            idle_delay = Duration::from_millis(500);
        } else {
            // Empty sweeps back off so an idle worker stops paying a full
            // per-community claim scan every 500ms forever.
            tokio::time::sleep(idle_delay).await;
            idle_delay = (idle_delay * 2).min(IDLE_POLL_CEILING);
        }
    }
}

async fn deliver_one(
    state: &AppState,
    http: &reqwest::Client,
    claimed: buzz_db::push::ClaimedWake,
) {
    let outcome = match state
        .db
        .revalidate_push_wake(claimed.community, claimed.id, claimed.claim_id)
        .await
    {
        Ok(buzz_db::push::RevalidateWakeOutcome::Deliver(wake)) => wake,
        Ok(buzz_db::push::RevalidateWakeOutcome::Suppressed) => {
            let _ = state
                .db
                .fail_push_wake(claimed.community, claimed.id, claimed.claim_id)
                .await;
            record_delivery("suppressed");
            return;
        }
        Err(e) => {
            warn!(wake=%claimed.id, "push revalidation failed: {e}");
            record_delivery("worker_error");
            return;
        }
    };
    if outcome.attempt == 1 {
        let wake_queue_seconds = Utc::now()
            .signed_duration_since(outcome.queued_at)
            .num_milliseconds()
            .max(0) as f64
            / 1_000.0;
        metrics::histogram!("buzz_push_wake_queue_seconds").record(wake_queue_seconds);
    }
    if let Some(channel) = outcome.channel_id {
        match state
            .db
            .is_member(outcome.community, channel, &outcome.author)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                let _ = state
                    .db
                    .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                    .await;
                record_delivery("suppressed");
                return;
            }
            Err(e) => {
                warn!(wake=%outcome.id, "push membership revalidation failed: {e}");
                let _ = state
                    .db
                    .retry_push_wake(
                        outcome.community,
                        outcome.id,
                        outcome.claim_id,
                        Utc::now() + TimeDelta::seconds(2),
                    )
                    .await;
                record_delivery("retry");
                return;
            }
        }
    }
    // Membership I/O above can race lease replacement. Re-run the generation
    // fence as the final database operation before transport.
    let outcome = match state
        .db
        .revalidate_push_wake(outcome.community, outcome.id, outcome.claim_id)
        .await
    {
        Ok(buzz_db::push::RevalidateWakeOutcome::Deliver(wake)) => wake,
        Ok(buzz_db::push::RevalidateWakeOutcome::Suppressed) => {
            let _ = state
                .db
                .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
            record_delivery("suppressed");
            return;
        }
        Err(e) => {
            warn!(wake=%outcome.id, "final push revalidation failed: {e}");
            record_delivery("worker_error");
            return;
        }
    };
    let serving_write = match buzz_deletion::acquire_serving_write(
        &state.db,
        outcome.community,
        "push_delivery",
    )
    .await
    {
        Ok(guard) => guard,
        Err(error) => {
            warn!(wake=%outcome.id, %error, "push delivery suppressed by community deletion fence");
            let _ = state
                .db
                .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
            record_delivery("suppressed");
            return;
        }
    };
    let Some(url) = state.config.push_gateway_delivery_url.as_ref() else {
        record_delivery("configuration_error");
        return;
    };
    let thread_root = web_thread_root(state, &outcome).await;
    let body = match delivery_body(
        &outcome.endpoint_grant,
        outcome.id,
        outcome.expires_at,
        web_hint(&outcome, thread_root.as_deref()),
    ) {
        Ok(body) => body,
        Err(error) => {
            warn!(wake=%outcome.id, %error, "push delivery body encoding failed");
            record_delivery("worker_error");
            return;
        }
    };
    let auth = match nip98_header(&state.relay_keypair, url.as_str(), &body) {
        Ok(auth) => auth,
        Err(e) => {
            warn!(wake=%outcome.id, "push auth failed: {e}");
            record_delivery("worker_error");
            return;
        }
    };
    if let Err(error) = serving_write.verify().await {
        warn!(wake=%outcome.id, %error, "push serving lease lost before delivery");
        record_delivery("suppressed");
        return;
    }
    metrics::counter!("buzz_push_gateway_requests_total").increment(1);
    let gateway_started = Instant::now();
    let protected = serving_write
        .protect(send_gateway_request(http, url, body, auth))
        .await;
    metrics::histogram!("buzz_push_gateway_request_seconds")
        .record(gateway_started.elapsed().as_secs_f64());
    let response = match protected {
        Ok(response) => response,
        Err(error) => {
            warn!(wake=%outcome.id, %error, "push serving lease lost during delivery");
            record_delivery("suppressed");
            return;
        }
    };
    match response {
        Ok(r) if r.status().is_success() => match r.json::<DeliveryResponse>().await {
            Ok(DeliveryResponse::Accepted) => {
                let _ = state
                    .db
                    .complete_push_wake(outcome.community, outcome.id, outcome.claim_id)
                    .await;
                record_delivery("accepted");
            }
            _ => {
                let _ = state
                    .db
                    .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                    .await;
                record_delivery("failed");
            }
        },
        Ok(r) if r.status() == reqwest::StatusCode::GONE => {
            match r.json::<DeliveryResponse>().await {
                Ok(DeliveryResponse::InvalidEndpoint {
                    generation,
                    invalid_at,
                }) => {
                    if generation == outcome.lease_generation {
                        let _ = state
                            .db
                            .disable_push_endpoint(
                                outcome.community,
                                &outcome.author,
                                &outcome.installation_id,
                                generation,
                            )
                            .await;
                    }
                    warn!(wake=%outcome.id, ?invalid_at, "push endpoint permanently invalid");
                }
                _ => warn!(wake=%outcome.id, "invalid closed-protocol 410 response"),
            }
            let _ = state
                .db
                .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
            record_delivery("invalid_endpoint");
        }
        Ok(r) if r.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            let delay = match r.json::<DeliveryResponse>().await {
                Ok(DeliveryResponse::Retry {
                    retry_after_seconds,
                }) => retry_after_seconds
                    .filter(|seconds| *seconds > 0)
                    .unwrap_or(2),
                _ => 2,
            };
            record_delivery(retry_or_fail(state, &outcome, delay).await);
        }
        Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            record_delivery(retry_or_fail(state, &outcome, 2).await);
        }
        // A timed-out terminal attempt burns the stable request id. Its replay
        // is indistinguishable from another invalid-grant 404, but sending a
        // fresh id would double-deliver and defeat the gateway replay fence.
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND && outcome.attempt > 1 => {
            let _ = state
                .db
                .complete_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
            record_delivery("replay_terminal");
        }
        Err(e) if e.is_timeout() || e.is_connect() => {
            record_delivery(retry_or_fail(state, &outcome, 2).await);
        }
        _ => {
            let _ = state
                .db
                .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
            record_delivery("failed");
        }
    }
    if let Err(error) = serving_write.finish().await {
        warn!(wake=%outcome.id, %error, "failed to release community serving lease after push delivery");
    }
}

fn delivery_body(
    endpoint_grant: &str,
    request_id: uuid::Uuid,
    expires_at: i64,
    hint: Option<WebPushHint>,
) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&DeliveryRequest {
        v: 1,
        endpoint_grant,
        request_id,
        expires_at,
        hint,
    })?)
}

async fn send_gateway_request(
    http: &reqwest::Client,
    url: &url::Url,
    body: Vec<u8>,
    auth: String,
) -> reqwest::Result<reqwest::Response> {
    http.post(url.clone())
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
}

fn record_delivery(outcome: &'static str) {
    metrics::counter!("buzz_push_deliveries_total", "outcome" => outcome).increment(1);
}

async fn retry_or_fail(
    state: &AppState,
    wake: &buzz_db::push::ClaimedWake,
    delay: i64,
) -> &'static str {
    if wake.attempt >= MAX_ATTEMPTS {
        let _ = state
            .db
            .fail_push_wake(wake.community, wake.id, wake.claim_id)
            .await;
        "exhausted"
    } else {
        let secs = delay * (1_i64 << (wake.attempt - 1).clamp(0, 6));
        let _ = state
            .db
            .retry_push_wake(
                wake.community,
                wake.id,
                wake.claim_id,
                Utc::now() + TimeDelta::seconds(secs),
            )
            .await;
        "retry"
    }
}

fn class_rank(_: &str) -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::{OriginalUri, State},
        http::HeaderMap,
        routing::post,
        Json, Router,
    };
    use nostr::{EventBuilder, Kind, Tag};
    use serde_json::Value;
    use std::{future::IntoFuture, sync::Arc};
    use tokio::sync::Mutex;

    #[test]
    fn gift_wrap_match_requires_self_p_filter_and_recipient() {
        let recipient = nostr::Keys::generate();
        let other = nostr::Keys::generate();
        let sender = nostr::Keys::generate();
        let recipient_hex = recipient.public_key().to_hex();
        let event = EventBuilder::new(Kind::GiftWrap, "ciphertext")
            .tag(Tag::public_key(other.public_key()))
            .sign_with_keys(&sender)
            .unwrap();
        let self_filter = Filter::new().pubkey(recipient.public_key());
        assert!(!push_filter_authorized_for_event(
            &self_filter,
            &event,
            &recipient_hex
        ));

        let event = EventBuilder::new(Kind::GiftWrap, "ciphertext")
            .tag(Tag::public_key(recipient.public_key()))
            .sign_with_keys(&sender)
            .unwrap();
        assert!(push_filter_authorized_for_event(
            &self_filter,
            &event,
            &recipient_hex
        ));
        assert!(!push_filter_authorized_for_event(
            &Filter::new().author(sender.public_key()),
            &event,
            &recipient_hex
        ));
    }

    async fn capture(
        State(seen): State<Arc<Mutex<Vec<Value>>>>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Json<Value> {
        let url: url::Url = format!("http://{}{}", headers["host"].to_str().unwrap(), uri)
            .parse()
            .unwrap();
        assert!(!headers.contains_key("x-forwarded-proto"));
        nostr::nips::nip98::verify_auth_header(
            headers["authorization"].to_str().unwrap(),
            &url,
            nostr::nips::nip98::HttpMethod::POST,
            nostr::Timestamp::now(),
            Some(&body),
        )
        .unwrap();
        seen.lock()
            .await
            .push(serde_json::from_slice(&body).unwrap());
        Json(serde_json::json!({"status":"accepted"}))
    }

    fn wake(profile: Option<&str>) -> buzz_db::push::ClaimedWake {
        buzz_db::push::ClaimedWake {
            community: buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4()),
            id: uuid::Uuid::new_v4(),
            claim_id: uuid::Uuid::new_v4(),
            event_id: vec![0xab; 32],
            channel_id: Some(uuid::Uuid::nil()),
            author: vec![1; 32],
            installation_id: "i".into(),
            lease_generation: 1,
            endpoint_grant: "g".into(),
            class: "mention".into(),
            app_profile: profile.map(str::to_owned),
            expires_at: 0,
            queued_at: Utc::now(),
            attempt: 1,
        }
    }

    fn scoped_context(
        member: &nostr::Keys,
        subs: serde_json::Value,
        channel: uuid::Uuid,
    ) -> MatchContext {
        MatchContext {
            leases: vec![buzz_db::push::MatchLease {
                author: member.public_key().to_bytes().to_vec(),
                installation_id: "web".into(),
                generation: 1,
                subscriptions: subs,
                expires_at: Utc::now().timestamp() + 3600,
            }],
            memberships: [(channel, member.public_key().to_bytes().to_vec())]
                .into_iter()
                .collect(),
            dm_channels: Default::default(),
            thread_roots: Default::default(),
            thread_participants: Default::default(),
            member_prefs: Default::default(),
        }
    }

    fn channel_message(author: &nostr::Keys, channel: uuid::Uuid) -> buzz_db::push::BatchedMatch {
        let event = EventBuilder::new(Kind::Custom(9), "hi")
            .tag(Tag::parse(["h", &channel.to_string()]).unwrap())
            .sign_with_keys(author)
            .unwrap();
        buzz_db::push::BatchedMatch {
            event: buzz_core::StoredEvent::new(event, Some(channel)),
            attempt: 1,
        }
    }

    #[test]
    fn thread_scope_matches_participants_only() {
        let (member, other) = (nostr::Keys::generate(), nostr::Keys::generate());
        let channel = uuid::Uuid::new_v4();
        let subs = serde_json::json!([{"filter":{"kinds":[9]},"class":"thread_reply","ignore":[],"scope":"thread"}]);
        let mut ctx = scoped_context(&member, subs, channel);
        let reply = channel_message(&other, channel);
        // Not a thread reply -> no wake.
        assert!(match_job(&reply, &ctx).unwrap().is_empty());
        let root = vec![9u8; 32];
        ctx.thread_roots
            .insert(reply.event.event.id.as_bytes().to_vec(), root.clone());
        // Reply in a thread the member never touched -> no wake.
        assert!(match_job(&reply, &ctx).unwrap().is_empty());
        ctx.thread_participants
            .insert((root, member.public_key().to_bytes().to_vec()));
        let wakes = match_job(&reply, &ctx).unwrap();
        assert_eq!(wakes.len(), 1);
        assert_eq!(wakes[0].class, "thread_reply");
        // The member's own reply never wakes them.
        let own = channel_message(&member, channel);
        ctx.thread_roots
            .insert(own.event.event.id.as_bytes().to_vec(), vec![9u8; 32]);
        assert!(match_job(&own, &ctx).unwrap().is_empty());
    }

    #[test]
    fn dm_scope_matches_any_dm_channel_the_member_is_in() {
        let (member, other) = (nostr::Keys::generate(), nostr::Keys::generate());
        let channel = uuid::Uuid::new_v4();
        let subs =
            serde_json::json!([{"filter":{"kinds":[9]},"class":"dm","ignore":[],"scope":"dm"}]);
        let mut ctx = scoped_context(&member, subs, channel);
        let msg = channel_message(&other, channel);
        assert!(match_job(&msg, &ctx).unwrap().is_empty());
        ctx.dm_channels.insert(channel);
        assert_eq!(match_job(&msg, &ctx).unwrap()[0].class, "dm");
        // Not a member of that DM -> membership gate still blocks it.
        ctx.memberships.clear();
        assert!(match_job(&msg, &ctx).unwrap().is_empty());
    }

    #[test]
    fn member_prefs_silence_off_classes_and_muted_channels() {
        let (member, other) = (nostr::Keys::generate(), nostr::Keys::generate());
        let channel = uuid::Uuid::new_v4();
        let subs =
            serde_json::json!([{"filter":{"kinds":[9]},"class":"dm","ignore":[],"scope":"dm"}]);
        let mut ctx = scoped_context(&member, subs, channel);
        ctx.dm_channels.insert(channel);
        let msg = channel_message(&other, channel);
        assert_eq!(match_job(&msg, &ctx).unwrap().len(), 1);
        let author = member.public_key().to_bytes().to_vec();
        ctx.member_prefs.insert(
            author.clone(),
            serde_json::from_value(serde_json::json!({"off":["dm"]})).unwrap(),
        );
        assert!(match_job(&msg, &ctx).unwrap().is_empty());
        ctx.member_prefs.insert(
            author.clone(),
            serde_json::from_value(serde_json::json!({"muted_channels":[channel.to_string()]}))
                .unwrap(),
        );
        assert!(match_job(&msg, &ctx).unwrap().is_empty());
        ctx.member_prefs.insert(
            author,
            serde_json::from_value(serde_json::json!({"off":["mention"]})).unwrap(),
        );
        assert_eq!(match_job(&msg, &ctx).unwrap().len(), 1);
    }

    #[test]
    fn member_prefs_decrypt_from_executor_key() {
        let (relay, member) = (nostr::Keys::generate(), nostr::Keys::generate());
        let ct = nostr::nips::nip44::encrypt(
            member.secret_key(),
            &relay.public_key(),
            r#"{"v":1,"off":["thread_reply"]}"#,
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();
        let prefs = decode_member_prefs(&relay, &member.public_key().to_bytes(), &ct).unwrap();
        assert!(prefs.off.contains("thread_reply"));
        assert!(decode_member_prefs(&relay, &member.public_key().to_bytes(), "junk").is_none());
    }

    #[test]
    fn prefetch_hint_is_scoped_to_wake() {
        let key = b"0123456789abcdef0123456789abcdef";
        let w = wake(Some("buzz-web"));
        let h = crate::push_prefetch::hint_with(
            key,
            "https://r.example/push/prefetch",
            &w,
            Some(&[0xcd; 32]),
            1000,
        )
        .unwrap();
        let c = crate::push_prefetch::verify(key, &h.token, 1000).unwrap();
        assert_eq!(c.ch, uuid::Uuid::nil());
        assert_eq!(c.c, *w.community.as_uuid());
        assert_eq!(c.a, "01".repeat(32));
        assert_eq!(c.s, "ab".repeat(32));
        assert_eq!(c.r.as_deref(), Some("cd".repeat(32).as_str()));
        assert_eq!(c.e, 1000 + crate::push_prefetch::TOKEN_TTL_SECS);
        assert!(h.token.len() <= 512);
        let mut no_channel = w.clone();
        no_channel.channel_id = None;
        assert!(crate::push_prefetch::hint_with(key, "https://r", &no_channel, None, 0).is_none());
    }

    #[test]
    fn hint_only_for_web_profile_and_content_free() {
        assert_eq!(
            web_hint(&wake(Some("buzz-ios-dogfood")), Some(&[7; 32])),
            None
        );
        assert_eq!(web_hint(&wake(None), None), None);
        let hint = web_hint(&wake(Some("buzz-web")), Some(&[0xcd; 32])).unwrap();
        assert_eq!(
            hint.thread_root_id.as_deref(),
            Some("cd".repeat(32).as_str())
        );
        assert_eq!(hint.class, "mention");
        assert_eq!(hint.event_id, "ab".repeat(32));
        let body: serde_json::Value =
            serde_json::from_slice(&delivery_body("g", uuid::Uuid::nil(), 1, Some(hint)).unwrap())
                .unwrap();
        assert_eq!(body["hint"].as_object().unwrap().len(), 4);
        let apns: serde_json::Value =
            serde_json::from_slice(&delivery_body("g", uuid::Uuid::nil(), 1, None).unwrap())
                .unwrap();
        assert!(apns.get("hint").is_none());
    }

    #[tokio::test]
    async fn gateway_retries_send_the_same_request_id_over_http() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/v1/deliveries/apns", post(capture))
                    .with_state(seen.clone()),
            )
            .into_future(),
        );
        let url: url::Url = format!("http://{address}/v1/deliveries/apns")
            .parse()
            .unwrap();
        let http = reqwest::Client::new();
        let keys = nostr::Keys::generate();
        let request_id = uuid::Uuid::new_v4();
        for _ in 0..2 {
            let body = delivery_body(
                "opaque-grant",
                request_id,
                Utc::now().timestamp() + 60,
                None,
            )
            .unwrap();
            let auth = nip98_header(&keys, url.as_str(), &body).unwrap();
            let response = send_gateway_request(&http, &url, body, auth).await.unwrap();
            assert!(response.status().is_success());
        }
        server.abort();
        let bodies = seen.lock().await;
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["request_id"], request_id.to_string());
        assert_eq!(bodies[1]["request_id"], request_id.to_string());
    }
}
