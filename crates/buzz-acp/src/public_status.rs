//! Channel-visible agent work status: NIP-AW events (kind 30181) with the
//! channel UUID as both the `h` tag (NIP-29 channel scope — reads and writes
//! inherit channel-membership gating on the relay) and the `d` tag, so each
//! agent stores at most one replaceable status snapshot per channel.
//!
//! This is the public counterpart of the owner-encrypted observer stream
//! (NIP-AO, kind 24200): a whitelist projection of observer frames down to
//! status transitions, the current model id, and redacted tool-call titles.
//! Content bodies, tool arguments, prompts, and thoughts never enter the
//! payload — the projection copies only the fields named in `apply_event`.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;

use crate::observer::{ObserverEvent, ObserverHandle};
use crate::relay::RestClient;

/// NIP-AW agent work status kind; `h` = `d` = channel UUID, one live
/// snapshot per (agent, channel), channel-membership-gated on the relay.
const PUBLIC_STATUS_KIND: u16 = buzz_core::kind::KIND_AGENT_WORK_STATUS as u16;
/// Newest-last redacted activity entries retained per channel.
const MAX_ACTIVITY_ENTRIES: usize = 20;
/// Hard cap on a single redacted title.
const MAX_TITLE_LEN: usize = 160;
/// Floor between non-urgent republications of one channel's status.
const MIN_PUBLISH_INTERVAL: Duration = Duration::from_secs(5);
/// Cadence of the flush loop that drains throttled updates.
const FLUSH_TICK: Duration = Duration::from_secs(1);
/// Budget for one status publication; failures are logged, never fatal.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(3);

/// One redacted activity item: a title and a coarse status, nothing else.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub at: String,
    pub kind: String,
    pub title: String,
    pub status: String,
}

/// The published content body. Consumers key on `v` + `source` to
/// distinguish harness telemetry from ordinary NIP-38 statuses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicStatusPayload<'a> {
    v: u32,
    source: &'static str,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_started_at: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<&'a str>,
    updated_at: String,
    activity: Vec<&'a ActivityEntry>,
}

/// Mutable per-channel projection of the observer stream.
#[derive(Debug)]
pub struct ChannelWork {
    status: &'static str,
    model: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    turn_started_at: Option<String>,
    completed_at: Option<String>,
    stop_reason: Option<String>,
    activity: VecDeque<ActivityEntry>,
    /// Something changed since the last publication.
    dirty: bool,
    /// A state transition that should bypass the publish floor.
    urgent: bool,
    last_published: Option<Instant>,
}

impl Default for ChannelWork {
    fn default() -> Self {
        Self {
            status: "idle",
            model: None,
            session_id: None,
            turn_id: None,
            turn_started_at: None,
            completed_at: None,
            stop_reason: None,
            activity: VecDeque::new(),
            dirty: false,
            urgent: false,
            last_published: None,
        }
    }
}

impl ChannelWork {
    fn push_activity(&mut self, entry: ActivityEntry) {
        if self.activity.len() == MAX_ACTIVITY_ENTRIES {
            self.activity.pop_front();
        }
        self.activity.push_back(entry);
    }

    /// Whether this channel's status should be published now.
    pub fn should_publish(&self, now: Instant) -> bool {
        if self.urgent {
            return true;
        }
        if !self.dirty {
            return false;
        }
        match self.last_published {
            None => true,
            Some(at) => now.duration_since(at) >= MIN_PUBLISH_INTERVAL,
        }
    }

    fn mark_published(&mut self, now: Instant) {
        self.dirty = false;
        self.urgent = false;
        self.last_published = Some(now);
    }

    fn payload_json(&self, updated_at: String) -> serde_json::Value {
        let payload = PublicStatusPayload {
            v: 1,
            source: "buzz-acp",
            status: self.status,
            model: self.model.as_deref(),
            session_id: self.session_id.as_deref(),
            turn_id: self.turn_id.as_deref(),
            turn_started_at: self.turn_started_at.as_deref(),
            completed_at: self.completed_at.as_deref(),
            stop_reason: self.stop_reason.as_deref(),
            updated_at,
            activity: self.activity.iter().collect(),
        };
        serde_json::to_value(&payload).unwrap_or_else(|_| serde_json::json!({}))
    }
}

fn truncate_title(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_LEN {
        return title.to_string();
    }
    let mut out: String = title.chars().take(MAX_TITLE_LEN - 1).collect();
    out.push('…');
    out
}

/// Map an ACP tool-call status string onto the coarse public vocabulary.
fn coarse_tool_status(status: &str) -> &'static str {
    match status {
        "completed" => "complete",
        "failed" => "failed",
        _ => "running",
    }
}

/// Fold one observer event into the per-channel projections. Every field
/// copied out of `event.payload` is named here — this function IS the
/// redaction boundary, so additions must stay whitelist-shaped.
pub fn apply_event(states: &mut HashMap<String, ChannelWork>, event: &ObserverEvent) {
    let Some(channel_id) = event.channel_id.as_deref() else {
        return;
    };
    let state = states.entry(channel_id.to_string()).or_default();

    match event.kind.as_str() {
        "turn_started" => {
            *state = ChannelWork {
                status: "working",
                model: state.model.take(),
                session_id: event.session_id.clone(),
                turn_id: event.turn_id.clone(),
                turn_started_at: event.started_at.clone().or(Some(event.timestamp.clone())),
                last_published: state.last_published,
                dirty: true,
                urgent: true,
                ..ChannelWork::default()
            };
            state.push_activity(ActivityEntry {
                at: event.timestamp.clone(),
                kind: "lifecycle".into(),
                title: "Turn started".into(),
                status: "complete".into(),
            });
        }
        "session_resolved"
            if event.session_id.is_some() && state.session_id != event.session_id =>
        {
            state.session_id = event.session_id.clone();
            state.dirty = true;
        }
        "session_config_captured" => {
            if let Some(model) = event.payload["models"]["currentModelId"].as_str() {
                if state.model.as_deref() != Some(model) {
                    state.model = Some(model.to_string());
                    state.dirty = true;
                }
            }
        }
        // Refresh updatedAt for viewers computing elapsed time.
        "turn_liveness" if state.status == "working" => {
            state.dirty = true;
        }
        "turn_completed" => {
            state.status = "complete";
            state.completed_at = Some(event.timestamp.clone());
            state.dirty = true;
            state.urgent = true;
        }
        "turn_error" => {
            state.status = "error";
            state.completed_at = Some(event.timestamp.clone());
            // `outcome` is a closed vocabulary; the error message itself may
            // carry payload content and is deliberately not copied.
            state.stop_reason = event.payload["outcome"].as_str().map(str::to_string);
            state.dirty = true;
            state.urgent = true;
        }
        "acp_read" => {
            let update = &event.payload["params"]["update"];
            match update["sessionUpdate"].as_str() {
                Some("tool_call") => {
                    let title = update["title"].as_str().unwrap_or("Tool call");
                    state.push_activity(ActivityEntry {
                        at: event.timestamp.clone(),
                        kind: "tool".into(),
                        title: truncate_title(title),
                        status: coarse_tool_status(update["status"].as_str().unwrap_or(""))
                            .to_string(),
                    });
                    state.dirty = true;
                }
                Some("tool_call_update") => {
                    if let Some(status) = update["status"].as_str() {
                        if let Some(entry) = state
                            .activity
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.kind == "tool" && entry.status == "running")
                        {
                            entry.status = coarse_tool_status(status).into();
                            state.dirty = true;
                        }
                    }
                }
                Some("agent_message_chunk") => {
                    let already_streaming = state
                        .activity
                        .back()
                        .is_some_and(|entry| entry.kind == "message" && entry.status == "running");
                    if !already_streaming {
                        state.push_activity(ActivityEntry {
                            at: event.timestamp.clone(),
                            kind: "message".into(),
                            title: "Streaming reply".into(),
                            status: "running".into(),
                        });
                        state.dirty = true;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Build the signed NIP-AW replaceable status event for one channel.
///
/// The relay requires `h` (channel scope + membership gating) and rejects any
/// event whose `d` tag differs from the `h` channel UUID.
fn build_status_event(
    keys: &nostr::Keys,
    channel_id: &str,
    state: &ChannelWork,
) -> Result<nostr::Event, String> {
    let updated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let content = state.payload_json(updated_at).to_string();
    let h_tag = nostr::Tag::parse(["h", channel_id]).map_err(|e| e.to_string())?;
    let d_tag = nostr::Tag::parse(["d", channel_id]).map_err(|e| e.to_string())?;
    nostr::EventBuilder::new(nostr::Kind::Custom(PUBLIC_STATUS_KIND), content)
        .tags([h_tag, d_tag])
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())
}

async fn publish_channel_status(rest: &RestClient, channel_id: &str, state: &ChannelWork) {
    let event = match build_status_event(&rest.keys, channel_id, state) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(target: "public_status", channel_id, "sign failed: {error}");
            return;
        }
    };
    match tokio::time::timeout(PUBLISH_TIMEOUT, rest.submit_event(&event)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            tracing::warn!(target: "public_status", channel_id, "publish failed: {error}");
        }
        Err(_) => {
            tracing::warn!(target: "public_status", channel_id, "publish timed out");
        }
    }
}

/// Spawn the public-status projection task: a second consumer of the observer
/// bus, independent of the owner-encrypted NIP-AO publisher.
pub fn spawn_public_status_publisher(
    observer: ObserverHandle,
    rest: RestClient,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Subscribe before snapshotting so no event falls between the two —
        // the same loss-window closure the NIP-AO publisher uses. Snapshot
        // replay is deduped by the monotonic `seq` high-water mark.
        let rx = observer.subscribe();
        let snapshot = observer.snapshot();
        run_public_status_publisher(snapshot, rx, rest).await;
    })
}

async fn run_public_status_publisher(
    snapshot: Vec<ObserverEvent>,
    mut rx: tokio::sync::broadcast::Receiver<ObserverEvent>,
    rest: RestClient,
) {
    let mut states: HashMap<String, ChannelWork> = HashMap::new();
    let max_snapshot_seq = snapshot.iter().map(|event| event.seq).max().unwrap_or(0);
    for event in &snapshot {
        apply_event(&mut states, event);
    }

    let mut flush_tick = tokio::time::interval(FLUSH_TICK);
    flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut closed = false;
    loop {
        tokio::select! {
            result = rx.recv(), if !closed => {
                match result {
                    Ok(event) => {
                        if event.seq <= max_snapshot_seq {
                            continue;
                        }
                        apply_event(&mut states, &event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(target: "public_status", dropped = count, "publisher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        closed = true;
                    }
                }
            }
            _ = flush_tick.tick() => {
                let now = Instant::now();
                for (channel_id, state) in states.iter_mut() {
                    if state.should_publish(now) {
                        publish_channel_status(&rest, channel_id, state).await;
                        state.mark_published(now);
                    }
                }
                if closed {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, channel: Option<&str>, payload: serde_json::Value) -> ObserverEvent {
        ObserverEvent {
            seq: 1,
            timestamp: "2026-08-22T19:00:00Z".into(),
            kind: kind.into(),
            agent_index: Some(0),
            channel_id: channel.map(str::to_string),
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            started_at: Some("2026-08-22T18:59:59Z".into()),
            payload,
        }
    }

    fn session_update(update: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"method": "session/update", "params": {"update": update}})
    }

    const CH: &str = "0b7c0958-3f7f-48c8-af3f-31e549b10e31";

    #[test]
    fn turn_lifecycle_projects_working_then_complete() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        let state = &states[CH];
        assert_eq!(state.status, "working");
        assert!(state.urgent);
        assert_eq!(
            state.turn_started_at.as_deref(),
            Some("2026-08-22T18:59:59Z")
        );

        apply_event(
            &mut states,
            &event("turn_completed", Some(CH), serde_json::json!({})),
        );
        let state = &states[CH];
        assert_eq!(state.status, "complete");
        assert_eq!(state.completed_at.as_deref(), Some("2026-08-22T19:00:00Z"));
        assert!(state.urgent);
    }

    #[test]
    fn turn_error_copies_outcome_but_never_the_error_message() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        apply_event(
            &mut states,
            &event(
                "turn_error",
                Some(CH),
                serde_json::json!({"outcome": "timeout", "error": "secret detail"}),
            ),
        );
        let state = &states[CH];
        assert_eq!(state.status, "error");
        assert_eq!(state.stop_reason.as_deref(), Some("timeout"));
        let json = state.payload_json("t".into()).to_string();
        assert!(!json.contains("secret detail"));
    }

    #[test]
    fn tool_calls_are_projected_to_titles_and_updated_in_place() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        apply_event(
            &mut states,
            &event(
                "acp_read",
                Some(CH),
                session_update(serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "title": "Shell command",
                    "kind": "execute",
                    "rawInput": {"command": "cat /etc/secret"},
                })),
            ),
        );
        let state = &states[CH];
        let tool = state.activity.back().unwrap();
        assert_eq!(tool.title, "Shell command");
        assert_eq!(tool.status, "running");
        assert!(!state
            .payload_json("t".into())
            .to_string()
            .contains("/etc/secret"));

        apply_event(
            &mut states,
            &event(
                "acp_read",
                Some(CH),
                session_update(serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "t1",
                    "status": "completed",
                })),
            ),
        );
        assert_eq!(states[CH].activity.back().unwrap().status, "complete");
    }

    #[test]
    fn titles_are_truncated_and_activity_is_capped() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        let long_title = "x".repeat(500);
        for _ in 0..(MAX_ACTIVITY_ENTRIES + 10) {
            apply_event(
                &mut states,
                &event(
                    "acp_read",
                    Some(CH),
                    session_update(serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "title": long_title,
                    })),
                ),
            );
        }
        let state = &states[CH];
        assert_eq!(state.activity.len(), MAX_ACTIVITY_ENTRIES);
        assert!(state.activity.back().unwrap().title.chars().count() <= MAX_TITLE_LEN);
    }

    #[test]
    fn streaming_chunks_collapse_into_one_running_entry() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        for _ in 0..5 {
            apply_event(
                &mut states,
                &event(
                    "acp_read",
                    Some(CH),
                    session_update(serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"text": "private streamed text"},
                    })),
                ),
            );
        }
        let state = &states[CH];
        let streams = state
            .activity
            .iter()
            .filter(|entry| entry.kind == "message")
            .count();
        assert_eq!(streams, 1);
        assert!(!state
            .payload_json("t".into())
            .to_string()
            .contains("private streamed"));
    }

    #[test]
    fn model_is_captured_from_session_config() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event(
                "session_config_captured",
                Some(CH),
                serde_json::json!({"models": {"currentModelId": "gpt-5.6-sol"}}),
            ),
        );
        assert_eq!(states[CH].model.as_deref(), Some("gpt-5.6-sol"));
    }

    #[test]
    fn unknown_frames_and_channelless_events_are_ignored() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("acp_write", Some(CH), serde_json::json!({"x": 1})),
        );
        apply_event(
            &mut states,
            &event("turn_started", None, serde_json::json!({})),
        );
        assert!(states.get(CH).is_none_or(|s| s.status == "idle"));
        assert_eq!(states.len(), 1);
    }

    #[test]
    fn publish_gating_is_urgent_or_floored() {
        let mut state = ChannelWork {
            dirty: true,
            urgent: false,
            last_published: Some(Instant::now()),
            ..ChannelWork::default()
        };
        let now = Instant::now();
        assert!(!state.should_publish(now));
        state.urgent = true;
        assert!(state.should_publish(now));
        state.urgent = false;
        state.last_published = Some(now - MIN_PUBLISH_INTERVAL);
        assert!(state.should_publish(now));
        state.dirty = false;
        assert!(!state.should_publish(now));
    }

    #[test]
    fn status_event_envelope_is_channel_scoped_nip_aw() {
        let keys = nostr::Keys::generate();
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        let signed = build_status_event(&keys, CH, &states[CH]).unwrap();
        assert_eq!(
            signed.kind,
            nostr::Kind::Custom(buzz_core::kind::KIND_AGENT_WORK_STATUS as u16)
        );
        let tag_value = |name: &str| {
            signed.tags.iter().find_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().map(|part| part.as_str()) == Some(name))
                    .then(|| parts.get(1).map(|part| part.as_str()))
                    .flatten()
            })
        };
        // The relay requires `h` for channel scoping and rejects `d` != `h`.
        assert_eq!(tag_value("h"), Some(CH));
        assert_eq!(tag_value("d"), Some(CH));
    }

    #[test]
    fn payload_carries_telemetry_markers_and_snake_free_keys() {
        let mut states = HashMap::new();
        apply_event(
            &mut states,
            &event("turn_started", Some(CH), serde_json::json!({})),
        );
        let json = states[CH].payload_json("2026-08-22T19:00:10Z".into());
        assert_eq!(json["v"], 1);
        assert_eq!(json["source"], "buzz-acp");
        assert_eq!(json["status"], "working");
        assert_eq!(json["turnStartedAt"], "2026-08-22T18:59:59Z");
        assert_eq!(json["updatedAt"], "2026-08-22T19:00:10Z");
        assert!(json.get("completedAt").is_none());
    }
}
