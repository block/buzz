//! STT/TTS pipeline lifecycle management.
//!
//! Handles starting, hot-starting, and spawning transcription tasks for
//! the voice pipelines. Extracted from mod.rs to keep the command layer thin.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use nostr::JsonUtil;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::events;

use super::models;
use super::relay_api::{self, fetch_channel_members, parse_channel_uuid};
use super::state::{HuddlePhase, VoiceInputMode};
use super::stt;
use super::tts;

pub(crate) async fn post_connect_setup(
    state: &AppState,
    ephemeral_channel_id: &str,
) -> Result<(), String> {
    // Hydrate agent pubkeys and participants from relay in parallel
    // (authoritative — overrides local guesses).
    let (agents_result, all_members_result) = tokio::join!(
        fetch_channel_members(ephemeral_channel_id, Some("bot"), state),
        fetch_channel_members(ephemeral_channel_id, None, state),
    );
    if let Ok(agents) = agents_result {
        let hs = state.huddle()?;
        *hs.agent_pubkeys.lock().unwrap_or_else(|e| e.into_inner()) = agents;
    }

    if let Ok(all_members) = all_members_result {
        if !all_members.is_empty() {
            let mut hs = state.huddle()?;
            hs.participants = all_members;
        }
    }

    // Prepare TTS for agent voice. STT is transcript-specific and starts only
    // when transcription is explicitly enabled.
    if let Some(mgr) = models::global_model_manager() {
        mgr.start_tts_download(state.http_client.clone());
    }

    // Connect audio relay WebSocket (Opus encode/decode pipeline).
    // This is the core audio path — failure is fatal for the huddle.
    let parent_id = {
        let hs = state.huddle()?;
        hs.parent_channel_id.clone()
    };
    let (cancel, pcm_tx) =
        relay_api::connect_audio_relay(ephemeral_channel_id, parent_id.as_deref(), state).await?;
    {
        let mut hs = state.huddle()?;
        hs.audio_ws_cancel = Some(cancel);
        hs.audio_relay_pcm_tx = Some(pcm_tx);
    }

    // Start TTS immediately. STT/transcript posting is opt-in and starts only
    // after the user explicitly enables transcription.
    if let Err(e) = maybe_start_tts_pipeline(state).await {
        eprintln!("buzz-desktop: TTS pipeline failed to start: {e}");
    }

    Ok(())
}

/// Attempt to start the STT pipeline if models are present.
///
/// Returns `Ok(true)` if the pipeline was started, `Ok(false)` if models are
/// not ready (voice-only mode), or `Err` on a real failure.
///
/// Creates the shared `tts_active` flag and passes it to the STT pipeline
/// for barge-in / echo gating. The same flag is later passed to the TTS
/// pipeline so it can signal when audio is playing.
pub(crate) async fn maybe_start_stt_pipeline(
    state: &AppState,
    ephemeral_channel_id: &str,
) -> Result<bool, String> {
    {
        let hs = state.huddle()?;
        if !hs.transcription_enabled {
            return Ok(false);
        }
    }

    if !models::is_stt_ready() {
        return Ok(false); // Models not downloaded yet — voice-only mode.
    }
    let model_dir = models::stt_model_dir().ok_or("STT model directory not found")?;

    let channel_uuid = parse_channel_uuid(ephemeral_channel_id)?;

    // Atomically claim the construction slot (mirrors tts_starting pattern).
    {
        let hs = state.huddle()?;
        if hs.stt_starting.swap(true, Ordering::AcqRel) {
            return Ok(false); // Another caller is already constructing.
        }
    }

    // Grab shared flags, agent pubkeys, and session generation from HuddleState.
    // If replacing an existing pipeline, bump generation first so the old
    // transcription task's next POST sees a stale generation and exits.
    // Take the old pipeline OUT of the lock before dropping — Drop joins
    // the worker thread (~200ms) and must not block under the mutex.
    let (tts_active, tts_cancel, agent_pubkeys_arc, session_gen, ptt_active_for_stt, old_stt) = {
        let mut hs = state.huddle()?;
        // Invalidate any existing transcription task before replacing the pipeline.
        if hs.stt_pipeline.is_some() {
            hs.session_generation.fetch_add(1, Ordering::Release);
        }
        let old = hs.stt_pipeline.take();
        if let Some(ref p) = old {
            p.shutdown();
        }
        let ptt = if hs.voice_input_mode == VoiceInputMode::PushToTalk {
            Some(Arc::clone(&hs.ptt_active))
        } else {
            None
        };
        (
            Arc::clone(&hs.tts_active),
            Some(Arc::clone(&hs.tts_cancel)),
            Arc::clone(&hs.agent_pubkeys),
            Arc::clone(&hs.session_generation),
            ptt,
            old,
        )
    };
    // Drop the old pipeline OUTSIDE the lock — thread join happens here.
    drop(old_stt);

    let constructed = tokio::task::spawn_blocking(move || {
        stt::SttPipeline::new(model_dir, tts_active, tts_cancel, ptt_active_for_stt)
    })
    .await;
    let (pipeline, text_rx) = match constructed {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            let hs = state.huddle()?;
            hs.stt_starting.store(false, Ordering::Release);
            return Err(e);
        }
        Err(e) => {
            let hs = state.huddle()?;
            hs.stt_starting.store(false, Ordering::Release);
            return Err(format!("spawn_blocking failed: {e}"));
        }
    };
    let pipeline = Arc::new(pipeline);

    {
        let mut hs = state.huddle()?;
        hs.stt_starting.store(false, Ordering::Release);
        // Phase check: huddle may have been torn down during construction.
        if !hs.transcription_enabled
            || !matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active)
        {
            return Ok(false);
        }
        hs.stt_pipeline = Some(Arc::clone(&pipeline));
    }

    spawn_transcription_task(text_rx, channel_uuid, agent_pubkeys_arc, session_gen, state);
    Ok(true)
}

/// Attempt to start the TTS pipeline if TTS models are present and TTS is enabled.
///
/// Returns `Ok(true)` if the pipeline was started, `Ok(false)` if preconditions
/// aren't met (model not ready, pipeline exists, TTS disabled), or `Err` on failure.
///
/// Uses `tts_starting` sentinel to prevent TOCTOU races: two concurrent callers
/// (e.g. `check_pipeline_hotstart` + `speak_agent_message` lazy-start) could both
/// pass the `is_some()` check, both construct pipelines, and the loser's thread
/// leaks ~200MB of ONNX sessions. The sentinel is set under the lock before
/// releasing it for the expensive construction step.
pub(crate) async fn maybe_start_tts_pipeline(state: &AppState) -> Result<bool, String> {
    if !models::is_tts_ready() {
        return Ok(false); // TTS model not downloaded yet — TTS unavailable.
    }

    let model_dir = match models::tts_model_dir() {
        Some(d) => d,
        None => return Ok(false),
    };

    // Atomically check preconditions and claim the construction slot.
    // The sentinel prevents a second caller from starting construction
    // while we're building outside the lock.
    let (tts_active, tts_cancel) = {
        let hs = state.huddle()?;
        if hs.tts_pipeline.is_some() {
            return Ok(false);
        }
        if !hs.tts_enabled {
            return Ok(false);
        }
        if hs.tts_starting.swap(true, Ordering::AcqRel) {
            return Ok(false); // Another caller is already constructing.
        }
        (Arc::clone(&hs.tts_active), Arc::clone(&hs.tts_cancel))
    };

    // Construct outside the lock — this spawns the TTS worker thread and
    // loads ONNX sessions (~200ms). If this fails, clear the sentinel.
    let output_device = state
        .audio_output_device
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let constructed = tokio::task::spawn_blocking(move || {
        tts::TtsPipeline::new(model_dir, tts_active, tts_cancel, output_device)
    })
    .await;
    let pipeline = match constructed {
        Ok(Ok(p)) => Arc::new(p),
        Ok(Err(e)) => {
            let hs = state.huddle()?;
            hs.tts_starting.store(false, Ordering::Release);
            return Err(e);
        }
        Err(e) => {
            let hs = state.huddle()?;
            hs.tts_starting.store(false, Ordering::Release);
            return Err(format!("spawn_blocking failed: {e}"));
        }
    };

    {
        let mut hs = state.huddle()?;
        hs.tts_starting.store(false, Ordering::Release);
        // Phase check: huddle may have been torn down during construction.
        if !matches!(hs.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Ok(false);
        }
        // Final check: another path may have created a pipeline while we were constructing.
        if hs.tts_pipeline.is_some() {
            return Ok(false);
        }
        hs.tts_pipeline = Some(pipeline);
    }

    Ok(true)
}

/// Spawn a tokio task that reads text_rx and posts kind:9 events.
///
/// Fix 1: `agent_pubkeys_arc` is an `Arc<Mutex<Vec<String>>>` cloned from
///        `HuddleState` — the task reads it at post time so p-tags are always
///        current, not a stale snapshot.
/// Fix 3: no `.unwrap()` on mutex — poisoned locks are recovered gracefully.
/// Fix 4: `text_rx` is a `tokio::sync::mpsc::Receiver` — fully async `.recv().await`
///        never blocks a Tokio worker thread (unlike std `recv_timeout`).
pub(crate) fn spawn_transcription_task(
    mut text_rx: tokio::sync::mpsc::Receiver<String>,
    channel_uuid: Uuid,
    agent_pubkeys_arc: Arc<Mutex<Vec<String>>>,
    session_generation: Arc<AtomicU64>,
    state: &AppState,
) {
    // Capture the current generation at spawn time.
    let spawned_gen = session_generation.load(Ordering::Acquire);

    let http_client = state.http_client.clone();
    let keys = match state.keys.lock() {
        Ok(k) => k.clone(),
        Err(_) => return,
    };
    let relay_base_url = crate::relay::relay_api_base_url_with_override(state);

    tauri::async_runtime::spawn(async move {
        // recv().await yields (not blocks) until text arrives or sender is dropped.
        // When the STT worker exits and drops its Sender, recv() returns None → loop ends.
        while let Some(t) = text_rx.recv().await {
            if t.is_empty() {
                continue;
            }

            // Session guard: if the generation has changed, this task is stale.
            // Drop the transcript silently — the huddle has ended or been replaced.
            if session_generation.load(Ordering::Acquire) != spawned_gen {
                break; // Exit the loop entirely — no more posts from this task.
            }

            // Fix 1: read current agent pubkeys at post time.
            let agent_pubkeys: Vec<String> = agent_pubkeys_arc
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();

            let p_tags: Vec<&str> = agent_pubkeys.iter().map(|s| s.as_str()).collect();
            let builder =
                match events::build_message(channel_uuid, &t, None, &p_tags, &[], &[], &[]) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("buzz-desktop: STT build_message: {e}");
                        continue;
                    }
                };
            let event = match builder.sign_with_keys(&keys) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("buzz-desktop: STT sign event: {e}");
                    continue;
                }
            };
            let body_bytes = event.as_json().into_bytes();
            let url = format!("{relay_base_url}/events");
            let auth_header = match crate::relay::build_nip98_auth_header_for_keys(
                &keys,
                &reqwest::Method::POST,
                &url,
                &body_bytes,
            ) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("buzz-desktop: STT NIP-98 auth: {e}");
                    continue;
                }
            };

            let response = http_client
                .post(&url)
                .header("Authorization", auth_header)
                .header("Content-Type", "application/json")
                .body(body_bytes)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    eprintln!(
                        "buzz-desktop: STT kind:9 post failed: HTTP {}",
                        resp.status()
                    );
                }
                Err(e) => {
                    eprintln!("buzz-desktop: STT kind:9 post failed: {e}");
                }
            }
        }
    });
}
