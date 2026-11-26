//! Fan-out of stored deliveries into per-channel messages.
//!
//! ```text
//!   deliveries (state = queued, next_attempt_at <= now)
//!        │ claim_due: lease rows, bump attempts
//!        ▼
//!   live subscriptions of the integration
//!        │ per subscription: filter? ── false → nothing
//!        │                           ── true  → render message template
//!        │                           ── error → failed message (visible)
//!        ▼
//!   complete_fanout: messages + delivery → accepted, in one transaction,
//!   only if those are still the live subscriptions (else due again now)
//! ```
//!
//! CEL runs on a blocking thread: each program is parsed and evaluated on a
//! scoped thread with a large stack, and that must not stall the runtime.
use crate::filter::{
    compile_filter, compile_message_template, Budget, CompileError, Delivery, EvalError,
};
use crate::metrics::{record_fanout, record_forwarder_failure, FanoutResult};
use crate::store::{
    storable, ClaimedDelivery, Evaluated, FanoutCommit, FanoutOutcome, NewMessage, Store,
    StoreError, Subscription,
};
use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;
use tokio::task::JoinSet;

const CLAIM_LEASE: Duration = Duration::seconds(60);
const IDLE_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// The limits on each subscription program. A message is capped at the
/// outbox column's 16 KiB.
pub const BUDGET: Budget = Budget {
    max_iterations: 10_000,
    timeout: std::time::Duration::from_millis(100),
    max_output_bytes: 16 * 1024,
};

/// What evaluating one delivery against its subscriptions produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Evaluation {
    Settled(FanoutOutcome),
    /// The evaluator itself was unavailable; the lease returns the delivery.
    Retry(String),
}

/// The message a subscription without a message template sends.
pub fn default_message(delivery: &ClaimedDelivery) -> String {
    format!("{}: {}", delivery.provider.as_str(), delivery.event_type)
}

/// Evaluate every subscription's programs against one delivery.
pub fn evaluate(delivery: &ClaimedDelivery, subscriptions: &[Subscription]) -> Evaluation {
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&delivery.body) else {
        return Evaluation::Settled(FanoutOutcome::Failed(
            "stored payload is not JSON".to_owned(),
        ));
    };
    let input = Delivery::new(delivery.provider.as_str(), &delivery.event_type, &payload);
    let mut messages = Vec::new();
    for subscription in subscriptions {
        let content = match render(delivery, &input, subscription) {
            Ok(None) => continue,
            Ok(Some(text)) if text.contains('\0') => {
                Err("message contains a NUL character".to_owned())
            }
            Ok(Some(text)) => Ok(text),
            Err(EvalError::Unavailable(error)) => return Evaluation::Retry(error),
            Err(error) => Err(storable(&error.to_string())),
        };
        messages.push(NewMessage {
            subscription_id: subscription.id,
            channel_id: subscription.channel_id,
            content,
        });
    }
    Evaluation::Settled(FanoutOutcome::Fanned {
        messages,
        evaluated: Evaluated::of(subscriptions),
    })
}

/// A program that compiled when its subscription was written can still fail
/// to compile now; only a parser that could not start is worth a retry.
fn compile_failure(error: CompileError) -> EvalError {
    match error {
        CompileError::Unavailable(error) => EvalError::Unavailable(error),
        error => EvalError::Failed(error.to_string()),
    }
}

/// `None` when the filter does not match. Programs were checked when the
/// subscription was written, so a compile error here is reported as a failed
/// message rather than trusted away.
fn render(
    delivery: &ClaimedDelivery,
    input: &Delivery,
    subscription: &Subscription,
) -> Result<Option<String>, EvalError> {
    if let Some(source) = &subscription.filter {
        if !compile_filter(source)
            .map_err(compile_failure)?
            .matches(input, &BUDGET)?
        {
            return Ok(None);
        }
    }
    match &subscription.message_template {
        Some(source) => compile_message_template(source)
            .map_err(compile_failure)?
            .render(input, &BUDGET)
            .map(Some),
        None => Ok(Some(default_message(delivery))),
    }
}

/// Drains queued deliveries into the message outbox.
pub struct Fanout {
    pub store: Arc<dyn Store>,
    /// Deliveries claimed and evaluated per poll.
    pub concurrency: usize,
    /// How long after receipt a delivery keeps retrying before it fails.
    pub retry_max_age: Duration,
    /// Injectable clock.
    pub now: fn() -> DateTime<Utc>,
}

impl Fanout {
    /// Claim one batch and fan it out. Returns how many deliveries were claimed.
    pub async fn run_once(&self) -> Result<usize, StoreError> {
        let now = (self.now)();
        let batch = self
            .store
            .claim_due(now, CLAIM_LEASE, self.concurrency.max(1))
            .await?;
        let claimed = batch.len();
        let mut tasks = JoinSet::new();
        for delivery in batch {
            let store = Arc::clone(&self.store);
            let retry_max_age = self.retry_max_age;
            let now_fn = self.now;
            tasks.spawn(async move {
                if let Err(error) = fan_out(&*store, delivery, retry_max_age, now_fn).await {
                    record_forwarder_failure();
                    tracing::warn!(%error, "ingress gateway fan-out could not reach the store");
                }
            });
        }
        while tasks.join_next().await.is_some() {}
        Ok(claimed)
    }

    /// Poll the store until the task is aborted.
    pub async fn run(self) {
        loop {
            match self.run_once().await {
                Ok(0) => tokio::time::sleep(IDLE_POLL).await,
                Ok(_) => {}
                Err(error) => {
                    record_forwarder_failure();
                    tracing::warn!(%error, "ingress gateway fan-out poll failed");
                    tokio::time::sleep(IDLE_POLL).await;
                }
            }
        }
    }
}

async fn fan_out(
    store: &dyn Store,
    delivery: ClaimedDelivery,
    retry_max_age: Duration,
    now: fn() -> DateTime<Utc>,
) -> Result<(), StoreError> {
    let subscriptions = store.live_subscriptions(delivery.integration_id).await?;
    let (integration_id, delivery_id, received_at, attempts) = (
        delivery.integration_id,
        delivery.delivery_id.clone(),
        delivery.received_at,
        delivery.attempts,
    );
    let evaluation = tokio::task::spawn_blocking(move || evaluate(&delivery, &subscriptions))
        .await
        .unwrap_or_else(|e| Evaluation::Retry(format!("evaluator task failed: {e}")));
    let outcome = match evaluation {
        Evaluation::Settled(outcome) => outcome,
        Evaluation::Retry(error) if received_at + retry_max_age <= now() => {
            FanoutOutcome::Failed(format!("retry budget exhausted: {error}"))
        }
        Evaluation::Retry(error) => {
            // Leave the row leased; it returns when the lease ends.
            record_fanout(FanoutResult::Retry);
            tracing::warn!(%error, "ingress gateway fan-out deferred");
            return Ok(());
        }
    };
    let result = match &outcome {
        FanoutOutcome::Fanned { messages, .. } => {
            FanoutResult::Fanned(u64::try_from(messages.len()).unwrap_or(u64::MAX))
        }
        FanoutOutcome::Failed(_) => FanoutResult::Failed,
    };
    match store
        .complete_fanout(integration_id, &delivery_id, attempts, outcome, now())
        .await?
    {
        FanoutCommit::Settled => record_fanout(result),
        // The subscriptions changed under the evaluation; it runs again now.
        FanoutCommit::Stale => record_fanout(FanoutResult::Retry),
        FanoutCommit::Lost => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        DeliveryState, MemoryStore, MessageKey, Mutation, NewDelivery, Operation, Provider,
        SubscriptionChange, SubscriptionStore, SubscriptionWrite,
    };
    use chrono::TimeZone;
    use uuid::Uuid;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap()
    }

    fn one_second_later() -> DateTime<Utc> {
        t0() + Duration::seconds(1)
    }

    async fn subscribe(
        store: &MemoryStore,
        integration_id: Uuid,
        filter: Option<&str>,
        message_template: Option<&str>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        store
            .write_subscription(
                SubscriptionWrite {
                    id,
                    integration_id,
                    revision: 1,
                    change: SubscriptionChange::Put {
                        channel_id: Uuid::new_v4(),
                        filter: filter.map(str::to_owned),
                        message_template: message_template.map(str::to_owned),
                    },
                },
                t0(),
            )
            .await
            .unwrap();
        id
    }

    async fn deliver(store: &MemoryStore, integration_id: Uuid, id: &str, body: &[u8]) {
        store
            .insert_delivery(NewDelivery {
                integration_id,
                delivery_id: id.to_owned(),
                event_type: "pull_request.opened".to_owned(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: body.to_vec(),
                received_at: t0(),
            })
            .await
            .unwrap();
    }

    #[test]
    fn only_a_parser_that_could_not_start_is_retried() {
        assert!(matches!(
            compile_failure(CompileError::Unavailable("no threads".to_owned())),
            EvalError::Unavailable(_)
        ));
        assert!(matches!(
            compile_failure(CompileError::Empty),
            EvalError::Failed(_)
        ));
    }

    #[tokio::test]
    async fn deliveries_fan_out_to_matching_subscriptions_only() {
        let store = Arc::new(MemoryStore::default());
        let integration_id = Uuid::new_v4();
        store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id,
                    revision: 1,
                    mutation: Mutation::PutIntegration {
                        community_host: "acme.communities.example.org".to_owned(),
                        provider: Provider::Github,
                    },
                },
                t0(),
            )
            .await
            .unwrap();
        let everything = subscribe(&store, integration_id, None, None).await;
        let opened = subscribe(
            &store,
            integration_id,
            Some(r#"payload.action == "opened""#),
            Some(r#""PR #" + string(payload.number) + ": " + payload.title"#),
        )
        .await;
        let closed = subscribe(
            &store,
            integration_id,
            Some(r#"payload.action == "closed""#),
            None,
        )
        .await;
        let broken = subscribe(&store, integration_id, Some("payload.missing == 1"), None).await;
        let unstorable =
            subscribe(&store, integration_id, None, Some(r#""x" + payload.nul"#)).await;
        deliver(
            &store,
            integration_id,
            "d-1",
            br#"{"action":"opened","number":7,"title":"Fix it","nul":"\u0000"}"#,
        )
        .await;
        deliver(&store, integration_id, "d-2", b"not json").await;

        let fanout = Fanout {
            store: store.clone(),
            concurrency: 4,
            retry_max_age: Duration::hours(24),
            now: one_second_later,
        };
        assert_eq!(fanout.run_once().await.unwrap(), 2);
        assert_eq!(fanout.run_once().await.unwrap(), 0, "settled exactly once");

        let delivery = store.delivery(integration_id, "d-1").unwrap();
        assert_eq!(delivery.state, DeliveryState::Accepted);
        assert!(delivery.delivery.body.is_empty());
        let key = |subscription_id| MessageKey {
            integration_id,
            delivery_id: "d-1".to_owned(),
            subscription_id,
        };
        let plain = store.message(&key(everything)).unwrap();
        assert_eq!(plain.content, "github: pull_request.opened");
        assert_eq!(plain.state, DeliveryState::Queued);
        assert_eq!(plain.received_at, t0());
        assert_eq!(
            store.message(&key(opened)).unwrap().content,
            "PR #7: Fix it"
        );
        assert!(store.message(&key(closed)).is_none());
        let failed = store.message(&key(broken)).unwrap();
        assert_eq!(failed.state, DeliveryState::Failed);
        assert!(failed.last_error.unwrap().contains("missing"));
        let nul = store.message(&key(unstorable)).unwrap();
        assert_eq!(
            nul.state,
            DeliveryState::Failed,
            "only this subscription fails"
        );
        assert_eq!(
            nul.last_error.as_deref(),
            Some("message contains a NUL character")
        );

        let bad = store.delivery(integration_id, "d-2").unwrap();
        assert_eq!(bad.state, DeliveryState::Failed);
        assert_eq!(
            bad.last_error.as_deref(),
            Some("stored payload is not JSON")
        );
    }
}
