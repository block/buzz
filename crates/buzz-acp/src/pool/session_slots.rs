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

use super::{apply_model_switch, AgentPool, OwnedAgent, SessionOwner};
use crate::acp::{resolve_model_switch_method, AcpClient, AcpError};
use crate::queue::EventQueue;
use crate::scope::SessionScope;

/// Upper bound on the maintenance tick interval.
pub(crate) const MAINTENANCE_MAX_INTERVAL: Duration = Duration::from_secs(30);
/// At most this many `session/close` requests per maintenance tick (all agents).
const MAX_CLOSES_PER_TICK: usize = 8;
/// Bound on each `session/close`; the tick runs on the main loop.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Total bound on draining queued closes at the start of a turn.
const TURN_CLOSE_BUDGET: Duration = Duration::from_secs(5);
/// Queued closes kept per worker; the oldest is dropped (and only leaks) beyond it.
const MAX_PENDING_CLOSES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
struct Slot {
    id: String,
    dormant: bool,
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
        if !slot.dormant {
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

    /// Mark a live session dormant after its `session/close` succeeded. Fenced
    /// on the id so a slot replaced meanwhile is never overwritten.
    #[must_use]
    fn mark_dormant(&mut self, scope: &SessionScope, id: &str) -> bool {
        match self.slots.get_mut(scope) {
            Some(slot) if !slot.dormant && slot.id == id => {
                slot.dormant = true;
                true
            }
            _ => false,
        }
    }

    fn dormant(&self, scope: &SessionScope) -> Option<(String, serde_json::Value)> {
        let slot = self.slots.get(scope).filter(|slot| slot.dormant)?;
        Some((slot.id.clone(), slot.resume_params.clone()?))
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

/// Send queued closes until `budget` is spent; `Err` only when the agent is
/// wedged or gone (stop talking to it this round).
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

/// Prepare `scope`'s session before a turn: drain queued closes (bounded),
/// then resume a dormant session. A failed resume invalidates the scope so the
/// caller falls through to `session/new` with full thread context. `Err` is
/// returned only for [`AcpError::AgentExited`].
pub(super) async fn prepare_turn_session(
    agent: &mut OwnedAgent,
    scope: Option<&SessionScope>,
) -> Result<(), AcpError> {
    let mut budget = MAX_PENDING_CLOSES;
    let drained = tokio::time::timeout(
        TURN_CLOSE_BUDGET,
        drain_pending_closes(&mut agent.acp, &mut agent.state.sessions, &mut budget),
    )
    .await;
    if let Ok(Err(AcpError::AgentExited)) = drained {
        return Err(AcpError::AgentExited);
    }
    let Some(scope) = scope else {
        return Ok(());
    };
    let Some((id, params)) = agent.state.sessions.dormant(scope) else {
        return Ok(());
    };
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
    // A resumed session runs the transcript's model; re-apply a live override.
    let desired = agent
        .desired_model
        .clone()
        .filter(|_| agent.model_overridden);
    if let Some(desired) = desired {
        if let Some(method) = resolve_model_switch_method(&resumed, &desired) {
            match apply_model_switch(&mut agent.acp, &id, &desired, &method).await {
                Ok(_) => {}
                Err(AcpError::AgentExited) => return Err(AcpError::AgentExited),
                Err(error) => {
                    tracing::warn!(
                        target: "pool::session",
                        "model re-apply on resumed session {id} failed: {error}; starting a new session"
                    );
                    reset_scope_keeping_owner(agent, scope);
                }
            }
        }
    }
    Ok(())
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
    /// Maintenance tick for idle workers: drain queued `session/close`s and,
    /// when `idle_bound` is non-zero, close thread sessions idle at least that
    /// long, marking them dormant. Runs on the main loop, so no dispatch can
    /// interleave; each close is bounded and the tick is capped in total.
    pub(crate) async fn maintain_idle_sessions(
        &mut self,
        queue: &EventQueue,
        idle_bound: Duration,
        now: Instant,
    ) {
        let mut budget = MAX_CLOSES_PER_TICK;
        for agent in self.agents.iter_mut().flatten() {
            if budget == 0 {
                break;
            }
            let state = &agent.state;
            let due: Vec<(SessionScope, String)> = state
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
                            agent_supports_evict: agent.acp.session_evict_supported(),
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
                .map(|(scope, slot)| (scope.clone(), slot.id.clone()))
                .collect();
            let sessions = &mut agent.state.sessions;
            if drain_pending_closes(&mut agent.acp, sessions, &mut budget)
                .await
                .is_err()
            {
                continue;
            }
            for (scope, id) in due {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                match agent.acp.session_close(&id, CLOSE_TIMEOUT).await {
                    Ok(()) => {
                        if sessions.mark_dormant(&scope, &id) {
                            tracing::info!(
                                target: "pool::session",
                                agent = agent.index,
                                scope = %scope.telemetry_label(),
                                "evicted idle session {id}"
                            );
                        }
                    }
                    Err(error) => {
                        // Stays live; retried next tick.
                        tracing::warn!(
                            target: "pool::session",
                            agent = agent.index,
                            "idle eviction close of {id} failed: {error}"
                        );
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::StopReason;
    use crate::config::DedupMode;
    use crate::pool::tests::make_prompt_context_no_owner;
    use crate::pool::{run_prompt_task, ChannelInfoResolver, PromptContext, PromptOutcome};
    use crate::queue::{BatchEvent, FlushBatch, QueuedEvent};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One dispatch-equivalent turn: claim, assign ownership, run, return.
    async fn turn(
        pool: &mut AgentPool,
        scope: &SessionScope,
        event: nostr::Event,
        ctx: Arc<PromptContext>,
    ) {
        let mut agent = pool.try_claim(Some(scope)).expect("idle worker");
        let generation = pool.record_scope_owner(scope.clone(), agent.index);
        agent
            .state
            .set_scope_owner_generation(scope.clone(), generation);
        let batch = FlushBatch {
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
        };
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
        assert!(matches!(
            result.outcome,
            PromptOutcome::Ok(StopReason::EndTurn)
        ));
        pool.return_agent(result.agent);
    }

    /// Evict idle threads (not one with queued work), resume one, fall back to
    /// `session/new` with full context when resume fails, and close a rotated
    /// session, all through `run_prompt_task` and the real maintenance tick.
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
        let roots = [
            post("thread a", None),
            post("thread b", None),
            post("thread c", None),
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

        // Fake agent: advertises close+resume, logs requests, fails resume of sess-2.
        let capture =
            std::env::temp_dir().join(format!("buzz-acp-evict-{}.ndjson", uuid::Uuid::new_v4()));
        let script = format!(
            r#"count=0; n=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> '{}'
  case "$line" in
    *'"method":"initialize"'*) r='{{"agentCapabilities":{{"sessionCapabilities":{{"close":{{}},"resume":{{}}}}}}}}' ;;
    *'"method":"session/new"'*) n=$((n + 1)); r="{{\"sessionId\":\"sess-$n\"}}" ;;
    *'"method":"session/resume"'*'"sess-2"'*) r=fail ;;
    *'"method":"session/prompt"'*) r='{{"stopReason":"end_turn"}}' ;;
    *) r='{{}}' ;;
  esac
  if [ "$r" = fail ]; then
    printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$count,\"error\":{{\"code\":-32002,\"message\":\"not found\"}}}}"
  else
    printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$count,\"result\":$r}}"
  fi
  count=$((count + 1))
done"#,
            capture.display()
        );
        let mut acp = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .unwrap();
        acp.initialize().await.unwrap();
        let mut pool = AgentPool::from_slots(vec![Some(OwnedAgent {
            index: 0,
            acp,
            state: Default::default(),
            model_capabilities: None,
            desired_model: None,
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

        for (i, root) in roots.iter().enumerate() {
            turn(&mut pool, &scope(i), root.clone(), ctx(0)).await; // sess-1..3
        }
        let mut queue = EventQueue::new(DedupMode::Drop);
        queue.push(QueuedEvent {
            channel_id,
            scope: scope(2),
            event: post("queued for c", Some(&roots[2])),
            received_at: std::time::Instant::now(),
            prompt_tag: "@mention".into(),
            edit: None,
        });
        let idle = Duration::from_secs(600);
        pool.maintain_idle_sessions(&queue, idle, Instant::now() + idle)
            .await;
        turn(
            &mut pool,
            &scope(0),
            post("follow-up a", Some(&roots[0])),
            ctx(0),
        )
        .await;
        turn(
            &mut pool,
            &scope(1),
            post("follow-up b", Some(&roots[1])),
            ctx(0),
        )
        .await;
        turn(
            &mut pool,
            &scope(2),
            post("follow-up c", Some(&roots[2])),
            ctx(1),
        )
        .await; // rotates
        pool.maintain_idle_sessions(
            &EventQueue::new(DedupMode::Drop),
            Duration::ZERO,
            Instant::now(),
        )
        .await;
        for agent in pool.agents.iter_mut().flatten() {
            agent.acp.shutdown().await;
        }
        server.abort();

        let requests: Vec<serde_json::Value> = std::fs::read_to_string(&capture)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        std::fs::remove_file(&capture).unwrap();
        let lifecycle: Vec<String> = requests
            .iter()
            .filter(|r| r["method"] != "session/prompt" && r["method"] != "initialize")
            .map(|r| {
                format!(
                    "{} {}",
                    r["method"].as_str().unwrap(),
                    r["params"]["sessionId"].as_str().unwrap_or("")
                )
            })
            .collect();
        let mut evicted = lifecycle[3..5].to_vec();
        evicted.sort();
        assert_eq!(
            evicted,
            ["session/close sess-1", "session/close sess-2"],
            "C has queued work: {lifecycle:?}"
        );
        assert_eq!(
            lifecycle[5..],
            [
                "session/resume sess-1",
                "session/resume sess-2",
                "session/new ",
                "session/close sess-2",
                "session/close sess-3"
            ],
            "{lifecycle:?}"
        );
        let news: Vec<_> = requests
            .iter()
            .filter(|r| r["method"] == "session/new")
            .collect();
        assert_eq!(
            news[3]["params"], news[1]["params"],
            "B's fallback session/new is built like its first session"
        );
        let new_a = &news[0]["params"];
        let resume_a = &requests
            .iter()
            .find(|r| r["method"] == "session/resume")
            .unwrap()["params"];
        assert_eq!(
            (&resume_a["cwd"], &resume_a["_meta"]),
            (&new_a["cwd"], &new_a["_meta"])
        );
        let prompts: Vec<(&str, String)> = requests
            .iter()
            .filter(|r| r["method"] == "session/prompt")
            .map(|r| {
                let text = r["params"]["prompt"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect();
                (r["params"]["sessionId"].as_str().unwrap(), text)
            })
            .collect();
        assert_eq!(
            prompts[3].0, "sess-1",
            "follow-up in A runs on the resumed session"
        );
        assert!(
            prompts[3]
                .1
                .contains("Earlier thread context is already available in this session"),
            "{}",
            prompts[3].1
        );
        assert_eq!(
            prompts[4].0, "sess-4",
            "failed resume falls back to session/new"
        );
        assert!(
            prompts[4].1.contains("Thread context included below"),
            "{}",
            prompts[4].1
        );
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
