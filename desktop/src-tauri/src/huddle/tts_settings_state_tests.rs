//! Additional state-transition tests split from tts_settings.rs.

use super::*;

const EVE_VOICE_KEY: &str = "pocket:eve";

#[test]
fn disabling_cancels_runtime_before_persistence_can_fail() {
    let mut huddle = super::super::HuddleState {
        tts_enabled: true,
        ..super::super::HuddleState::default()
    };
    assert!(!huddle.tts_cancel.load(std::sync::atomic::Ordering::Acquire));
    assert!(cancel_huddle_speech(&mut huddle).is_none());
    assert!(!huddle.tts_enabled);
    assert!(huddle.tts_cancel.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn pocket_voice_update_preserves_the_latest_toggle_and_other_backends() {
    let current = TtsSettings {
        agent_text_to_speech: false,
        voice_preferences: vec!["siri:aaron".to_string(), MARY_VOICE_KEY.to_string()],
        ..TtsSettings::default()
    };
    let updated =
        settings_with_pocket_voice_from_registry(current, EVE_VOICE_KEY, &bundled_voice_registry())
            .expect("available voice");
    assert!(!updated.agent_text_to_speech);
    assert_eq!(updated.voice_preferences, vec!["siri:aaron", EVE_VOICE_KEY]);
}

#[test]
fn failed_off_persistence_cannot_be_undone_by_a_later_voice_update() {
    let state = crate::app_state::build_app_state();
    commit_effective_off(&state).expect("commit effective OFF state");

    // This models the next command after the OFF save fails: it must merge
    // from effective memory state, not the stale last-persisted ON value.
    let current = state.huddle_audio.tts.lock().expect("settings").clone();
    let voice_update =
        settings_with_pocket_voice_from_registry(current, EVE_VOICE_KEY, &bundled_voice_registry())
            .expect("available voice");
    assert!(!voice_update.agent_text_to_speech);
}

#[test]
fn failed_disabled_voice_save_does_not_change_the_remembered_voice() {
    let state = crate::app_state::build_app_state();
    state
        .huddle_audio
        .tts
        .lock()
        .expect("settings")
        .agent_text_to_speech = false;
    let current = state.huddle_audio.tts.lock().expect("settings").clone();
    let unsaved =
        settings_with_pocket_voice_from_registry(current, EVE_VOICE_KEY, &bundled_voice_registry())
            .expect("available voice");

    // This is the only pre-persistence mutation for an OFF candidate.
    commit_effective_off(&state).expect("commit effective OFF state");
    let remembered = state.huddle_audio.tts.lock().expect("settings").clone();
    assert_eq!(remembered.voice_preferences, vec![MARY_VOICE_KEY]);
    assert_eq!(unsaved.voice_preferences, vec![EVE_VOICE_KEY]);
}
