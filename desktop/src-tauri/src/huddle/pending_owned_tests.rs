//! Tests for the huddle channel pending-owned overlay lifecycle
//! (`start_huddle` mark → `clear_pending_owned_huddle_channel`).

use super::clear_pending_owned_huddle_channel;

const CREATOR_PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_PK: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CHANNEL_ID: &str = "11111111-2222-3333-4444-555555555555";

/// `start_huddle` marks the ephemeral channel pending-owned under the
/// identity that signed the create, so the huddle window's member-only
/// channel poll resolves it before relay kind:39002 membership propagates;
/// the end helper removes that entry.
#[test]
fn end_clears_the_creator_mark() {
    let state = crate::app_state::build_app_state();
    state.mark_pending_owned_channel(CREATOR_PK, CHANNEL_ID);
    assert!(state.is_pending_owned_channel(CREATOR_PK, CHANNEL_ID));

    clear_pending_owned_huddle_channel(&state, CHANNEL_ID);
    assert!(!state.is_pending_owned_channel(CREATOR_PK, CHANNEL_ID));
}

/// The clear must remove the *creator's* mark even when the current signing
/// identity is no longer the creator (in-process identity swap while the
/// huddle ran) or is unavailable entirely (recovery mode). The archived
/// channel's mark must not leak; other channels' entries must survive.
#[test]
fn clear_removes_the_creator_mark_after_identity_swap_or_recovery() {
    let state = crate::app_state::build_app_state();
    state.mark_pending_owned_channel(CREATOR_PK, CHANNEL_ID);
    state.mark_pending_owned_channel(OTHER_PK, "other-channel");

    // Recovery mode: no signable identity at clear time.
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    clear_pending_owned_huddle_channel(&state, CHANNEL_ID);

    assert!(!state.is_pending_owned_channel(CREATOR_PK, CHANNEL_ID));
    assert!(state.is_pending_owned_channel(OTHER_PK, "other-channel"));
}
