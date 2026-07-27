//! Tests for message_tts.rs — the app-scoped chat read-aloud lifecycle.
//!
//! Covers the pure helpers (text preparation, truncation disclosure) and the
//! concurrency contracts (generation-based supersede, stop invalidation,
//! stale-watcher silence). The pipeline itself is exercised by huddle tests;
//! here we prove the *coordination* around it is deterministic.

use super::*;

// ── prepare_speech_text ───────────────────────────────────────────────────

#[test]
fn short_message_passes_through_untruncated() {
    let (text, truncated) = prepare_speech_text("Hello there, friend.").unwrap();
    assert_eq!(text, "Hello there, friend.");
    assert!(!truncated);
}

#[test]
fn long_message_is_truncated_with_spoken_disclosure() {
    let raw = "word ".repeat(1000); // 5000 chars
    let (text, truncated) = prepare_speech_text(&raw).unwrap();
    assert!(truncated);
    // The spoken suffix survives preprocessing so the listener hears the cut.
    assert!(
        text.ends_with("message truncated."),
        "expected spoken disclosure, got tail: {:?}",
        &text[text.len().saturating_sub(40)..]
    );
    // Cap applies to the source text; result stays in the same ballpark.
    assert!(text.chars().count() <= MAX_MESSAGE_TTS_CHARS + TRUNCATION_SUFFIX.chars().count());
}

#[test]
fn exactly_at_limit_is_not_truncated() {
    let raw = "a".repeat(MAX_MESSAGE_TTS_CHARS);
    let (_, truncated) = prepare_speech_text(&raw).unwrap();
    assert!(!truncated);
}

#[test]
fn one_past_limit_is_truncated() {
    let raw = "a".repeat(MAX_MESSAGE_TTS_CHARS + 1);
    let (_, truncated) = prepare_speech_text(&raw).unwrap();
    assert!(truncated);
}

#[test]
fn truncation_counts_chars_not_bytes() {
    // é is 2 bytes in UTF-8; a byte-indexed slice at the limit would panic
    // or split a code point. chars().take() must handle it.
    let raw = "é".repeat(MAX_MESSAGE_TTS_CHARS + 10);
    let (text, truncated) = prepare_speech_text(&raw).unwrap();
    assert!(truncated);
    assert!(text.starts_with('é'));
}

#[test]
fn unspeakable_message_is_an_error_not_silence() {
    // URL-only content preprocesses to a placeholder, but emoji/whitespace-only
    // reduces to nothing — must surface as Err so the UI shows "failed"
    // instead of a playback that never starts.
    let err = prepare_speech_text("   \n\t  ").unwrap_err();
    assert!(err.contains("no speakable text"), "got: {err}");
}

#[test]
fn code_only_message_yields_omission_placeholder_not_error() {
    // A fenced block preprocesses to "code block omitted" — still speakable.
    let (text, _) = prepare_speech_text("```rust\nfn main() {}\n```").unwrap();
    assert!(text.contains("code block omitted"), "got: {text}");
}

#[test]
fn markdown_is_preprocessed_for_speech() {
    let (text, _) = prepare_speech_text("**Done.** See https://example.com").unwrap();
    assert!(!text.contains("**"), "got: {text}");
    assert!(!text.contains("example.com"), "got: {text}");
    assert!(text.contains("link omitted"), "got: {text}");
}

// ── Generation / supersede contract ───────────────────────────────────────

#[test]
fn watcher_owns_state_until_generation_moves() {
    let generation = AtomicU64::new(7);
    assert!(watcher_owns_state(&generation, 7));
    generation.fetch_add(1, Ordering::AcqRel);
    assert!(!watcher_owns_state(&generation, 7));
    assert!(watcher_owns_state(&generation, 8));
}

#[test]
fn stop_invalidates_watchers_and_clears_current_message() {
    let state = crate::app_state::build_app_state();
    // Simulate an in-flight playback: watcher spawned at the current gen.
    let spawned_gen = state.message_tts.generation.load(Ordering::Acquire);
    state.message_tts.lock().current_message_id = Some("msg-1".into());
    state.message_tts.active.store(true, Ordering::Release);

    stop_message_playback(&state);

    // Watcher for the old playback must no longer own UI state — its
    // falling-active "completed" emit is suppressed.
    assert!(!watcher_owns_state(
        &state.message_tts.generation,
        spawned_gen
    ));
    // The worker is told to drain.
    assert!(state.message_tts.cancel.load(Ordering::Acquire));
    // Message bookkeeping is cleared so a later stop is a no-op.
    assert!(state.message_tts.lock().current_message_id.is_none());
}

#[test]
fn stop_with_nothing_playing_is_a_quiet_no_op() {
    let state = crate::app_state::build_app_state();
    let gen_before = state.message_tts.generation.load(Ordering::Acquire);
    stop_message_playback(&state);
    // Generation still bumps (harmless — no watcher exists), no panic, and
    // there was no message to clear.
    assert!(state.message_tts.lock().current_message_id.is_none());
    assert!(state.message_tts.generation.load(Ordering::Acquire) > gen_before);
}

#[test]
fn newer_play_supersedes_older_watcher_deterministically() {
    // Models the exact interleaving speak_chat_message() produces:
    // play A (gen 1) → play B (gen 2). A's watcher observes the moved
    // generation and exits without emitting, regardless of when A's audio
    // actually stops — so B's state can never be overwritten by A.
    let state = crate::app_state::build_app_state();

    // Play A.
    state.message_tts.lock().current_message_id = Some("msg-a".into());
    let gen_a = state.message_tts.generation.fetch_add(1, Ordering::AcqRel) + 1;
    assert!(watcher_owns_state(&state.message_tts.generation, gen_a));

    // Play B supersedes: replaces the current message, bumps gen, cancels.
    let old = state
        .message_tts
        .lock()
        .current_message_id
        .replace("msg-b".into());
    assert_eq!(old.as_deref(), Some("msg-a"));
    let gen_b = state.message_tts.generation.fetch_add(1, Ordering::AcqRel) + 1;
    state.message_tts.cancel.store(true, Ordering::Release);

    assert!(!watcher_owns_state(&state.message_tts.generation, gen_a));
    assert!(watcher_owns_state(&state.message_tts.generation, gen_b));
}

#[test]
fn rapid_stop_play_stop_leaves_consistent_state() {
    let state = crate::app_state::build_app_state();
    for i in 0..50 {
        let id = format!("msg-{i}");
        state.message_tts.lock().current_message_id = Some(id.clone());
        state.message_tts.generation.fetch_add(1, Ordering::AcqRel);
        stop_message_playback(&state);
        assert!(state.message_tts.lock().current_message_id.is_none());
    }
    // Cancel flag is set (worker would consume it); no watcher owns any
    // historical generation.
    assert!(state.message_tts.cancel.load(Ordering::Acquire));
}

#[test]
fn concurrent_stops_from_many_threads_never_deadlock_or_double_clear() {
    use std::thread;
    let state = std::sync::Arc::new(crate::app_state::build_app_state());
    state.message_tts.lock().current_message_id = Some("msg-x".into());

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let state = std::sync::Arc::clone(&state);
            thread::spawn(move || stop_message_playback(&state))
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert!(state.message_tts.lock().current_message_id.is_none());
}

// ── Event payload shape (frontend contract) ───────────────────────────────

#[test]
fn playback_event_serializes_with_stable_field_names() {
    let ev = PlaybackEvent {
        message_id: "abc".into(),
        status: "playing",
        truncated: true,
        error: None,
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["message_id"], "abc");
    assert_eq!(json["status"], "playing");
    assert_eq!(json["truncated"], true);
    assert!(json["error"].is_null());
}

#[test]
fn failed_event_carries_the_error_text() {
    let ev = PlaybackEvent {
        message_id: "abc".into(),
        status: "failed",
        truncated: false,
        error: Some("voice model is not downloaded yet".into()),
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["status"], "failed");
    assert_eq!(json["error"], "voice model is not downloaded yet");
}
