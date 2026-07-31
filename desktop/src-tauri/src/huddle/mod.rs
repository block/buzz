//! Huddle (voice) state machine and Tauri commands.
//!
//! Mental model:
//!   parent channel → start_huddle → ephemeral channel + audio WS relay
//!   other clients  → join_huddle  → audio WS relay
//!   any client     → leave_huddle → lifecycle event, clear local state
//!   creator        → end_huddle   → archive ephemeral channel, clear state
//!
//! HuddleState is stored in AppState and serialized for get_huddle_state.
//!
//! ## Synchronization Protocol
//!
//! `HuddleState` lives behind a single `Mutex` in `AppState`. Rules:
//!
//! 1. **Never hold the outer lock across `.await`** — acquire, read/write, release.
//! 2. **Pipeline construction happens outside the lock** — the `stt_starting` /
//!    `tts_starting` sentinels prevent TOCTOU races during the ~200ms window.
//! 3. **`agent_pubkeys` has its own inner `Arc<Mutex>`** — the transcription task
//!    clones the `Arc` and reads at post time without the outer lock.
//! 4. **Atomics for cross-thread signaling** — `tts_active`, `tts_cancel`,
//!    `ptt_active`, `session_generation` are shared with pipeline worker threads.
//! 5. **Pipeline teardown extracts handles before dropping** — `teardown_huddle`
//!    takes `stt_pipeline`/`tts_pipeline` out of the lock, then calls `shutdown()`
//!    and drops them outside the lock (thread joins can block ~200ms).

pub mod agents;
pub mod audio_output;
pub mod jitter;
pub mod models;
pub mod pipeline;
pub mod playout;
pub mod pocket;
pub mod preprocessing;
pub mod reconnect;
pub mod relay_api;
pub mod state;
pub mod stt;
pub mod transcription;
pub mod tts;
pub mod wire;

// ── Shared utilities ──────────────────────────────────────────────────────────

/// Drain and discard all pending messages until shutdown or disconnect.
/// Shared by both the STT and TTS worker threads for graceful degradation
/// when model files are missing or initialization fails.
pub(super) fn drain_until_shutdown<T>(
    rx: std::sync::mpsc::Receiver<T>,
    shutdown: &std::sync::atomic::AtomicBool,
) {
    loop {
        if shutdown.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use state::{HuddleJoinInfo, HuddlePhase, HuddleState, VoiceInputMode};
pub use transcription::{set_huddle_transcription_enabled, start_stt_pipeline};

// ── Imports ───────────────────────────────────────────────────────────────────

use std::sync::atomic::Ordering;
use tauri::State;
use uuid::Uuid;

use crate::{app_state::AppState, events, relay::submit_event};
pub use pipeline::check_pipeline_hotstart;
use pipeline::{
    maybe_start_stt_pipeline, maybe_start_tts_pipeline, post_connect_setup,
    start_auto_enabled_transcription, PostConnectOutcome,
};
use relay_api::{
    count_human_members, fetch_channel_members, parse_channel_uuid, validate_pubkey_hex,
    MAX_HUDDLE_AGENTS,
};

fn normalize_huddle_channel_name(candidate: Option<String>, fallback: &str) -> String {
    let normalized = candidate
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let name = if normalized.is_empty() {
        fallback
    } else {
        normalized.as_str()
    };

    name.chars().take(80).collect()
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Set the voice input mode (push-to-talk or voice-activity detection).
///
/// When switching mid-huddle, restarts the STT pipeline so it picks up the
/// new mode (PTT gating vs continuous VAD with barge-in). The pipeline
/// captures the mode at construction time, so a restart is required.
#[tauri::command]
pub async fn set_voice_input_mode(
    mode: VoiceInputMode,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let needs_restart = {
        let mut hs = state.huddle()?;
        let old_mode = hs.voice_input_mode.clone();
        hs.voice_input_mode = mode.clone();
        // Restart STT if mode changed and a huddle is active with a pipeline running.
        old_mode != mode
            && matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active)
            && hs.stt_pipeline.is_some()
            && hs.transcription_enabled
    };

    if needs_restart {
        let eph_id = {
            let hs = state.huddle()?;
            hs.ephemeral_channel_id.clone()
        };
        if let Some(eph_id) = eph_id {
            // Best-effort restart — if models aren't ready, the pipeline
            // stays down until the next hotstart cycle picks it up.
            if let Err(e) = maybe_start_stt_pipeline(&state, &eph_id).await {
                eprintln!("buzz-desktop: STT pipeline restart on mode switch failed: {e}");
            }
        }
    }

    // A mode switch changes whether the global shortcut should be reserved but
    // does not otherwise emit; emit here so the frontend sees the new mode and
    // the shortcut registration is re-synced.
    state.emit_huddle_state_changed();

    Ok(())
}

/// Return the current voice input mode.
#[tauri::command]
pub fn get_voice_input_mode(state: State<'_, AppState>) -> Result<VoiceInputMode, String> {
    let hs = state.huddle()?;
    Ok(hs.voice_input_mode.clone())
}

/// Start a new huddle in the given parent channel.
///
/// Steps:
/// 1. Create an ephemeral channel (kind 9007, ttl=3600).
/// 2. Post voice-mode guidelines (kind 48106).
/// 3. Add each invited member to the ephemeral channel (kind 9000).
/// 4. Emit KIND_HUDDLE_STARTED to the parent channel (kind 48100).
/// 5. Store state and return join info.
///
/// If ANY step fails (including channel creation), the orphaned ephemeral
/// channel is archived (best-effort) and state is reset to Idle.
#[tauri::command]
pub async fn start_huddle(
    parent_channel_id: String,
    member_pubkeys: Vec<String>,
    channel_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<HuddleJoinInfo, String> {
    // Validate inputs at the Tauri boundary.
    if member_pubkeys.len() > MAX_HUDDLE_AGENTS {
        return Err(format!(
            "too many agents: {} (max {})",
            member_pubkeys.len(),
            MAX_HUDDLE_AGENTS
        ));
    }
    // Dedup and validate pubkey format.
    let member_pubkeys: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for pk in member_pubkeys {
            validate_pubkey_hex(&pk)?;
            if seen.insert(pk.clone()) {
                deduped.push(pk);
            }
        }
        deduped
    };

    // Transition to Creating.
    let huddle_generation = {
        let mut hs = state.huddle()?;
        if hs.phase != HuddlePhase::Idle {
            return Err(format!(
                "cannot start huddle: already in phase {:?}",
                hs.phase
            ));
        }
        let generation = hs.begin_huddle_lifetime();
        hs.phase = HuddlePhase::Creating;
        hs.parent_channel_id = Some(parent_channel_id.clone());
        generation
    };

    let ephemeral_uuid = Uuid::new_v4();
    let ephemeral_channel_id = ephemeral_uuid.to_string();
    let short_id = &ephemeral_channel_id[..8];
    let fallback_channel_name = format!("huddle-{short_id}");
    let channel_name = normalize_huddle_channel_name(channel_name, &fallback_channel_name);

    // All steps wrapped so we can roll back on ANY failure, including step 1.
    // channel_was_created tracks whether we need to archive on rollback.
    let mut channel_was_created = false;

    let result: Result<Vec<String>, String> = async {
        // 1. Create ephemeral channel.
        let create_builder = events::build_create_channel(
            ephemeral_uuid,
            &channel_name,
            "private",
            "stream",
            None,
            Some(3600),
        )?;
        submit_event(create_builder, &state).await?;
        channel_was_created = true;

        // 2. Post voice-mode guidelines as kind:48106 BEFORE adding agents.
        //    Agents auto-subscribe on membership notification (kind:9000) and may
        //    complete EOSE before guidelines are stored if we post them after.
        //    Best-effort: don't fail the huddle if this fails.
        let guidelines = agents::voice_mode_guidelines(&parent_channel_id);
        if let Ok(guidelines_builder) =
            events::build_huddle_guidelines(&ephemeral_channel_id, &guidelines)
        {
            if let Err(e) = submit_event(guidelines_builder, &state).await {
                eprintln!("buzz-desktop: huddle guidelines (kind:48106) failed: {e}");
            }
        }

        // 3. Add members to the ephemeral channel; only keep successfully enrolled ones.
        let mut successful_agents: Vec<String> = Vec::new();
        for pubkey in &member_pubkeys {
            let add_builder = events::build_add_member(ephemeral_uuid, pubkey, Some("bot"))?;
            match submit_event(add_builder, &state).await {
                Ok(_) => successful_agents.push(pubkey.clone()),
                Err(e) => {
                    eprintln!("buzz-desktop: huddle add_member failed for {pubkey}: {e}");
                    // Intentionally not added — policy rejected this agent.
                }
            }
        }

        // 4. Emit HUDDLE_STARTED to parent channel.
        let started_builder =
            events::build_huddle_started(&parent_channel_id, &ephemeral_channel_id)?;
        submit_event(started_builder, &state).await?;

        Ok(successful_agents)
    }
    .await;

    match result {
        Ok(successful_agents) => {
            // 5. Store active state.
            let committed = {
                let mut hs = state.huddle()?;
                if !hs.owns_huddle_lifetime(huddle_generation, HuddlePhase::Creating) {
                    false
                } else {
                    hs.phase = HuddlePhase::Connected;
                    hs.is_creator = true;
                    hs.ephemeral_channel_id = Some(ephemeral_channel_id.clone());
                    *hs.agent_pubkeys.lock().unwrap_or_else(|e| e.into_inner()) =
                        successful_agents.clone();
                    hs.maybe_auto_enable_transcription_for_agents();
                    let own_pubkey = state
                        .keys
                        .lock()
                        .map(|k| k.public_key().to_hex())
                        .unwrap_or_default();
                    let mut participants = successful_agents.clone();
                    if !own_pubkey.is_empty() && !participants.contains(&own_pubkey) {
                        participants.insert(0, own_pubkey);
                    }
                    hs.participants = participants;
                    true
                }
            };
            if !committed {
                emit_end_and_archive(&parent_channel_id, &ephemeral_channel_id, &state).await;
                return Err("huddle start was superseded".to_owned());
            }

            // 6. Notify frontend of state change.
            state.emit_huddle_state_changed();

            // 7. Hydrate members, download models, start pipelines (incl. audio relay).
            // Audio relay failure is fatal — no point in a huddle without audio.
            match post_connect_setup(&state, &ephemeral_channel_id, huddle_generation).await {
                Ok(PostConnectOutcome::Ready) => {}
                Ok(PostConnectOutcome::Stale) => {
                    return Err("huddle start was superseded".to_owned());
                }
                Err(e) => {
                    // Roll back only if this failed setup still owns the active
                    // huddle. A stale failure must not tear down its replacement.
                    let still_current = state
                        .huddle()
                        .map(|hs| hs.is_current_huddle(&ephemeral_channel_id, huddle_generation))
                        .unwrap_or(false);
                    if still_current {
                        emit_end_and_archive(&parent_channel_id, &ephemeral_channel_id, &state)
                            .await;
                        if let Ok(mut hs) = state.huddle_state.lock() {
                            if hs.is_current_huddle(&ephemeral_channel_id, huddle_generation) {
                                hs.reset_preserving_generation();
                            }
                        }
                        state.emit_huddle_state_changed();
                    }
                    return Err(e);
                }
            }

            Ok(HuddleJoinInfo {
                ephemeral_channel_id,
            })
        }
        Err(e) => {
            // Rollback: archive the orphaned ephemeral channel if it was created.
            if channel_was_created {
                if let Ok(archive_builder) = events::build_archive(ephemeral_uuid) {
                    if let Err(ae) = submit_event(archive_builder, &state).await {
                        eprintln!(
                            "buzz-desktop: rollback archive of {ephemeral_channel_id} failed: {ae}"
                        );
                    }
                }
            }
            // Reset only if this failed attempt still owns the Creating state.
            if let Ok(mut hs) = state.huddle_state.lock() {
                if hs.owns_huddle_lifetime(huddle_generation, HuddlePhase::Creating) {
                    hs.reset_preserving_generation();
                }
            }
            Err(e)
        }
    }
}

/// Join an existing huddle in the given parent channel.
///
/// Steps:
/// 1. Transition to Connecting.
/// 2. Store state and return join info.
/// 3. Post-connect setup (audio relay WS, pipelines, model hydration).
///
/// The relay emits kind:48101 (participant joined) when the audio WS authenticates.
#[tauri::command]
pub async fn join_huddle(
    parent_channel_id: String,
    ephemeral_channel_id: String,
    state: State<'_, AppState>,
) -> Result<HuddleJoinInfo, String> {
    // Transition to Connecting.
    let huddle_generation = {
        let mut hs = state.huddle()?;
        if hs.phase != HuddlePhase::Idle {
            return Err(format!(
                "cannot join huddle: already in phase {:?}",
                hs.phase
            ));
        }
        let generation = hs.begin_huddle_lifetime();
        hs.phase = HuddlePhase::Connecting;
        hs.parent_channel_id = Some(parent_channel_id.clone());
        hs.ephemeral_channel_id = Some(ephemeral_channel_id.clone());
        generation
    };

    // Seed participant list with own pubkey as a fallback until relay responds.
    let own_pubkey = state
        .keys
        .lock()
        .map(|k| k.public_key().to_hex())
        .unwrap_or_default();

    let committed = {
        let mut hs = state.huddle()?;
        if !hs.owns_huddle_lifetime(huddle_generation, HuddlePhase::Connecting) {
            false
        } else {
            hs.phase = HuddlePhase::Connected;
            if !own_pubkey.is_empty() {
                hs.participants = vec![own_pubkey];
            }
            true
        }
    };
    if !committed {
        return Err("huddle join was superseded".to_owned());
    }

    // Notify frontend of state change.
    state.emit_huddle_state_changed();

    // Hydrate members, download models, start pipelines (incl. audio relay).
    // Audio relay failure is fatal — no point in a huddle without audio.
    match post_connect_setup(&state, &ephemeral_channel_id, huddle_generation).await {
        Ok(PostConnectOutcome::Ready) => {}
        Ok(PostConnectOutcome::Stale) => {
            return Err("huddle join was superseded".to_owned());
        }
        Err(e) => {
            // Reset only the huddle lifetime that failed.
            let mut did_reset = false;
            if let Ok(mut hs) = state.huddle_state.lock() {
                if hs.is_current_huddle(&ephemeral_channel_id, huddle_generation) {
                    hs.reset_preserving_generation();
                    did_reset = true;
                }
            }
            if did_reset {
                state.emit_huddle_state_changed();
            }
            return Err(e);
        }
    }

    Ok(HuddleJoinInfo {
        ephemeral_channel_id,
    })
}

/// Shut down all pipelines and reset huddle state to Idle.
///
/// Used by both `leave_huddle` and `end_huddle` to avoid duplicating the
/// shutdown-then-reset sequence.
fn teardown_huddle(state: &AppState) -> Result<(), String> {
    // Take pipeline handles out of state and drop the lock before shutdown.
    // Pipeline Drop impls join worker threads — this avoids blocking while
    // the mutex is held (ONNX inference can take ~200ms).
    let (old_stt, old_tts, _audio_cancel) = {
        let mut hs = state.huddle()?;
        // Increment generation first — this immediately invalidates any
        // in-flight transcription task, even before pipelines shut down.
        hs.session_generation.fetch_add(1, Ordering::Release);
        let stt = hs.stt_pipeline.take();
        let tts = hs.tts_pipeline.take();
        let cancel = hs.audio_ws_cancel.take();
        // Cancel the relay token BEFORE dropping the sender. If we drop
        // pcm_tx first, the send task sees None from recv() and can exit
        // the pipeline before is_cancelled() is true — causing a spurious
        // huddle-audio-disconnected event on intentional teardown.
        if let Some(ref c) = cancel {
            c.cancel();
        }
        hs.audio_relay_pcm_tx.take(); // Drop sender — signals the relay task.
        hs.reset_preserving_generation();
        (stt, tts, cancel)
    };
    // Shut down STT/TTS outside the lock — thread joins happen here.
    if let Some(ref p) = old_stt {
        p.shutdown();
    }
    if let Some(ref p) = old_tts {
        p.shutdown();
    }
    // Drop the Arcs here (implicit) — triggers thread join via Drop.
    drop(old_stt);
    drop(old_tts);
    // Notify frontend that we're back to Idle.
    state.emit_huddle_state_changed();
    Ok(())
}

/// Emit HUDDLE_ENDED to the parent channel and archive the ephemeral channel.
///
/// Both steps are best-effort — failures are logged but do not propagate.
/// Called from `leave_huddle` (auto-end path) and `end_huddle`.
async fn emit_end_and_archive(
    parent_channel_id: &str,
    ephemeral_channel_id: &str,
    state: &AppState,
) {
    if !parent_channel_id.is_empty() && !ephemeral_channel_id.is_empty() {
        if let Ok(ended_builder) =
            events::build_huddle_ended(parent_channel_id, ephemeral_channel_id)
        {
            if let Err(e) = submit_event(ended_builder, state).await {
                eprintln!("buzz-desktop: huddle_ended event failed: {e}");
            }
        }
    }
    remove_huddle_agents(ephemeral_channel_id, state).await;
    if !ephemeral_channel_id.is_empty() {
        if let Ok(uuid) = parse_channel_uuid(ephemeral_channel_id) {
            if let Ok(archive_builder) = events::build_archive(uuid) {
                if let Err(e) = submit_event(archive_builder, state).await {
                    eprintln!("buzz-desktop: archive ephemeral channel failed: {e}");
                }
            }
        }
    }
}

/// Best-effort agent removal before archiving an ended huddle.
///
/// Archive should make the ephemeral channel unavailable on its own, but removing
/// bot-role members first prevents an agent-only huddle from lingering if the
/// archive publish is delayed or rejected.
async fn remove_huddle_agents(ephemeral_channel_id: &str, state: &AppState) {
    if ephemeral_channel_id.is_empty() {
        return;
    }
    let Ok(eph_uuid) = parse_channel_uuid(ephemeral_channel_id) else {
        return;
    };

    let agent_pubkeys = match fetch_channel_members(ephemeral_channel_id, Some("bot"), state).await
    {
        Ok(pubkeys) => pubkeys,
        Err(e) => {
            eprintln!("buzz-desktop: fetch huddle agents for cleanup failed: {e}");
            return;
        }
    };

    for pubkey in agent_pubkeys {
        let Ok(remove_builder) = events::build_remove_member(eph_uuid, &pubkey) else {
            continue;
        };
        if let Err(e) = submit_event(remove_builder, state).await {
            eprintln!("buzz-desktop: remove huddle agent {pubkey} failed: {e}");
        }
    }
}

/// Leave the current huddle.
///
/// Steps:
/// 1. Transition to Leaving.
/// 2. Auto-end check: if last human, emit HUDDLE_ENDED + archive.
/// 3. Shut down pipelines and audio relay.
///
/// The relay emits kind:48102 (participant left) when the audio WS disconnects.
#[tauri::command]
pub async fn leave_huddle(state: State<'_, AppState>) -> Result<(), String> {
    let (parent_channel_id, ephemeral_channel_id) = {
        let mut hs = state.huddle()?;
        if hs.phase == HuddlePhase::Idle {
            return Ok(()); // Nothing to leave.
        }
        hs.phase = HuddlePhase::Leaving;
        (
            hs.parent_channel_id.clone().unwrap_or_default(),
            hs.ephemeral_channel_id.clone().unwrap_or_default(),
        )
    };

    // Auto-end: check if any human participants remain. If not, end the huddle
    // (emit HUDDLE_ENDED + archive). If others remain, just remove self from
    // membership so the participant roster stays accurate.
    //
    // We check BEFORE removing self — the relay counts us as a member until
    // we leave. So "1 human remaining" means WE are the last one.
    if !parent_channel_id.is_empty() && !ephemeral_channel_id.is_empty() {
        let humans_remaining = count_human_members(&ephemeral_channel_id, &state)
            .await
            // On fetch failure, assume 2 humans remain (safe default).
            // unwrap_or(1) would mean "I'm the last human" → triggers auto-archive,
            // ending the huddle for everyone on a transient REST failure. Using 2
            // means we skip the auto-end path and just remove ourselves — the huddle
            // stays alive and the next real leave will clean up correctly.
            .unwrap_or(2);

        if humans_remaining <= 1 {
            // We're the last human — end the huddle entirely.
            // Archive subsumes leave (the channel is gone, membership is moot).
            // This avoids the "cannot remove the last owner" relay error that
            // build_leave hits when the creator is the sole remaining member.
            eprintln!("buzz-desktop: last human left huddle — auto-ending");
            emit_end_and_archive(&parent_channel_id, &ephemeral_channel_id, &state).await;
        } else {
            // Other humans still in the huddle — just remove self from membership.
            if let Ok(eph_uuid) = parse_channel_uuid(&ephemeral_channel_id) {
                if let Ok(leave_builder) = events::build_leave(eph_uuid) {
                    if let Err(e) = submit_event(leave_builder, &state).await {
                        eprintln!("buzz-desktop: huddle leave ephemeral channel failed: {e}");
                    }
                }
            }
        }
    }

    teardown_huddle(&state)?;

    Ok(())
}

/// End the current huddle (creator only).
///
/// Steps:
/// 1. Emit KIND_HUDDLE_ENDED to the parent channel.
/// 2. Archive the ephemeral channel.
/// 3. Shut down the STT pipeline (Fix 5).
/// 4. Clear local huddle state.
#[tauri::command]
pub async fn end_huddle(force: Option<bool>, state: State<'_, AppState>) -> Result<(), String> {
    let (parent_channel_id, ephemeral_channel_id) = {
        let mut hs = state.huddle()?;
        if hs.phase == HuddlePhase::Idle {
            return Ok(()); // Nothing to end.
        }
        // Only the creator can end the huddle for everyone. Non-creators
        // should use leave_huddle (which auto-ends if they're the last human).
        // The `force` flag allows recovery when the creator has disconnected
        // ungracefully — the UI should gate this behind a confirmation dialog.
        if !hs.is_creator && !force.unwrap_or(false) {
            return Err("only the huddle creator can end it — use leave_huddle instead".into());
        }
        hs.phase = HuddlePhase::Leaving;
        (
            hs.parent_channel_id.clone().unwrap_or_default(),
            hs.ephemeral_channel_id.clone().unwrap_or_default(),
        )
    };

    emit_end_and_archive(&parent_channel_id, &ephemeral_channel_id, &state).await;

    teardown_huddle(&state)?;

    Ok(())
}

/// Confirm that the frontend has established mic + AudioWorklet.
/// Transitions from Connected → Active. No-op if already Active.
#[tauri::command]
pub async fn confirm_huddle_active(state: State<'_, AppState>) -> Result<(), String> {
    let transitioned = {
        let mut hs = state.huddle()?;
        match hs.phase {
            HuddlePhase::Connected => {
                hs.phase = HuddlePhase::Active;
                true
            }
            HuddlePhase::Active => false, // Already active — idempotent.
            ref other => return Err(format!("cannot confirm active: phase is {:?}", other)),
        }
    };
    if transitioned {
        state.emit_huddle_state_changed();
    }
    Ok(())
}

/// Return the current HuddleState (serialized for the frontend).
#[tauri::command]
pub fn get_huddle_state(state: State<'_, AppState>) -> Result<HuddleState, String> {
    let hs = state.huddle()?;
    Ok(hs.clone())
}

/// Return the authoritative list of agent (bot-role) pubkeys in the active huddle.
///
/// Fetches from the relay's channel membership API — works for both creators
/// and joiners. Returns `Ok(Vec::new())` if no huddle is active. Returns
/// `Err` on relay fetch failure so the frontend can keep `agentsLoaded = false`
/// rather than treating a failed lookup as "zero agents".
#[tauri::command]
pub async fn get_huddle_agent_pubkeys(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let eph_id = {
        let hs = state.huddle()?;
        hs.ephemeral_channel_id.clone()
    };
    match eph_id {
        Some(id) => fetch_channel_members(&id, Some("bot"), &state).await,
        None => Ok(Vec::new()),
    }
}

/// Maximum IPC audio batch size: 100 KB.
/// A 100 ms batch at 48 kHz mono f32 is ~19 KB; 100 KB allows headroom
/// without letting a malformed IPC call allocate unbounded memory.
const MAX_AUDIO_BATCH_BYTES: usize = 100 * 1024;

/// Receive raw PCM audio bytes from the AudioWorklet and feed the STT pipeline.
///
/// Expects a raw binary body of f32 LE samples at 48 kHz mono.
/// If no STT pipeline is active, the bytes are silently discarded.
#[tauri::command]
pub fn push_audio_pcm(
    request: tauri::ipc::Request<'_>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            if bytes.len() > MAX_AUDIO_BATCH_BYTES {
                return Err(format!(
                    "audio batch too large: {} bytes (max {})",
                    bytes.len(),
                    MAX_AUDIO_BATCH_BYTES
                ));
            }
            if let Ok(hs) = state.huddle() {
                // Fan out to STT pipeline.
                if let Some(ref pipeline) = hs.stt_pipeline {
                    pipeline.push_audio(bytes.to_vec())?;
                }
                // Fan out to audio relay encoder (best-effort, non-blocking).
                if let Some(ref pcm_tx) = hs.audio_relay_pcm_tx {
                    let _ = pcm_tx.try_send(bytes.to_vec());
                }
            }
            Ok(())
        }
        _ => Err("expected raw binary body".to_string()),
    }
}

/// Trigger a background download of voice models (Parakeet STT + Pocket TTS).
///
/// Returns immediately — downloads run in tokio background tasks.
/// Poll `get_model_status` to track progress.
/// Safe to call multiple times: no-op if already downloading or ready.
#[tauri::command]
pub async fn download_voice_models(state: State<'_, AppState>) -> Result<(), String> {
    let manager = models::global_model_manager()
        .ok_or("model manager unavailable (home directory could not be resolved)")?;
    manager.start_stt_download(state.http_client.clone());
    manager.start_tts_download(state.http_client.clone());
    Ok(())
}

/// Return the current download status for all voice models.
#[tauri::command]
pub fn get_model_status(_state: State<'_, AppState>) -> Result<models::VoiceModelStatus, String> {
    let manager = models::global_model_manager()
        .ok_or("model manager unavailable (home directory could not be resolved)")?;
    Ok(models::VoiceModelStatus {
        stt: manager.stt_status(),
        tts: manager.tts_status(),
    })
}

/// Enable or disable TTS output.
///
/// When disabled, the TTS pipeline is shut down and audio output stops.
/// When re-enabled, the pipeline is restarted if TTS models are available.
///
/// Takes the pipeline handle out of the lock before calling shutdown() — the
/// thread join in Drop can block for ~200 ms (ONNX inference) and we don't
/// want to hold the HuddleState mutex during that time.
#[tauri::command]
pub async fn set_tts_enabled(enabled: bool, state: State<'_, AppState>) -> Result<(), String> {
    let old_pipeline = {
        let mut hs = state.huddle()?;
        hs.tts_enabled = enabled;
        if !enabled {
            hs.tts_pipeline.take() // Take out of lock.
        } else {
            None
        }
    };
    // Shut down outside the lock — thread join happens here.
    if let Some(ref pipeline) = old_pipeline {
        pipeline.shutdown();
    }
    drop(old_pipeline);

    if enabled {
        // Re-start TTS pipeline if models are available and huddle is active.
        let phase = {
            let hs = state.huddle()?;
            hs.phase.clone()
        };
        if matches!(phase, HuddlePhase::Connected | HuddlePhase::Active) {
            if let Err(e) = maybe_start_tts_pipeline(&state).await {
                eprintln!("buzz-desktop: TTS pipeline restart failed: {e}");
            }
        }
    }

    Ok(())
}

/// Speak an agent message via TTS.
///
/// Maximum text length accepted for TTS synthesis.
/// ~2000 chars ≈ 1–2 minutes of speech. Longer messages are truncated.
const MAX_TTS_TEXT_LEN: usize = 2000;

/// Called by the WebView when it receives an incoming agent kind:9 message.
/// Lazily starts the TTS pipeline if models are ready but the pipeline hasn't
/// been created yet (e.g. models finished downloading after huddle started).
///
/// No-op if TTS is disabled or models aren't ready.
#[tauri::command]
pub async fn speak_agent_message(text: String, state: State<'_, AppState>) -> Result<(), String> {
    // Truncate oversized messages — agents shouldn't monologue in a voice huddle.
    // Use char count (not byte length) to avoid panicking on multi-byte UTF-8.
    let text = if text.chars().count() > MAX_TTS_TEXT_LEN {
        let mut truncated: String = text.chars().take(MAX_TTS_TEXT_LEN).collect();
        truncated.push_str("... message truncated.");
        truncated
    } else {
        text
    };

    let needs_pipeline = {
        let hs = state.huddle()?;
        hs.tts_enabled
            && hs.tts_pipeline.is_none()
            && matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active)
    };

    // Lazy-start: models may have finished downloading after the huddle began.
    if needs_pipeline {
        if let Err(e) = maybe_start_tts_pipeline(&state).await {
            eprintln!("buzz-desktop: TTS lazy-start failed: {e}");
        }
    }

    let hs = state.huddle()?;
    if hs.tts_enabled {
        if let Some(ref pipeline) = hs.tts_pipeline {
            pipeline.speak(text)?;
        }
    }
    Ok(())
}

/// Add an agent to the active huddle.
///
/// Steps:
/// 1. Validates the huddle is in the Connected or Active phase.
/// 2. Adds the agent to both the ephemeral and parent channels (kind:9000).
/// 3. Only appends the agent pubkey to `agent_pubkeys` if the ephemeral add
///    succeeded — failed adds (policy rejection) are NOT p-tagged.
///
/// Returns a structured `AgentAddResult` so the frontend can surface
/// parent-channel errors without treating them as hard failures.
///
/// The running ACP process for this agent auto-subscribes when it receives
/// the kind:9000 membership notification — no separate process spawn needed.
#[tauri::command]
pub async fn add_agent_to_huddle(
    agent_pubkey: String,
    state: State<'_, AppState>,
) -> Result<agents::AgentAddResult, String> {
    validate_pubkey_hex(&agent_pubkey)?;

    let (eph_id, parent_id, huddle_generation) = {
        let hs = state.huddle()?;
        if !matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Err("no active huddle".to_string());
        }

        // Enforce agent cap on incremental adds too.
        let current_agent_count = hs
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        if current_agent_count >= MAX_HUDDLE_AGENTS {
            return Err(format!(
                "agent limit reached: {} (max {})",
                current_agent_count, MAX_HUDDLE_AGENTS
            ));
        }

        let eph = hs
            .ephemeral_channel_id
            .clone()
            .ok_or("no ephemeral channel")?;
        let parent = hs.parent_channel_id.clone().ok_or("no parent channel")?;
        (eph, parent, hs.huddle_generation)
    };

    let eph_uuid = Uuid::parse_str(&eph_id).map_err(|e| e.to_string())?;
    let parent_uuid = Uuid::parse_str(&parent_id).map_err(|e| e.to_string())?;

    // Returns Err only if the ephemeral add fails — parent failure is in the result.
    let result = agents::add_agent_to_huddle(eph_uuid, parent_uuid, &agent_pubkey, &state).await?;

    // Ephemeral add succeeded — register it only if this is still the huddle
    // that initiated the relay operation.
    let transcription_auto_enabled = {
        let mut hs = state.huddle()?;
        if !hs.is_current_huddle(&eph_id, huddle_generation) {
            return Ok(result);
        }
        let mut pubkeys = hs.agent_pubkeys.lock().unwrap_or_else(|e| e.into_inner());
        if !pubkeys.contains(&agent_pubkey) {
            pubkeys.push(agent_pubkey.clone());
        }
        drop(pubkeys);
        if !hs.participants.contains(&agent_pubkey) {
            hs.participants.push(agent_pubkey.clone());
        }
        hs.maybe_auto_enable_transcription_for_agents()
    };

    // No guidelines re-post needed — the agent sees the original kind:48106
    // guidelines via EOSE replay when it subscribes to the ephemeral channel.
    if transcription_auto_enabled {
        start_auto_enabled_transcription(&state, &eph_id).await;
    } else {
        state.emit_huddle_state_changed();
    }

    Ok(result)
}
