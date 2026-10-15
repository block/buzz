//! One fresh process/session for a local task, with bounded cleanup.
use crate::{acp, pool, AcpClient, OwnedAgent, PoolStartup, PromptContext, SessionState};
use std::time::Duration;
use tokio::sync::oneshot;

pub(crate) enum Outcome {
    Completed(String),
    Cancelled,
    Deadline,
    Failed(&'static str),
}

pub(crate) async fn execute(
    startup: &PoolStartup,
    ctx: &PromptContext,
    prompt: &str,
    duration: Duration,
    mut cancel: oneshot::Receiver<()>,
) -> Outcome {
    if !matches!(cancel.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
        return Outcome::Cancelled;
    }
    let deadline = tokio::time::Instant::now() + duration;
    let mut acp = match AcpClient::spawn(
        &startup.command,
        &startup.args,
        &startup.extra_env,
        startup.has_generated_codex_config,
    )
    .await
    {
        Ok(acp) => acp,
        Err(_) => return Outcome::Failed("agent_spawn_failed"),
    };
    let initialized = tokio::select! {
        biased;
        _ = &mut cancel => Err(Outcome::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Outcome::Deadline),
        result = acp.initialize() => result.map_err(|_| Outcome::Failed("agent_initialize_failed")),
    };
    let init = match initialized {
        Ok(init) => init,
        Err(outcome) => {
            acp.shutdown().await;
            return outcome;
        }
    };
    let mut agent = OwnedAgent {
        index: 0,
        acp,
        state: SessionState::default(),
        model_capabilities: None,
        desired_model: startup.model.clone(),
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: startup.effort_level.clone(),
        agent_name: crate::normalized_agent_name(&init),
        goose_system_prompt_supported: None,
        protocol_version: init["protocolVersion"].as_u64().unwrap_or(1) as u32,
    };
    let mut session = None;
    let outcome = tokio::select! {
        biased;
        _ = &mut cancel => Outcome::Cancelled,
        _ = tokio::time::sleep_until(deadline) => Outcome::Deadline,
        result = pool::run_isolated_prompt(&mut agent, ctx, prompt, duration, &mut session) => {
            match result {
                Ok(acp::StopReason::Cancelled) => Outcome::Cancelled,
                Ok(acp::StopReason::EndTurn) => Outcome::Completed("end_turn".into()),
                Ok(acp::StopReason::MaxTokens) => Outcome::Completed("max_tokens".into()),
                Ok(acp::StopReason::MaxTurnRequests) => Outcome::Completed("max_turn_requests".into()),
                Ok(acp::StopReason::Refusal) => Outcome::Completed("refusal".into()),
                Err(acp::AcpError::HardTimeout { .. }) => Outcome::Deadline,
                Err(acp::AcpError::IdleTimeout(_)) => Outcome::Failed("agent_idle_timeout"),
                Err(_) => Outcome::Failed("agent_execution_failed"),
            }
        }
    };
    if matches!(outcome, Outcome::Cancelled | Outcome::Deadline) {
        if let Some(session) = session {
            // Cancellation is best effort; the terminal status remains the cause
            // that won above, even if the adapter returns end_turn during drain.
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                agent
                    .acp
                    .cancel_with_cleanup_grace(&session, Duration::from_secs(5)),
            )
            .await;
        }
    }
    agent.acp.shutdown().await;
    outcome
}
