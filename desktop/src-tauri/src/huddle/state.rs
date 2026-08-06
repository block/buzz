//! Huddle state types and serialization.
//!
//! Contains `HuddleState` (the god-object behind `AppState.huddle_state`),
//! phase enum, voice input mode, and response types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

use super::agent_voice::AgentVoiceSettings;
use super::{stt, tts};

/// Voice input mode: push-to-talk (PTT) or voice-activity detection (VAD).
///
/// PTT (the default): mic is gated by a global shortcut (Ctrl+Space). Pressing the key sets
/// `ptt_active` and immediately cancels any playing TTS. Releasing the key
/// (after a 200 ms delay) stops mic capture and flushes the utterance.
///
/// VAD (default): the earshot VAD runs continuously and speech is accumulated
/// whenever the probability exceeds the threshold. While local TTS is playing,
/// mic frames are discarded because VAD has no echo reference with which to
/// distinguish the app's own playback from a human interruption.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInputMode {
    #[default]
    PushToTalk,
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
    /// Root event for the huddle's visible parent-channel thread. Transcript
    /// messages reply here while audio coordination stays ephemeral.
    pub huddle_thread_event_id: Option<String>,
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
    /// Local, huddle-scoped playback choices for each participating agent.
    pub agent_voice_settings: BTreeMap<String, AgentVoiceSettings>,
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
    /// Whether STT transcript posting is enabled for this huddle.
    pub transcription_enabled: bool,
    /// Whether the user has explicitly used the transcription control in this
    /// huddle. Agent presence may auto-enable transcription only while this is
    /// false, so membership refreshes never undo an explicit user choice.
    ///
    /// This is backend-only session state: keeping it in `HuddleState` makes it
    /// survive frontend remounts and audio reconnects, while huddle teardown
    /// resets it for the next session.
    #[serde(skip)]
    pub transcription_user_controlled: bool,
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
    /// Monotonic identity for a local huddle lifetime. Unlike transcript
    /// generation, this changes only when a new start/join attempt begins.
    #[serde(skip)]
    pub huddle_generation: u64,
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
    /// True while the clickable microphone control is manually unmuted.
    /// In PTT mode, either this flag or `ptt_active` opens the STT gate.
    #[serde(skip)]
    pub manual_mic_unmuted: Arc<AtomicBool>,
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
            huddle_thread_event_id: self.huddle_thread_event_id.clone(),
            audio_ws_cancel: None,    // Never clone handles.
            audio_relay_pcm_tx: None, // Never clone handles.
            participants: self.participants.clone(),
            agent_pubkeys: Arc::new(Mutex::new(agent_pubkeys_snapshot)),
            agent_voice_settings: self.agent_voice_settings.clone(),
            stt_pipeline: None, // Never clone the pipeline handle.
            tts_pipeline: None, // Never clone the pipeline handle.
            is_creator: self.is_creator,
            tts_enabled: self.tts_enabled,
            transcription_enabled: self.transcription_enabled,
            transcription_user_controlled: self.transcription_user_controlled,
            tts_active: Arc::clone(&self.tts_active),
            tts_cancel: Arc::clone(&self.tts_cancel),
            tts_starting: Arc::clone(&self.tts_starting),
            stt_starting: Arc::clone(&self.stt_starting),
            last_agent_refresh: self.last_agent_refresh,
            huddle_generation: self.huddle_generation,
            session_generation: Arc::clone(&self.session_generation),
            voice_input_mode: self.voice_input_mode.clone(),
            ptt_active: Arc::clone(&self.ptt_active),
            manual_mic_unmuted: Arc::clone(&self.manual_mic_unmuted),
        }
    }
}

impl Default for HuddleState {
    fn default() -> Self {
        Self {
            phase: HuddlePhase::Idle,
            parent_channel_id: None,
            ephemeral_channel_id: None,
            huddle_thread_event_id: None,
            audio_ws_cancel: None,
            audio_relay_pcm_tx: None,
            participants: Vec::new(),
            agent_pubkeys: Arc::new(Mutex::new(Vec::new())),
            agent_voice_settings: BTreeMap::new(),
            stt_pipeline: None,
            tts_pipeline: None,
            is_creator: false,
            tts_enabled: true,
            transcription_enabled: false,
            transcription_user_controlled: false,
            tts_active: Arc::new(AtomicBool::new(false)),
            tts_cancel: Arc::new(AtomicBool::new(false)),
            tts_starting: Arc::new(AtomicBool::new(false)),
            stt_starting: Arc::new(AtomicBool::new(false)),
            last_agent_refresh: None,
            huddle_generation: 0,
            session_generation: Arc::new(AtomicU64::new(0)),
            voice_input_mode: VoiceInputMode::default(),
            ptt_active: Arc::new(AtomicBool::new(false)),
            manual_mic_unmuted: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl HuddleState {
    /// Begin a new local huddle lifetime and return its identity.
    pub(crate) fn begin_huddle_lifetime(&mut self) -> u64 {
        self.huddle_generation = self.huddle_generation.wrapping_add(1);
        self.huddle_generation
    }

    pub(crate) fn owns_huddle_lifetime(&self, huddle_generation: u64, phase: HuddlePhase) -> bool {
        self.huddle_generation == huddle_generation && self.phase == phase
    }

    /// Whether an async result still belongs to the active huddle that
    /// initiated it. The channel id is the huddle-session identity; transcript
    /// generation changes within the same huddle must not invalidate it.
    pub(crate) fn is_current_huddle(
        &self,
        ephemeral_channel_id: &str,
        huddle_generation: u64,
    ) -> bool {
        matches!(self.phase, HuddlePhase::Connected | HuddlePhase::Active)
            && self.ephemeral_channel_id.as_deref() == Some(ephemeral_channel_id)
            && self.huddle_generation == huddle_generation
    }

    /// Whether an STT construction still belongs to the current transcript
    /// generation within the active huddle.
    pub(crate) fn is_current_transcription_generation(
        &self,
        ephemeral_channel_id: &str,
        huddle_generation: u64,
        session_generation: u64,
    ) -> bool {
        self.is_current_huddle(ephemeral_channel_id, huddle_generation)
            && self.session_generation.load(Ordering::Acquire) == session_generation
    }

    /// Invalidate in-flight transcription work and give the next constructor a
    /// fresh sentinel that stale constructors cannot clear.
    pub(crate) fn invalidate_transcription_pipeline(&mut self) {
        self.session_generation.fetch_add(1, Ordering::Release);
        self.stt_starting = Arc::new(AtomicBool::new(false));
    }

    /// Record an explicit transcription choice made through the existing user
    /// control. Later agent membership refreshes must preserve this choice.
    pub(crate) fn set_transcription_enabled_by_user(&mut self, enabled: bool) {
        self.transcription_enabled = enabled;
        self.transcription_user_controlled = true;
    }

    /// Enable transcription when an agent is present and the user has not
    /// explicitly chosen a transcription state for this huddle.
    ///
    /// Returns true only for the transition from disabled to enabled, allowing
    /// callers to start models/pipelines and emit state exactly once. Removing
    /// the last agent deliberately leaves the current state unchanged.
    pub(crate) fn maybe_auto_enable_transcription_for_agents(&mut self) -> bool {
        let has_agent = !self
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        if has_agent && !self.transcription_user_controlled && !self.transcription_enabled {
            self.transcription_enabled = true;
            return true;
        }
        false
    }

    /// Reset to default state while preserving the session generation counter.
    /// Used by start_huddle rollback, join_huddle rollback, and teardown_huddle
    /// to invalidate in-flight transcription tasks without losing the generation.
    pub(crate) fn reset_preserving_generation(&mut self) {
        let gen = Arc::clone(&self.session_generation);
        let huddle_generation = self.huddle_generation;
        let tts_enabled = self.tts_enabled;
        *self = Self::default();
        self.session_generation = gen;
        self.huddle_generation = huddle_generation;
        self.tts_enabled = tts_enabled;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::HuddleState;

    fn set_agents(state: &HuddleState, agents: &[&str]) {
        *state
            .agent_pubkeys
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            agents.iter().map(|agent| (*agent).to_owned()).collect();
    }

    #[test]
    fn first_agent_auto_enables_transcription_once() {
        let mut state = HuddleState::default();
        set_agents(&state, &["agent"]);

        assert!(state.maybe_auto_enable_transcription_for_agents());
        assert!(state.transcription_enabled);
        assert!(!state.maybe_auto_enable_transcription_for_agents());
    }

    #[test]
    fn defaults_to_push_to_talk_with_an_open_microphone() {
        let state = HuddleState::default();
        assert_eq!(state.voice_input_mode, super::VoiceInputMode::PushToTalk);
        assert!(state.manual_mic_unmuted.load(Ordering::Acquire));
    }

    #[test]
    fn explicit_user_disable_is_not_undone_by_agent_presence() {
        let mut state = HuddleState::default();
        set_agents(&state, &["agent"]);
        assert!(state.maybe_auto_enable_transcription_for_agents());

        state.set_transcription_enabled_by_user(false);

        assert!(!state.maybe_auto_enable_transcription_for_agents());
        assert!(!state.transcription_enabled);
    }

    #[test]
    fn last_agent_leaving_preserves_current_transcription_state() {
        let mut state = HuddleState::default();
        set_agents(&state, &["agent"]);
        assert!(state.maybe_auto_enable_transcription_for_agents());

        set_agents(&state, &[]);

        assert!(!state.maybe_auto_enable_transcription_for_agents());
        assert!(state.transcription_enabled);
    }

    #[test]
    fn clone_preserves_user_control_across_frontend_state_reads() {
        let mut state = HuddleState::default();
        state.set_transcription_enabled_by_user(false);

        let mut clone = state.clone();
        set_agents(&clone, &["agent"]);

        assert!(clone.transcription_user_controlled);
        assert!(!clone.maybe_auto_enable_transcription_for_agents());
    }

    #[test]
    fn stale_huddle_identity_is_rejected_after_replacement() {
        let mut state = HuddleState {
            phase: super::HuddlePhase::Active,
            ephemeral_channel_id: Some("huddle-a".to_owned()),
            ..HuddleState::default()
        };
        let huddle_generation = state.begin_huddle_lifetime();
        let generation = state.session_generation.load(Ordering::Acquire);
        assert!(state.is_current_huddle("huddle-a", huddle_generation));
        assert!(state.is_current_transcription_generation(
            "huddle-a",
            huddle_generation,
            generation
        ));

        state.session_generation.fetch_add(1, Ordering::Release);
        assert!(state.is_current_huddle("huddle-a", huddle_generation));
        assert!(!state.is_current_transcription_generation(
            "huddle-a",
            huddle_generation,
            generation
        ));

        state.ephemeral_channel_id = Some("huddle-b".to_owned());

        assert!(!state.is_current_huddle("huddle-a", huddle_generation));
    }

    #[test]
    fn same_channel_rejoin_gets_a_new_huddle_lifetime() {
        let mut state = HuddleState {
            phase: super::HuddlePhase::Active,
            ephemeral_channel_id: Some("huddle".to_owned()),
            ..HuddleState::default()
        };
        let first_generation = state.begin_huddle_lifetime();
        state.reset_preserving_generation();
        state.phase = super::HuddlePhase::Active;
        state.ephemeral_channel_id = Some("huddle".to_owned());
        let second_generation = state.begin_huddle_lifetime();

        assert_ne!(first_generation, second_generation);
        assert!(!state.is_current_huddle("huddle", first_generation));
        assert!(state.is_current_huddle("huddle", second_generation));
    }

    #[test]
    fn superseded_create_cannot_commit_or_reset_replacement_lifetime() {
        let mut state = HuddleState::default();
        let first_generation = state.begin_huddle_lifetime();
        state.phase = super::HuddlePhase::Creating;
        assert!(state.owns_huddle_lifetime(first_generation, super::HuddlePhase::Creating));

        state.reset_preserving_generation();
        let replacement_generation = state.begin_huddle_lifetime();
        state.phase = super::HuddlePhase::Creating;

        assert!(!state.owns_huddle_lifetime(first_generation, super::HuddlePhase::Creating));
        assert!(state.owns_huddle_lifetime(replacement_generation, super::HuddlePhase::Creating));
    }

    #[test]
    fn superseded_join_cannot_commit_replacement_lifetime() {
        let mut state = HuddleState::default();
        let first_generation = state.begin_huddle_lifetime();
        state.phase = super::HuddlePhase::Connecting;

        state.reset_preserving_generation();
        let replacement_generation = state.begin_huddle_lifetime();
        state.phase = super::HuddlePhase::Connecting;

        assert!(!state.owns_huddle_lifetime(first_generation, super::HuddlePhase::Connecting));
        assert!(state.owns_huddle_lifetime(replacement_generation, super::HuddlePhase::Connecting));
    }

    #[test]
    fn teardown_preserves_installation_global_tts_preference() {
        let mut state = HuddleState {
            tts_enabled: false,
            phase: super::HuddlePhase::Active,
            ..HuddleState::default()
        };
        state.reset_preserving_generation();
        assert!(!state.tts_enabled);
        assert_eq!(state.phase, super::HuddlePhase::Idle);
    }

    #[test]
    fn stale_constructor_cannot_clear_replacement_sentinel() {
        let mut state = HuddleState::default();
        let stale_sentinel = std::sync::Arc::clone(&state.stt_starting);
        stale_sentinel.store(true, Ordering::Release);

        state.invalidate_transcription_pipeline();
        state.stt_starting.store(true, Ordering::Release);
        stale_sentinel.store(false, Ordering::Release);

        assert!(state.stt_starting.load(Ordering::Acquire));
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
    // Reserve Ctrl+Space with the OS only while a huddle is live in PTT mode.
    crate::ptt_shortcut::sync_registration(app, state);
}

// ── Response types ────────────────────────────────────────────────────────────

/// Returned by start_huddle and join_huddle.
#[derive(Debug, Serialize, Deserialize)]
pub struct HuddleJoinInfo {
    pub ephemeral_channel_id: String,
}
