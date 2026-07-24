//! Content-free mention-to-reply latency correlation.
//!
//! The collector consumes semantic observer boundaries emitted by the harness
//! and derives one `mention_reply_latency` sample after both the first reply and
//! turn completion are observed. Durations use the process-local monotonic
//! observer clock; RFC3339/Nostr timestamps remain correlation metadata only.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{json, Map, Value};

use crate::observer::{ObserverContext, ObserverEvent};

const TRACE_TTL_MS: u64 = 15 * 60 * 1_000;
const SUMMARY_WINDOW: usize = 100;

/// Saturating millisecond conversion for semantic timing payloads.
pub(crate) fn duration_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug)]
struct TriggerTiming {
    received_ms: u64,
    queued_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct TurnTiming {
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    triggering_event_ids: Vec<String>,
    started_ms: u64,
    session_resolved_ms: Option<u64>,
    prompt_dispatched_ms: Option<u64>,
    first_output_ms: Option<u64>,
    reply_observed_ms: Option<u64>,
    completed_ms: Option<u64>,
    is_new_session: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
struct SampleMetrics {
    receive_to_queue_ms: u64,
    queue_wait_ms: u64,
    session_resolve_ms: Option<u64>,
    post_session_setup_ms: Option<u64>,
    turn_setup_ms: Option<u64>,
    time_to_first_output_ms: Option<u64>,
    first_output_to_reply_ms: Option<u64>,
    turn_duration_ms: u64,
    total_ms: u64,
}

/// Derived sample ready to be emitted into the observer feed.
pub(crate) struct CompletedLatency {
    pub(crate) agent_index: Option<usize>,
    pub(crate) context: ObserverContext,
    pub(crate) payload: Value,
}

/// Stateful correlator for semantic observer events.
#[derive(Default)]
pub(crate) struct LatencyCollector {
    triggers: HashMap<String, TriggerTiming>,
    event_to_turn: HashMap<String, String>,
    turns: HashMap<String, TurnTiming>,
    windows: HashMap<String, VecDeque<SampleMetrics>>,
}

impl LatencyCollector {
    /// Ingest one observer event and return a completed latency sample, if this
    /// event closes a trace. Unknown/raw event kinds are deliberately ignored.
    pub(crate) fn ingest(&mut self, event: &ObserverEvent) -> Option<CompletedLatency> {
        self.prune(event.monotonic_ms);

        let mut candidate_turn_id = event.turn_id.clone();
        match event.kind.as_str() {
            "event_received" => {
                let event_id = string_field(&event.payload, "eventId")?;
                let receipt_to_observer_ms =
                    u64_field(&event.payload, "receiptToObserverMs").unwrap_or_default();
                self.triggers.insert(
                    event_id,
                    TriggerTiming {
                        received_ms: event.monotonic_ms.saturating_sub(receipt_to_observer_ms),
                        queued_ms: None,
                    },
                );
                return None;
            }
            "event_queued" => {
                let event_id = string_field(&event.payload, "eventId")?;
                self.triggers.get_mut(&event_id)?.queued_ms = Some(event.monotonic_ms);
                return None;
            }
            "turn_started" => {
                let turn_id = event.turn_id.clone()?;
                let triggering_event_ids = string_array_field(&event.payload, "triggeringEventIds");
                for event_id in &triggering_event_ids {
                    self.event_to_turn.insert(event_id.clone(), turn_id.clone());
                }
                self.turns.insert(
                    turn_id.clone(),
                    TurnTiming {
                        agent_index: event.agent_index,
                        channel_id: event.channel_id.clone(),
                        session_id: event.session_id.clone(),
                        triggering_event_ids,
                        started_ms: event.monotonic_ms,
                        ..TurnTiming::default()
                    },
                );
                candidate_turn_id = Some(turn_id);
            }
            "session_resolved" => {
                if let Some(turn_id) = event.turn_id.as_deref() {
                    if let Some(turn) = self.turns.get_mut(turn_id) {
                        turn.session_id = event.session_id.clone();
                        turn.session_resolved_ms.get_or_insert(event.monotonic_ms);
                        turn.is_new_session =
                            event.payload.get("isNewSession").and_then(Value::as_bool);
                    }
                }
            }
            "prompt_dispatched" => {
                if let Some(turn_id) = event.turn_id.as_deref() {
                    if let Some(turn) = self.turns.get_mut(turn_id) {
                        turn.prompt_dispatched_ms.get_or_insert(event.monotonic_ms);
                    }
                }
            }
            "turn_first_output" => {
                if let Some(turn_id) = event.turn_id.as_deref() {
                    if let Some(turn) = self.turns.get_mut(turn_id) {
                        turn.first_output_ms.get_or_insert(event.monotonic_ms);
                    }
                }
            }
            "reply_observed" => {
                if candidate_turn_id.is_none() {
                    candidate_turn_id = ["parentEventId", "rootEventId"]
                        .iter()
                        .filter_map(|key| string_field(&event.payload, key))
                        .find_map(|event_id| self.event_to_turn.get(&event_id).cloned());
                }
                if candidate_turn_id.is_none() {
                    // Buzz keeps human-facing conversations flat, so a reply can
                    // point at the thread root instead of the mention that started
                    // this turn. At most one turn is active per channel; retain
                    // completed traces briefly and select the newest channel turn
                    // when thread tags cannot provide a direct event correlation.
                    candidate_turn_id = event.channel_id.as_deref().and_then(|channel_id| {
                        self.turns
                            .iter()
                            .filter(|(_, turn)| turn.channel_id.as_deref() == Some(channel_id))
                            .max_by_key(|(_, turn)| turn.started_ms)
                            .map(|(turn_id, _)| turn_id.clone())
                    });
                }
                if let Some(turn_id) = candidate_turn_id.as_deref() {
                    if let Some(turn) = self.turns.get_mut(turn_id) {
                        // First published reply is the user-visible response
                        // latency boundary. Later progress/final posts do not
                        // overwrite it.
                        turn.reply_observed_ms.get_or_insert(event.monotonic_ms);
                    }
                }
            }
            "turn_completed" => {
                if let Some(turn_id) = event.turn_id.as_deref() {
                    if let Some(turn) = self.turns.get_mut(turn_id) {
                        turn.completed_ms.get_or_insert(event.monotonic_ms);
                    }
                }
            }
            _ => return None,
        }

        candidate_turn_id.and_then(|turn_id| self.try_complete(&turn_id))
    }

    fn try_complete(&mut self, turn_id: &str) -> Option<CompletedLatency> {
        let turn = self.turns.get(turn_id)?.clone();
        let reply_ms = turn.reply_observed_ms?;
        let completed_ms = turn.completed_ms?;
        let (primary_event_id, trigger, queued_ms) = turn
            .triggering_event_ids
            .iter()
            .filter_map(|event_id| {
                self.triggers.get(event_id).and_then(|timing| {
                    timing
                        .queued_ms
                        .map(|queued_ms| (event_id.clone(), timing.clone(), queued_ms))
                })
            })
            .min_by_key(|(_, timing, _)| timing.received_ms)?;

        let metrics = SampleMetrics {
            receive_to_queue_ms: queued_ms.saturating_sub(trigger.received_ms),
            queue_wait_ms: turn.started_ms.saturating_sub(queued_ms),
            session_resolve_ms: turn
                .session_resolved_ms
                .map(|resolved_ms| resolved_ms.saturating_sub(turn.started_ms)),
            post_session_setup_ms: turn
                .session_resolved_ms
                .zip(turn.prompt_dispatched_ms)
                .map(|(resolved_ms, prompt_ms)| prompt_ms.saturating_sub(resolved_ms)),
            turn_setup_ms: turn
                .prompt_dispatched_ms
                .map(|prompt_ms| prompt_ms.saturating_sub(turn.started_ms)),
            time_to_first_output_ms: turn
                .prompt_dispatched_ms
                .zip(turn.first_output_ms)
                .map(|(prompt_ms, output_ms)| output_ms.saturating_sub(prompt_ms)),
            first_output_to_reply_ms: turn
                .first_output_ms
                .map(|output_ms| reply_ms.saturating_sub(output_ms)),
            turn_duration_ms: completed_ms.saturating_sub(turn.started_ms),
            total_ms: reply_ms.saturating_sub(trigger.received_ms),
        };

        let path = match turn.is_new_session {
            Some(true) => "cold",
            Some(false) => "warm",
            None => "unknown",
        };
        let window = self.windows.entry(path.to_string()).or_default();
        if window.len() >= SUMMARY_WINDOW {
            window.pop_front();
        }
        window.push_back(metrics);

        let mut payload = Map::new();
        payload.insert("traceId".into(), Value::String(primary_event_id));
        payload.insert(
            "measurementStart".into(),
            Value::String("harness_relay_receipt".into()),
        );
        payload.insert(
            "measurementEnd".into(),
            Value::String("harness_relay_fanout".into()),
        );
        payload.insert(
            "triggeringEventIds".into(),
            json!(turn.triggering_event_ids),
        );
        payload.insert("path".into(), Value::String(path.to_string()));
        insert_u64(
            &mut payload,
            "receiveToQueueMs",
            metrics.receive_to_queue_ms,
        );
        insert_u64(&mut payload, "queueWaitMs", metrics.queue_wait_ms);
        insert_optional_u64(&mut payload, "sessionResolveMs", metrics.session_resolve_ms);
        insert_optional_u64(
            &mut payload,
            "postSessionSetupMs",
            metrics.post_session_setup_ms,
        );
        insert_optional_u64(&mut payload, "turnSetupMs", metrics.turn_setup_ms);
        insert_optional_u64(
            &mut payload,
            "timeToFirstOutputMs",
            metrics.time_to_first_output_ms,
        );
        insert_optional_u64(
            &mut payload,
            "firstOutputToReplyMs",
            metrics.first_output_to_reply_ms,
        );
        insert_u64(&mut payload, "turnDurationMs", metrics.turn_duration_ms);
        insert_u64(&mut payload, "totalMs", metrics.total_ms);
        payload.insert("summary".into(), build_summary(window));

        self.turns.remove(turn_id);
        for event_id in &turn.triggering_event_ids {
            self.triggers.remove(event_id);
            self.event_to_turn.remove(event_id);
        }

        Some(CompletedLatency {
            agent_index: turn.agent_index,
            context: ObserverContext {
                channel_id: turn.channel_id,
                session_id: turn.session_id,
                turn_id: Some(turn_id.to_string()),
                started_at: None,
            },
            payload: Value::Object(payload),
        })
    }

    fn prune(&mut self, now_ms: u64) {
        self.triggers
            .retain(|_, timing| now_ms.saturating_sub(timing.received_ms) <= TRACE_TTL_MS);
        self.turns
            .retain(|_, turn| now_ms.saturating_sub(turn.started_ms) <= TRACE_TTL_MS);
        let active_turns: HashSet<&str> = self.turns.keys().map(String::as_str).collect();
        let active_triggers: HashSet<&str> = self.triggers.keys().map(String::as_str).collect();
        self.event_to_turn.retain(|event_id, turn_id| {
            active_triggers.contains(event_id.as_str()) && active_turns.contains(turn_id.as_str())
        });
    }
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload.get(key)?.as_str().map(ToOwned::to_owned)
}

fn u64_field(payload: &Value, key: &str) -> Option<u64> {
    payload.get(key)?.as_u64()
}

fn string_array_field(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn insert_u64(payload: &mut Map<String, Value>, key: &str, value: u64) {
    payload.insert(key.to_string(), Value::from(value));
}

fn insert_optional_u64(payload: &mut Map<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        insert_u64(payload, key, value);
    }
}

fn build_summary(window: &VecDeque<SampleMetrics>) -> Value {
    let mut summary = Map::new();
    summary.insert("windowSize".into(), Value::from(window.len() as u64));
    insert_stats(
        &mut summary,
        "receiveToQueueMs",
        window.iter().map(|sample| Some(sample.receive_to_queue_ms)),
    );
    insert_stats(
        &mut summary,
        "queueWaitMs",
        window.iter().map(|sample| Some(sample.queue_wait_ms)),
    );
    insert_stats(
        &mut summary,
        "turnSetupMs",
        window.iter().map(|sample| sample.turn_setup_ms),
    );
    insert_stats(
        &mut summary,
        "sessionResolveMs",
        window.iter().map(|sample| sample.session_resolve_ms),
    );
    insert_stats(
        &mut summary,
        "postSessionSetupMs",
        window.iter().map(|sample| sample.post_session_setup_ms),
    );
    insert_stats(
        &mut summary,
        "timeToFirstOutputMs",
        window.iter().map(|sample| sample.time_to_first_output_ms),
    );
    insert_stats(
        &mut summary,
        "firstOutputToReplyMs",
        window.iter().map(|sample| sample.first_output_to_reply_ms),
    );
    insert_stats(
        &mut summary,
        "turnDurationMs",
        window.iter().map(|sample| Some(sample.turn_duration_ms)),
    );
    insert_stats(
        &mut summary,
        "totalMs",
        window.iter().map(|sample| Some(sample.total_ms)),
    );
    Value::Object(summary)
}

fn insert_stats<I>(summary: &mut Map<String, Value>, key: &str, values: I)
where
    I: Iterator<Item = Option<u64>>,
{
    let mut values: Vec<u64> = values.flatten().collect();
    if values.is_empty() {
        return;
    }
    values.sort_unstable();
    let p50 = nearest_rank(&values, 50);
    let p95 = nearest_rank(&values, 95);
    let max = values.last().copied().unwrap_or_default();
    summary.insert(
        key.to_string(),
        json!({
            "samples": values.len(),
            "p50": p50,
            "p95": p95,
            "max": max,
        }),
    );
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    let rank = (percentile * sorted.len()).div_ceil(100);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        kind: &str,
        monotonic_ms: u64,
        turn_id: Option<&str>,
        payload: Value,
    ) -> ObserverEvent {
        ObserverEvent {
            seq: monotonic_ms,
            monotonic_ms,
            timestamp: "2026-07-22T00:00:00Z".to_string(),
            kind: kind.to_string(),
            agent_index: Some(0),
            channel_id: Some("11111111-1111-1111-1111-111111111111".to_string()),
            session_id: turn_id.map(|_| "session-1".to_string()),
            turn_id: turn_id.map(ToOwned::to_owned),
            started_at: None,
            payload,
        }
    }

    fn complete_sample(
        collector: &mut LatencyCollector,
        event_id: &str,
        turn_id: &str,
        offset: u64,
        total_ms: u64,
        is_new_session: bool,
    ) -> CompletedLatency {
        let received = offset;
        let queued = received + 3;
        let started = queued + 10;
        let session_resolved = started + 2;
        let prompt = started + 5;
        let first_output = prompt + 15;
        let reply = received + total_ms;
        let completed = reply + 5;

        assert!(collector
            .ingest(&event(
                "event_received",
                received,
                None,
                json!({"eventId": event_id, "receiptToObserverMs": 0}),
            ))
            .is_none());
        assert!(collector
            .ingest(&event(
                "event_queued",
                queued,
                None,
                json!({"eventId": event_id}),
            ))
            .is_none());
        assert!(collector
            .ingest(&event(
                "turn_started",
                started,
                Some(turn_id),
                json!({"triggeringEventIds": [event_id]}),
            ))
            .is_none());
        assert!(collector
            .ingest(&event(
                "session_resolved",
                session_resolved,
                Some(turn_id),
                json!({"isNewSession": is_new_session}),
            ))
            .is_none());
        assert!(collector
            .ingest(&event(
                "prompt_dispatched",
                prompt,
                Some(turn_id),
                json!({}),
            ))
            .is_none());
        assert!(collector
            .ingest(&event(
                "turn_first_output",
                first_output,
                Some(turn_id),
                json!({}),
            ))
            .is_none());
        assert!(collector
            .ingest(&event(
                "reply_observed",
                reply,
                Some(turn_id),
                json!({"eventId": "reply-id"}),
            ))
            .is_none());
        collector
            .ingest(&event(
                "turn_completed",
                completed,
                Some(turn_id),
                json!({}),
            ))
            .expect("completed latency sample")
    }

    #[test]
    fn correlates_content_free_stage_durations() {
        let mut collector = LatencyCollector::default();
        let completed = complete_sample(&mut collector, "trigger-1", "turn-1", 7, 48, false);

        assert_eq!(completed.payload["traceId"], "trigger-1");
        assert_eq!(completed.payload["path"], "warm");
        assert_eq!(completed.payload["receiveToQueueMs"], 3);
        assert_eq!(completed.payload["queueWaitMs"], 10);
        assert_eq!(completed.payload["sessionResolveMs"], 2);
        assert_eq!(completed.payload["postSessionSetupMs"], 3);
        assert_eq!(completed.payload["turnSetupMs"], 5);
        assert_eq!(completed.payload["timeToFirstOutputMs"], 15);
        assert_eq!(completed.payload["firstOutputToReplyMs"], 15);
        assert_eq!(completed.payload["turnDurationMs"], 40);
        assert_eq!(completed.payload["totalMs"], 48);
        assert_eq!(completed.payload["summary"]["totalMs"]["p50"], 48);
        assert_eq!(completed.payload["summary"]["totalMs"]["p95"], 48);
        assert_eq!(completed.payload["summary"]["totalMs"]["max"], 48);

        let serialized = serde_json::to_string(&completed.payload).unwrap();
        for forbidden in ["message", "content", "prompt", "modelOutput", "credential"] {
            assert!(
                !serialized.contains(forbidden),
                "performance telemetry must not contain {forbidden}: {serialized}"
            );
        }
    }

    #[test]
    fn keeps_warm_and_cold_percentile_windows_separate() {
        let mut collector = LatencyCollector::default();
        let first = complete_sample(&mut collector, "warm-1", "turn-1", 0, 20, false);
        assert_eq!(first.payload["summary"]["windowSize"], 1);

        let second = complete_sample(&mut collector, "warm-2", "turn-2", 100, 40, false);
        assert_eq!(second.payload["summary"]["windowSize"], 2);
        assert_eq!(second.payload["summary"]["totalMs"]["p50"], 20);
        assert_eq!(second.payload["summary"]["totalMs"]["p95"], 40);
        assert_eq!(second.payload["summary"]["totalMs"]["max"], 40);

        let cold = complete_sample(&mut collector, "cold-1", "turn-3", 200, 80, true);
        assert_eq!(cold.payload["path"], "cold");
        assert_eq!(cold.payload["summary"]["windowSize"], 1);
        assert_eq!(cold.payload["summary"]["totalMs"]["p50"], 80);
    }

    #[test]
    fn correlates_flat_thread_reply_after_completion_from_channel() {
        let mut collector = LatencyCollector::default();
        collector.ingest(&event(
            "event_received",
            10,
            None,
            json!({"eventId": "trigger-1", "receiptToObserverMs": 2}),
        ));
        collector.ingest(&event(
            "event_queued",
            10,
            None,
            json!({"eventId": "trigger-1"}),
        ));
        collector.ingest(&event(
            "turn_started",
            15,
            Some("turn-1"),
            json!({"triggeringEventIds": ["trigger-1"]}),
        ));
        collector.ingest(&event("turn_completed", 30, Some("turn-1"), json!({})));

        let completed = collector
            .ingest(&event(
                "reply_observed",
                35,
                None,
                json!({
                    "eventId": "reply-1",
                    "parentEventId": "older-thread-root",
                }),
            ))
            .expect("channel fallback should recover the completed flat-thread turn");
        assert_eq!(completed.payload["traceId"], "trigger-1");
        assert_eq!(completed.payload["totalMs"], 27);
    }

    #[test]
    fn prunes_incomplete_traces_after_ttl() {
        let mut collector = LatencyCollector::default();
        collector.ingest(&event(
            "event_received",
            10,
            None,
            json!({"eventId": "stale", "receiptToObserverMs": 1}),
        ));
        collector.ingest(&event(
            "event_queued",
            10,
            None,
            json!({"eventId": "stale"}),
        ));
        collector.ingest(&event(
            "turn_started",
            20,
            Some("stale-turn"),
            json!({"triggeringEventIds": ["stale"]}),
        ));
        collector.ingest(&event(
            "event_received",
            TRACE_TTL_MS + 21,
            None,
            json!({"eventId": "fresh", "receiptToObserverMs": 1}),
        ));

        assert!(!collector.triggers.contains_key("stale"));
        assert!(!collector.turns.contains_key("stale-turn"));
        assert!(!collector.event_to_turn.contains_key("stale"));
    }
}
