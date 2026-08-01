//! The kind sets the desktop backend queries the relay with.
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

use buzz_core_pkg::kind::{
    kinds_with, KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_GIT_ISSUE, KIND_GIT_PR_UPDATE,
    KIND_GIT_PULL_REQUEST, KIND_GIT_STATUS_CLOSED, KIND_GIT_STATUS_DRAFT, KIND_GIT_STATUS_MERGED,
    KIND_GIT_STATUS_OPEN, KIND_HUDDLE_STARTED, KIND_SURFACE, KIND_TEXT_NOTE,
};

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

/// Kinds a `#p` mention can land on — the Home mention feed.
///
/// Conversational kinds (so surfaces notify like any other message) plus the
/// non-timeline kinds that also carry mentions. Keep in sync with
/// `FEED_MENTION_KINDS` in `buzz-db`, which backs the relay-side query.
pub(crate) fn mention_kinds() -> Vec<u32> {
    kinds_with(&[
        KIND_TEXT_NOTE,
        KIND_FORUM_POST,
        KIND_FORUM_COMMENT,
        KIND_GIT_PULL_REQUEST,
        KIND_GIT_PR_UPDATE,
        KIND_GIT_ISSUE,
        KIND_GIT_STATUS_OPEN,
        KIND_GIT_STATUS_MERGED,
        KIND_GIT_STATUS_CLOSED,
        KIND_GIT_STATUS_DRAFT,
    ])
}

/// Kinds that can be a thread parent — anything repliable.
pub(crate) fn thread_parent_kinds() -> Vec<u32> {
    kinds_with(&[KIND_FORUM_POST, KIND_FORUM_COMMENT, KIND_HUDDLE_STARTED])
}

/// Kinds that can be a forum thread root.
///
/// Forum posts plus surfaces — NIP-SC allows a surface as a thread root, and
/// without it a card posted to a forum channel is invisible in the index even
/// though the thread view renders it.
pub(crate) fn forum_root_kinds() -> Vec<u32> {
    vec![KIND_FORUM_POST, KIND_SURFACE]
}

/// Kinds a forum thread is built from (root lookup and reply fan-out).
pub(crate) fn forum_thread_kinds() -> Vec<u32> {
    kinds_with(&[KIND_FORUM_POST, KIND_FORUM_COMMENT])
}

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
    fn every_query_set_carries_surfaces() {
        // A surface is conversational content: mention feeds, thread-parent
        // resolution and forum threads must all accept it, or cards silently
        // vanish from those paths.
        for (name, kinds) in [
            ("mention", super::mention_kinds()),
            ("thread_parent", super::thread_parent_kinds()),
            ("forum_thread", super::forum_thread_kinds()),
            ("forum_root", super::forum_root_kinds()),
        ] {
            assert!(
                kinds.contains(&buzz_core_pkg::kind::KIND_SURFACE),
                "{name} query set must include surfaces"
            );
        }
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
