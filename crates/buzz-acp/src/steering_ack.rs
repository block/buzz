//! Queue-side completion of a native steering acknowledgement.

use crate::{
    pool, relay, signal_in_flight_task_for_scope, spawn_failure_notice, AgentPool, ControlSignal,
    EventQueue, FlushBatch, SteerAckEvent, PERSONA_HOOK_REJECTION_NOTICE,
};

/// Apply one watcher acknowledgement without interrupting unrelated work.
pub(super) fn handle_steer_ack(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    ack_event: SteerAckEvent,
    max_turn_duration_secs: u64,
    rest_client: Option<&relay::RestClient>,
) {
    let SteerAckEvent {
        channel_id,
        scope,
        event_id,
        event,
        ack,
    } = ack_event;
    // Mid-turn steer attempt resolved (either transport:
    // `_goose/unstable/session/steer` or `_session/steering`).
    // Locked semantics (Eva + Max + Perci, unanimous on Option X):
    //
    //   Success
    //     The agent received the steer via the non-cancelling
    //     path. Drop the withheld event so normal dispatch
    //     never redelivers it.
    //
    //     Also covers `_session/steering`'s `startedNewTurn`
    //     outcome: the message was delivered, but into a fresh
    //     turn because the one being steered had already
    //     finished. Delivery is what this arm keys on, so the
    //     event is still dropped. The read loop deliberately
    //     does NOT renew its hard deadline in that case (the
    //     awaited turn is settled), while
    //     `extend_in_flight_deadline` below still applies —
    //     the agent really is running more work, so the
    //     channel's in-flight budget should reflect it.
    //
    //   Err(_) where the write never landed (Transport /
    //   ExpectedRunIdMissing):
    //     Delivery state of the underlying message is "never
    //     attempted on the wire". Release withheld back to the
    //     queue front AND issue the cancel+merge fallback so
    //     the message still reaches the agent.
    //
    //   Err(OutcomeRejected { .. })
    //     A `_session/steering` request returned a JSON-RPC
    //     success whose `outcome` was not `injected` or
    //     `startedNewTurn` (codex's `failed`, an unknown value,
    //     or a bare `{}` with no `outcome` at all). The steer
    //     did not land, so this is treated exactly like a write
    //     that never happened: release withheld AND fire the
    //     cancel+merge fallback. Handled by the catch-all
    //     `Err(_)` arm below.
    //
    //   Err(AgentError { code: -32601, .. })
    //     The agent returned method_not_found — it does not
    //     implement the steer extension. Release withheld AND
    //     fire the cancel+merge fallback so the message still
    //     reaches the agent via the universal path.
    //
    //   Err(AgentError { code: other, .. })
    //     The write landed and the agent returned a JSON-RPC
    //     error at the application level (e.g. wrong run id).
    //     The agent's turn is still running (or just completed).
    //     Release withheld for normal dispatch; do NOT fire the
    //     fallback signal — the agent already saw the steer
    //     attempt. If the turn is still running, normal dispatch
    //     re-delivers when it completes. If the turn already
    //     ended, there is nothing to cancel.
    //
    //   PromptCompletedNeutral
    //     The read loop wrote the steer (or was preparing to)
    //     but the prompt completed before the response landed.
    //     Delivery state is unknown — but the prompt completing
    //     means there is no in-flight turn to signal anymore.
    //     Release withheld for normal dispatch; do NOT fire
    //     the fallback signal (it would target a turn that
    //     just ended; normal dispatch already handles
    //     redelivery via the released queue entry).
    //
    //   Err(PromptCompleted)
    //     `SteerError::PromptCompleted` is returned synchronously
    //     by `pool::send_steer` when no task is in flight (handled
    //     in `try_native_steer`'s Err branch, which falls through
    //     to cancel+merge). It is never routed through the ack
    //     channel, so this variant never appears in `SteerAckEvent`.
    //
    //   Watcher Err (oneshot dropped)
    //     Should not happen — the read loop drains
    //     pending_steer on every return path. If it does,
    //     treat as PromptCompletedNeutral to avoid leaking
    //     the withheld event in `withheld_native_steer`.
    let (release_withheld, drop_withheld, signal_fallback) = match &ack {
        Ok(pool::SteerAck::Success { .. }) => (false, true, false),
        Ok(pool::SteerAck::Err(pool::SteerError::PersonaHookRejected)) => {
            // Terminal local refusal: report this follow-up, never cancel the
            // active turn or release the event for a later prompt.
            let batch = FlushBatch {
                channel_id,
                scope: scope.clone(),
                events: vec![*event],
                cancelled_events: vec![],
                cancel_reason: None,
            };
            spawn_failure_notice(rest_client, &batch, PERSONA_HOOK_REJECTION_NOTICE.into());
            (false, true, false)
        }
        // -32601 = method_not_found: agent does not implement the
        // steer extension. Fire cancel+merge so the message still
        // reaches the agent.
        Ok(pool::SteerAck::Err(pool::SteerError::AgentError { code, .. })) if *code == -32601 => {
            (true, false, true)
        }
        // AgentError: write landed, agent rejected it at the
        // application level (e.g. wrong run id). Release for
        // normal dispatch; no fallback signal (the turn is still
        // running or just ended — either way there is nothing to
        // cancel).
        Ok(pool::SteerAck::Err(pool::SteerError::AgentError { .. })) => (true, false, false),
        // Transport / ExpectedRunIdMissing / OutcomeRejected: the
        // steer did not land. Release and fire the cancel+merge
        // fallback so the message still reaches the agent.
        Ok(pool::SteerAck::Err(_)) => (true, false, true),
        Ok(pool::SteerAck::PromptCompletedNeutral) => (true, false, false),
        Err(_recv_err) => (true, false, false),
    };
    tracing::info!(
        channel = %channel_id,
        event_id = %event_id,
        ?ack,
        release_withheld,
        drop_withheld,
        signal_fallback,
        "non-cancelling steer ack received"
    );
    if let Ok(pool::SteerAck::Success { session_id }) = &ack {
        queue.extend_in_flight_deadline(&scope, max_turn_duration_secs);
        if !pool.record_successful_steer(&scope, event_id.clone(), session_id.clone()) {
            tracing::warn!(
                channel = %channel_id,
                event_id = %event_id,
                "successful steer lost its in-flight delivery ledger"
            );
        }
    }
    if drop_withheld {
        queue.remove_event(&scope, &event_id);
    }
    if release_withheld {
        queue.release_native_steer(&scope, &event_id);
    }
    if signal_fallback {
        // Universal cancel+merge fallback. Note: the
        // queued event has already been released to the
        // front of `queues[scope]`, so the cancel will pick
        // it up as part of the merged batch and re-prompt the
        // agent. Scope-exact so the fallback cancels the
        // steered event's OWN thread, not a sibling thread
        // in the same channel.
        signal_in_flight_task_for_scope(pool, &scope, ControlSignal::Steer);
    }
}
