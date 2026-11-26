//! Background delivery of queued channel messages to community relays.
//!
//! ```text
//!   messages (state = queued, next_attempt_at <= now)
//!        │ claim_due_messages: lease rows, bump attempts, signed_at ??= now
//!        ▼
//!   MessageSigner::signer_for(target) ── the author of the message
//!        │
//!        ├─ signed_at fresh ── build kind 9 at signed_at: the same id on
//!        │                     every retry, so the relay dedupes a resend
//!        │
//!        └─ signed_at stale ── the relay would refuse it, so ask whether an
//!                              earlier send landed:
//!                                found   → state accepted
//!                                missing → Restamp: due now, re-signed fresh
//!        ▼
//!   RelayClient::submit ──▶ Accepted  → state accepted
//!                       ──▶ Rejected  → state failed
//!                       ──▶ Retry     → next_attempt_at = backoff(attempts),
//!                                       or failed once the budget is spent
//! ```
mod event;
#[cfg(test)]
mod tests;

pub use event::{
    build_message_event, message_event_id, message_tag, relay_url, CHANNEL_MESSAGE_KIND,
    MESSAGE_TAG,
};

use crate::config::RelayScheme;
use crate::metrics::{record_forward, record_forwarder_failure, ForwardResult};
use crate::relay_client::{LookupResult, RelayClient, SubmitResult};
use crate::signer::{MessageSigner, SignerError, SigningTarget};
use crate::store::{ClaimedMessage, ForwardOutcome, Store, StoreError};
use chrono::{DateTime, Duration, Utc};
use nostr::{NostrSigner, Url};
use std::sync::Arc;
use tokio::task::JoinSet;

const BACKOFF: [Duration; 6] = [
    Duration::seconds(1),
    Duration::seconds(5),
    Duration::seconds(30),
    Duration::minutes(2),
    Duration::minutes(10),
    Duration::minutes(30),
];
const CLAIM_LEASE: Duration = Duration::seconds(60);
const IDLE_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// How long one `signed_at` is used. The relay refuses an event whose
/// `created_at` is more than 15 minutes from its clock, duplicates included;
/// the margin covers clock skew and the time from claim to send.
pub const RESTAMP_AFTER: Duration = Duration::minutes(10);

/// Delay before the next attempt after `attempts` tries so far.
pub fn next_attempt(attempts: i32, now: DateTime<Utc>) -> DateTime<Utc> {
    let index = usize::try_from(attempts.max(1) - 1).unwrap_or(usize::MAX);
    now + BACKOFF[index.min(BACKOFF.len() - 1)]
}

/// Drains queued messages to their community relays.
pub struct Forwarder {
    pub store: Arc<dyn Store>,
    /// Transport to the community relays.
    pub client: Arc<dyn RelayClient>,
    /// Picks the identity that authors each message.
    pub signer: Arc<dyn MessageSigner>,
    /// Scheme used to reach community hosts.
    pub scheme: RelayScheme,
    /// Messages claimed and sent per poll.
    pub concurrency: usize,
    /// How long after the delivery's receipt a message keeps retrying.
    pub retry_max_age: Duration,
    /// Injectable clock so tests can drive the backoff schedule.
    pub now: fn() -> DateTime<Utc>,
}

impl Forwarder {
    /// Claim one batch and send it. Returns how many messages were claimed.
    pub async fn run_once(&self) -> Result<usize, StoreError> {
        let now = (self.now)();
        let batch = self
            .store
            .claim_due_messages(now, CLAIM_LEASE, self.concurrency.max(1))
            .await?;
        let claimed = batch.len();
        let mut tasks = JoinSet::new();
        for message in batch {
            let store = Arc::clone(&self.store);
            let send = Sender {
                client: Arc::clone(&self.client),
                signer: Arc::clone(&self.signer),
                scheme: self.scheme,
                retry_max_age: self.retry_max_age,
                now: self.now,
            };
            tasks.spawn(async move {
                let outcome = send.attempt(&message).await;
                if let Err(error) = store
                    .complete_message(&message, outcome, (send.now)())
                    .await
                {
                    record_forwarder_failure();
                    tracing::warn!(%error, "ingress gateway could not record send outcome");
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
                    tracing::warn!(%error, "ingress gateway forwarder poll failed");
                    tokio::time::sleep(IDLE_POLL).await;
                }
            }
        }
    }
}

/// One attempt's collaborators.
struct Sender {
    client: Arc<dyn RelayClient>,
    signer: Arc<dyn MessageSigner>,
    scheme: RelayScheme,
    retry_max_age: Duration,
    now: fn() -> DateTime<Utc>,
}

fn failed(event_id: Option<String>, error: String) -> ForwardOutcome {
    record_forward(ForwardResult::Failed, None);
    ForwardOutcome::Failed { event_id, error }
}

impl Sender {
    fn budget_spent(&self, message: &ClaimedMessage, now: DateTime<Utc>) -> bool {
        message.received_at + self.retry_max_age <= now
    }

    fn retry_or_fail(
        &self,
        message: &ClaimedMessage,
        event_id: Option<String>,
        error: String,
        elapsed: Option<f64>,
    ) -> ForwardOutcome {
        let now = (self.now)();
        if self.budget_spent(message, now) {
            record_forward(ForwardResult::Failed, elapsed);
            return ForwardOutcome::Failed {
                event_id,
                error: format!("retry budget exhausted: {error}"),
            };
        }
        record_forward(ForwardResult::Retry, elapsed);
        ForwardOutcome::Retry {
            event_id,
            error,
            next_attempt_at: next_attempt(message.attempts, now),
        }
    }

    async fn attempt(&self, message: &ClaimedMessage) -> ForwardOutcome {
        let host = &message.community_host;
        let (Some(events), Some(query)) = (
            relay_url(self.scheme, host, "events"),
            relay_url(self.scheme, host, "query"),
        ) else {
            return failed(None, "community host is not a valid relay URL".to_owned());
        };
        let target = SigningTarget {
            community_host: host,
            integration_id: message.key.integration_id,
            subscription_id: message.key.subscription_id,
            channel_id: message.channel_id,
        };
        let author = match self.signer.signer_for(&target).await {
            Ok(author) => author,
            Err(error @ SignerError::Unavailable(_)) => {
                return self.retry_or_fail(message, None, error.to_string(), None)
            }
            Err(error @ SignerError::Unconfigured(_)) => return failed(None, error.to_string()),
        };
        if (self.now)() - message.signed_at >= RESTAMP_AFTER {
            return self.recover(&query, &author, message).await;
        }
        let event = match build_message_event(&author, message).await {
            Ok(event) => event,
            Err(error) => {
                let error = format!("event signing failed: {error}");
                return self.retry_or_fail(message, None, error, None);
            }
        };
        let event_id = event.id.to_hex();
        let started = std::time::Instant::now();
        let result = self.client.submit(&events, &event, &author).await;
        let elapsed = started.elapsed().as_secs_f64();
        match result {
            SubmitResult::Accepted => {
                record_forward(ForwardResult::Accepted, Some(elapsed));
                ForwardOutcome::Accepted { event_id }
            }
            SubmitResult::Rejected(error) => {
                record_forward(ForwardResult::Failed, Some(elapsed));
                ForwardOutcome::Failed {
                    event_id: Some(event_id),
                    error,
                }
            }
            SubmitResult::Retry(error) => {
                self.retry_or_fail(message, Some(event_id), error, Some(elapsed))
            }
        }
    }

    /// `signed_at` is too old to send again. An earlier attempt may have
    /// landed without its answer arriving, so look for it before signing a
    /// new event that the relay would not recognize as its duplicate.
    async fn recover(
        &self,
        query: &Url,
        author: &Arc<dyn NostrSigner>,
        message: &ClaimedMessage,
    ) -> ForwardOutcome {
        let pubkey = match author.get_public_key().await {
            Ok(pubkey) => pubkey,
            Err(error) => {
                let error = format!("signer unavailable: {error}");
                return self.retry_or_fail(message, None, error, None);
            }
        };
        let mut ids = vec![message_event_id(pubkey, message).to_hex()];
        if let Some(sent) = message.event_id.as_ref().filter(|id| !ids.contains(id)) {
            ids.push(sent.clone());
        }
        let checking = |e: String| format!("could not check for an earlier send: {e}");
        match self
            .client
            .find(query, &ids, message.channel_id, author)
            .await
        {
            LookupResult::Found(event_id) => {
                record_forward(ForwardResult::Accepted, None);
                ForwardOutcome::Accepted { event_id }
            }
            LookupResult::Missing if self.budget_spent(message, (self.now)()) => failed(
                None,
                "retry budget exhausted: signed timestamp expired".to_owned(),
            ),
            LookupResult::Missing => {
                record_forward(ForwardResult::Retry, None);
                ForwardOutcome::Restamp {
                    error: "signed timestamp expired; re-signing".to_owned(),
                }
            }
            LookupResult::Retry(error) => self.retry_or_fail(message, None, checking(error), None),
            LookupResult::Rejected(error) => failed(None, checking(error)),
        }
    }
}
