//! The code half of a kind's policy: resolving a `RequiredScope::Dynamic`
//! kind's actual scope from the event.
//!
//! This seam is intentionally minimal in this PR — only `required_scope` is
//! defined, since the only kind that needs `RequiredScope::Dynamic` today
//! (NIP-29 `kind:9002` edit-metadata) needs nothing else. Per-event
//! authorization and payload-validation hooks (the relay's inline
//! board-branch-style AND-gates and its `validate_*` ladder) are a natural
//! follow-up to fold into this trait; adding methods with defaults later is
//! non-breaking for every existing implementor.

use buzz_auth::Scope;
use nostr::Event;

/// Per-kind scope-resolution hook for `RequiredScope::Dynamic` kinds.
pub trait KindExtension: Send + Sync {
    /// Resolve the required scope for a `RequiredScope::Dynamic` kind.
    ///
    /// The default fails closed: it demands an unrecognized scope no token
    /// can hold, so a kind marked `Dynamic` that forgets to override this is
    /// rejected rather than silently granted.
    fn required_scope(&self, _event: &Event) -> Scope {
        Scope::Unknown("dynamic-scope-not-implemented".to_string())
    }
}
