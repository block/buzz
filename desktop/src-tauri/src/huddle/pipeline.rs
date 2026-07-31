//! STT/TTS pipeline lifecycle management.
//!
//! Handles starting, hot-starting, and spawning transcription tasks for
//! the voice pipelines. Extracted from mod.rs to keep the command layer thin.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use nostr::JsonUtil;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::events;

use super::models;
use super::relay_api::{self, fetch_channel_members, parse_channel_uuid};
use super::state::{HuddlePhase, HuddleState, VoiceInputMode};
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
    let (
        tts_active,
        tts_synthesizing,
        tts_cancel,
        agent_pubkeys_arc,
        session_gen,
        ptt_active_for_stt,
        old_stt,
    ) = {
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
            Arc::clone(&hs.tts_synthesizing),
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
        stt::SttPipeline::new(
            model_dir,
            tts_active,
            tts_synthesizing,
            tts_cancel,
            ptt_active_for_stt,
        )
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
    let (tts_active, tts_synthesizing, tts_cancel, tts_starting) = {
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
        (
            Arc::clone(&hs.tts_active),
            Arc::clone(&hs.tts_synthesizing),
            Arc::clone(&hs.tts_cancel),
            Arc::clone(&hs.tts_starting),
        )
    };
    let _starting_guard = TtsStartingGuard(tts_starting);

    // Construct outside the lock — this spawns the TTS worker thread and
    // loads ONNX sessions (~200ms). If this fails, clear the sentinel.
    let output_device = state
        .huddle_audio
        .output_device
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let initial_voice = state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))
        .map(|settings| {
            super::tts_settings::pocket_voice_name(&settings.voice_preferences).to_string()
        })?;
    let constructed_voice = initial_voice.clone();
    let constructed = tokio::task::spawn_blocking(move || {
        tts::TtsPipeline::new_with_voice(
            model_dir,
            tts_active,
            tts_synthesizing,
            tts_cancel,
            &initial_voice,
            output_device,
        )
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

    finalize_tts_pipeline_start(state, move |voice, huddle| {
        if should_reselect_constructed_voice(&constructed_voice, voice) {
            pipeline.select_voice_before_publish(voice);
        }
        huddle.tts_pipeline = Some(pipeline);
    })
}

/// Wait for a concurrent TTS constructor to publish or fail.
///
/// `maybe_start_tts_pipeline` deliberately lets only one caller construct the
/// engine. A live message that loses that race must wait for the owner instead
/// of observing the temporary empty slot and being dropped.
pub(crate) async fn await_inflight_tts_start(state: &AppState) -> Result<(), String> {
    let starting = {
        let huddle = state.huddle()?;
        Arc::clone(&huddle.tts_starting)
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        while starting.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "TTS pipeline startup did not finish before timeout".to_string())?;
    // The owner clears the sentinel while holding the huddle lock, before it
    // publishes. Reacquiring that lock ensures publication is visible before
    // the losing caller looks up the sender.
    drop(state.huddle()?);
    Ok(())
}

struct TtsStartingGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for TtsStartingGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Publish a constructed TTS pipeline against the latest settings.
///
/// Construction happens outside locks and can overlap a voice change or OFF
/// transition. Holding the huddle lock while re-reading settings gives either
/// transition a safe ordering: it updates the installed pipeline afterward,
/// or this finalizer observes the new setting before publishing.
fn finalize_tts_pipeline_start(
    state: &AppState,
    publish: impl FnOnce(&str, &mut HuddleState),
) -> Result<bool, String> {
    let mut huddle = state.huddle()?;
    huddle.tts_starting.store(false, Ordering::Release);
    if !huddle.tts_enabled
        || !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active)
        || huddle.tts_pipeline.is_some()
    {
        return Ok(false);
    }
    let voice = state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))
        .map(|settings| {
            super::tts_settings::pocket_voice_name(&settings.voice_preferences).to_string()
        })?;
    publish(&voice, &mut huddle);
    Ok(true)
}

fn should_reselect_constructed_voice(constructed_voice: &str, latest_voice: &str) -> bool {
    constructed_voice != latest_voice
}

/// Sign an STT transcript event and produce the guarded POST body.
///
/// Factored out of the transcription loop so egress boundary 5 (huddle STT)
/// has a directly testable seam: the NIP-49 egress guard runs here, before
/// any bytes can reach the network.
pub(crate) fn sign_and_guard_stt_body(
    builder: nostr::EventBuilder,
    keys: &nostr::Keys,
) -> Result<Vec<u8>, String> {
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("sign event: {e}"))?;
    let body_bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "huddle STT publish")?;
    Ok(body_bytes)
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
            // Wait before signing: the relay enforces NIP-98 freshness (±60s)
            // and the gate may hold for up to MAX_HINT_SECONDS (300s). Sign
            // the kind event and build NIP-98 auth after the wait so both
            // timestamps are fresh — single clean order: wait → sign → auth → send.
            crate::relay_admission::wait_for_rate_limit().await;
            let body_bytes = match sign_and_guard_stt_body(builder, &keys) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("buzz-desktop: STT publish: {e}");
                    continue;
                }
            };
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

            let response = {
                http_client
                    .post(&url)
                    .header("Authorization", auth_header)
                    .header("Content-Type", "application/json")
                    .body(body_bytes)
                    .send()
                    .await
            };

            match response {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    // Route through relay_error_message so a 429 arms the
                    // admission gate for subsequent relay sends.
                    let msg = crate::relay::relay_error_message(resp).await;
                    eprintln!("buzz-desktop: STT kind:9 post failed: {msg}");
                }
                Err(e) => {
                    eprintln!("buzz-desktop: STT kind:9 post failed: {e}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tts_start_race_tests {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier, Mutex,
    };
    use std::time::Duration;

    use crate::app_state::build_app_state;

    use super::{
        await_inflight_tts_start, finalize_tts_pipeline_start, should_reselect_constructed_voice,
        HuddlePhase,
    };

    #[tokio::test]
    async fn a_losing_starter_observes_publication_before_resuming() {
        let state = Arc::new(build_app_state());
        {
            let mut huddle = state.huddle().expect("huddle state");
            huddle.phase = HuddlePhase::Active;
            huddle.tts_enabled = true;
            huddle.tts_starting.store(true, Ordering::Release);
        }
        let published = Arc::new(AtomicBool::new(false));
        let owner_state = Arc::clone(&state);
        let owner_published = Arc::clone(&published);
        let owner = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            finalize_tts_pipeline_start(&owner_state, |_, _| {
                owner_published.store(true, Ordering::Release);
            })
        });

        await_inflight_tts_start(&state)
            .await
            .expect("wait for pipeline owner");
        assert!(published.load(Ordering::Acquire));
        assert!(owner.join().expect("pipeline owner").expect("finalize"));
    }

    #[test]
    fn constructor_fallback_survives_unchanged_preference_at_publication() {
        let selected_voice = Mutex::new(super::super::pocket::DEFAULT_VOICE.to_string());
        let constructed_voice = "eve";
        let latest_voice = "eve";

        if should_reselect_constructed_voice(constructed_voice, latest_voice) {
            *selected_voice.lock().expect("selected voice") = latest_voice.to_string();
        }

        assert_eq!(
            selected_voice.lock().expect("selected voice").as_str(),
            super::super::pocket::DEFAULT_VOICE
        );
    }

    #[test]
    fn construction_reconciles_a_voice_selected_while_starting() {
        let state = Arc::new(build_app_state());
        {
            let mut huddle = state.huddle().expect("huddle state");
            huddle.phase = HuddlePhase::Active;
            huddle.tts_enabled = true;
            huddle.tts_starting.store(true, Ordering::Release);
        }

        let constructed = Arc::new(Barrier::new(2));
        let publish = Arc::new(Barrier::new(2));
        let selected_voice = Arc::new(Mutex::new(None));
        let worker_state = Arc::clone(&state);
        let worker_constructed = Arc::clone(&constructed);
        let worker_publish = Arc::clone(&publish);
        let worker_voice = Arc::clone(&selected_voice);
        let worker = std::thread::spawn(move || {
            worker_constructed.wait();
            worker_publish.wait();
            finalize_tts_pipeline_start(&worker_state, |voice, _| {
                *worker_voice.lock().expect("selected voice") = Some(voice.to_string());
            })
        });

        constructed.wait();
        assert!(state
            .huddle()
            .expect("huddle state")
            .tts_starting
            .load(Ordering::Acquire));
        state
            .huddle_audio
            .tts
            .lock()
            .expect("text-to-speech settings")
            .voice_preferences = vec!["pocket:eve".to_string()];
        publish.wait();

        assert!(worker.join().expect("starter thread").expect("finalize"));
        assert_eq!(
            *selected_voice.lock().expect("selected voice"),
            Some("eve".to_string())
        );
    }

    #[test]
    fn construction_is_discarded_when_disabled_while_starting() {
        let state = Arc::new(build_app_state());
        {
            let mut huddle = state.huddle().expect("huddle state");
            huddle.phase = HuddlePhase::Active;
            huddle.tts_enabled = true;
            huddle.tts_starting.store(true, Ordering::Release);
        }

        let constructed = Arc::new(Barrier::new(2));
        let publish = Arc::new(Barrier::new(2));
        let did_publish = Arc::new(Mutex::new(false));
        let worker_state = Arc::clone(&state);
        let worker_constructed = Arc::clone(&constructed);
        let worker_publish = Arc::clone(&publish);
        let worker_did_publish = Arc::clone(&did_publish);
        let worker = std::thread::spawn(move || {
            worker_constructed.wait();
            worker_publish.wait();
            finalize_tts_pipeline_start(&worker_state, |_, _| {
                *worker_did_publish.lock().expect("publish flag") = true;
            })
        });

        constructed.wait();
        {
            let mut huddle = state.huddle().expect("huddle state");
            huddle.tts_enabled = false;
        }
        publish.wait();

        assert!(!worker.join().expect("starter thread").expect("finalize"));
        assert!(!*did_publish.lock().expect("publish flag"));
        assert!(!state
            .huddle()
            .expect("huddle state")
            .tts_starting
            .load(Ordering::Acquire));
    }
}
