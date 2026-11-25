//! NIP-WH subscription worker: turns queued deliveries into records.
//!
//! Every pod runs one worker while delivery intake is on. It sweeps the
//! communities with due deliveries, claims a few from each, and for each
//! delivery evaluates the current subscriptions to its integration:
//!
//! 1. A revision whose signer is no longer an owner or admin of its channel is
//!    recorded as `skipped_signer`.
//! 2. Otherwise its filter, then its transform, run within [`BUDGET`]. A
//!    content, compile or evaluation failure is recorded as `error`, for that
//!    subscription only. A filter that does not match records nothing.
//! 3. A match stores a relay-signed record in the channel, in the same
//!    transaction as its outcome, and fans it out.
//!
//! An outcome is recorded once per delivery and subscription, so a retry
//! evaluates only the subscriptions an earlier attempt did not finish, and
//! never publishes a second record. A fan-out lost to a crash is not retried:
//! the record is stored, and readers find it.

use std::sync::Arc;
use std::time::Duration;

use buzz_core::kind::KIND_WEBHOOK_RECORD;
use buzz_core::tenant::TenantContext;
use buzz_db::webhook::{ClaimedDelivery, Outcome, Recorded, SubscriptionHead};
use buzz_webhook_filter::{Budget, CompileError, Delivery, EvalError};
use nostr::{Event, EventBuilder, Kind, Tag};
use serde::Deserialize;
use tracing::{debug, error, warn};

use crate::state::AppState;

/// Limits on one filter or transform. Each evaluation allocates until its
/// deadline at worst, about 1.6 MB a millisecond, so the deadline is short:
/// a filter over a delivery takes microseconds.
pub const BUDGET: Budget = Budget {
    max_iterations: 10_000,
    timeout: Duration::from_millis(20),
    max_output_bytes: 64 * 1024,
};
/// Deliveries claimed from one community per turn, so a busy community
/// cannot starve the others.
const CLAIM_BATCH: i64 = 16;
/// How long a claim lasts before another worker may take the delivery.
const LEASE: Duration = Duration::from_secs(60);
/// Attempts before a delivery fails for good.
const MAX_ATTEMPTS: i32 = 8;
/// Longest wait before retrying a delivery.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);
/// Idle poll floor and ceiling.
const IDLE_FLOOR: Duration = Duration::from_millis(250);
const IDLE_CEILING: Duration = Duration::from_secs(2);
/// How long finished deliveries are kept, so a gateway resending one it
/// already delivered is still a duplicate.
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How often finished deliveries are deleted, and how many per statement.
const REAP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const REAP_BATCH: i64 = 1_000;

/// The parts of the delivery content the programs read. Intake checked the
/// rest.
#[derive(Deserialize)]
struct DeliveryContent {
    #[serde(rename = "type")]
    event_type: String,
    payload: serde_json::Value,
}

/// Why a delivery could not finish this attempt.
enum Failure {
    /// Retry later: the database or the evaluator was unavailable.
    Transient(String),
    /// Retrying cannot help.
    Permanent(String),
}

/// Run the worker until the process exits.
pub async fn run(state: Arc<AppState>) {
    let mut idle = IDLE_FLOOR;
    let mut last_reap: Option<tokio::time::Instant> = None;
    loop {
        if last_reap.is_none_or(|at| at.elapsed() >= REAP_INTERVAL) {
            reap(&state).await;
            last_reap = Some(tokio::time::Instant::now());
        }
        if sweep(&state).await > 0 {
            idle = IDLE_FLOOR;
        } else {
            tokio::time::sleep(idle).await;
            idle = (idle * 2).min(IDLE_CEILING);
        }
    }
}

/// Process the due deliveries of every community once, a batch from each.
/// Returns how many deliveries were claimed.
pub(crate) async fn sweep(state: &Arc<AppState>) -> usize {
    let mut claimed = 0;
    let mut after = None;
    loop {
        let page = match state.db.due_webhook_delivery_communities(after).await {
            Ok(page) if page.is_empty() => break,
            Ok(page) => page,
            Err(e) => {
                error!("webhook worker discovery failed: {e}");
                break;
            }
        };
        after = page.last().map(|(community, _)| *community);
        for (community, host) in page {
            let tenant = TenantContext::resolved(community, host);
            match state
                .db
                .claim_webhook_deliveries(community, CLAIM_BATCH, LEASE)
                .await
            {
                Ok(deliveries) => {
                    claimed += deliveries.len();
                    for delivery in deliveries {
                        process(state, &tenant, delivery).await;
                    }
                }
                // Discovery is advisory; admission still enforces lifecycle.
                Err(e @ buzz_db::DbError::AccessDenied(_)) => {
                    debug!(%community, "webhook delivery claim refused: {e}")
                }
                Err(e) => warn!(%community, "webhook delivery claim failed: {e}"),
            }
        }
    }
    claimed
}

async fn process(state: &Arc<AppState>, tenant: &TenantContext, delivery: ClaimedDelivery) {
    let settled = match evaluate_delivery(state, tenant, &delivery).await {
        Ok(()) => {
            job("done");
            state.db.finish_webhook_delivery(&delivery).await
        }
        Err(failure) => {
            let (error, give_up) = match failure {
                Failure::Transient(error) => (error, delivery.attempts >= MAX_ATTEMPTS),
                Failure::Permanent(error) => (error, true),
            };
            warn!(
                community = %delivery.community,
                delivery = %delivery.event.id,
                attempts = delivery.attempts,
                give_up,
                "webhook delivery failed: {error}"
            );
            job(if give_up { "failed" } else { "retry" });
            state
                .db
                .retry_webhook_delivery(&delivery, retry_delay(delivery.attempts), give_up, &error)
                .await
        }
    };
    match settled {
        Ok(true) => {}
        // A newer claim owns it: this one outlived its lease.
        Ok(false) => debug!(delivery = %delivery.event.id, "webhook delivery claim was superseded"),
        // The lease expires and another attempt resumes from the recorded outcomes.
        Err(e) => warn!(delivery = %delivery.event.id, "webhook delivery settle failed: {e}"),
    }
}

/// 1 s after the first attempt, doubling up to [`MAX_RETRY_DELAY`].
fn retry_delay(attempts: i32) -> Duration {
    let doublings = u32::try_from(attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(16);
    Duration::from_secs(1 << doublings).min(MAX_RETRY_DELAY)
}

async fn evaluate_delivery(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    delivery: &ClaimedDelivery,
) -> Result<(), Failure> {
    let content: DeliveryContent = serde_json::from_str(&delivery.event.content)
        .map_err(|e| Failure::Permanent(format!("delivery content: {e}")))?;
    let heads = state
        .db
        .webhook_subscriptions_to_evaluate(
            delivery.community,
            delivery.integration,
            &delivery.event.id,
        )
        .await
        .map_err(|e| Failure::Transient(format!("loading subscriptions: {e}")))?;
    if heads.is_empty() {
        return Ok(());
    }
    let input = Arc::new(Delivery::new(
        &delivery.provider,
        &content.event_type,
        &content.payload,
    ));
    for head in heads {
        let evaluated = if head.signer_is_admin {
            evaluate(Arc::clone(&input), head.revision.event.clone()).await?
        } else {
            Evaluated::SkippedSigner
        };
        let record;
        let outcome = match &evaluated {
            Evaluated::NoMatch => {
                evaluation("no_match");
                continue;
            }
            Evaluated::SkippedSigner => Outcome::SkippedSigner,
            Evaluated::Error(error) => Outcome::Error(error),
            Evaluated::Matched(payload) => {
                record = record_event(
                    state,
                    delivery,
                    &content.event_type,
                    &head,
                    payload.as_ref(),
                )
                .map_err(Failure::Transient)?;
                Outcome::Matched(&record)
            }
        };
        let recorded = state
            .db
            .record_webhook_outcome(delivery.community, &delivery.event.id, &head, outcome)
            .await
            .map_err(|e| Failure::Transient(format!("recording outcome: {e}")))?;
        evaluation(match (&evaluated, &recorded) {
            (_, Recorded::Already) => "already_recorded",
            (_, Recorded::SkippedChannel) => "skipped_channel",
            (Evaluated::Matched(_), _) => "matched",
            (Evaluated::Error(_), _) => "error",
            _ => "skipped_signer",
        });
        if let Recorded::New(Some(stored)) = recorded {
            crate::handlers::event::dispatch_persistent_event(
                tenant,
                state,
                &stored,
                KIND_WEBHOOK_RECORD,
                &state.relay_keypair.public_key().to_hex(),
                None,
            )
            .await;
        }
    }
    Ok(())
}

/// One subscription's result for one delivery.
enum Evaluated {
    /// The filter matched, with the transform's result when there is one.
    Matched(Option<serde_json::Map<String, serde_json::Value>>),
    NoMatch,
    /// The revision's content, compile or evaluation failed.
    Error(String),
    SkippedSigner,
}

/// Parse, compile and run one revision's programs off the async workers.
async fn evaluate(input: Arc<Delivery>, revision: Event) -> Result<Evaluated, Failure> {
    tokio::task::spawn_blocking(move || {
        let (filter, transform) = match crate::handlers::webhook_subscription::programs(&revision) {
            Ok(programs) => programs,
            Err(error) => return Ok(Evaluated::Error(error)),
        };
        let matched = buzz_webhook_filter::compile_filter(&filter)
            .map_err(|e| compile_failure("filter", e))
            .and_then(|filter| {
                filter
                    .matches(&input, &BUDGET)
                    .map_err(|e| eval_failure("filter", e))
            });
        match matched {
            Ok(false) => return Ok(Evaluated::NoMatch),
            Ok(true) => {}
            Err(ProgramFailure::Unavailable(error)) => return Err(Failure::Transient(error)),
            Err(ProgramFailure::Program(error)) => return Ok(Evaluated::Error(error)),
        }
        let Some(transform) = transform else {
            return Ok(Evaluated::Matched(None));
        };
        match buzz_webhook_filter::compile_transform(&transform)
            .map_err(|e| compile_failure("transform", e))
            .and_then(|transform| {
                transform
                    .apply(&input, &BUDGET)
                    .map_err(|e| eval_failure("transform", e))
            }) {
            Ok(payload) => Ok(Evaluated::Matched(Some(payload))),
            Err(ProgramFailure::Unavailable(error)) => Err(Failure::Transient(error)),
            Err(ProgramFailure::Program(error)) => Ok(Evaluated::Error(error)),
        }
    })
    .await
    .map_err(|e| Failure::Transient(format!("evaluation task failed: {e}")))?
}

enum ProgramFailure {
    Unavailable(String),
    Program(String),
}

fn compile_failure(field: &str, error: CompileError) -> ProgramFailure {
    match error {
        CompileError::Unavailable(_) => ProgramFailure::Unavailable(error.to_string()),
        error => ProgramFailure::Program(format!("{field}: {error}")),
    }
}

fn eval_failure(field: &str, error: EvalError) -> ProgramFailure {
    match error {
        EvalError::Unavailable(_) => ProgramFailure::Unavailable(error.to_string()),
        error => ProgramFailure::Program(format!("{field}: {error}")),
    }
}

/// The relay-signed record of `head` matching `delivery`.
fn record_event(
    state: &AppState,
    delivery: &ClaimedDelivery,
    event_type: &str,
    head: &SubscriptionHead,
    payload: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<Event, String> {
    let mut content = serde_json::json!({
        "version": 1,
        "type": event_type,
        "delivery_event_id": delivery.event.id.to_hex(),
        "subscription_id": head.subscription_id.to_string(),
        "subscription_revision": head.revision.event.id.to_hex(),
    });
    if let Some(payload) = payload {
        content["payload"] = serde_json::Value::Object(payload.clone());
    }
    let tags = [
        ["h".to_owned(), head.channel_id.to_string()],
        ["integration".to_owned(), delivery.integration.to_string()],
        ["t".to_owned(), format!("provider:{}", delivery.provider)],
        [
            "t".to_owned(),
            format!("subscription:{}", head.subscription_id),
        ],
    ];
    let kind = u16::try_from(KIND_WEBHOOK_RECORD).map_err(|e| e.to_string())?;
    EventBuilder::new(Kind::Custom(kind), content.to_string())
        .tags(
            tags.into_iter()
                .map(Tag::parse)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("record tags: {e}"))?,
        )
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| format!("signing record: {e}"))
}

/// Delete finished deliveries past [`RETENTION`] in every community.
pub(crate) async fn reap(state: &AppState) {
    let Ok(retention) = chrono::TimeDelta::from_std(RETENTION) else {
        return;
    };
    let before = chrono::Utc::now() - retention;
    let mut after = None;
    loop {
        let page = match state.db.reapable_webhook_communities(before, after).await {
            Ok(page) if page.is_empty() => return,
            Ok(page) => page,
            Err(e) => {
                warn!("webhook delivery retention discovery failed: {e}");
                return;
            }
        };
        after = page.last().copied();
        for community in page {
            loop {
                match state
                    .db
                    .reap_webhook_deliveries(community, before, REAP_BATCH)
                    .await
                {
                    Ok(reaped) if reaped == REAP_BATCH as u64 => continue,
                    Ok(_) => break,
                    Err(e) => {
                        warn!(%community, "webhook delivery retention failed: {e}");
                        break;
                    }
                }
            }
        }
    }
}

fn job(result: &'static str) {
    metrics::counter!("buzz_webhook_delivery_jobs_total", "result" => result).increment(1);
}

fn evaluation(result: &'static str) {
    metrics::counter!("buzz_webhook_subscription_evaluations_total", "result" => result)
        .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_to_a_ceiling() {
        let delays: Vec<u64> = [1, 2, 3, 9, 10, i32::MAX]
            .into_iter()
            .map(|attempt| retry_delay(attempt).as_secs())
            .collect();
        assert_eq!(delays, [1, 2, 4, 256, 300, 300]);
    }
}
