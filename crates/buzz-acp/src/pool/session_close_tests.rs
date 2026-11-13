//! Discarded provider sessions must reach the agent as ACP `session/close`.
//!
//! Without it claude-agent-acp keeps one `claude` subprocess per session for
//! the life of the adapter, so a long-running harness accumulates one process
//! per thread/channel it ever served. These tests drive the production seams
//! (`SessionState` invalidation paths, `AgentPool::return_agent`, and
//! `AgentPool::flush_idle_pending_closes`) against a scripted ACP agent that
//! records every request it receives.

use super::*;
use crate::scope::SessionScope;

/// Scripted ACP agent: answers `initialize` (advertising `close` iff
/// `advertise_close`) and every other request with `{}`, appending each
/// received frame to `log`.
pub(crate) async fn recording_agent(log: &std::path::Path, advertise_close: bool) -> OwnedAgent {
    let caps = if advertise_close {
        r#"{"sessionCapabilities":{"close":{}}}"#
    } else {
        r#"{}"#
    };
    let script = format!(
        r#"while IFS= read -r line; do
  printf '%s\n' "$line" >> '{log}'
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  [ -n "$id" ] || continue
  case "$line" in
    *'"method":"initialize"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"agentCapabilities":{caps}}}}}\n' "$id" ;;
    *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id" ;;
  esac
done"#,
        log = log.display(),
    );
    let mut acp = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
        .await
        .expect("spawn recording ACP agent");
    acp.initialize().await.expect("initialize");
    OwnedAgent {
        index: 0,
        acp,
        state: SessionState::default(),
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: None,
        agent_name: "test".into(),
        goose_system_prompt_supported: None,
        protocol_version: 2,
    }
}

fn frames(log: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect()
}

/// Session ids the agent received `session/close` for, in order.
pub(crate) fn closed_ids(log: &std::path::Path) -> Vec<String> {
    frames(log)
        .into_iter()
        .filter(|m| m["method"] == "session/close")
        .filter_map(|m| m["params"]["sessionId"].as_str().map(str::to_owned))
        .collect()
}

fn conversation(channel_id: Uuid) -> SessionScope {
    SessionScope::Conversation { channel_id }
}

#[tokio::test]
async fn every_discard_path_closes_the_provider_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("wire.log");
    let removed = Uuid::new_v4();
    let rotated = Uuid::new_v4();
    let kept = Uuid::new_v4();
    let fenced = Uuid::new_v4();

    let mut agent = recording_agent(&log, true).await;
    for (ch, sid) in [
        (removed, "s-removed"),
        (rotated, "s-rotated"),
        (kept, "s-kept"),
        (fenced, "s-fenced"),
    ] {
        agent.state.sessions.insert(conversation(ch), sid.into());
    }
    agent.state.heartbeat_session = Some("s-heartbeat".into());

    // Rotation / per-source invalidation (run_prompt_task's `invalidate`).
    agent
        .state
        .invalidate(&PromptSource::Channel(conversation(rotated)));
    agent.state.invalidate(&PromptSource::Heartbeat);

    // Return with no recorded owner for `fenced` while `kept` stays owned:
    // the stale-ownership fence discards `fenced`.
    let mut pool = AgentPool::from_slots(vec![None]);
    for ch in [removed, kept] {
        let generation = pool.record_scope_owner(conversation(ch), 0);
        agent
            .state
            .set_scope_owner_generation(conversation(ch), generation);
    }
    pool.return_agent(agent);

    // Channel-wide invalidation (membership removal / archived sweep).
    assert_eq!(pool.invalidate_channel_sessions(removed), 1);

    pool.flush_idle_pending_closes().await;

    let mut closed = closed_ids(&log);
    closed.sort();
    assert_eq!(
        closed,
        ["s-fenced", "s-heartbeat", "s-removed", "s-rotated"],
        "each discarded session is closed exactly once; the live one is not"
    );
    let agent = pool.agents[0].as_ref().expect("idle agent");
    assert!(agent.state.pending_closes.is_empty());
    assert!(agent.state.sessions.contains_key(&conversation(kept)));
}

#[tokio::test]
async fn closes_are_skipped_without_capability_and_dropped_on_agent_loss() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Agent that does not advertise `sessionCapabilities.close`: never sent.
    let log = dir.path().join("no-cap.log");
    let mut agent = recording_agent(&log, false).await;
    let ch = Uuid::new_v4();
    agent.state.sessions.insert(conversation(ch), "s".into());
    assert!(agent.state.invalidate_scope(&conversation(ch)));
    agent.flush_pending_closes().await;
    assert!(closed_ids(&log).is_empty());
    assert!(agent.state.pending_closes.is_empty());

    // Agent process lost (exit / hard timeout): sessions died with it, so the
    // queue is discarded instead of being replayed at the respawned process.
    let log = dir.path().join("lost.log");
    let mut agent = recording_agent(&log, true).await;
    let ch = Uuid::new_v4();
    agent.state.sessions.insert(conversation(ch), "s-a".into());
    agent
        .state
        .sessions
        .insert(conversation(Uuid::new_v4()), "s-b".into());
    agent.state.invalidate_scope(&conversation(ch));
    agent.state.invalidate_all();
    agent.flush_pending_closes().await;
    assert!(closed_ids(&log).is_empty());
}

#[tokio::test]
async fn a_turn_closes_discarded_sessions_before_it_prompts() {
    // Rotation and the stale-ownership fence discard sessions while the worker
    // is checked out or about to be; the next turn on that worker must release
    // them before it puts a prompt on the shared stdio stream.
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("wire.log");
    let mut agent = recording_agent(&log, true).await;
    agent.state.pending_closes.push("s-rotated".into());
    agent.state.heartbeat_session = Some("s-live".into());

    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    run_prompt_task(
        agent,
        None,
        Some("heartbeat".into()),
        Arc::new(super::tests::make_prompt_context_no_owner()),
        result_tx,
        None,
        "turn".into(),
        Default::default(),
    )
    .await;
    result_rx.recv().await.expect("prompt result");

    let methods: Vec<String> = frames(&log)
        .iter()
        .filter_map(|m| m["method"].as_str().map(str::to_owned))
        .collect();
    let close = methods.iter().position(|m| m == "session/close");
    let prompt = methods.iter().position(|m| m == "session/prompt");
    assert!(
        matches!((close, prompt), (Some(c), Some(p)) if c < p),
        "close must precede the turn's prompt: {methods:?}"
    );
    assert_eq!(closed_ids(&log), ["s-rotated"]);
}
