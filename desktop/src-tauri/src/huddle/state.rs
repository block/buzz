//! Huddle state types and serialization.
//!
//! Contains `HuddleState` (the god-object behind `AppState.huddle_state`),
//! phase enum, voice input mode, and response types.

use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc, Mutex,
};

use super::{stt, tts};

/// Voice input mode: push-to-talk (PTT) or voice-activity detection (VAD).
///
/// PTT: mic is gated by a global shortcut (Ctrl+Space). Pressing the key sets
/// `ptt_active` and immediately cancels any playing TTS. Releasing the key
/// (after a 200 ms delay) stops mic capture and flushes the utterance.
///
/// VAD (default): the earshot VAD runs continuously and speech is accumulated
/// whenever the probability exceeds the threshold. Barge-in is enabled in this
/// mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputMode {
    PushToTalk,
    #[default]
    VoiceActivity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HuddlePhase {
    Idle,
    Creating,
    Connecting,
    Connected, // Backend ready, waiting for frontend media confirmation.
    Active,
    Leaving,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HuddleState {
    pub phase: HuddlePhase,
    pub parent_channel_id: Option<String>,
    pub ephemeral_channel_id: Option<String>,
    /// Cancellation token for the audio relay WS task.
    #[serde(skip)]
    pub audio_ws_cancel: Option<tokio_util::sync::CancellationToken>,
    /// Sends PCM batches from push_audio_pcm to the audio relay encode thread.
    #[serde(skip)]
    pub audio_relay_pcm_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    /// Participant pubkey hex strings (all members, including humans).
    pub participants: Vec<String>,
    /// Agent pubkeys only — used as p-tags on transcribed messages.
    ///
    /// Stored as `Arc<Mutex<Vec<String>>>` so the transcription task can clone
    /// the `Arc` and read the current list at post time without holding the
    /// outer `HuddleState` lock across an await point.
    ///
    /// Populated from `member_pubkeys` in `start_huddle` (the UI sends agent
    /// pubkeys specifically). Joiners don't add agents — they were already
    /// added by the creator. Serialized as a plain `Vec<String>` for the
    /// frontend via the custom `Serialize`/`Deserialize` impls below.
    #[serde(
        serialize_with = "serialize_agent_pubkeys",
        deserialize_with = "deserialize_agent_pubkeys"
    )]
    pub agent_pubkeys: Arc<Mutex<Vec<String>>>,
    /// Active STT pipeline — not serialized, not cloned.
    #[serde(skip)]
    pub stt_pipeline: Option<Arc<stt::SttPipeline>>,
    /// Active TTS pipeline — not serialized, not cloned.
    #[serde(skip)]
    pub tts_pipeline: Option<Arc<tts::TtsPipeline>>,
    /// Whether this client created the huddle (vs. joined it).
    /// Used to enforce that only the creator can end/archive the huddle.
    pub is_creator: bool,
    /// Whether TTS output is enabled (user-toggled).
    pub tts_enabled: bool,
    /// Shared flag: true while TTS is playing audio.
    /// Shared with the STT pipeline for barge-in / echo gating.
    #[serde(skip)]
    pub tts_active: Arc<AtomicBool>,
    /// Shared barge-in cancel flag. Set by STT when it detects speech during TTS.
    /// Read by TTS to stop playback. Lives in HuddleState so it survives pipeline
    /// restarts — both STT and TTS reference the same flag for the entire huddle.
    #[serde(skip)]
    pub tts_cancel: Arc<AtomicBool>,
    /// Sentinel: true while a TTS pipeline is being constructed (outside the lock).
    /// Prevents TOCTOU races where two concurrent callers both pass the `is_some()`
    /// check and both spawn TTS worker threads — the loser's thread would leak.
    #[serde(skip)]
    pub tts_starting: Arc<AtomicBool>,
    /// Sentinel: true while an STT pipeline is being constructed.
    /// Mirrors `tts_starting` — prevents TOCTOU races where two concurrent
    /// callers both pass the `is_some()` check and both spawn STT workers.
    #[serde(skip)]
    pub stt_starting: Arc<AtomicBool>,
    /// Timestamp of the last agent pubkey refresh from the relay.
    /// Used to throttle the refresh in check_pipeline_hotstart to every 15 s.
    #[serde(skip)]
    pub last_agent_refresh: Option<std::time::Instant>,
    /// Session generation — incremented on every teardown. The transcription
    /// task captures this at spawn time and checks before each POST. If the
    /// generation has changed, the task silently drops the transcript.
    #[serde(skip)]
    pub session_generation: Arc<AtomicU64>,
    /// Voice input mode: push-to-talk or voice-activity detection.
    pub voice_input_mode: VoiceInputMode,
    /// True while the PTT key is held (+ 200 ms release delay).
    /// Shared with the STT pipeline for mic gating.
    #[serde(skip)]
    pub ptt_active: Arc<AtomicBool>,
}

fn serialize_agent_pubkeys<S>(v: &Arc<Mutex<Vec<String>>>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let guard = v.lock().unwrap_or_else(|e| e.into_inner());
    let mut seq = s.serialize_seq(Some(guard.len()))?;
    for item in guard.iter() {
        seq.serialize_element(item)?;
    }
    seq.end()
}

fn deserialize_agent_pubkeys<'de, D>(d: D) -> Result<Arc<Mutex<Vec<String>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Vec<String> = serde::Deserialize::deserialize(d)?;
    Ok(Arc::new(Mutex::new(v)))
}

impl Clone for HuddleState {
    fn clone(&self) -> Self {
        let agent_pubkeys_snapshot = self
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Self {
            phase: self.phase.clone(),
            parent_channel_id: self.parent_channel_id.clone(),
            ephemeral_channel_id: self.ephemeral_channel_id.clone(),
            audio_ws_cancel: None,    // Never clone handles.
            audio_relay_pcm_tx: None, // Never clone handles.
            participants: self.participants.clone(),
            agent_pubkeys: Arc::new(Mutex::new(agent_pubkeys_snapshot)),
            stt_pipeline: None, // Never clone the pipeline handle.
            tts_pipeline: None, // Never clone the pipeline handle.
            is_creator: self.is_creator,
            tts_enabled: self.tts_enabled,
            tts_active: Arc::clone(&self.tts_active),
            tts_cancel: Arc::clone(&self.tts_cancel),
            tts_starting: Arc::clone(&self.tts_starting),
            stt_starting: Arc::clone(&self.stt_starting),
            last_agent_refresh: self.last_agent_refresh,
            session_generation: Arc::clone(&self.session_generation),
            voice_input_mode: self.voice_input_mode.clone(),
            ptt_active: Arc::clone(&self.ptt_active),
        }
    }
}

impl Default for HuddleState {
    fn default() -> Self {
        Self {
            phase: HuddlePhase::Idle,
            parent_channel_id: None,
            ephemeral_channel_id: None,
            audio_ws_cancel: None,
            audio_relay_pcm_tx: None,
            participants: Vec::new(),
            agent_pubkeys: Arc::new(Mutex::new(Vec::new())),
            stt_pipeline: None,
            tts_pipeline: None,
            is_creator: false,
            tts_enabled: true,
            tts_active: Arc::new(AtomicBool::new(false)),
            tts_cancel: Arc::new(AtomicBool::new(false)),
            tts_starting: Arc::new(AtomicBool::new(false)),
            stt_starting: Arc::new(AtomicBool::new(false)),
            last_agent_refresh: None,
            session_generation: Arc::new(AtomicU64::new(0)),
            voice_input_mode: VoiceInputMode::default(),
            ptt_active: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl HuddleState {
    /// Reset to default state while preserving the session generation counter.
    /// Used by start_huddle rollback, join_huddle rollback, and teardown_huddle
    /// to invalidate in-flight transcription tasks without losing the generation.
    pub(crate) fn reset_preserving_generation(&mut self) {
        let gen = Arc::clone(&self.session_generation);
        *self = Self::default();
        self.session_generation = gen;
    }
}

// ── Event emission ────────────────────────────────────────────────────────────

/// Emit the current huddle state to the frontend via a Tauri event.
///
/// The frontend listens for `"huddle-state-changed"` and updates its UI
/// immediately, replacing the previous 2-second polling loop.
///
/// **Call sites** — called from `AppState::emit_huddle_state_changed()` in
/// `app_state.rs` after every state transition the frontend needs to observe
/// (phase changes, participant updates, tts_enabled toggle).
///
/// Best-effort — silently ignores errors (e.g., no listeners attached yet).
pub fn emit_huddle_state(app: &tauri::AppHandle, state: &HuddleState) {
    use tauri::Emitter;
    let _ = app.emit("huddle-state-changed", state);
}

// ── Response types ────────────────────────────────────────────────────────────

/// Returned by start_huddle and join_huddle.
#[derive(Debug, Serialize, Deserialize)]
pub struct HuddleJoinInfo {
    pub ephemeral_channel_id: String,
}
