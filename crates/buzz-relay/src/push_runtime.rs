//! Durable NIP-PL event matcher and gateway delivery worker.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use buzz_core::filter::{filters_match, reader_authorized_for_event};
use chrono::{TimeDelta, Utc};
use nostr::Filter;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, warn};

use crate::{
    handlers::{
        push_lease::Subscription,
        system_channels::{filter_channel_scope, NamedOnlyChannels},
    },
    nip98::nip98_header,
    state::AppState,
};

// Dropping a SQLx transaction rolls it back; committed claims remain leased
// until expiry. Dropping HTTP cannot recall requests already accepted by the
// gateway. Do not acknowledge, delete, or retry a job merely on cancellation.
async fn run_push_task(
    enabled: bool,
    cancel: &tokio_util::sync::CancellationToken,
    work: impl std::future::Future<Output = ()>,
) {
    if !enabled {
        return;
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {},
        _ = work => {},
    }
}

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
    run_push_task(
        state.config.push_enabled,
        &state.push_cancel,
        matcher_loop(&state),
    )
    .await;
}

async fn matcher_loop(state: &AppState) {
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
                process_match_batch(state, batch).await;
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

/// Per-batch state shared by every job: the community's active leases, the
/// exact (channel, lease author) membership pairs the jobs can consult, and
/// the batch's named-only channels (system channels and any type added later).
struct MatchContext {
    leases: Vec<buzz_db::push::MatchLease>,
    memberships: std::collections::HashSet<(uuid::Uuid, Vec<u8>)>,
    named_only: NamedOnlyChannels,
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
    let named_only = NamedOnlyChannels::from_typed(
        state
            .db
            .named_only_channels_among(batch.community, &channels)
            .await?,
    );
    Ok(MatchContext {
        leases,
        memberships,
        named_only,
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
                || !push_filter_reads_channel(&filter, job.event.channel_id, &context.named_only)
                || !filters_match(std::slice::from_ref(&filter), &job.event)
            {
                continue;
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

/// Whether a lease filter may push an event in `channel`, by the same rule as
/// REQ: a legacy-type channel is in every filter's scope (membership is
/// checked separately), and a named-only channel is in scope only when the
/// filter names it (see [`crate::handlers::system_channels`]). Lease filters
/// cannot carry `#t` or `#P`, so for push "names it" means `#h`.
fn push_filter_reads_channel(
    filter: &Filter,
    channel: Option<uuid::Uuid>,
    named_only: &NamedOnlyChannels,
) -> bool {
    let Some(channel) = channel else {
        return true;
    };
    if !named_only.ids().contains(&channel) {
        return true;
    }
    filter_channel_scope(filter, &[], named_only).contains(&channel)
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
    run_push_task(
        state.config.push_enabled,
        &state.push_cancel,
        delivery_worker_loop(&state),
    )
    .await;
}

async fn delivery_worker_loop(state: &AppState) {
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
        match state.db.active_community_hosts().await {
            Ok(communities) => {
                for community in communities {
                    let community = buzz_core::CommunityId::from_uuid(community.id);
                    let until = Utc::now() + TimeDelta::seconds(CLAIM_SECS);
                    match state.db.claim_due_push_wakes(community, 16, until).await {
                        Ok(wakes) => {
                            for wake in wakes {
                                found = true;
                                deliver_one(state, &http, wake).await;
                            }
                        }
                        // The community left `active` after the host scan; the
                        // next scan skips it, so this is not worth a warning.
                        Err(e @ buzz_db::DbError::AccessDenied(_)) => {
                            debug!(%community, "push wake claim refused: {e}")
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
    let body = match delivery_body(&outcome.endpoint_grant, outcome.id, outcome.expires_at) {
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
) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&DeliveryRequest {
        v: 1,
        endpoint_grant,
        request_id,
        expires_at,
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

    #[test]
    fn named_only_channel_pushes_only_to_a_filter_that_names_it() {
        let member = nostr::Keys::generate();
        let outsider = nostr::Keys::generate();
        let sender = nostr::Keys::generate();
        let legacy = uuid::Uuid::new_v4();
        let system = uuid::Uuid::new_v4();
        let future = uuid::Uuid::new_v4();
        let author = |keys: &nostr::Keys| keys.public_key().to_bytes().to_vec();
        let context = |keys: &nostr::Keys, filter: Value| MatchContext {
            leases: vec![buzz_db::push::MatchLease {
                author: author(keys),
                installation_id: "phone".into(),
                generation: 1,
                subscriptions: serde_json::json!([{"filter": filter, "class": "default"}]),
                expires_at: Utc::now().timestamp() + 3600,
            }],
            memberships: [legacy, system, future]
                .into_iter()
                .map(|channel| (channel, author(&member)))
                .collect(),
            named_only: NamedOnlyChannels::from_typed([
                (system, "system".to_owned()),
                (future, "later-type".to_owned()),
            ]),
        };
        let job = |channel: Option<uuid::Uuid>| {
            let mut event = EventBuilder::new(Kind::Custom(9), "hi");
            if let Some(channel) = channel {
                event = event.tag(Tag::parse(["h", &channel.to_string()]).unwrap());
            }
            buzz_db::push::BatchedMatch {
                event: buzz_core::StoredEvent::new(event.sign_with_keys(&sender).unwrap(), channel),
                attempt: 1,
            }
        };
        let wakes = |keys: &nostr::Keys, filter: Value, channel| {
            match_job(&job(channel), &context(keys, filter))
                .unwrap()
                .len()
        };
        let names =
            |channel: uuid::Uuid| serde_json::json!({"kinds": [9], "#h": [channel.to_string()]});
        let broad = serde_json::json!({"kinds": [9]});

        // A filter that names no channel reads legacy-type channels and
        // channel-less events, but no named-only channel.
        assert_eq!(wakes(&member, broad.clone(), Some(legacy)), 1, "legacy");
        assert_eq!(wakes(&member, broad.clone(), None), 1, "no channel");
        assert_eq!(
            wakes(&member, broad.clone(), Some(system)),
            0,
            "unnamed system"
        );
        assert_eq!(wakes(&member, broad, Some(future)), 0, "unnamed later type");

        // Naming a named-only channel by #h reads it, as in REQ: a system
        // channel and a later-type channel alike.
        assert_eq!(
            wakes(&member, names(future), Some(future)),
            1,
            "named later type"
        );
        assert_eq!(
            wakes(&member, names(system), Some(system)),
            1,
            "named system"
        );
        assert_eq!(
            wakes(&outsider, names(system), Some(system)),
            0,
            "non-member system"
        );
        // Naming one channel does not widen the filter to another.
        let other = uuid::Uuid::new_v4();
        assert_eq!(wakes(&member, names(other), Some(future)), 0, "other #h");
        // Membership is still required.
        assert_eq!(
            wakes(&outsider, names(future), Some(future)),
            0,
            "non-member"
        );
        assert_eq!(
            wakes(&outsider, names(legacy), Some(legacy)),
            0,
            "non-member legacy"
        );
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
            let body =
                delivery_body("opaque-grant", request_id, Utc::now().timestamp() + 60).unwrap();
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

#[cfg(test)]
mod shutdown_tests {
    use super::run_push_task;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn relay_shutdown_stops_both_worker_entry_points() {
        let mut state = crate::state::tests::test_state().await;
        let config = &mut Arc::get_mut(&mut state).unwrap().config;
        Arc::get_mut(config).unwrap().push_enabled = true;
        state.begin_shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(
                super::run_matcher(state.clone()),
                super::run_delivery_worker(state.clone())
            );
        })
        .await
        .expect("both real workers must honor shutdown before touching dependencies");
    }

    #[tokio::test]
    async fn disabled_or_cancelled_worker_never_polls_work() {
        for enabled in [false, true] {
            let cancel = CancellationToken::new();
            if enabled {
                cancel.cancel();
            }
            run_push_task(enabled, &cancel, async { panic!("worker must not start") }).await;
        }
    }

    #[tokio::test]
    async fn shutdown_drops_in_flight_work_without_waiting_for_it() {
        struct OnDrop(Arc<AtomicBool>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = OnDrop(dropped.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            run_push_task(true, &worker_cancel, async move {
                let _guard = guard;
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            })
            .await;
        });
        ready.await.unwrap();
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::{load_match_context, match_job};
    use nostr::{EventBuilder, Kind, Tag};

    /// The matcher on real channel rows: a member's lease that names no
    /// channel wakes for an ordinary channel's message but not for a system
    /// channel's. A member's lease that names the system channel by `#h`
    /// wakes for that channel only. A non-member's lease wakes for neither.
    /// Mutations: let system channel events reach a lease that does not name
    /// them → RED (the outsider or the broad lease wakes); never push system
    /// channels → RED (the named lease does not wake).
    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn system_channel_messages_wake_only_named_member_leases() {
        use crate::handlers::group_state_postgres_tests::{create_channel, http_auth};
        use crate::handlers::ingest::ingest_event;

        let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
            .await
            .expect("local Postgres and Redis");
        let host = format!("push-system-{}.local", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("ensure community")
            .id;
        let tenant = buzz_core::TenantContext::resolved(community, &host);
        let member = nostr::Keys::generate();
        let outsider = nostr::Keys::generate();

        let ordinary = create_channel(&state, &tenant, &member, &[]).await;
        let system = create_channel(&state, &tenant, &member, &[&["channel_type", "system"]]).await;

        let unnamed = serde_json::json!([{"filter": {"kinds": [9]}, "class": "default"}]);
        let named = serde_json::json!([{
            "filter": {"kinds": [9], "#h": [system.to_string()]},
            "class": "default"
        }]);
        let leases = [
            (&member, "unnamed", &unnamed),
            (&member, "named", &named),
            (&outsider, "unnamed", &unnamed),
        ];
        for (n, (keys, install, subscriptions)) in leases.into_iter().enumerate() {
            let source = [n as u8 + 1; 32];
            assert_eq!(
                buzz_db::push::replace_active_lease(
                    state.db.pool(),
                    community,
                    &keys.public_key().to_bytes(),
                    install,
                    buzz_db::push::LeaseVersion {
                        source_event_id: &source,
                        source_created_at: 10,
                        generation: 1,
                        expires_at: i64::MAX / 2,
                    },
                    buzz_db::push::ActiveLease {
                        endpoint_hash: &[n as u8 + 42; 32],
                        endpoint_grant: "opaque-grant",
                        max_class: "default",
                        subscriptions,
                    },
                )
                .await
                .expect("activate lease"),
                buzz_db::push::ReplaceLeaseOutcome::Accepted
            );
        }

        let mut jobs = Vec::new();
        for channel in [ordinary, system] {
            let event = EventBuilder::new(Kind::Custom(9), "hello")
                .tag(Tag::parse(["h", &channel.to_string()]).unwrap())
                .sign_with_keys(&member)
                .unwrap();
            let id = event.id;
            ingest_event(&state, &tenant, event, http_auth(&member))
                .await
                .expect("ingest message");
            let stored = state
                .db
                .get_event_by_id(community, id.as_bytes())
                .await
                .expect("load message")
                .expect("stored message");
            assert_eq!(stored.channel_id, Some(channel));
            jobs.push(buzz_db::push::BatchedMatch {
                event: stored,
                attempt: 1,
            });
        }
        let batch = buzz_db::push::ClaimedMatchBatch {
            community,
            claim_id: uuid::Uuid::new_v4(),
            jobs,
        };

        let context = load_match_context(&state, &batch)
            .await
            .expect("match context");
        let woken: Vec<Vec<(Vec<u8>, String)>> = batch
            .jobs
            .iter()
            .map(|job| {
                match_job(job, &context)
                    .expect("match")
                    .into_iter()
                    .map(|wake| (wake.author, wake.installation_id))
                    .collect()
            })
            .collect();
        let member_key = member.public_key().to_bytes().to_vec();
        assert_eq!(
            woken,
            vec![
                vec![(member_key.clone(), "unnamed".to_owned())],
                vec![(member_key, "named".to_owned())],
            ],
            "ordinary channel wakes its member's broad lease; system channel wakes only the lease that names it"
        );
    }
}
