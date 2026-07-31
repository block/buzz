//! The channel-timeline kind set, shared by every history-fetch path.
//!
//! History is fetched by the Rust backend, not from the TypeScript kind
//! constants, so this list is what decides which kinds survive a cold restart.
//! It previously existed as two identical literals — one in `messages.rs`, one
//! in `channel_window.rs` — which could drift apart silently: live events would
//! still render while history quietly dropped a kind. One definition, two
//! consumers, no drift.
//!
//! Keep in sync with `CHANNEL_TIMELINE_CONTENT_KINDS` in
//! `desktop/src/shared/constants/kinds.ts`.

use buzz_core_pkg::kind::{KIND_HUDDLE_STARTED, KIND_SURFACE};

/// Visible content kinds the main timeline renders as their own rows.
pub(crate) const TIMELINE_KINDS: [u32; 12] = [
    9,     // stream message
    40002, // stream message v2
    40008, // diff message
    KIND_SURFACE,
    40099, // system message
    43001, // job request
    43002, // job accepted
    43003, // job progress
    43004, // job result
    43005, // job cancel
    43006, // job error
    KIND_HUDDLE_STARTED,
];

#[cfg(test)]
mod tests {
    use super::TIMELINE_KINDS;

    #[test]
    fn timeline_kinds_include_surfaces() {
        assert!(
            TIMELINE_KINDS.contains(&buzz_core_pkg::kind::KIND_SURFACE),
            "surfaces must load from history, not only live subscriptions"
        );
    }

    #[test]
    fn timeline_kinds_have_no_duplicates() {
        let mut sorted = TIMELINE_KINDS;
        sorted.sort_unstable();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            TIMELINE_KINDS.len(),
            "duplicate kind in TIMELINE_KINDS"
        );
    }
}
