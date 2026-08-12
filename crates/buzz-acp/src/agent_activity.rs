use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use buzz_core::agent_activity::{
    AgentActivity, AgentActivityClass, AgentActivityFrame, AgentActivityStatus,
    AgentActivityToolKind, AgentActivityUsage, AGENT_ACTIVITY_FRAME_VERSION,
    AGENT_ACTIVITY_MAX_DURATION_MS, AGENT_ACTIVITY_MAX_ITEMS, AGENT_ACTIVITY_MAX_TOKEN_COUNT,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

const MAX_TRACKED_TURNS: usize = 256;
const MAX_RAW_ID_BYTES: usize = 128;
/// A global two-second cadence caps summary traffic at 30 events/minute.
pub(crate) const ACTIVITY_PUBLISH_TICK: Duration = Duration::from_secs(2);
/// Shutdown draining stays below the relay's 10 frames/second admission limit.
const ACTIVITY_SHUTDOWN_PUBLISH_INTERVAL: Duration = Duration::from_millis(125);

pub(crate) struct ProjectedActivity {
    pub(crate) channel_id: Uuid,
    pub(crate) activity: AgentActivity,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TurnKey {
    channel_id: Uuid,
    raw_turn_id: String,
}

#[derive(Clone, Copy)]
struct TurnState {
    activity_id: Uuid,
    started_at: DateTime<Utc>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ToolKey {
    channel_id: Uuid,
    raw_turn_id: String,
    raw_tool_id: String,
}

#[derive(Clone, Copy)]
struct ToolState {
    activity_id: Uuid,
    started_at: DateTime<Utc>,
    tool_kind: AgentActivityToolKind,
}

#[derive(Default)]
pub(crate) struct ActivityProjector {
    turns: HashMap<TurnKey, TurnState>,
    turn_order: VecDeque<TurnKey>,
    tools: HashMap<ToolKey, ToolState>,
    tool_order: VecDeque<ToolKey>,
}

impl ActivityProjector {
    pub(crate) fn project(
        &mut self,
        event: &crate::observer::ObserverEvent,
    ) -> Option<ProjectedActivity> {
        match event.kind.as_str() {
            "turn_started" => self.project_turn_started(event),
            "turn_liveness" => self.project_turn_running(event),
            "agent_activity_turn_terminal" => self.project_turn_terminal(event),
            "agent_activity_turn_usage" => self.project_turn_usage(event),
            "acp_read" => self.project_acp_tool(event),
            _ => None,
        }
    }

    fn turn_event_fields(
        event: &crate::observer::ObserverEvent,
    ) -> Option<(Uuid, DateTime<Utc>, TurnKey)> {
        let channel_id = event.channel_id.as_deref()?.parse().ok()?;
        let occurred_at = event.timestamp.parse().ok()?;
        let raw_turn_id = bounded_raw_id(event.turn_id.as_deref()?)?.to_owned();
        Some((
            channel_id,
            occurred_at,
            TurnKey {
                channel_id,
                raw_turn_id,
            },
        ))
    }

    fn project_turn_started(
        &mut self,
        event: &crate::observer::ObserverEvent,
    ) -> Option<ProjectedActivity> {
        let (channel_id, occurred_at, key) = Self::turn_event_fields(event)?;
        let state = if let Some(state) = self.turns.get(&key) {
            *state
        } else {
            while self.turns.len() >= MAX_TRACKED_TURNS {
                let oldest = self.turn_order.pop_front()?;
                self.turns.remove(&oldest);
            }
            let state = TurnState {
                activity_id: Uuid::new_v4(),
                started_at: occurred_at,
            };
            self.turns.insert(key.clone(), state);
            self.turn_order.push_back(key);
            state
        };
        Some(ProjectedActivity {
            channel_id,
            activity: turn_activity(
                state.activity_id,
                occurred_at,
                AgentActivityStatus::Started,
                None,
            ),
        })
    }

    fn project_turn_running(
        &mut self,
        event: &crate::observer::ObserverEvent,
    ) -> Option<ProjectedActivity> {
        let (channel_id, occurred_at, key) = Self::turn_event_fields(event)?;
        let state = *self.turns.get(&key)?;
        (occurred_at >= state.started_at).then_some(ProjectedActivity {
            channel_id,
            activity: turn_activity(
                state.activity_id,
                occurred_at,
                AgentActivityStatus::Running,
                None,
            ),
        })
    }

    fn project_turn_terminal(
        &mut self,
        event: &crate::observer::ObserverEvent,
    ) -> Option<ProjectedActivity> {
        let (channel_id, occurred_at, key) = Self::turn_event_fields(event)?;
        let status = match event.payload.get("status")?.as_str()? {
            "completed" => AgentActivityStatus::Completed,
            "failed" => AgentActivityStatus::Failed,
            "cancelled" => AgentActivityStatus::Cancelled,
            _ => return None,
        };
        let state = *self.turns.get(&key)?;
        let elapsed_ms = occurred_at
            .signed_duration_since(state.started_at)
            .num_milliseconds();
        let duration_ms = u64::try_from(elapsed_ms)
            .ok()?
            .min(AGENT_ACTIVITY_MAX_DURATION_MS);
        self.turns.remove(&key);
        self.turn_order.retain(|pending| pending != &key);
        Some(ProjectedActivity {
            channel_id,
            activity: turn_activity(state.activity_id, occurred_at, status, Some(duration_ms)),
        })
    }

    fn project_turn_usage(
        &mut self,
        event: &crate::observer::ObserverEvent,
    ) -> Option<ProjectedActivity> {
        let (channel_id, occurred_at, key) = Self::turn_event_fields(event)?;
        self.turns.get(&key)?;
        if !event.payload.get("deltaReliable")?.as_bool()? {
            return None;
        }
        let usage = AgentActivityUsage {
            input_tokens: bounded_token_count(&event.payload, "inputTokens")?,
            output_tokens: bounded_token_count(&event.payload, "outputTokens")?,
            total_tokens: bounded_token_count(&event.payload, "totalTokens")?,
            cache_read_tokens: bounded_token_count(&event.payload, "cacheReadTokens")?,
            cache_write_tokens: bounded_token_count(&event.payload, "cacheWriteTokens")?,
        };
        if [
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
        ]
        .iter()
        .all(Option::is_none)
        {
            return None;
        }
        Some(ProjectedActivity {
            channel_id,
            activity: AgentActivity {
                activity_id: Uuid::new_v4(),
                occurred_at,
                activity_class: AgentActivityClass::Usage,
                status: AgentActivityStatus::Completed,
                tool_kind: None,
                duration_ms: None,
                usage: Some(usage),
            },
        })
    }

    fn project_acp_tool(
        &mut self,
        event: &crate::observer::ObserverEvent,
    ) -> Option<ProjectedActivity> {
        if event.payload.get("method")?.as_str()? != "session/update" {
            return None;
        }
        let channel_id = event.channel_id.as_deref()?.parse().ok()?;
        let occurred_at: DateTime<Utc> = event.timestamp.parse().ok()?;
        let raw_turn_id = bounded_raw_id(event.turn_id.as_deref()?)?.to_owned();
        let update = event.payload.pointer("/params/update")?.as_object()?;
        let update_type = update.get("sessionUpdate")?.as_str()?;
        let raw_tool_id = bounded_raw_id(update.get("toolCallId")?.as_str()?)?.to_owned();
        let key = ToolKey {
            channel_id,
            raw_turn_id,
            raw_tool_id,
        };

        match update_type {
            "tool_call" => {
                let status = match update.get("status")?.as_str()? {
                    "pending" => AgentActivityStatus::Pending,
                    "in_progress" => AgentActivityStatus::Running,
                    _ => return None,
                };
                let tool_kind = safe_tool_kind(update.get("kind")?.as_str()?);
                let state = if let Some(state) = self.tools.get(&key) {
                    *state
                } else {
                    while self.tools.len() >= MAX_TRACKED_TURNS {
                        let oldest = self.tool_order.pop_front()?;
                        self.tools.remove(&oldest);
                    }
                    let state = ToolState {
                        activity_id: Uuid::new_v4(),
                        started_at: occurred_at,
                        tool_kind,
                    };
                    self.tools.insert(key.clone(), state);
                    self.tool_order.push_back(key);
                    state
                };
                Some(ProjectedActivity {
                    channel_id,
                    activity: tool_activity(
                        state.activity_id,
                        occurred_at,
                        status,
                        state.tool_kind,
                        None,
                    ),
                })
            }
            "tool_call_update" => {
                let status = match update.get("status")?.as_str()? {
                    "in_progress" => AgentActivityStatus::Running,
                    "completed" => AgentActivityStatus::Completed,
                    "failed" => AgentActivityStatus::Failed,
                    "cancelled" => AgentActivityStatus::Cancelled,
                    _ => return None,
                };
                let state = *self.tools.get(&key)?;
                if occurred_at < state.started_at {
                    return None;
                }
                let terminal = matches!(
                    status,
                    AgentActivityStatus::Completed
                        | AgentActivityStatus::Failed
                        | AgentActivityStatus::Cancelled
                );
                let duration_ms = terminal.then(|| {
                    u64::try_from(
                        occurred_at
                            .signed_duration_since(state.started_at)
                            .num_milliseconds(),
                    )
                    .expect("non-negative duration checked")
                    .min(AGENT_ACTIVITY_MAX_DURATION_MS)
                });
                if terminal {
                    self.tools.remove(&key);
                    self.tool_order.retain(|pending| pending != &key);
                }
                Some(ProjectedActivity {
                    channel_id,
                    activity: tool_activity(
                        state.activity_id,
                        occurred_at,
                        status,
                        state.tool_kind,
                        duration_ms,
                    ),
                })
            }
            _ => None,
        }
    }
}

fn is_terminal_status(status: AgentActivityStatus) -> bool {
    matches!(
        status,
        AgentActivityStatus::Completed
            | AgentActivityStatus::Failed
            | AgentActivityStatus::Cancelled
    )
}

fn safe_tool_kind(value: &str) -> AgentActivityToolKind {
    match value {
        "read" => AgentActivityToolKind::Read,
        "edit" => AgentActivityToolKind::Edit,
        "delete" => AgentActivityToolKind::Delete,
        "move" => AgentActivityToolKind::Move,
        "search" => AgentActivityToolKind::Search,
        "execute" => AgentActivityToolKind::Execute,
        "think" => AgentActivityToolKind::Think,
        "fetch" => AgentActivityToolKind::Fetch,
        "switch_mode" => AgentActivityToolKind::SwitchMode,
        "other" => AgentActivityToolKind::Other,
        _ => AgentActivityToolKind::Other,
    }
}

fn tool_activity(
    activity_id: Uuid,
    occurred_at: DateTime<Utc>,
    status: AgentActivityStatus,
    tool_kind: AgentActivityToolKind,
    duration_ms: Option<u64>,
) -> AgentActivity {
    AgentActivity {
        activity_id,
        occurred_at,
        activity_class: AgentActivityClass::Tool,
        status,
        tool_kind: Some(tool_kind),
        duration_ms,
        usage: None,
    }
}

fn turn_activity(
    activity_id: Uuid,
    occurred_at: DateTime<Utc>,
    status: AgentActivityStatus,
    duration_ms: Option<u64>,
) -> AgentActivity {
    AgentActivity {
        activity_id,
        occurred_at,
        activity_class: AgentActivityClass::Turn,
        status,
        tool_kind: None,
        duration_ms,
        usage: None,
    }
}

fn bounded_raw_id(value: &str) -> Option<&str> {
    (!value.is_empty() && value.len() <= MAX_RAW_ID_BYTES).then_some(value)
}

fn bounded_token_count(payload: &serde_json::Value, field: &str) -> Option<Option<u64>> {
    match payload.get(field) {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(value) => value
            .as_u64()
            .filter(|count| *count <= AGENT_ACTIVITY_MAX_TOKEN_COUNT)
            .map(Some),
    }
}

pub(crate) fn usage_observer_payload(usage: &crate::usage::TurnUsage) -> Option<serde_json::Value> {
    if !usage.delta_reliable {
        return None;
    }
    let mut payload = serde_json::Map::new();
    payload.insert("deltaReliable".into(), serde_json::Value::Bool(true));
    for (field, count) in [
        ("inputTokens", usage.turn_input_tokens),
        ("outputTokens", usage.turn_output_tokens),
        ("totalTokens", usage.turn_total_tokens),
        ("cacheReadTokens", usage.turn_cache_read_tokens),
        ("cacheWriteTokens", usage.turn_cache_write_tokens),
    ] {
        if let Some(count) = count {
            if count > AGENT_ACTIVITY_MAX_TOKEN_COUNT {
                return None;
            }
            payload.insert(field.into(), serde_json::Value::from(count));
        }
    }
    (payload.len() > 1).then_some(serde_json::Value::Object(payload))
}

/// Aggregate bounds for activity waiting on the global publication pacer.
const ACTIVITY_PENDING_MAX_CHANNELS: usize = 128;
const ACTIVITY_PENDING_MAX_ITEMS: usize = 1_024;
const ACTIVITY_PENDING_MAX_BYTES: usize = 256 * 1_024;

struct QueuedActivity {
    sequence: u64,
    bytes: usize,
    activity: AgentActivity,
}

/// Bounded, per-channel activity FIFO with fair frame rotation.
#[derive(Default)]
pub(crate) struct ActivityPublishQueue {
    channels: HashMap<Uuid, VecDeque<QueuedActivity>>,
    channel_order: VecDeque<Uuid>,
    next_sequence: u64,
    pending_items: usize,
    pending_bytes: usize,
    dropped_items: u64,
    dropped_bytes: u64,
}

impl ActivityPublishQueue {
    /// Queue a sanitized activity update, returning `false` if accepting it
    /// caused any queued update to be dropped.
    pub(crate) fn ingest(&mut self, projected: ProjectedActivity) -> bool {
        let bytes = match serde_json::to_vec(&projected.activity) {
            Ok(serialized) => serialized.len(),
            Err(error) => {
                tracing::warn!("failed to size sanitized agent activity: {error}");
                return false;
            }
        };
        let channel_id = projected.channel_id;
        if let Some(activities) = self.channels.get_mut(&channel_id) {
            let mut removed_items = 0usize;
            let mut removed_bytes = 0usize;
            activities.retain(|queued| {
                if queued.activity.activity_id == projected.activity.activity_id {
                    removed_items += 1;
                    removed_bytes += queued.bytes;
                    false
                } else {
                    true
                }
            });
            self.pending_items -= removed_items;
            self.pending_bytes -= removed_bytes;
        } else {
            self.channels.insert(channel_id, VecDeque::new());
            self.channel_order.push_back(channel_id);
        }
        self.next_sequence = self.next_sequence.wrapping_add(1);
        if let Some(activities) = self.channels.get_mut(&channel_id) {
            activities.push_back(QueuedActivity {
                sequence: self.next_sequence,
                bytes,
                activity: projected.activity,
            });
        } else {
            return false;
        }
        self.pending_items += 1;
        self.pending_bytes += bytes;
        self.enforce_bounds()
    }

    fn enforce_bounds(&mut self) -> bool {
        let mut dropped_items = 0u64;
        let mut dropped_bytes = 0u64;
        while self.channels.len() > ACTIVITY_PENDING_MAX_CHANNELS
            || self.pending_items > ACTIVITY_PENDING_MAX_ITEMS
            || self.pending_bytes > ACTIVITY_PENDING_MAX_BYTES
        {
            let Some((channel_id, index)) = self.oldest_item() else {
                break;
            };
            let Some(queued) = self
                .channels
                .get_mut(&channel_id)
                .and_then(|activities| activities.remove(index))
            else {
                break;
            };
            self.pending_items -= 1;
            self.pending_bytes -= queued.bytes;
            dropped_items += 1;
            dropped_bytes += queued.bytes as u64;
            self.remove_empty_channel(channel_id);
        }
        if dropped_items > 0 {
            self.dropped_items += dropped_items;
            self.dropped_bytes += dropped_bytes;
            tracing::warn!(
                dropped_items,
                dropped_bytes,
                total_dropped_items = self.dropped_items,
                pending_items = self.pending_items,
                pending_bytes = self.pending_bytes,
                "agent activity queue over bound; dropped oldest updates"
            );
        }
        dropped_items == 0
    }

    fn oldest_item(&self) -> Option<(Uuid, usize)> {
        self.channels
            .iter()
            .flat_map(|(channel_id, activities)| {
                activities.iter().enumerate().map(move |(index, queued)| {
                    (
                        *channel_id,
                        index,
                        is_terminal_status(queued.activity.status),
                        queued.sequence,
                    )
                })
            })
            .min_by_key(|(_, _, terminal, sequence)| (*terminal, *sequence))
            .map(|(channel_id, index, _, _)| (channel_id, index))
    }

    fn remove_empty_channel(&mut self, channel_id: Uuid) {
        if self
            .channels
            .get(&channel_id)
            .is_some_and(VecDeque::is_empty)
        {
            self.channels.remove(&channel_id);
            self.channel_order.retain(|pending| *pending != channel_id);
        }
    }

    pub(crate) fn next_frame(&mut self) -> Option<(Uuid, AgentActivityFrame)> {
        let channel_id = self.channel_order.pop_front()?;
        let mut queued = self.channels.remove(&channel_id)?;
        let mut activities = Vec::new();

        while activities.len() < AGENT_ACTIVITY_MAX_ITEMS {
            let Some(next) = queued.pop_front() else {
                break;
            };
            let mut candidate = activities.clone();
            candidate.push(next.activity.clone());
            let frame = AgentActivityFrame {
                version: AGENT_ACTIVITY_FRAME_VERSION,
                activities: candidate,
            };
            if frame.to_json().is_err() {
                queued.push_front(next);
                break;
            }
            activities.push(next.activity);
            self.pending_items -= 1;
            self.pending_bytes -= next.bytes;
        }

        if !queued.is_empty() {
            self.channels.insert(channel_id, queued);
            self.channel_order.push_back(channel_id);
        }
        if activities.is_empty() {
            return None;
        }
        Some((
            channel_id,
            AgentActivityFrame {
                version: AGENT_ACTIVITY_FRAME_VERSION,
                activities,
            },
        ))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending_items == 0
    }

    #[cfg(test)]
    fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

pub(crate) struct RelayActivityPublisherTask {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<bool>,
}

impl RelayActivityPublisherTask {
    #[cfg(test)]
    pub(crate) fn abort(self) {
        self.handle.abort();
    }

    /// Stop intake, drain already-delivered updates, and enqueue sanitized
    /// frames into the relay transport before returning.
    pub(crate) async fn shutdown(mut self, timeout: Duration) -> bool {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        let abort_handle = self.handle.abort_handle();
        match tokio::time::timeout(timeout, self.handle).await {
            Ok(Ok(drained)) => drained,
            Ok(Err(error)) => {
                tracing::warn!("agent activity publisher exited during shutdown: {error}");
                false
            }
            Err(_) => {
                tracing::warn!("agent activity publisher drain timed out; aborting");
                abort_handle.abort();
                false
            }
        }
    }
}

pub(crate) fn spawn_relay_activity_publisher(
    observer: crate::observer::ObserverHandle,
    publisher: crate::relay::RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    channel_info: crate::pool::ChannelInfoResolver,
) -> RelayActivityPublisherTask {
    // Subscribe synchronously so activity emitted immediately after this call is
    // live input, while pre-existing snapshot entries remain intentionally absent.
    let rx = observer.subscribe();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        run_relay_activity_publisher(
            rx,
            shutdown_rx,
            publisher,
            keys,
            agent_pubkey_hex,
            channel_info,
        )
        .await
    });
    RelayActivityPublisherTask {
        shutdown_tx: Some(shutdown_tx),
        handle,
    }
}

async fn run_relay_activity_publisher(
    mut rx: tokio::sync::broadcast::Receiver<crate::observer::ObserverEvent>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    publisher: crate::relay::RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    channel_info: crate::pool::ChannelInfoResolver,
) -> bool {
    let mut projector = ActivityProjector::default();
    let mut queue = ActivityPublishQueue::default();
    let mut all_enqueued = true;
    let mut publish_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + ACTIVITY_PUBLISH_TICK,
        ACTIVITY_PUBLISH_TICK,
    );
    publish_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut closed = false;

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                loop {
                    match rx.try_recv() {
                        Ok(event) => {
                            if let Some(projected) = projector.project(&event) {
                                all_enqueued &= queue.ingest(projected);
                            }
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                            all_enqueued = false;
                            tracing::warn!(
                                dropped = count,
                                "agent activity publisher lagged during shutdown"
                            );
                        }
                        Err(
                            tokio::sync::broadcast::error::TryRecvError::Empty
                            | tokio::sync::broadcast::error::TryRecvError::Closed,
                        ) => break,
                    }
                }
                while !queue.is_empty() {
                    all_enqueued &= publish_next_activity_frame(
                        &mut queue,
                        &publisher,
                        &keys,
                        &agent_pubkey_hex,
                        &channel_info,
                    ).await;
                    if !queue.is_empty() {
                        tokio::time::sleep(ACTIVITY_SHUTDOWN_PUBLISH_INTERVAL).await;
                    }
                }
                break;
            }
            result = rx.recv(), if !closed => {
                match result {
                    Ok(event) => {
                        if let Some(projected) = projector.project(&event) {
                            all_enqueued &= queue.ingest(projected);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        all_enqueued = false;
                        tracing::warn!(dropped = count, "agent activity publisher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        closed = true;
                    }
                }
            }
            _ = publish_tick.tick() => {
                all_enqueued &= publish_next_activity_frame(
                    &mut queue,
                    &publisher,
                    &keys,
                    &agent_pubkey_hex,
                    &channel_info,
                ).await;
                if closed && queue.is_empty() {
                    break;
                }
            }
        }
    }
    all_enqueued
}

async fn publish_next_activity_frame(
    queue: &mut ActivityPublishQueue,
    publisher: &crate::relay::RelayEventPublisher,
    keys: &nostr::Keys,
    agent_pubkey_hex: &str,
    channel_info: &crate::pool::ChannelInfoResolver,
) -> bool {
    let Some((channel_id, frame)) = queue.next_frame() else {
        return true;
    };
    let channel_type = channel_info
        .resolve(channel_id)
        .await
        .map(|info| info.channel_type);
    if is_shared_activity_channel_type(channel_type.as_deref()) {
        publish_activity_frame(publisher, keys, agent_pubkey_hex, channel_id, frame).await
    } else {
        tracing::debug!(
            channel_id = %channel_id,
            "sanitized agent activity suppressed for non-shared channel"
        );
        true
    }
}

fn is_shared_activity_channel_type(channel_type: Option<&str>) -> bool {
    matches!(channel_type, Some("stream" | "forum"))
}

async fn publish_activity_frame(
    publisher: &crate::relay::RelayEventPublisher,
    keys: &nostr::Keys,
    agent_pubkey_hex: &str,
    channel_id: Uuid,
    frame: AgentActivityFrame,
) -> bool {
    let builder = match buzz_sdk::build_agent_activity_summary(channel_id, agent_pubkey_hex, &frame)
    {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!("failed to build sanitized agent activity: {error}");
            return false;
        }
    };
    let signed = match builder.sign_with_keys(keys) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!("failed to sign sanitized agent activity: {error}");
            return false;
        }
    };
    match publisher.publish_event(signed).await {
        Ok(()) => true,
        Err(error) => {
            // Summary publication is telemetry: relay failure must never surface to
            // or delay the prompt task that generated it, but shutdown must report
            // that not every produced frame reached the relay transport.
            tracing::warn!("sanitized agent activity dropped: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use buzz_core::agent_activity::{
        AgentActivityClass, AgentActivityStatus, AgentActivityToolKind,
    };
    use uuid::Uuid;

    fn observer_event(
        kind: &str,
        channel_id: Uuid,
        turn_id: &str,
        payload: serde_json::Value,
    ) -> crate::observer::ObserverEvent {
        observer_event_at(kind, channel_id, turn_id, "2026-08-12T10:00:00Z", payload)
    }

    fn observer_event_at(
        kind: &str,
        channel_id: Uuid,
        turn_id: &str,
        timestamp: &str,
        payload: serde_json::Value,
    ) -> crate::observer::ObserverEvent {
        crate::observer::ObserverEvent {
            seq: 1,
            timestamp: timestamp.to_string(),
            kind: kind.to_string(),
            agent_index: Some(0),
            channel_id: Some(channel_id.to_string()),
            session_id: Some("raw-session-secret".to_string()),
            turn_id: Some(turn_id.to_string()),
            started_at: Some("2026-08-12T09:59:59Z".to_string()),
            payload,
        }
    }

    #[test]
    fn turn_started_projects_only_closed_safe_fields_with_opaque_id() {
        let channel_id = Uuid::new_v4();
        let raw_turn_id = "raw-turn-secret";
        let event = observer_event(
            "turn_started",
            channel_id,
            raw_turn_id,
            serde_json::json!({
                "prompt": "SECRET PROMPT",
                "thought": "SECRET THOUGHT",
                "plan": "SECRET PLAN",
                "message": "SECRET MESSAGE",
                "title": "SECRET TITLE",
                "args": {"path": "/secret/path", "url": "https://secret.invalid"},
                "result": "SECRET RESULT",
                "error": "SECRET ERROR",
                "triggeringEventIds": ["raw-event-secret"]
            }),
        );

        let mut projector = ActivityProjector::default();
        let projected = projector.project(&event).expect("safe turn update");
        assert_eq!(projected.channel_id, channel_id);
        assert_eq!(projected.activity.activity_class, AgentActivityClass::Turn);
        assert_eq!(projected.activity.status, AgentActivityStatus::Started);
        assert_ne!(projected.activity.activity_id.to_string(), raw_turn_id);

        let serialized = serde_json::to_string(&projected.activity).unwrap();
        for secret in [
            "SECRET PROMPT",
            "SECRET THOUGHT",
            "SECRET PLAN",
            "SECRET MESSAGE",
            "SECRET TITLE",
            "SECRET RESULT",
            "SECRET ERROR",
            "/secret/path",
            "https://secret.invalid",
            "raw-event-secret",
            "raw-session-secret",
            raw_turn_id,
        ] {
            assert!(
                !serialized.contains(secret),
                "leaked {secret}: {serialized}"
            );
        }
    }

    #[test]
    fn turn_lifecycle_reuses_opaque_id_bounds_duration_and_removes_terminal_state() {
        let cases = [
            ("completed", AgentActivityStatus::Completed),
            ("failed", AgentActivityStatus::Failed),
            ("cancelled", AgentActivityStatus::Cancelled),
        ];

        for (terminal, expected_status) in cases {
            let channel_id = Uuid::new_v4();
            let raw_turn_id = format!("raw-turn-{terminal}");
            let mut projector = ActivityProjector::default();
            let started = projector
                .project(&observer_event_at(
                    "turn_started",
                    channel_id,
                    &raw_turn_id,
                    "2026-08-12T10:00:00Z",
                    serde_json::json!({}),
                ))
                .expect("started");
            let running = projector
                .project(&observer_event_at(
                    "turn_liveness",
                    channel_id,
                    &raw_turn_id,
                    "2026-08-12T10:00:01Z",
                    serde_json::json!({}),
                ))
                .expect("running");
            let terminal_update = projector
                .project(&observer_event_at(
                    "agent_activity_turn_terminal",
                    channel_id,
                    &raw_turn_id,
                    "2026-08-20T10:00:00Z",
                    serde_json::json!({"status": terminal}),
                ))
                .expect("terminal");

            assert_eq!(running.activity.status, AgentActivityStatus::Running);
            assert_eq!(running.activity.activity_id, started.activity.activity_id);
            assert_eq!(terminal_update.activity.status, expected_status);
            assert_eq!(
                terminal_update.activity.activity_id,
                started.activity.activity_id
            );
            assert_eq!(
                terminal_update.activity.duration_ms,
                Some(buzz_core::agent_activity::AGENT_ACTIVITY_MAX_DURATION_MS),
                "eight-day duration is capped at the core seven-day bound"
            );

            assert!(
                projector
                    .project(&observer_event_at(
                        "turn_liveness",
                        channel_id,
                        &raw_turn_id,
                        "2026-08-20T10:00:01Z",
                        serde_json::json!({}),
                    ))
                    .is_none(),
                "terminal removes raw turn state"
            );
        }
    }

    #[test]
    fn acp_tool_updates_project_only_closed_kind_status_and_opaque_id() {
        let channel_id = Uuid::new_v4();
        let raw_turn_id = "raw-turn-secret";
        let raw_tool_id = "raw-tool-secret";
        let mut projector = ActivityProjector::default();
        assert!(projector
            .project(&observer_event_at(
                "turn_started",
                channel_id,
                raw_turn_id,
                "2026-08-12T10:00:00Z",
                serde_json::json!({}),
            ))
            .is_some());

        let pending = projector
            .project(&observer_event_at(
                "acp_read",
                channel_id,
                raw_turn_id,
                "2026-08-12T10:00:01Z",
                serde_json::json!({
                    "method": "session/update",
                    "params": {"update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": raw_tool_id,
                        "kind": "read",
                        "status": "pending",
                        "title": "SECRET TITLE",
                        "name": "SECRET NAME",
                        "rawInput": {"path": "/secret/path", "url": "https://secret.invalid"},
                        "content": "SECRET CONTENT",
                        "result": "SECRET RESULT",
                        "error": "SECRET ERROR"
                    }}
                }),
            ))
            .expect("pending tool");
        let running = projector
            .project(&observer_event_at(
                "acp_read",
                channel_id,
                raw_turn_id,
                "2026-08-12T10:00:02Z",
                serde_json::json!({
                    "method": "session/update",
                    "params": {"update": {
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": raw_tool_id,
                        "status": "in_progress",
                        "content": "SECRET UPDATE"
                    }}
                }),
            ))
            .expect("running tool");
        let completed = projector
            .project(&observer_event_at(
                "acp_read",
                channel_id,
                raw_turn_id,
                "2026-08-12T10:00:03.250Z",
                serde_json::json!({
                    "method": "session/update",
                    "params": {"update": {
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": raw_tool_id,
                        "status": "completed",
                        "content": "SECRET OUTPUT",
                        "rawOutput": {"error": "SECRET RAW ERROR"}
                    }}
                }),
            ))
            .expect("completed tool");

        assert_eq!(pending.activity.activity_class, AgentActivityClass::Tool);
        assert_eq!(pending.activity.status, AgentActivityStatus::Pending);
        assert_eq!(
            pending.activity.tool_kind,
            Some(AgentActivityToolKind::Read)
        );
        assert_eq!(running.activity.status, AgentActivityStatus::Running);
        assert_eq!(running.activity.activity_id, pending.activity.activity_id);
        assert_eq!(completed.activity.status, AgentActivityStatus::Completed);
        assert_eq!(completed.activity.activity_id, pending.activity.activity_id);
        assert_eq!(completed.activity.duration_ms, Some(2_250));

        let serialized =
            serde_json::to_string(&[pending.activity, running.activity, completed.activity])
                .unwrap();
        for secret in [
            raw_turn_id,
            raw_tool_id,
            "SECRET TITLE",
            "SECRET NAME",
            "/secret/path",
            "https://secret.invalid",
            "SECRET CONTENT",
            "SECRET RESULT",
            "SECRET ERROR",
            "SECRET UPDATE",
            "SECRET OUTPUT",
            "SECRET RAW ERROR",
        ] {
            assert!(
                !serialized.contains(secret),
                "leaked {secret}: {serialized}"
            );
        }
        assert!(
            projector
                .project(&observer_event(
                    "acp_read",
                    channel_id,
                    raw_turn_id,
                    serde_json::json!({
                        "method": "session/update",
                        "params": {"update": {
                            "sessionUpdate": "tool_call_update",
                            "toolCallId": raw_tool_id,
                            "status": "failed"
                        }}
                    }),
                ))
                .is_none(),
            "terminal removes raw tool state"
        );
    }

    #[test]
    fn tool_kind_mapping_is_closed_and_unknown_kind_is_other() {
        let cases = [
            ("read", AgentActivityToolKind::Read),
            ("edit", AgentActivityToolKind::Edit),
            ("delete", AgentActivityToolKind::Delete),
            ("move", AgentActivityToolKind::Move),
            ("search", AgentActivityToolKind::Search),
            ("execute", AgentActivityToolKind::Execute),
            ("think", AgentActivityToolKind::Think),
            ("fetch", AgentActivityToolKind::Fetch),
            ("switch_mode", AgentActivityToolKind::SwitchMode),
            ("other", AgentActivityToolKind::Other),
            ("shell-with-secret-name", AgentActivityToolKind::Other),
        ];
        for (index, (kind, expected)) in cases.into_iter().enumerate() {
            let channel_id = Uuid::new_v4();
            let mut projector = ActivityProjector::default();
            let event = observer_event(
                "acp_read",
                channel_id,
                "turn-a",
                serde_json::json!({
                    "method": "session/update",
                    "params": {"update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": format!("tool-{index}"),
                        "kind": kind,
                        "status": "in_progress"
                    }}
                }),
            );
            assert_eq!(
                projector
                    .project(&event)
                    .expect("trusted tool event")
                    .activity
                    .tool_kind,
                Some(expected)
            );
        }
    }

    #[test]
    fn unsafe_or_unknown_tool_status_event_and_ids_emit_nothing() {
        let channel_id = Uuid::new_v4();
        let mut projector = ActivityProjector::default();
        let tool_call = |session_update: &str, status: &str, tool_id: &str| {
            observer_event(
                "acp_read",
                channel_id,
                "turn-a",
                serde_json::json!({
                    "method": "session/update",
                    "params": {"update": {
                        "sessionUpdate": session_update,
                        "toolCallId": tool_id,
                        "kind": "read",
                        "status": status
                    }}
                }),
            )
        };

        for event in [
            tool_call("tool_call", "user-controlled-status", "tool-a"),
            tool_call("untrusted_update", "pending", "tool-b"),
            tool_call("tool_call", "pending", ""),
            tool_call("tool_call", "pending", &"x".repeat(MAX_RAW_ID_BYTES + 1)),
        ] {
            assert!(projector.project(&event).is_none());
        }
        let mut wrong_method = tool_call("tool_call", "pending", "tool-c");
        wrong_method.payload["method"] = serde_json::json!("evil/update");
        assert!(projector.project(&wrong_method).is_none());
        assert!(projector
            .project(&tool_call("tool_call_update", "completed", "unknown-tool"))
            .is_none());
    }

    #[test]
    fn reliable_per_turn_usage_projects_counts_without_sensitive_or_cumulative_fields() {
        let channel_id = Uuid::new_v4();
        let raw_turn_id = "raw-turn-secret";
        let mut projector = ActivityProjector::default();
        assert!(projector
            .project(&observer_event(
                "turn_started",
                channel_id,
                raw_turn_id,
                serde_json::json!({}),
            ))
            .is_some());
        let projected = projector
            .project(&observer_event(
                "agent_activity_turn_usage",
                channel_id,
                raw_turn_id,
                serde_json::json!({
                    "deltaReliable": true,
                    "inputTokens": 11,
                    "outputTokens": 7,
                    "totalTokens": 18,
                    "cacheReadTokens": 3,
                    "cacheWriteTokens": 2,
                    "model": "SECRET MODEL",
                    "provider": "SECRET PROVIDER",
                    "costUsd": 99.0,
                    "cumulativeInputTokens": 999,
                    "sessionId": "SECRET SESSION"
                }),
            ))
            .expect("reliable per-turn usage");

        assert_eq!(projected.activity.activity_class, AgentActivityClass::Usage);
        assert_eq!(projected.activity.status, AgentActivityStatus::Completed);
        let usage = projected.activity.usage.as_ref().expect("usage counts");
        assert_eq!(usage.input_tokens, Some(11));
        assert_eq!(usage.output_tokens, Some(7));
        assert_eq!(usage.total_tokens, Some(18));
        assert_eq!(usage.cache_read_tokens, Some(3));
        assert_eq!(usage.cache_write_tokens, Some(2));
        let serialized = serde_json::to_string(&projected.activity).unwrap();
        for secret in [
            raw_turn_id,
            "SECRET MODEL",
            "SECRET PROVIDER",
            "SECRET SESSION",
            "costUsd",
            "cumulativeInputTokens",
        ] {
            assert!(
                !serialized.contains(secret),
                "leaked {secret}: {serialized}"
            );
        }
    }

    #[test]
    fn unreliable_empty_overflow_or_unknown_turn_usage_emits_nothing() {
        let channel_id = Uuid::new_v4();
        let mut projector = ActivityProjector::default();
        assert!(projector
            .project(&observer_event(
                "turn_started",
                channel_id,
                "known-turn",
                serde_json::json!({}),
            ))
            .is_some());
        for (turn_id, payload) in [
            (
                "known-turn",
                serde_json::json!({"deltaReliable": false, "inputTokens": 1}),
            ),
            ("known-turn", serde_json::json!({"deltaReliable": true})),
            (
                "known-turn",
                serde_json::json!({
                    "deltaReliable": true,
                    "inputTokens": buzz_core::agent_activity::AGENT_ACTIVITY_MAX_TOKEN_COUNT + 1
                }),
            ),
            (
                "unknown-turn",
                serde_json::json!({"deltaReliable": true, "inputTokens": 1}),
            ),
        ] {
            assert!(projector
                .project(&observer_event(
                    "agent_activity_turn_usage",
                    channel_id,
                    turn_id,
                    payload,
                ))
                .is_none());
        }
    }

    #[test]
    fn usage_observer_payload_exposes_only_reliable_per_turn_counts() {
        let reliable = crate::usage::TurnUsage {
            session_id: "SECRET SESSION".into(),
            turn_seq: 8,
            delta_reliable: true,
            turn_input_tokens: Some(11),
            turn_output_tokens: Some(7),
            turn_total_tokens: Some(18),
            turn_cost_usd: Some(99.0),
            turn_cache_read_tokens: Some(3),
            turn_cache_write_tokens: Some(2),
            cumulative_input_tokens: Some(1_111),
            cumulative_output_tokens: Some(777),
            cumulative_total_tokens: Some(1_888),
            cumulative_cost_usd: Some(999.0),
            cumulative_cache_read_tokens: Some(333),
            cumulative_cache_write_tokens: Some(222),
            model: Some("SECRET MODEL".into()),
            pricing_identity: None,
        };
        let payload = usage_observer_payload(&reliable).expect("reliable payload");
        assert_eq!(payload["inputTokens"], 11);
        assert_eq!(payload["outputTokens"], 7);
        assert_eq!(payload["totalTokens"], 18);
        assert_eq!(payload["cacheReadTokens"], 3);
        assert_eq!(payload["cacheWriteTokens"], 2);
        let serialized = payload.to_string();
        for forbidden in [
            "SECRET SESSION",
            "SECRET MODEL",
            "cost",
            "cumulative",
            "provider",
            "pricing",
            "turnSeq",
        ] {
            assert!(
                !serialized
                    .to_ascii_lowercase()
                    .contains(&forbidden.to_ascii_lowercase()),
                "leaked {forbidden}: {serialized}"
            );
        }

        let mut unreliable = reliable;
        unreliable.delta_reliable = false;
        assert!(usage_observer_payload(&unreliable).is_none());
    }

    #[test]
    fn terminal_updates_replace_pending_lifecycle_and_survive_overflow() {
        let channel_id = Uuid::from_u128(7);
        let activity_id = Uuid::from_u128(77);
        let mut queue = ActivityPublishQueue::default();
        for status in [
            AgentActivityStatus::Started,
            AgentActivityStatus::Running,
            AgentActivityStatus::Completed,
        ] {
            queue.ingest(ProjectedActivity {
                channel_id,
                activity: AgentActivity {
                    activity_id,
                    status,
                    duration_ms: (status == AgentActivityStatus::Completed).then_some(10),
                    ..test_turn_activity(77)
                },
            });
        }
        assert_eq!(
            queue.pending_items, 1,
            "terminal state supersedes queued lifecycle noise"
        );

        for index in 0..ACTIVITY_PENDING_MAX_ITEMS {
            queue.ingest(ProjectedActivity {
                channel_id,
                activity: test_turn_activity(100_000 + index as u128),
            });
        }

        assert_eq!(queue.pending_items, ACTIVITY_PENDING_MAX_ITEMS);
        assert!(
            queue.channels.values().flatten().any(|queued| {
                queued.activity.activity_id == activity_id
                    && queued.activity.status == AgentActivityStatus::Completed
            }),
            "terminal state has eviction priority over non-terminal activity"
        );
    }

    #[test]
    fn activity_queue_enforces_channel_item_and_byte_bounds_with_oldest_drop_metrics() {
        let mut queue = ActivityPublishQueue::default();
        let channels: Vec<_> = (0..=ACTIVITY_PENDING_MAX_CHANNELS)
            .map(|index| Uuid::from_u128(10_000 + index as u128))
            .collect();
        for (index, channel_id) in channels.iter().copied().enumerate() {
            queue.ingest(ProjectedActivity {
                channel_id,
                activity: test_turn_activity(index as u128 + 1),
            });
        }
        assert!(queue.channel_count() <= ACTIVITY_PENDING_MAX_CHANNELS);
        assert!(queue.pending_items <= ACTIVITY_PENDING_MAX_ITEMS);
        assert!(queue.pending_bytes <= ACTIVITY_PENDING_MAX_BYTES);
        assert_eq!(queue.dropped_items, 1, "oldest channel item evicted");
        assert!(queue.dropped_bytes > 0);
        assert!(!queue.channels.contains_key(&channels[0]));
        assert!(queue.channels.contains_key(channels.last().unwrap()));

        let newest_id = Uuid::from_u128(99_999);
        for index in 0..=ACTIVITY_PENDING_MAX_ITEMS {
            queue.ingest(ProjectedActivity {
                channel_id: channels[1],
                activity: AgentActivity {
                    activity_id: if index == ACTIVITY_PENDING_MAX_ITEMS {
                        newest_id
                    } else {
                        Uuid::from_u128(20_000 + index as u128)
                    },
                    ..test_turn_activity(500_000 + index as u128)
                },
            });
        }
        assert!(queue.channel_count() <= ACTIVITY_PENDING_MAX_CHANNELS);
        assert!(queue.pending_items <= ACTIVITY_PENDING_MAX_ITEMS);
        assert!(queue.pending_bytes <= ACTIVITY_PENDING_MAX_BYTES);
        assert!(queue.dropped_items > 1);
        let retained_newest = queue
            .channels
            .values()
            .flatten()
            .any(|queued| queued.activity.activity_id == newest_id);
        assert!(
            retained_newest,
            "newest item survives oldest-first eviction"
        );
    }

    #[test]
    fn activity_queue_overflow_is_reported_to_caller() {
        let channel_id = Uuid::from_u128(42);
        let mut queue = ActivityPublishQueue::default();
        for index in 0..ACTIVITY_PENDING_MAX_ITEMS {
            assert!(
                queue.ingest(ProjectedActivity {
                    channel_id,
                    activity: test_turn_activity(index as u128 + 1),
                }),
                "items within the queue bound must be accepted without loss"
            );
        }

        assert!(
            !queue.ingest(ProjectedActivity {
                channel_id,
                activity: test_turn_activity(ACTIVITY_PENDING_MAX_ITEMS as u128 + 1),
            }),
            "evicting a queued update must be reported to the publisher"
        );
        assert_eq!(queue.dropped_items, 1);
    }

    #[test]
    fn activity_frames_use_core_limits_and_rotate_channels_fairly() {
        let channel_a = Uuid::from_u128(1);
        let channel_b = Uuid::from_u128(2);
        let channel_c = Uuid::from_u128(3);
        let mut queue = ActivityPublishQueue::default();
        for index in 0..40 {
            queue.ingest(ProjectedActivity {
                channel_id: channel_a,
                activity: test_turn_activity(100 + index),
            });
        }
        for (channel_id, index) in [(channel_b, 1_000), (channel_c, 2_000)] {
            queue.ingest(ProjectedActivity {
                channel_id,
                activity: test_turn_activity(index),
            });
        }

        let first = queue.next_frame().expect("channel a frame");
        let second = queue.next_frame().expect("channel b frame");
        let third = queue.next_frame().expect("channel c frame");
        let fourth = queue.next_frame().expect("remaining channel a frame");
        assert_eq!(
            [first.0, second.0, third.0, fourth.0],
            [channel_a, channel_b, channel_c, channel_a]
        );
        for (_, frame) in [first, second, third, fourth] {
            let json = frame.to_json().expect("core-valid frame");
            assert!(frame.activities.len() <= buzz_core::agent_activity::AGENT_ACTIVITY_MAX_ITEMS);
            assert!(json.len() <= buzz_core::agent_activity::AGENT_ACTIVITY_MAX_FRAME_BYTES);
        }
        assert!(queue.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn live_only_publisher_skips_replay_dm_and_caps_one_frame_per_tick() {
        let stream_id = Uuid::from_u128(101);
        let forum_id = Uuid::from_u128(102);
        let dm_id = Uuid::from_u128(103);
        let observer = crate::observer::ObserverHandle::in_process();
        observer.emit(
            "turn_started",
            Some(0),
            &crate::observer::context_for(
                Some(stream_id),
                Some("replay-secret-session".into()),
                Some("replay-secret-turn".into()),
            ),
            serde_json::json!({"prompt": "REPLAY SECRET"}),
        );

        let resolver = crate::pool::ChannelInfoResolver::new(
            HashMap::from([
                (
                    stream_id,
                    crate::relay::ChannelInfo {
                        name: "stream".into(),
                        channel_type: "stream".into(),
                        description: None,
                    },
                ),
                (
                    forum_id,
                    crate::relay::ChannelInfo {
                        name: "forum".into(),
                        channel_type: "forum".into(),
                        description: None,
                    },
                ),
                (
                    dm_id,
                    crate::relay::ChannelInfo {
                        name: "dm".into(),
                        channel_type: "dm".into(),
                        description: None,
                    },
                ),
            ]),
            crate::relay::RestClient {
                http: reqwest::Client::new(),
                base_url: "http://127.0.0.1:9".into(),
                keys: nostr::Keys::generate(),
                auth_tag_json: None,
            },
        );
        let (publisher, mut published) = crate::relay::RelayEventPublisher::test_pair();
        let keys = nostr::Keys::generate();
        let agent_pubkey = keys.public_key().to_hex();
        let handle = spawn_relay_activity_publisher(
            observer.clone(),
            publisher,
            keys,
            agent_pubkey.clone(),
            resolver,
        );

        tokio::time::advance(ACTIVITY_PUBLISH_TICK).await;
        tokio::task::yield_now().await;
        assert!(
            published.try_recv().is_err(),
            "snapshot/replay must not publish"
        );

        for (channel_id, turn_id) in [
            (dm_id, "dm-secret-turn"),
            (stream_id, "stream-secret-turn"),
            (forum_id, "forum-secret-turn"),
        ] {
            observer.emit(
                "turn_started",
                Some(0),
                &crate::observer::context_for(
                    Some(channel_id),
                    Some("live-secret-session".into()),
                    Some(turn_id.into()),
                ),
                serde_json::json!({"message": "LIVE SECRET MESSAGE"}),
            );
        }
        tokio::task::yield_now().await;

        tokio::time::advance(ACTIVITY_PUBLISH_TICK).await;
        tokio::task::yield_now().await;
        assert!(published.try_recv().is_err(), "DM frame must be discarded");

        tokio::time::advance(ACTIVITY_PUBLISH_TICK).await;
        tokio::task::yield_now().await;
        let stream = published
            .try_recv()
            .expect("stream frame on second live tick");
        assert_eq!(stream.kind.as_u16(), 24_201);
        let stream_json = stream.content.clone();
        assert!(!stream_json.contains("SECRET"));
        assert_eq!(
            AgentActivityFrame::parse(&stream_json)
                .unwrap()
                .activities
                .len(),
            1
        );
        assert_eq!(
            stream
                .tags
                .iter()
                .map(|tag| tag.as_slice())
                .collect::<Vec<_>>(),
            vec![
                &["h".to_string(), stream_id.to_string()][..],
                &["agent".to_string(), agent_pubkey.clone()][..],
            ]
        );
        assert!(published.try_recv().is_err(), "at most one frame per tick");

        tokio::time::advance(ACTIVITY_PUBLISH_TICK).await;
        tokio::task::yield_now().await;
        let forum = published.try_recv().expect("forum frame on next tick");
        assert!(forum
            .tags
            .iter()
            .any(|tag| { tag.as_slice() == ["h".to_string(), forum_id.to_string()] }));
        assert!(published.try_recv().is_err());
        handle.abort();
    }

    fn activity_test_resolver(channel_id: Uuid) -> crate::pool::ChannelInfoResolver {
        crate::pool::ChannelInfoResolver::new(
            HashMap::from([(
                channel_id,
                crate::relay::ChannelInfo {
                    name: "stream".into(),
                    channel_type: "stream".into(),
                    description: None,
                },
            )]),
            crate::relay::RestClient {
                http: reqwest::Client::new(),
                base_url: "http://127.0.0.1:9".into(),
                keys: nostr::Keys::generate(),
                auth_tag_json: None,
            },
        )
    }

    fn emit_terminal_activity(observer: &crate::observer::ObserverHandle, channel_id: Uuid) {
        let context = crate::observer::context_for(
            Some(channel_id),
            Some("raw-session-secret".into()),
            Some("raw-turn-secret".into()),
        );
        observer.emit(
            "turn_started",
            Some(0),
            &context,
            serde_json::json!({"prompt": "SECRET"}),
        );
        observer.emit(
            "agent_activity_turn_terminal",
            Some(0),
            &context,
            serde_json::json!({"status": "completed", "result": "SECRET"}),
        );
    }

    #[tokio::test]
    async fn empty_publish_slot_is_a_successful_noop() {
        let channel_id = Uuid::from_u128(200);
        let mut queue = ActivityPublishQueue::default();
        let publisher = crate::relay::RelayEventPublisher::disconnected_test_publisher();
        let keys = nostr::Keys::generate();

        assert!(
            publish_next_activity_frame(
                &mut queue,
                &publisher,
                &keys,
                &keys.public_key().to_hex(),
                &activity_test_resolver(channel_id),
            )
            .await,
            "no queued frame means there was no enqueue failure"
        );
    }

    #[tokio::test]
    async fn graceful_shutdown_drains_latest_terminal_update_before_exit() {
        let channel_id = Uuid::from_u128(201);
        let observer = crate::observer::ObserverHandle::in_process();
        let (publisher, mut published) = crate::relay::RelayEventPublisher::test_pair();
        let keys = nostr::Keys::generate();
        let task = spawn_relay_activity_publisher(
            observer.clone(),
            publisher,
            keys.clone(),
            keys.public_key().to_hex(),
            activity_test_resolver(channel_id),
        );
        emit_terminal_activity(&observer, channel_id);

        assert!(
            task.shutdown(Duration::from_secs(2)).await,
            "activity publisher must drain cleanly"
        );
        let event = published.recv().await.expect("terminal frame published");
        let frame = AgentActivityFrame::parse(&event.content).expect("valid safe frame");
        assert_eq!(frame.activities.len(), 1);
        assert_eq!(frame.activities[0].status, AgentActivityStatus::Completed);
        assert!(!event.content.contains("SECRET"));
        assert!(published.try_recv().is_err());
    }

    #[tokio::test]
    async fn graceful_shutdown_reports_closed_relay_command_channel() {
        let channel_id = Uuid::from_u128(202);
        let observer = crate::observer::ObserverHandle::in_process();
        let publisher = crate::relay::RelayEventPublisher::disconnected_test_publisher();
        let keys = nostr::Keys::generate();
        let task = spawn_relay_activity_publisher(
            observer.clone(),
            publisher,
            keys.clone(),
            keys.public_key().to_hex(),
            activity_test_resolver(channel_id),
        );
        emit_terminal_activity(&observer, channel_id);

        assert!(
            !task.shutdown(Duration::from_secs(2)).await,
            "a closed relay command channel must make the publisher drain fail"
        );
    }

    #[tokio::test]
    async fn graceful_shutdown_reports_observer_bus_lag() {
        let channel_id = Uuid::from_u128(203);
        let (tx, rx) = tokio::sync::broadcast::channel(1);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let keys = nostr::Keys::generate();
        let publisher = crate::relay::RelayEventPublisher::test_pair().0;
        for turn_id in ["turn-a", "turn-b"] {
            assert!(tx
                .send(observer_event(
                    "ignored",
                    channel_id,
                    turn_id,
                    serde_json::json!({}),
                ))
                .is_ok());
        }
        shutdown_tx.send(()).expect("publisher shutdown receiver");

        let drained = run_relay_activity_publisher(
            rx,
            shutdown_rx,
            publisher,
            keys.clone(),
            keys.public_key().to_hex(),
            activity_test_resolver(channel_id),
        )
        .await;

        assert!(
            !drained,
            "broadcast lag dropped observer activity and must make the drain visibly fail"
        );
    }

    #[test]
    fn only_stream_and_forum_channel_types_are_shareable() {
        assert!(is_shared_activity_channel_type(Some("stream")));
        assert!(is_shared_activity_channel_type(Some("forum")));
        for channel_type in [
            Some("dm"),
            Some("private"),
            Some("workflow"),
            Some("unknown"),
            None,
        ] {
            assert!(!is_shared_activity_channel_type(channel_type));
        }
    }

    #[tokio::test]
    async fn publication_failure_is_best_effort() {
        let (publisher, published) = crate::relay::RelayEventPublisher::test_pair();
        drop(published);
        let keys = nostr::Keys::generate();
        publish_activity_frame(
            &publisher,
            &keys,
            &keys.public_key().to_hex(),
            Uuid::new_v4(),
            AgentActivityFrame {
                version: AGENT_ACTIVITY_FRAME_VERSION,
                activities: vec![test_turn_activity(1)],
            },
        )
        .await;
    }

    fn test_turn_activity(index: u128) -> AgentActivity {
        AgentActivity {
            activity_id: Uuid::from_u128(index),
            occurred_at: "2026-08-12T10:00:00Z".parse().unwrap(),
            activity_class: AgentActivityClass::Turn,
            status: AgentActivityStatus::Running,
            tool_kind: None,
            duration_ms: None,
            usage: None,
        }
    }

    #[test]
    fn invalid_or_unknown_turn_inputs_emit_nothing() {
        let channel_id = Uuid::new_v4();
        let mut projector = ActivityProjector::default();

        let mut invalid_channel =
            observer_event("turn_started", channel_id, "turn-a", serde_json::json!({}));
        invalid_channel.channel_id = Some("not-a-uuid".into());
        assert!(projector.project(&invalid_channel).is_none());

        let mut invalid_timestamp =
            observer_event("turn_started", channel_id, "turn-b", serde_json::json!({}));
        invalid_timestamp.timestamp = "not-a-timestamp".into();
        assert!(projector.project(&invalid_timestamp).is_none());

        for raw_id in ["".to_string(), "x".repeat(MAX_RAW_ID_BYTES + 1)] {
            assert!(projector
                .project(&observer_event(
                    "turn_started",
                    channel_id,
                    &raw_id,
                    serde_json::json!({}),
                ))
                .is_none());
        }

        assert!(projector
            .project(&observer_event(
                "not_a_trusted_event",
                channel_id,
                "turn-c",
                serde_json::json!({"status": "completed"}),
            ))
            .is_none());
        assert!(projector
            .project(&observer_event(
                "agent_activity_turn_terminal",
                channel_id,
                "unknown-turn",
                serde_json::json!({"status": "completed"}),
            ))
            .is_none());
        assert!(projector
            .project(&observer_event(
                "turn_started",
                channel_id,
                "turn-negative-duration",
                serde_json::json!({}),
            ))
            .is_some());
        assert!(projector
            .project(&observer_event_at(
                "agent_activity_turn_terminal",
                channel_id,
                "turn-negative-duration",
                "2026-08-12T09:59:59Z",
                serde_json::json!({"status": "failed"}),
            ))
            .is_none());
    }
}
