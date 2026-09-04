//! The declarative half of a kind's policy.
//!
//! A [`KindDescriptor`] captures everything about an event kind that today is
//! spread across the relay's `required_scope_for_kind` match and the
//! `is_global_only_kind` / `requires_h_channel_scope` boolean allowlists.
//! Expressing it as one struct literal per kind makes a kind's policy legible
//! in one place and makes one class of misconfiguration structurally
//! impossible (see [`Scoping`]).

use buzz_auth::Scope;

use crate::extension::KindExtension;

/// The base scope a transport token must hold to write a kind.
#[derive(Clone)]
pub enum RequiredScope {
    /// A fixed scope, the common case (e.g. `Scope::MessagesWrite`).
    Static(Scope),
    /// The scope depends on the event's content/tags; the kind's
    /// [`KindExtension::required_scope`] is consulted. Only kinds whose scope
    /// genuinely varies per event (today: the NIP-29 edit-metadata archive
    /// toggle) need this.
    Dynamic,
}

/// How a kind relates to a NIP-29 channel (`h` tag).
///
/// This single enum replaces the two independent boolean allowlists
/// (`is_global_only_kind` and `requires_h_channel_scope`) that previously had
/// to be kept mutually exclusive by a runtime test. Encoding the choice as one
/// enum makes double-classification unrepresentable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scoping {
    /// The event is workspace-global; any `h` tag is ignored and the stored
    /// `channel_id` is forced to `None`.
    Global,
    /// The event MUST carry an `h` tag naming its channel; absence is rejected.
    ChannelRequired,
    /// The event may carry an `h` tag (honored) or not (treated as global) —
    /// e.g. reactions and deletions whose channel is derived from their target.
    ChannelOptional,
}

/// Read-side sensitivity of a kind.
///
/// Mirrors `buzz-core`'s `P_GATED_KINDS` / `AUTHOR_ONLY_KINDS` /
/// `RESULT_GATED_KINDS` slices. Not yet consulted by the read path in this
/// PR — see the crate-level docs — but declared here so a kind's full policy
/// is legible in one place and so a future read-path flip has a ready-made,
/// testable target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadGate {
    /// Normal fan-out; world-readable and searchable.
    Public,
    /// Global reads must carry `#p` matching the requester's pubkey.
    PGated,
    /// Readable only by the event's author.
    AuthorOnly,
    /// Even an explicit `ids`/`authors` query must match `#p` to return it.
    ResultGated,
}

/// Whether clients may submit a kind, or it is produced only by the relay.
///
/// Not yet consulted by the write path in this PR — `buzz_core::kind::
/// is_relay_only_kind` remains the live authority (it runs before the
/// registry lookup). Declared for completeness and as a follow-up target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Authorship {
    /// A client may publish this kind (subject to scope checks).
    ClientWritable,
    /// Only the relay itself produces this kind; client submissions are refused.
    RelayOnly,
}

/// The complete policy for one event kind.
#[derive(Clone)]
pub struct KindDescriptor {
    /// The Nostr event kind integer (matches a `KIND_*` constant in `buzz-core`).
    pub kind: u32,
    /// A short stable name, for diagnostics.
    pub name: &'static str,
    /// Base scope required to write the kind.
    pub required_scope: RequiredScope,
    /// Channel-relationship classification.
    pub scoping: Scoping,
    /// Read-side sensitivity.
    pub read_gate: ReadGate,
    /// Client-writable vs relay-only.
    pub authorship: Authorship,
    /// Optional per-kind scope-resolution hook for `RequiredScope::Dynamic`
    /// kinds. `None` for every kind with a `RequiredScope::Static` scope.
    pub extension: Option<&'static dyn KindExtension>,
}
