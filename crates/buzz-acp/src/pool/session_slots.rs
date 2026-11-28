//! Per-scope ACP session slots, idle-session eviction, and resume.
//!
//! Every scope's provider session lives in one [`SessionSlots`] entry that is
//! either `Live` (loaded in the agent) or `Dormant` (closed with
//! `session/close` after `--session-idle-evict` seconds idle; its id, delivery
//! ledger, cached prompt sections, and owner generation are kept). Removing an
//! entry, on any invalidation path, drops the dormant state with it, and every
//! discarded `Live` id is queued for a best-effort `session/close` so rotated or
//! invalidated sessions do not keep their agent-side subprocesses alive. The
//! queue is drained with bounded timeouts at the start of the worker's next
//! turn and on the maintenance tick.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use tokio::time::Instant;

use super::{configure_session, AgentPool, OwnedAgent, PromptContext, SessionOwner};
use crate::acp::{AcpClient, AcpError};
use crate::queue::EventQueue;
use crate::scope::SessionScope;

/// Upper bound on the maintenance tick interval.
pub(crate) const MAINTENANCE_MAX_INTERVAL: Duration = Duration::from_secs(30);
/// At most this many `session/close` requests per maintenance tick, across
/// all workers. The tick runs inline on the main loop, so this and
/// [`CLOSE_TIMEOUT`] bound how long one tick can delay dispatch.
const MAX_CLOSES_PER_TICK: usize = 2;
/// Bound on waiting for one `session/close` response. The agent still finishes
/// a close we stopped waiting for; a late response is skipped by request id.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
/// Bound on re-closing an evicted session whose first close went unanswered,
/// before resuming it (on the turn path, so it may wait for teardown).
const CONFIRM_CLOSE_TIMEOUT: Duration = Duration::from_secs(30);
/// Queued closes sent at the start of a turn, before the user's prompt.
const MAX_CLOSES_PER_TURN: usize = 1;
/// Queued closes kept per worker; the oldest is dropped (and only leaks) beyond it.
const MAX_PENDING_CLOSES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
struct Slot {
    id: String,
    dormant: bool,
    /// For a dormant slot: the agent answered its `session/close`. An
    /// unconfirmed close may still be tearing the session down, so it is
    /// closed again (and waited for) before resume or on discard.
    close_confirmed: bool,
    last_active: Instant,
    /// Exact `session/new` params, resent by `session/resume`. A session
    /// without them is never evicted.
    resume_params: Option<serde_json::Value>,
}

/// Session id per scope, live or dormant, plus queued closes for discarded ids.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionSlots {
    slots: HashMap<SessionScope, Slot>,
    pending_close: VecDeque<String>,
}

impl SessionSlots {
    /// The **live** session id for `scope`; `None` when absent or dormant.
    pub fn get(&self, scope: &SessionScope) -> Option<&String> {
        self.slots
            .get(scope)
            .filter(|slot| !slot.dormant)
            .map(|slot| &slot.id)
    }

    /// Whether `scope` has a session on this worker, live or dormant. A dormant
    /// session still makes this worker the scope's owner, so the next event
    /// routes here and resumes it.
    pub fn contains_key(&self, scope: &SessionScope) -> bool {
        self.slots.contains_key(scope)
    }

    /// Scopes with a live or dormant session.
    pub fn keys(&self) -> impl Iterator<Item = &SessionScope> {
        self.slots.keys()
    }

    /// Number of scopes with a live or dormant session.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether no scope has a session.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Record a newly created live session for `scope`. A different live
    /// session it replaces is queued for close.
    pub fn insert(&mut self, scope: SessionScope, id: String) {
        let slot = Slot {
            id,
            dormant: false,
            close_confirmed: false,
            last_active: Instant::now(),
            resume_params: None,
        };
        let new_id = slot.id.clone();
        if let Some(old) = self.slots.insert(scope, slot) {
            if old.id != new_id {
                self.discard(old);
            }
        }
    }

    /// Remove `scope`'s session (any invalidation path). A live session is
    /// queued for close; a dormant one is already closed. Returns its id.
    pub fn remove(&mut self, scope: &SessionScope) -> Option<String> {
        let slot = self.slots.remove(scope)?;
        let id = slot.id.clone();
        self.discard(slot);
        Some(id)
    }

    /// Forget every session without closing: the agent process is gone.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.pending_close.clear();
    }

    fn discard(&mut self, slot: Slot) {
        if !slot.dormant || !slot.close_confirmed {
            self.schedule_close(slot.id);
        }
    }

    /// Queue a best-effort `session/close` for a session no slot refers to.
    pub(crate) fn schedule_close(&mut self, id: String) {
        if self.pending_close.len() >= MAX_PENDING_CLOSES {
            if let Some(dropped) = self.pending_close.pop_front() {
                tracing::warn!(target: "pool::session", "close queue full; not closing {dropped}");
            }
        }
        self.pending_close.push_back(id);
    }

    /// Remember the `session/new` params a resume must resend.
    pub(crate) fn set_resume_params(&mut self, scope: &SessionScope, params: serde_json::Value) {
        if let Some(slot) = self.slots.get_mut(scope) {
            slot.resume_params = Some(params);
        }
    }

    /// Stamp `scope`'s last activity (each turn completion).
    pub(crate) fn touch(&mut self, scope: &SessionScope, now: Instant) {
        if let Some(slot) = self.slots.get_mut(scope) {
            slot.last_active = now;
        }
    }

    /// Mark a live session dormant after its `session/close` was sent;
    /// `confirmed` when the agent answered it. Fenced on the id so a slot
    /// replaced meanwhile is never overwritten.
    #[must_use]
    fn mark_dormant(&mut self, scope: &SessionScope, id: &str, confirmed: bool) -> bool {
        match self.slots.get_mut(scope) {
            Some(slot) if !slot.dormant && slot.id == id => {
                slot.dormant = true;
                slot.close_confirmed = confirmed;
                true
            }
            _ => false,
        }
    }

    /// A dormant slot's id, resume params, and whether its close was confirmed.
    fn dormant(&self, scope: &SessionScope) -> Option<(String, serde_json::Value, bool)> {
        let slot = self.slots.get(scope).filter(|slot| slot.dormant)?;
        Some((
            slot.id.clone(),
            slot.resume_params.clone()?,
            slot.close_confirmed,
        ))
    }

    #[must_use]
    fn revive(&mut self, scope: &SessionScope, id: &str) -> bool {
        match self.slots.get_mut(scope) {
            Some(slot) if slot.dormant && slot.id == id => {
                slot.dormant = false;
                slot.last_active = Instant::now();
                true
            }
            _ => false,
        }
    }
}

/// Inputs to [`eviction_due`] for one live scope on one idle worker.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EvictionCheck {
    /// `--session-idle-evict`; zero disables eviction.
    pub idle_bound: Duration,
    /// The agent advertised both `session/close` and `session/resume`.
    pub agent_supports_evict: bool,
    pub is_thread: bool,
    /// The session was created here, so its `session/new` params are known.
    pub resumable: bool,
    pub idle_for: Duration,
    /// The pool still names this worker and generation as the scope's owner.
    pub owner_current: bool,
    /// Queued or undispatched (including retry-throttled) work for the scope.
    pub work_pending: bool,
    /// A turn (and so any steer into it) is in flight for the scope.
    pub in_flight: bool,
    /// The scope is held waiting for a busy owner.
    pub held: bool,
}

/// Whether an idle thread session should be closed and marked dormant.
#[must_use]
pub(crate) fn eviction_due(check: &EvictionCheck) -> bool {
    !check.idle_bound.is_zero()
        && check.agent_supports_evict
        && check.is_thread
        && check.resumable
        && check.owner_current
        && !check.work_pending
        && !check.in_flight
        && !check.held
        && check.idle_for >= check.idle_bound
}

/// Send up to `budget` queued closes; `Err` when the agent timed out or is
/// gone (stop talking to it this round). A timed-out close is not retried:
/// the agent still processes the request; only our wait was bounded.
async fn drain_pending_closes(
    acp: &mut AcpClient,
    slots: &mut SessionSlots,
    budget: &mut usize,
) -> Result<(), AcpError> {
    if !acp.session_close_supported() {
        slots.pending_close.clear();
        return Ok(());
    }
    while *budget > 0 {
        let Some(id) = slots.pending_close.pop_front() else {
            break;
        };
        *budget -= 1;
        match acp.session_close(&id, CLOSE_TIMEOUT).await {
            Ok(()) => tracing::info!(target: "pool::session", "closed discarded session {id}"),
            Err(error @ (AcpError::AgentError { .. } | AcpError::Protocol(_))) => {
                tracing::warn!(target: "pool::session", "close of discarded session {id} failed: {error}");
            }
            Err(error) => {
                tracing::warn!(target: "pool::session", "close of discarded session {id} failed: {error}");
                return Err(error);
            }
        }
    }
    Ok(())
}

impl OwnedAgent {
    /// Whether this worker's sessions may be evicted and resumed: the agent
    /// advertised `session/close` and `session/resume`. Goose is excluded
    /// because its system prompt is installed by a post-create extension call
    /// that resume does not replay.
    pub(crate) fn can_evict_sessions(&self) -> bool {
        self.acp.session_evict_supported() && self.agent_name != "goose"
    }
}

/// Prepare `scope`'s session before a turn: send at most one queued close
/// (the tick drains the rest), then resume a dormant session and re-apply its
/// per-session configuration. Any resume or configuration failure invalidates
/// the scope so the caller falls through to `session/new` with full thread
/// context. `Err` is returned only for [`AcpError::AgentExited`].
pub(super) async fn prepare_turn_session(
    agent: &mut OwnedAgent,
    ctx: &PromptContext,
    scope: Option<&SessionScope>,
) -> Result<(), AcpError> {
    let mut budget = MAX_CLOSES_PER_TURN;
    if let Err(AcpError::AgentExited) =
        drain_pending_closes(&mut agent.acp, &mut agent.state.sessions, &mut budget).await
    {
        return Err(AcpError::AgentExited);
    }
    let Some(scope) = scope else {
        return Ok(());
    };
    let Some((id, params, close_confirmed)) = agent.state.sessions.dormant(scope) else {
        return Ok(());
    };
    if !close_confirmed {
        // The eviction stopped waiting for this close, so the agent may still be
        // tearing the session down; a resume now could return the dying session.
        // Close again and wait: claude-agent-acp's close runs the teardown (or
        // finds it done) before answering, and is idempotent.
        match agent.acp.session_close(&id, CONFIRM_CLOSE_TIMEOUT).await {
            Ok(()) => {}
            Err(AcpError::AgentExited) => return Err(AcpError::AgentExited),
            Err(error) => {
                tracing::warn!(
                    target: "pool::session",
                    scope = %scope.telemetry_label(),
                    "re-close of evicted session {id} failed: {error}; starting a new session"
                );
                reset_scope_keeping_owner(agent, scope);
                agent.state.sessions.schedule_close(id);
                return Ok(());
            }
        }
    }
    let resumed = match agent.acp.session_resume(&id, &params).await {
        Ok(raw) => raw,
        Err(AcpError::AgentExited) => return Err(AcpError::AgentExited),
        Err(error) => {
            tracing::warn!(
                target: "pool::session",
                scope = %scope.telemetry_label(),
                "resume of dormant session {id} failed: {error}; starting a new session"
            );
            reset_scope_keeping_owner(agent, scope);
            // The agent may have loaded it despite the error; close is idempotent.
            agent.state.sessions.schedule_close(id);
            return Ok(());
        }
    };
    if !agent.state.sessions.revive(scope, &id) {
        return Ok(());
    }
    tracing::info!(
        target: "pool::session",
        scope = %scope.telemetry_label(),
        "resumed dormant session {id}"
    );
    agent.acp.notify_session_resumed(&id);
    // The adapter rebuilds a resumed session without the harness's model,
    // effort, or permission mode, so apply them exactly as for a new session.
    match configure_session(agent, ctx, &resumed, &id).await {
        Ok(()) => Ok(()),
        Err(AcpError::AgentExited) => Err(AcpError::AgentExited),
        Err(error) => {
            tracing::warn!(
                target: "pool::session",
                "configuring resumed session {id} failed: {error}; starting a new session"
            );
            reset_scope_keeping_owner(agent, scope);
            Ok(())
        }
    }
}

/// Invalidate `scope` mid-turn so this turn creates a fresh session, keeping
/// the dispatch-assigned owner generation: without it the pool would treat the
/// replacement session as stale when the worker returns.
fn reset_scope_keeping_owner(agent: &mut OwnedAgent, scope: &SessionScope) {
    let generation = agent.state.scope_owner_generations.get(scope).copied();
    agent.state.invalidate_scope(scope);
    if let Some(generation) = generation {
        agent
            .state
            .set_scope_owner_generation(scope.clone(), generation);
    }
}

impl AgentPool {
    /// Maintenance tick for idle workers: send one queued `session/close` and,
    /// when `idle_bound` is non-zero, close thread sessions idle at least that
    /// long and mark them dormant. It runs on the main loop, so no dispatch can
    /// interleave, and is bounded to [`MAX_CLOSES_PER_TICK`] closes of
    /// [`CLOSE_TIMEOUT`] each; a tick with nothing due does no I/O.
    pub(crate) async fn maintain_idle_sessions(
        &mut self,
        queue: &EventQueue,
        idle_bound: Duration,
        now: Instant,
    ) {
        let mut drain_budget = 1;
        let mut budget = MAX_CLOSES_PER_TICK;
        for agent in self.agents.iter_mut().flatten() {
            if budget == 0 {
                break;
            }
            let state = &agent.state;
            let supports_evict = agent.can_evict_sessions();
            let mut due: Vec<(Instant, SessionScope, String)> = state
                .sessions
                .slots
                .iter()
                .filter(|(scope, slot)| {
                    let owner = state
                        .scope_owner_generations
                        .get(*scope)
                        .map(|&generation| SessionOwner {
                            agent_index: agent.index,
                            generation,
                        });
                    !slot.dormant
                        && eviction_due(&EvictionCheck {
                            idle_bound,
                            agent_supports_evict: supports_evict,
                            is_thread: scope.is_thread(),
                            resumable: slot.resume_params.is_some(),
                            idle_for: now.saturating_duration_since(slot.last_active),
                            owner_current: owner.is_some()
                                && self.session_owners.get(*scope) == owner.as_ref(),
                            work_pending: queue.has_pending_scope(scope),
                            in_flight: queue.is_scope_in_flight(*scope),
                            held: self.held_since.contains_key(*scope),
                        })
                })
                .map(|(scope, slot)| (slot.last_active, scope.clone(), slot.id.clone()))
                .collect();
            // Longest-idle first, so a capped tick is deterministic.
            due.sort_by_key(|(last_active, _, _)| *last_active);
            let sessions = &mut agent.state.sessions;
            let before = drain_budget;
            let drained = drain_pending_closes(&mut agent.acp, sessions, &mut drain_budget).await;
            budget -= before - drain_budget;
            if drained.is_err() {
                continue;
            }
            for (_, scope, id) in due {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                let closed = agent.acp.session_close(&id, CLOSE_TIMEOUT).await;
                if let Err(AcpError::AgentExited) = closed {
                    break;
                }
                // Any other failure (an error, or a timeout while the agent
                // finishes teardown) still marks the session dormant: keeping a
                // possibly closed session live would fail every later prompt in
                // the thread, while resuming a still-loaded one returns it as is.
                if sessions.mark_dormant(&scope, &id, closed.is_ok()) {
                    match &closed {
                        Ok(()) => tracing::info!(
                            target: "pool::session",
                            agent = agent.index,
                            scope = %scope.telemetry_label(),
                            "evicted idle session {id}"
                        ),
                        Err(error) => tracing::warn!(
                            target: "pool::session",
                            agent = agent.index,
                            scope = %scope.telemetry_label(),
                            "idle eviction close of {id} failed ({error}); marked dormant, resume reloads it"
                        ),
                    }
                }
                // A slow or broken transport: leave this worker until next tick.
                if let Err(AcpError::Timeout(_) | AcpError::WriteTimeout(_) | AcpError::Io(_)) =
                    closed
                {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::StopReason;
    use crate::config::{DedupMode, PermissionMode};
    use crate::pool::tests::make_prompt_context_no_owner;
    use crate::pool::{run_prompt_task, ChannelInfoResolver, PromptContext, PromptOutcome};
    use crate::queue::{BatchEvent, FlushBatch, QueuedEvent};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One dispatch-equivalent turn: claim, assign ownership, run, return.
    /// Returns the outcome and the batch handed back for retry, if any.
    async fn dispatch(
        pool: &mut AgentPool,
        batch: FlushBatch,
        ctx: Arc<PromptContext>,
    ) -> (PromptOutcome, Option<FlushBatch>) {
        let scope = batch.scope.clone();
        let mut agent = pool.try_claim(Some(&scope)).expect("idle worker");
        let generation = pool.record_scope_owner(scope.clone(), agent.index);
        agent.state.set_scope_owner_generation(scope, generation);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        run_prompt_task(
            agent,
            Some(batch),
            None,
            ctx,
            tx,
            None,
            "t".into(),
            Default::default(),
        )
        .await;
        let result = rx.recv().await.expect("prompt result");
        pool.return_agent(result.agent);
        (result.outcome, result.batch)
    }

    fn batch_of(scope: &SessionScope, event: nostr::Event) -> FlushBatch {
        FlushBatch {
            channel_id: scope.channel_id(),
            scope: scope.clone(),
            events: vec![BatchEvent {
                edit: None,
                event,
                prompt_tag: "@mention".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        }
    }

    /// A turn that must complete normally.
    async fn turn(
        pool: &mut AgentPool,
        scope: &SessionScope,
        event: nostr::Event,
        ctx: Arc<PromptContext>,
    ) {
        let (outcome, _) = dispatch(pool, batch_of(scope, event), ctx).await;
        assert!(matches!(outcome, PromptOutcome::Ok(StopReason::EndTurn)));
    }

    /// Requests the fake agent logged since `seen`.
    fn requests_since(capture: &std::path::Path, seen: &mut usize) -> Vec<serde_json::Value> {
        let all: Vec<serde_json::Value> = std::fs::read_to_string(capture)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let new = all[*seen..].to_vec();
        *seen = all.len();
        new
    }

    /// Session lifecycle and configuration requests, as `method session [k=v]`.
    fn lifecycle(requests: &[serde_json::Value]) -> Vec<String> {
        requests
            .iter()
            .filter(|r| r["method"] != "session/prompt" && r["method"] != "initialize")
            .map(|r| {
                let p = &r["params"];
                let mut line = format!(
                    "{} {}",
                    r["method"].as_str().unwrap(),
                    p["sessionId"].as_str().unwrap_or("")
                );
                if let (Some(k), Some(v)) = (p["configId"].as_str(), p["value"].as_str()) {
                    line.push_str(&format!(" {k}={v}"));
                }
                line
            })
            .collect()
    }

    /// The one prompt in `requests`: its session id and text.
    fn prompt(requests: &[serde_json::Value]) -> (String, String) {
        let r = requests
            .iter()
            .find(|r| r["method"] == "session/prompt")
            .unwrap();
        let text = r["params"]["prompt"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect();
        (r["params"]["sessionId"].as_str().unwrap().to_owned(), text)
    }

    /// Fake claude-agent-acp: advertises close+resume and a model/mode
    /// catalog. The first close of sess-2 is never answered (the harness's
    /// wait times out), resume of sess-3 fails, and the second prompt on sess-1
    /// gets the SDK's "Session not found" internal error.
    const FAKE_AGENT: &str = r#"count=0; n=0; closes2=0; prompts1=0
cfg='"configOptions":[{"id":"model","category":"model","options":[{"value":"m-test"}]}],"modes":{"availableModes":[{"id":"plan"}]}'
while IFS= read -r line; do
  printf '%s\n' "$line" >> 'CAPTURE'
  case "$line" in
    *'"method":"initialize"'*) r='{"agentCapabilities":{"sessionCapabilities":{"close":{},"resume":{}}}}' ;;
    *'"method":"session/new"'*) n=$((n + 1)); r="{\"sessionId\":\"sess-$n\",$cfg}" ;;
    *'"method":"session/resume"'*'"sess-3"'*) r=fail ;;
    *'"method":"session/resume"'*) r="{$cfg}" ;;
    *'"method":"session/close"'*'"sess-2"'*) closes2=$((closes2 + 1)); if [ $closes2 -eq 1 ]; then r=silent; else r='{}'; fi ;;
    *'"method":"session/prompt"'*'"sess-1"'*) prompts1=$((prompts1 + 1)); if [ $prompts1 -eq 2 ]; then r=gone; else r='{"stopReason":"end_turn"}'; fi ;;
    *'"method":"session/prompt"'*) r='{"stopReason":"end_turn"}' ;;
    *) r='{}' ;;
  esac
  if [ "$r" = silent ]; then
    :
  elif [ "$r" = fail ]; then
    printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$count,\"error\":{\"code\":-32002,\"message\":\"not found\"}}"
  elif [ "$r" = gone ]; then
    printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$count,\"error\":{\"code\":-32603,\"message\":\"Internal error\",\"data\":{\"details\":\"Session not found\"}}}"
  else
    printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$count,\"result\":$r}"
  fi
  count=$((count + 1))
done"#;

    /// Through `run_prompt_task` and the real maintenance tick: idle threads are
    /// evicted longest-idle first, except one with queued work or a busy-owner
    /// hold; a dormant session is never closed twice; an unanswered close is
    /// repeated before resume; a resumed session gets the startup model and
    /// permission mode again; a failed resume, or a prompt on a session the
    /// agent lost, falls back to `session/new` with full thread context and the
    /// batch is delivered; discarded sessions are closed.
    #[tokio::test]
    async fn idle_thread_sessions_evict_resume_and_close_on_discard() {
        let channel_id = uuid::Uuid::new_v4();
        let human = Keys::generate();
        let post = |content: &str, root: Option<&nostr::Event>| {
            let mut tags = vec![Tag::parse(["h", channel_id.to_string().as_str()]).unwrap()];
            if let Some(root) = root {
                tags.push(Tag::parse(["e", root.id.to_hex().as_str(), "", "reply"]).unwrap());
            }
            EventBuilder::new(Kind::Custom(9), content)
                .tags(tags)
                .sign_with_keys(&human)
                .unwrap()
        };
        // Thread 0 is created first (longest idle) but stays busy.
        let roots = [
            post("busy", None),
            post("thread a", None),
            post("thread b", None),
        ];
        let scope = |i: usize| SessionScope::Thread {
            channel_id,
            root_event_id: roots[i].id.to_hex(),
        };

        // Relay stand-in: a thread's context is its root event.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let served = roots.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0; 16 * 1024];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let hits: Vec<_> = served
                    .iter()
                    .filter(|e| request.contains(&e.id.to_hex()))
                    .collect();
                let body = serde_json::to_string(&hits).unwrap();
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(format!("{head}{body}").as_bytes()).await;
            }
        });

        let capture =
            std::env::temp_dir().join(format!("buzz-acp-evict-{}.ndjson", uuid::Uuid::new_v4()));
        let script = FAKE_AGENT.replace("CAPTURE", &capture.display().to_string());
        let mut acp = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .unwrap();
        acp.initialize().await.unwrap();
        let mut pool = AgentPool::from_slots(vec![Some(OwnedAgent {
            index: 0,
            acp,
            state: Default::default(),
            model_capabilities: None,
            desired_model: Some("m-test".into()), // startup --model, not overridden
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "legacy-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        })]);
        let ctx = |max_turns_per_session| {
            let mut ctx = make_prompt_context_no_owner();
            ctx.context_message_limit = 10;
            ctx.max_turns_per_session = max_turns_per_session;
            ctx.permission_mode = PermissionMode::Plan;
            ctx.session_idle_evict = true;
            ctx.session_title = Some("agent".into());
            ctx.rest_client.base_url = base_url.clone();
            let info = crate::relay::ChannelInfo {
                name: "ch".into(),
                channel_type: "stream".into(),
                description: None,
            };
            ctx.channel_info = ChannelInfoResolver::new(
                HashMap::from([(channel_id, info)]),
                ctx.rest_client.clone(),
            );
            Arc::new(ctx)
        };
        let mut seen = 1; // initialize
        let configured = |id: &str| {
            [
                format!("session/set_config_option {id} model=m-test"),
                format!("session/set_config_option {id} mode=plan"),
            ]
        };

        let mut created = Vec::new();
        for (i, root) in roots.iter().enumerate() {
            turn(&mut pool, &scope(i), root.clone(), ctx(0)).await;
            created.push(requests_since(&capture, &mut seen));
        }
        let [model, mode] = configured("sess-1");
        assert_eq!(lifecycle(&created[0]), ["session/new ", &model, &mode]);

        // Tick 1: thread 0 has queued work; A's close goes unanswered, which
        // backs the tick off this worker.
        let idle = Duration::from_secs(600);
        let mut busy = EventQueue::new(DedupMode::Drop);
        busy.push(QueuedEvent {
            channel_id,
            scope: scope(0),
            event: post("queued", Some(&roots[0])),
            received_at: std::time::Instant::now(),
            prompt_tag: "@mention".into(),
            edit: None,
        });
        pool.maintain_idle_sessions(&busy, idle, Instant::now() + idle)
            .await;
        let tick = lifecycle(&requests_since(&capture, &mut seen));
        assert_eq!(tick, ["session/close sess-2"]);

        // Tick 2: thread 0 is held and A is dormant, so only B is closed.
        let quiet = EventQueue::new(DedupMode::Drop);
        pool.held_since.insert(scope(0), Instant::now());
        pool.maintain_idle_sessions(&quiet, idle, Instant::now() + idle)
            .await;
        pool.held_since.clear();
        let tick = lifecycle(&requests_since(&capture, &mut seen));
        assert_eq!(tick, ["session/close sess-3"]);

        // A: its close was unconfirmed, so it is closed again before resuming
        // with identical params and being reconfigured; the prompt relies on
        // the context the session already has.
        turn(
            &mut pool,
            &scope(1),
            post("more a", Some(&roots[1])),
            ctx(0),
        )
        .await;
        let step = requests_since(&capture, &mut seen);
        let [model, mode] = configured("sess-2");
        assert_eq!(
            lifecycle(&step),
            [
                "session/close sess-2",
                "session/resume sess-2",
                &model,
                &mode
            ]
        );
        let resume = &step[1]["params"];
        let new_a = &created[1][0]["params"];
        assert_eq!(
            (&resume["cwd"], &resume["_meta"]),
            (&new_a["cwd"], &new_a["_meta"])
        );
        let (session, text) = prompt(&step);
        assert_eq!(session, "sess-2");
        assert!(text.contains("Earlier thread context is already available in this session"));

        // B: resume fails, so a session built like its first one takes the batch.
        turn(
            &mut pool,
            &scope(2),
            post("more b", Some(&roots[2])),
            ctx(0),
        )
        .await;
        let step = requests_since(&capture, &mut seen);
        let [model, mode] = configured("sess-4");
        assert_eq!(
            lifecycle(&step),
            ["session/resume sess-3", "session/new ", &model, &mode]
        );
        assert_eq!(step[1]["params"], created[2][0]["params"]);
        let (session, text) = prompt(&step);
        assert_eq!(session, "sess-4");
        assert!(text.contains("Thread context included below"), "{text}");

        // Thread 0: the agent lost sess-1. The batch comes back for retry (even
        // under DedupMode::Drop) and lands on a new session with full context.
        let (outcome, retry) = dispatch(
            &mut pool,
            batch_of(&scope(0), post("more 0", Some(&roots[0]))),
            ctx(0),
        )
        .await;
        assert!(matches!(outcome, PromptOutcome::Error(ref e) if e.is_session_gone()));
        let step = lifecycle(&requests_since(&capture, &mut seen));
        assert_eq!(step, ["session/close sess-3"], "queued close of B's old id");
        let (outcome, _) = dispatch(&mut pool, retry.expect("batch retried"), ctx(1)).await;
        assert!(matches!(outcome, PromptOutcome::Ok(StopReason::EndTurn)));
        let step = requests_since(&capture, &mut seen);
        let [model, mode] = configured("sess-5");
        assert_eq!(
            lifecycle(&step),
            ["session/close sess-1", "session/new ", &model, &mode]
        );
        let (session, text) = prompt(&step);
        assert_eq!(session, "sess-5");
        assert!(text.contains("Thread context included below"), "{text}");

        // That turn rotated sess-5 (max one turn); the tick closes it.
        pool.maintain_idle_sessions(&quiet, Duration::ZERO, Instant::now())
            .await;
        let tick = lifecycle(&requests_since(&capture, &mut seen));
        assert_eq!(tick, ["session/close sess-5"]);

        for agent in pool.agents.iter_mut().flatten() {
            agent.acp.shutdown().await;
        }
        server.abort();
        std::fs::remove_file(&capture).unwrap();
    }

    #[test]
    fn eviction_due_requires_every_gate() {
        let due = EvictionCheck {
            idle_bound: Duration::from_secs(600),
            agent_supports_evict: true,
            is_thread: true,
            resumable: true,
            idle_for: Duration::from_secs(600),
            owner_current: true,
            work_pending: false,
            in_flight: false,
            held: false,
        };
        assert!(eviction_due(&due));
        type Flip = fn(&mut EvictionCheck);
        let cases: [(&str, Flip); 9] = [
            ("disabled", |c| c.idle_bound = Duration::ZERO),
            ("capabilities missing", |c| c.agent_supports_evict = false),
            ("dm/conversation scope", |c| c.is_thread = false),
            ("no session/new params", |c| c.resumable = false),
            ("not idle long enough", |c| {
                c.idle_for = Duration::from_secs(599)
            }),
            ("ownership moved", |c| c.owner_current = false),
            ("queued work", |c| c.work_pending = true),
            ("turn in flight", |c| c.in_flight = true),
            ("held for busy owner", |c| c.held = true),
        ];
        for (name, flip) in cases {
            let mut check = due;
            flip(&mut check);
            assert!(!eviction_due(&check), "{name} must block eviction");
        }
    }
}
