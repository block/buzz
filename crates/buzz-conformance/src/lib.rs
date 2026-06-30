//! Runtime trace schema + independent replay checker for
//! `docs/spec/MultiTenantRelay.tla`.
//!
//! North star (from the runtime-formal-compliance skill): don't ask "did the
//! model pass"; ask "did the running code emit a trace the model accepts."
//!
//! ## What this crate is
//!
//! - The **schema** ([`TraceStep`], [`TraceAction`], [`AbstractState`]) that
//!   the relay emits at its ingest/read accept-reject boundary.
//! - An **independent** replay checker ([`check_trace`]) that consumes a
//!   sequence of `TraceStep`s and validates them against the TLA+ spec's
//!   `Next` transition relation. The checker re-implements the relevant
//!   spec actions in Rust; it does NOT call any production reducer.
//!
//! ## What this crate is NOT
//!
//! - A proof. Trace conformance only checks executions you ran. Coverage is
//!   widened by integration tests, property tests, and adversarial fixtures.
//! - A re-export of production helpers. Sharing normalization helpers between
//!   the emitter (which projects implementation state) and the checker (which
//!   judges that projection) would let a bug in the helpers hide itself from
//!   both — exactly the failure the skill calls out.
//!
//! ## Failure modes (skill §Phase 4)
//!
//! - **Illegal transition** — the traced action is not allowed from the
//!   checker's current model state.
//! - **State mismatch** — `state_after.row_labels` includes a community other
//!   than the resolved tenant (`Inv_NonInterference`).
//! - **Coverage breach** — an unknown critical action, a critical seam exit
//!   without a trace step ([`TraceAction::ImplBug`]), or a scenario-required
//!   action that never appeared.
//!
//! Coverage breach is load-bearing. Without it, trace conformance is
//! decorative logging.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod checker;
pub mod transitions;

use serde::{Deserialize, Serialize};

/// Opaque community label — the underlying UUID a server-resolved
/// `TenantContext::community()` wraps, carried as a value type in the
/// trace schema.
///
/// This deliberately does NOT reuse `buzz_core::CommunityId`. Two reasons:
///
/// 1. **Production fence preservation.** `buzz_core::CommunityId` has no
///    `From<Uuid>`, no `Serialize`, no `Deserialize` — by design, so a
///    `CommunityId` cannot be conjured from client input. Adding Serde to
///    it for our convenience would punch a hole in that fence. Carrying
///    our own newtype keeps that fence intact.
/// 2. **Independence.** The checker re-implements the spec transition
///    relation; the schema sharing zero type machinery with production
///    means a buggy production type cannot launder its bug into the
///    checker mechanically.
///
/// The relay's emitter module converts at the seam:
/// `CommunityLabel::from_uuid(*tenant.community().as_uuid())`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommunityLabel(pub uuid::Uuid);

impl CommunityLabel {
    /// Wrap a UUID into a community label. Unlike `buzz_core::CommunityId`
    /// this conversion IS public — but consumers of `CommunityLabel` are
    /// the checker and test fixtures, not the relay's request path. The
    /// relay only constructs `CommunityLabel` from a `TenantContext` it
    /// already resolved.
    pub const fn from_uuid(id: uuid::Uuid) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for CommunityLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// Trace schema version. Bump on any backwards-incompatible field change.
pub const SCHEMA_VERSION: u32 = 1;

/// An opaque ID derived from an event id or other secret material. Stable,
/// no payload, no key bytes. Implementations pick a hash; the checker
/// compares strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpaqueId(pub String);

/// An opaque host label — produced by the relay from the bound `Host` header
/// via a configured registry, never the raw `Host` string. Mirrors the spec's
/// `Hosts` set abstractly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HostLabel(pub String);

/// An opaque channel label — the channel UUID directly. Channels are not
/// secret; the production code already exposes them in event tags.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelLabel(pub uuid::Uuid);

/// An opaque actor label — the lower 16 bytes of `blake3(pubkey)`. Stable,
/// non-reversible, secret-free.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorLabel(pub String);

/// Auth verdict — the closed alphabet from `AuthCheck` (spec line 794).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Authorized.
    Allow,
    /// Denied. The spec models a single Deny verdict; reason is not exposed
    /// at the trace boundary because the spec's error alphabet is closed.
    Deny,
}

/// The sanitized error alphabet (spec `Inv_SanitizedErrors`, M6 mutation).
///
/// Errors observed by the client must come from this closed set; raw error
/// strings are NOT projected into the trace because the spec requires error
/// observations carry no tenant-derived information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SanitizedReason {
    /// Host/channel/community fence rejected the request (relay-only kind,
    /// archived channel, scope-token mismatch, etc.) — spec "restricted".
    Restricted,
    /// Malformed event — spec "invalid".
    Invalid,
    /// Server fault — spec "server_error".
    ServerError,
}

/// The abstract state mirrored from `TenantContext`: which community the
/// server resolved, which host bound that resolution. This is what
/// `Inv_NonInterference` checks observations against.
///
/// Carries deliberately the things that reveal violations (claimed vs.
/// resolved community, opaque host) and deliberately not raw payloads,
/// pubkey bytes, signatures, or wall-clock timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractState {
    /// The server-resolved community for this request — the label
    /// `Inv_NonInterference` validates against. Sourced **only** from
    /// `TenantContext::community()`. Never from event tags, never from
    /// client input, never from `event.pubkey`.
    pub resolved_community: CommunityLabel,
    /// The host that bound this request to that community. Sourced from
    /// `TenantContext::host()` via a label registry.
    pub bound_host: HostLabel,
    /// The actor (authenticated pubkey) for this request, opaque-labelled.
    pub actor: ActorLabel,
}

/// One trace step emitted at the ingest/read accept-reject boundary.
///
/// Action vocabulary (spec actions in parentheses):
/// - [`TraceAction::WriteInsert`] (spec `WriteInsert`, lines 514–550)
/// - [`TraceAction::WriteInsertGlobal`] (spec `WriteInsertGlobal`, lines 559–595)
/// - [`TraceAction::WriteDuplicate`] (spec `WriteDuplicate`, lines 606–637)
/// - [`TraceAction::SanitizedError`] (spec `SanitizedError`, line 778)
/// - [`TraceAction::AuthCheck`] (spec `AuthCheck`, line 794) — M2/M8 target
/// - [`TraceAction::ReadMessageRows`] (spec `ReadMessageRows`, line 643)
/// - [`TraceAction::ReadByIdRows`] (spec `ReadByIdRows`, line 681)
/// - [`TraceAction::ReadHostFeedRows`] (spec `ReadHostFeedRows`, line ~720)
/// - [`TraceAction::ImplBug`] — emitted by the coverage-breach guard when
///   the seam exits without a known action; the checker treats this as a
///   coverage breach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceAction {
    /// Channel-bearing write (spec `WriteInsert`).
    WriteInsert {
        /// Opaque hash of the event id.
        msg_id: OpaqueId,
        /// The channel the event targets — the "real" community is
        /// `ChannelCommunity(channel)` per spec.
        channel: ChannelLabel,
        /// The community the client *claimed* via its `h` tag, if any.
        /// `None` means the client did not assert one. This stays distinct
        /// from `state_after.resolved_community` so M2/M8 mutations are
        /// visible in the trace.
        claimed_community: Option<CommunityLabel>,
    },
    /// Channel-less write resolved purely from the bound host
    /// (spec `WriteInsertGlobal`).
    WriteInsertGlobal {
        /// Opaque hash of the event id.
        msg_id: OpaqueId,
        /// The community the client *claimed*, if any. Ignored by the
        /// resolver but recorded for the audit trail.
        claimed_community: Option<CommunityLabel>,
    },
    /// Channel-bearing duplicate / no-op write (spec `WriteDuplicate`,
    /// `ON CONFLICT (community_id, id)` returning a duplicate result).
    WriteDuplicate {
        /// Opaque hash of the event id.
        msg_id: OpaqueId,
        /// The channel the duplicate hit.
        channel: ChannelLabel,
        /// The community the client *claimed*, if any.
        claimed_community: Option<CommunityLabel>,
    },
    /// Sanitized error (spec `SanitizedError`). Closed-alphabet reason
    /// only; no raw error string is projected.
    SanitizedError {
        /// One of the closed-alphabet reasons.
        reason: SanitizedReason,
    },
    /// Per-(channel, actor) authorization decision (spec `AuthCheck`).
    /// M2 and M8 explicitly target this action — leaving it out would
    /// make the gate blind to those mutations.
    AuthCheck {
        /// The channel the check is against.
        channel: ChannelLabel,
        /// The community the client claimed, if any.
        claimed_community: Option<CommunityLabel>,
        /// The Allow/Deny verdict the implementation produced.
        verdict: Verdict,
    },
    /// Per-channel-or-channelless row read returning concrete rows
    /// (spec `ReadMessageRows`).
    ReadMessageRows {
        /// Channel filter — `None` means channel-less.
        channel: Option<ChannelLabel>,
        /// The community label of EACH row returned. NOT deduped to a Set,
        /// NOT filtered to "matches resolved": the checker must see every
        /// leaked label to fail closed on `Inv_ReadConfinement` / M1/M4/M7.
        row_communities: Vec<CommunityLabel>,
    },
    /// Direct read by event id list (spec `ReadByIdRows`). The search lane
    /// emits this for each refetched hit.
    ReadByIdRows {
        /// Channel filter — `None` means channel-less.
        channel: Option<ChannelLabel>,
        /// Per-row community labels, same rules as `ReadMessageRows`.
        row_communities: Vec<CommunityLabel>,
    },
    /// Kinds-only feed read (spec `ReadHostFeedRows`). The relay derives
    /// the community from the bound host and fans out across that
    /// community's channel-less rows plus its accessible channels.
    ReadHostFeedRows {
        /// Per-row community labels.
        row_communities: Vec<CommunityLabel>,
    },
    /// Coverage-breach guard: the seam exited without a known action. The
    /// checker treats this as a coverage breach and fails closed.
    ImplBug {
        /// A short tag identifying the missing emit site (e.g.
        /// `"ingest_exited_without_trace"`).
        kind: String,
    },
}

impl TraceAction {
    /// A short stable string identifying the action kind, for fixture
    /// declarations and error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            TraceAction::WriteInsert { .. } => "write_insert",
            TraceAction::WriteInsertGlobal { .. } => "write_insert_global",
            TraceAction::WriteDuplicate { .. } => "write_duplicate",
            TraceAction::SanitizedError { .. } => "sanitized_error",
            TraceAction::AuthCheck { .. } => "auth_check",
            TraceAction::ReadMessageRows { .. } => "read_message_rows",
            TraceAction::ReadByIdRows { .. } => "read_by_id_rows",
            TraceAction::ReadHostFeedRows { .. } => "read_host_feed_rows",
            TraceAction::ImplBug { .. } => "impl_bug",
        }
    }

    /// Every action at this seam is critical: the spec requires every
    /// observation to be labelled. The skill's "coverage breach" mode
    /// hinges on every emit site being marked critical.
    pub const fn is_critical(&self) -> bool {
        true
    }
}

/// One step in the trace stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    /// Schema version — bump on backwards-incompatible field changes.
    pub schema_version: u32,
    /// The action that occurred at the seam.
    pub action: TraceAction,
    /// The abstract state the implementation observed at action time.
    /// The checker compares this against its independently-computed model
    /// state.
    pub state_after: AbstractState,
}

impl TraceStep {
    /// Build a step at the current schema version.
    pub fn new(action: TraceAction, state_after: AbstractState) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            action,
            state_after,
        }
    }
}

/// The emit trait the relay calls. The trait is the *only* surface the
/// production code touches; the schema types stay value types.
pub trait Tracer: Send + Sync {
    /// Record one trace step. Implementations MAY be no-ops in production
    /// builds and write to JSONL in tests.
    fn record(&self, step: TraceStep);
}

/// A no-op tracer for production. Zero cost: the build can omit emission
/// entirely behind a feature, or simply discard records here.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTracer;

impl Tracer for NoopTracer {
    fn record(&self, _step: TraceStep) {}
}
