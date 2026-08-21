//! Durable NIP-PL event matcher and gateway delivery worker.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use base64::Engine as _;
use buzz_core::filter::{filters_match, reader_authorized_for_event};
use chrono::{TimeDelta, Utc};
use nostr::{EventBuilder, Filter, Kind, Tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tracing::{error, warn};

use crate::{handlers::push_lease::Subscription, state::AppState};

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

/// One lease plus the lazily prepared form of its immutable subscription JSON.
///
/// The cache lives in [`MatchContext`], so every event in one claimed batch
/// shares it, while the next batch still reloads the current lease snapshot.
struct PreparedMatchLease {
    lease: buzz_db::push::MatchLease,
    prepared: OnceLock<Result<Vec<PreparedSubscription>, String>>,
    #[cfg(test)]
    preparation_count: std::sync::atomic::AtomicUsize,
}

struct PreparedSubscription {
    filter: Filter,
    class: String,
    ignore: Vec<Filter>,
    suppress: Option<crate::handlers::push_lease::Suppress>,
}

impl PreparedMatchLease {
    fn new(lease: buzz_db::push::MatchLease) -> Self {
        Self {
            lease,
            prepared: OnceLock::new(),
            #[cfg(test)]
            preparation_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn subscriptions(&self) -> anyhow::Result<&[PreparedSubscription]> {
        self.prepared
            .get_or_init(|| {
                #[cfg(test)]
                self.preparation_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                prepare_subscriptions(&self.lease.subscriptions)
            })
            .as_deref()
            .map_err(|message| anyhow::anyhow!(message.clone()))
    }
}

fn prepare_subscriptions(
    raw_subscriptions: &serde_json::Value,
) -> Result<Vec<PreparedSubscription>, String> {
    let subscriptions: Vec<Subscription> =
        serde_json::from_value(raw_subscriptions.clone()).map_err(|error| error.to_string())?;
    subscriptions
        .into_iter()
        .map(|subscription| {
            let filter = serde_json::from_value(serde_json::Value::Object(subscription.filter))
                .map_err(|error| error.to_string())?;
            let ignore = subscription
                .ignore
                .into_iter()
                // Preserve the previous matcher semantics: malformed ignore
                // filters are non-matches rather than poison lease data.
                .filter_map(|raw| serde_json::from_value(serde_json::Value::Object(raw)).ok())
                .collect();
            Ok(PreparedSubscription {
                filter,
                class: subscription.class,
                ignore,
                suppress: subscription.suppress,
            })
        })
        .collect()
}

/// Per-batch state shared by every job: the community's active leases and
/// the exact (channel, lease author) membership pairs the jobs can consult.
struct MatchContext {
    leases: Vec<PreparedMatchLease>,
    memberships: std::collections::HashSet<(uuid::Uuid, Vec<u8>)>,
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
    Ok(MatchContext {
        leases: leases.into_iter().map(PreparedMatchLease::new).collect(),
        memberships,
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
        let event_id = job.event.event.id.as_bytes().to_vec();
        match match_job(job, &context) {
            Ok(job_wakes) if job_wakes.is_empty() => completed.push(event_id),
            Ok(job_wakes) => {
                pending.push((event_id, job.attempt));
                wakes.extend(job_wakes);
            }
            Err(e) => {
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
        Ok(_) => completed.extend(pending.into_iter().map(|(event_id, _)| event_id)),
        Err(e) => {
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
    for prepared_lease in &context.leases {
        let lease = &prepared_lease.lease;
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
        let mut class: Option<&str> = None;
        for sub in prepared_lease.subscriptions()? {
            if !push_filter_authorized_for_event(&sub.filter, &job.event.event, &author_hex)
                || !filters_match(std::slice::from_ref(&sub.filter), &job.event)
            {
                continue;
            }
            let ignored = sub
                .ignore
                .iter()
                .any(|filter| filters_match(std::slice::from_ref(filter), &job.event));
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
    let http = reqwest::Client::builder()
        .timeout(state.config.push_gateway_timeout)
        .build()
        .expect("push HTTP client");
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
            return;
        }
        Err(e) => {
            warn!(wake=%claimed.id, "push revalidation failed: {e}");
            return;
        }
    };
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
            return;
        }
        Err(e) => {
            warn!(wake=%outcome.id, "final push revalidation failed: {e}");
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
            return;
        }
    };
    let Some(url) = state.config.push_gateway_delivery_url.as_ref() else {
        return;
    };
    let body = delivery_body(&outcome.endpoint_grant, outcome.id, outcome.expires_at);
    let auth = match nip98_header(&state.relay_keypair, url.as_str(), &body) {
        Ok(auth) => auth,
        Err(e) => {
            warn!(wake=%outcome.id, "push auth failed: {e}");
            return;
        }
    };
    if let Err(error) = serving_write.verify().await {
        warn!(wake=%outcome.id, %error, "push serving lease lost before delivery");
        return;
    }
    let response = match serving_write
        .protect(send_gateway_request(http, url, body, auth))
        .await
    {
        Ok(response) => response,
        Err(error) => {
            warn!(wake=%outcome.id, %error, "push serving lease lost during delivery");
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
            }
            _ => {
                let _ = state
                    .db
                    .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                    .await;
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
            retry_or_fail(state, &outcome, delay).await;
        }
        Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            retry_or_fail(state, &outcome, 2).await
        }
        // A timed-out terminal attempt burns the stable request id. Its replay
        // is indistinguishable from another invalid-grant 404, but sending a
        // fresh id would double-deliver and defeat the gateway replay fence.
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND && outcome.attempt > 1 => {
            let _ = state
                .db
                .complete_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
        }
        Err(e) if e.is_timeout() || e.is_connect() => retry_or_fail(state, &outcome, 2).await,
        _ => {
            let _ = state
                .db
                .fail_push_wake(outcome.community, outcome.id, outcome.claim_id)
                .await;
        }
    }
    if let Err(error) = serving_write.finish().await {
        warn!(wake=%outcome.id, %error, "failed to release community serving lease after push delivery");
    }
}

fn delivery_body(endpoint_grant: &str, request_id: uuid::Uuid, expires_at: i64) -> Vec<u8> {
    serde_json::to_vec(&DeliveryRequest {
        v: 1,
        endpoint_grant,
        request_id,
        expires_at,
    })
    .expect("closed delivery body")
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

async fn retry_or_fail(state: &AppState, wake: &buzz_db::push::ClaimedWake, delay: i64) {
    if wake.attempt >= MAX_ATTEMPTS {
        let _ = state
            .db
            .fail_push_wake(wake.community, wake.id, wake.claim_id)
            .await;
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
    }
}

fn nip98_header(keys: &nostr::Keys, url: &str, body: &[u8]) -> anyhow::Result<String> {
    let hash = hex::encode(Sha256::digest(body));
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", url])?,
            Tag::parse(["method", "POST"])?,
            Tag::parse(["payload", &hash])?,
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()])?,
        ])
        .sign_with_keys(keys)?;
    Ok(format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event)?)
    ))
}

fn class_rank(class: &str) -> u8 {
    match class {
        "silent" => 0,
        "default" => 1,
        "time_sensitive" => 2,
        "urgent" => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use serde_json::Value;
    use std::{future::IntoFuture, sync::Arc};
    use tokio::sync::Mutex;

    fn match_lease(author: &nostr::Keys, subscriptions: serde_json::Value) -> PreparedMatchLease {
        PreparedMatchLease::new(buzz_db::push::MatchLease {
            author: author.public_key().to_bytes().to_vec(),
            installation_id: "device-1".to_owned(),
            generation: 7,
            subscriptions,
            expires_at: Utc::now().timestamp() + 600,
        })
    }

    fn match_job_for(
        author: &nostr::Keys,
        channel_id: Option<uuid::Uuid>,
    ) -> buzz_db::push::BatchedMatch {
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(author)
            .unwrap();
        buzz_db::push::BatchedMatch {
            event: buzz_core::StoredEvent::new(event, channel_id),
            attempt: 1,
        }
    }

    fn valid_subscriptions(ignore: serde_json::Value) -> serde_json::Value {
        serde_json::json!([{
            "filter": {"kinds": [1]},
            "class": "urgent",
            "ignore": [ignore],
            "suppress": null
        }])
    }

    #[test]
    fn match_context_prepares_a_reached_lease_once_for_multiple_jobs() {
        let author = nostr::Keys::generate();
        let context = MatchContext {
            leases: vec![match_lease(
                &author,
                serde_json::json!([{
                    "filter": {"kinds": [1]},
                    "class": "urgent",
                    "ignore": [],
                    "suppress": null
                }]),
            )],
            memberships: Default::default(),
        };

        let first = match_job(&match_job_for(&author, None), &context).unwrap();
        let second = match_job(&match_job_for(&author, None), &context).unwrap();

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].class, "urgent");
        assert_eq!(second[0].class, "urgent");
        assert_eq!(
            context.leases[0]
                .preparation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn malformed_lease_is_parsed_only_after_authorization_and_membership() {
        let owner = nostr::Keys::generate();
        let other = nostr::Keys::generate();
        let private_event =
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_DM_VISIBILITY as u16), "")
                .tag(Tag::public_key(other.public_key()))
                .sign_with_keys(&other)
                .unwrap();
        let unauthorized = buzz_db::push::BatchedMatch {
            event: buzz_core::StoredEvent::new(private_event, None),
            attempt: 1,
        };
        let channel = uuid::Uuid::new_v4();
        let context = MatchContext {
            leases: vec![match_lease(&owner, serde_json::json!({"malformed": true}))],
            memberships: Default::default(),
        };

        assert!(match_job(&unauthorized, &context).unwrap().is_empty());
        assert_eq!(
            context.leases[0]
                .preparation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(match_job(&match_job_for(&owner, Some(channel)), &context)
            .unwrap()
            .is_empty());
        assert_eq!(
            context.leases[0]
                .preparation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(match_job(&match_job_for(&owner, None), &context).is_err());
        assert_eq!(
            context.leases[0]
                .preparation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(match_job(&match_job_for(&owner, None), &context).is_err());
        assert_eq!(
            context.leases[0]
                .preparation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "preparation failures must be cached too"
        );
    }

    #[test]
    fn malformed_ignore_filter_remains_a_non_match() {
        let owner = nostr::Keys::generate();
        let context = MatchContext {
            leases: vec![match_lease(
                &owner,
                valid_subscriptions(serde_json::json!({"kinds": "not-an-array"})),
            )],
            memberships: Default::default(),
        };

        let wakes = match_job(&match_job_for(&owner, None), &context).unwrap();

        assert_eq!(wakes.len(), 1);
        assert_eq!(wakes[0].class, "urgent");
    }

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
        Json(body): Json<Value>,
    ) -> Json<Value> {
        seen.lock().await.push(body);
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
                    .route("/deliver", post(capture))
                    .with_state(seen.clone()),
            )
            .into_future(),
        );
        let url: url::Url = format!("http://{address}/deliver").parse().unwrap();
        let http = reqwest::Client::new();
        let keys = nostr::Keys::generate();
        let request_id = uuid::Uuid::new_v4();
        for _ in 0..2 {
            let body = delivery_body("opaque-grant", request_id, Utc::now().timestamp() + 60);
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
