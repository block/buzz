//! Fan-out of stored deliveries into per-channel messages.
//!
//! ```text
//!   deliveries (state = queued, next_attempt_at <= now)
//!        │ claim_due: lease rows, bump attempts
//!        ▼
//!   live subscriptions of the integration
//!        │ per subscription: filter? ── false → nothing
//!        │                           ── true  → the integration's message,
//!        │                                      rendered once per delivery
//!        │                           ── error → failed message (visible)
//!        ▼
//!   complete_fanout: messages + delivery → accepted, in one transaction,
//!   only if those are still the live subscriptions (else due again now)
//! ```
//!
//! CEL runs on a blocking thread: each program is parsed and evaluated on a
//! scoped thread with a large stack, and that must not stall the runtime.
use crate::filter::{
    compile_filter, compile_message_expr, Budget, CompileError, Delivery, EvalError,
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

/// The message sent when the integration has no message expression.
pub fn default_message(delivery: &ClaimedDelivery) -> String {
    format!("{}: {}", delivery.provider.as_str(), delivery.event_type)
}

/// Evaluate every subscription's filter against one delivery, and render the
/// integration's message once for all the subscriptions that match.
pub fn evaluate(delivery: &ClaimedDelivery, subscriptions: &[Subscription]) -> Evaluation {
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&delivery.body) else {
        return Evaluation::Settled(FanoutOutcome::Failed(
            "stored payload is not JSON".to_owned(),
        ));
    };
    let input = Delivery::new(delivery.provider.as_str(), &delivery.event_type, &payload);
    let mut message: Option<Result<String, String>> = None;
    let mut messages = Vec::new();
    for subscription in subscriptions {
        let content = match filter_matches(&input, subscription) {
            Ok(false) => continue,
            Ok(true) => match &message {
                Some(content) => content.clone(),
                None => match message_for(delivery, &input) {
                    Ok(content) => message.insert(content).clone(),
                    Err(error) => return Evaluation::Retry(error),
                },
            },
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

/// A program that compiled when it was written can still fail to compile
/// now; only a parser that could not start is worth a retry.
fn compile_failure(error: CompileError) -> EvalError {
    match error {
        CompileError::Unavailable(error) => EvalError::Unavailable(error),
        error => EvalError::Failed(error.to_string()),
    }
}

/// Programs were checked when they were written, so a compile error here is
/// reported as a failed message rather than trusted away.
fn filter_matches(input: &Delivery, subscription: &Subscription) -> Result<bool, EvalError> {
    match &subscription.filter {
        Some(source) => compile_filter(source)
            .map_err(compile_failure)?
            .matches(input, &BUDGET),
        None => Ok(true),
    }
}

/// The message every matching subscription stores: the rendered text, or why
/// rendering failed. `Err` is a reason to retry the whole delivery.
fn message_for(
    delivery: &ClaimedDelivery,
    input: &Delivery,
) -> Result<Result<String, String>, String> {
    let rendered = match &delivery.message_expr {
        Some(source) => compile_message_expr(source)
            .map_err(compile_failure)
            .and_then(|expr| expr.render(input, &BUDGET)),
        None => Ok(default_message(delivery)),
    };
    match rendered {
        Ok(text) if text.contains('\0') => Ok(Err("message contains a NUL character".to_owned())),
        Ok(text) => Ok(Ok(text)),
        Err(EvalError::Unavailable(error)) => Err(error),
        Err(error) => Ok(Err(storable(&error.to_string()))),
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
                    tracing::warn!(%error, "webhook gateway fan-out could not reach the store");
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
                    tracing::warn!(%error, "webhook gateway fan-out poll failed");
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
            tracing::warn!(%error, "webhook gateway fan-out deferred");
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
        DeliveryState, IntegrationConfig, MemoryStore, MessageKey, Mutation, NewDelivery,
        Operation, Provider, SubscriptionChange, SubscriptionStore, SubscriptionWrite,
    };
    use chrono::TimeZone;
    use uuid::Uuid;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap()
    }

    fn one_second_later() -> DateTime<Utc> {
        t0() + Duration::seconds(1)
    }

    async fn integration(store: &MemoryStore, message_expr: Option<&str>) -> Uuid {
        let integration_id = Uuid::new_v4();
        let operation = Operation {
            operation_id: Uuid::new_v4(),
            integration_id,
            revision: 1,
            mutation: Mutation::PutIntegration {
                community_host: "acme.communities.example.org".to_owned(),
                provider: Provider::Github,
                config: IntegrationConfig {
                    message_expr: message_expr.map(str::to_owned),
                    ..IntegrationConfig::default()
                },
            },
        };
        store.apply(operation, t0()).await.unwrap();
        integration_id
    }

    async fn subscribe(store: &MemoryStore, integration_id: Uuid, filter: Option<&str>) -> Uuid {
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
    async fn deliveries_fan_out_to_matching_subscriptions_with_the_integration_message() {
        let store = Arc::new(MemoryStore::default());
        let opened = Some(r#"payload.action == "opened""#);
        let plain = integration(&store, None).await;
        let everything = subscribe(&store, plain, None).await;
        let closed = subscribe(&store, plain, Some(r#"payload.action == "closed""#)).await;
        let broken = subscribe(&store, plain, Some("payload.missing == 1")).await;
        let templated = integration(
            &store,
            Some(r#""PR #" + string(payload.number) + ": " + payload.title"#),
        )
        .await;
        let shared = [
            subscribe(&store, templated, None).await,
            subscribe(&store, templated, opened).await,
        ];
        let unstorable = integration(&store, Some(r#""x" + payload.nul"#)).await;
        let nul = subscribe(&store, unstorable, opened).await;
        let body = br#"{"action":"opened","number":7,"title":"Fix it","nul":"\u0000"}"#;
        for integration_id in [plain, templated, unstorable] {
            deliver(&store, integration_id, "d-1", body).await;
        }
        deliver(&store, plain, "d-2", b"not json").await;

        let fanout = Fanout {
            store: store.clone(),
            concurrency: 4,
            retry_max_age: Duration::hours(24),
            now: one_second_later,
        };
        assert_eq!(fanout.run_once().await.unwrap(), 4);
        assert_eq!(fanout.run_once().await.unwrap(), 0, "settled exactly once");

        let delivery = store.delivery(plain, "d-1").unwrap();
        assert_eq!(delivery.state, DeliveryState::Accepted);
        assert!(delivery.delivery.body.is_empty());
        let message = |integration_id, subscription_id| {
            store.message(&MessageKey {
                integration_id,
                delivery_id: "d-1".to_owned(),
                subscription_id,
            })
        };
        let default = message(plain, everything).unwrap();
        assert_eq!(default.content, "github: pull_request.opened");
        assert_eq!(default.state, DeliveryState::Queued);
        assert_eq!(default.received_at, t0());
        assert!(message(plain, closed).is_none());
        let failed = message(plain, broken).unwrap();
        assert_eq!(failed.state, DeliveryState::Failed);
        assert!(failed.last_error.unwrap().contains("missing"));
        for subscription_id in shared {
            let rendered = message(templated, subscription_id).unwrap();
            assert_eq!(rendered.content, "PR #7: Fix it");
            assert_eq!(rendered.state, DeliveryState::Queued);
        }
        let refused = message(unstorable, nul).unwrap();
        assert_eq!(refused.state, DeliveryState::Failed);
        assert_eq!(
            refused.last_error.as_deref(),
            Some("message contains a NUL character")
        );

        let bad = store.delivery(plain, "d-2").unwrap();
        assert_eq!(bad.state, DeliveryState::Failed);
        assert_eq!(
            bad.last_error.as_deref(),
            Some("stored payload is not JSON")
        );
    }
}
