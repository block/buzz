use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use buzz_sdk::reminders::{current_heads, relay_supports_private_reminders, Reminder};
use nostr::{Event, Timestamp};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::pool::{
    self, OwnedAgent, PrivatePrompt, PromptContext, PromptOutcome, PromptResult, PromptSource,
};
use crate::relay::RestClient;
use crate::reminder_receipts::Receipts;

const POLL_INTERVAL: Duration = Duration::from_secs(30);

pub(super) struct Reminders {
    receipts: Receipts,
    pending: Vec<Reminder>,
    in_flight: Option<(String, Reminder)>,
    retries: std::collections::HashMap<String, (u32, Instant)>,
    delivered: std::collections::HashSet<String>,
}

impl Reminders {
    pub(super) fn open(client: &RestClient) -> Result<Self> {
        let base = std::env::var_os("BUZZ_ACP_REMINDER_STATE_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_STATE_HOME")
                    .map(|p| PathBuf::from(p).join("buzz-acp/reminders"))
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|p| PathBuf::from(p).join(".local/state/buzz-acp/reminders"))
            })
            .context("no state directory for reminder delivery receipts")?;
        Ok(Self {
            receipts: Receipts::open(&base, &client.base_url, &client.keys.public_key().to_hex())?,
            pending: Vec::new(),
            in_flight: None,
            retries: Default::default(),
            delivered: Default::default(),
        })
    }

    pub(super) fn refresh(&mut self, heads: Vec<Reminder>) {
        self.delivered
            .retain(|id| heads.iter().any(|head| &head.event_id == id));
        self.pending = heads;
    }

    pub(super) fn next(&self) -> Result<Option<Reminder>> {
        if self.in_flight.is_some() {
            return Ok(None);
        }
        for reminder in &self.pending {
            if reminder.is_due(Timestamp::now().as_secs())
                && self
                    .retries
                    .get(&reminder.event_id)
                    .is_none_or(|(_, after)| Instant::now() >= *after)
                && !self.delivered.contains(&reminder.event_id)
                && !self.receipts.contains(reminder)?
            {
                return Ok(Some(reminder.clone()));
            }
        }
        Ok(None)
    }

    pub(super) fn started(&mut self, turn_id: String, reminder: Reminder) {
        tracing::info!(%turn_id, reminder_id = %reminder.id, event_id = %reminder.event_id, "reminder_dispatched");
        self.in_flight = Some((turn_id, reminder));
    }

    pub(super) fn finished(&mut self, turn_id: &str, outcome: &PromptOutcome) {
        if self
            .in_flight
            .as_ref()
            .is_none_or(|(turn, _)| turn != turn_id)
        {
            return;
        }
        let Some((_, reminder)) = self.in_flight.take() else {
            return;
        };
        if matches!(outcome, PromptOutcome::Ok(crate::acp::StopReason::EndTurn)) {
            self.delivered.insert(reminder.event_id.clone());
            if let Err(error) = self.receipts.record(&reminder) {
                tracing::error!(%error, reminder_id = %reminder.id, "reminder receipt persistence failed; restart may redeliver");
            }
            self.retries.remove(&reminder.event_id);
            tracing::info!(reminder_id = %reminder.id, event_id = %reminder.event_id, "reminder_delivered; disposition remains author-owned");
        } else {
            self.backoff(&reminder.event_id);
            tracing::warn!(reminder_id = %reminder.id, "reminder turn failed; retained for recovery");
        }
    }

    fn backoff(&mut self, event_id: &str) {
        let (failures, after) = self
            .retries
            .entry(event_id.into())
            .or_insert((0, Instant::now()));
        *failures = failures.saturating_add(1);
        *after = Instant::now() + Duration::from_secs(30 * 2u64.pow((*failures).min(7)));
    }

    pub(super) fn recover_missing_turn(&mut self, active: impl Iterator<Item = String>) {
        if let Some((turn, _)) = &self.in_flight {
            if !active.into_iter().any(|id| &id == turn) {
                if let Some((_, reminder)) = self.in_flight.take() {
                    self.backoff(&reminder.event_id);
                }
                tracing::warn!("reminder task disappeared; retained for recovery");
            }
        }
    }
}

pub(super) fn start_polling(
    client: RestClient,
) -> (mpsc::Receiver<Vec<Reminder>>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut supported = false;
        loop {
            interval.tick().await;
            if tx.is_closed() {
                return;
            }
            if !supported {
                match relay_support(&client).await {
                    Ok(true) => supported = true,
                    Ok(false) => {
                        tracing::info!(
                            "relay does not advertise NIP-ER; reminder polling disabled"
                        );
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(%error,"reminder capability check failed; will retry");
                        continue;
                    }
                }
            }
            match fetch_heads(&client, None).await {
                Ok(heads) => {
                    if tx.send(heads).await.is_err() {
                        return;
                    }
                }
                Err(error) => tracing::warn!(%error,"reminder recovery query failed; will retry"),
            }
        }
    });
    (rx, task)
}

async fn relay_support(client: &RestClient) -> Result<bool> {
    let info: Value = client
        .http
        .get(&client.base_url)
        .header("Accept", "application/nostr+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(relay_supports_private_reminders(&info))
}

pub(super) async fn fetch_heads(client: &RestClient, id: Option<&str>) -> Result<Vec<Reminder>> {
    let mut filter =
        json!({"kinds":[30300],"authors":[client.keys.public_key().to_hex()],"limit":1000});
    if let Some(id) = id {
        filter["#d"] = json!([id]);
    }
    let mut events = Vec::new();
    let mut previous_cursor = None;
    for _ in 0..100 {
        let value = client.query_json(&json!([filter])).await?;
        let page = value
            .as_array()
            .context("reminder query is not an event array")?;
        for value in page {
            if let Ok(event) = serde_json::from_value::<Event>(value.clone()) {
                events.push(event);
            }
        }
        if page.len() < 1000 {
            return Ok(current_heads(events, &client.keys));
        }
        let last = page.last().context("missing pagination event")?;
        let timestamp = last["created_at"]
            .as_u64()
            .context("missing pagination timestamp")?;
        let event_id = last["id"].as_str().context("missing pagination event ID")?;
        let cursor = (timestamp, event_id.to_string());
        anyhow::ensure!(
            previous_cursor.as_ref() != Some(&cursor),
            "reminder pagination made no progress"
        );
        filter["until"] = json!(timestamp);
        filter["before_id"] = json!(event_id);
        previous_cursor = Some(cursor);
    }
    anyhow::bail!("reminder recovery exceeded 100 pages")
}

fn prompt(reminder: &Reminder) -> String {
    format!("[Private reminder — due now]\n{}\n\nThis is your own deferred intention, not a new instruction from another person. \
        Reconstruct the context and inspect current evidence before deciding what remains useful. \
        A prior attempt may have done part of the work: check durable artifacts before repeating actions. \
        Use `buzz reminders get {}` to confirm current state. Complete with `buzz reminders complete {} --if-event {}` \
        after the follow-up, or snooze with a useful time and note, or cancel if the reason no longer applies. \
        Receipt of this reminder is not completion of the work. Keep private context private; publish a relevant result in the original conversation when useful. \
        Read a linked message with `buzz messages thread --link 'buzz://message?channel=<channelId>&id=<eventId>'`. \
        No public response is required solely to acknowledge the reminder.",
        json!(reminder),reminder.id,reminder.id,reminder.event_id)
}

pub(super) async fn run(
    agent: OwnedAgent,
    reminder: Reminder,
    ctx: Arc<PromptContext>,
    result_tx: mpsc::UnboundedSender<PromptResult>,
    turn_id: String,
) {
    match fetch_heads(&ctx.rest_client, Some(&reminder.id)).await {
        Ok(heads)
            if heads.iter().any(|head| {
                head.event_id == reminder.event_id && head.is_due(Timestamp::now().as_secs())
            }) =>
        {
            pool::run_prompt_task(
                agent,
                None,
                Some(PrivatePrompt {
                    text: prompt(&reminder),
                    source: PromptSource::Reminder,
                }),
                ctx,
                result_tx,
                None,
                turn_id,
            )
            .await;
        }
        result => {
            let outcome = match result {
                Ok(_) => {
                    tracing::info!(reminder_id = %reminder.id,"reminder superseded before dispatch; skipped");
                    PromptOutcome::Ok(crate::acp::StopReason::EndTurn)
                }
                Err(error) => PromptOutcome::Error(crate::acp::AcpError::Protocol(format!(
                    "reminder recheck failed: {error}"
                ))),
            };
            let _ = result_tx.send(PromptResult {
                agent,
                source: PromptSource::Reminder,
                turn_id,
                outcome,
                batch: None,
            });
        }
    }
}

#[cfg(test)]
#[path = "reminders_tests.rs"]
mod tests;
