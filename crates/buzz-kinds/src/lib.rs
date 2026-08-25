//! Declarative event-kind registry.
//!
//! This crate is the single home for a Buzz event kind's *policy*: the scope
//! required to write it, how it relates to a NIP-29 channel, and its
//! read-side sensitivity. The relay resolves an incoming event's base
//! write-scope and channel-scoping classification through a [`KindRegistry`]
//! instead of the hand-maintained `required_scope_for_kind` match plus the
//! `is_global_only_kind` / `requires_h_channel_scope` boolean allowlists that
//! previously had to be kept mutually exclusive by a runtime test alone.
//!
//! It sits between `buzz-core` (kind constants and range predicates) and
//! `buzz-auth` (the [`buzz_auth::Scope`] vocabulary a descriptor names). It is
//! a leaf crate — it performs no I/O and depends on nothing in `buzz-relay`.
//!
//! A kind whose required scope depends on the event itself (today, only the
//! NIP-29 `kind:9002` edit-metadata archive/non-archive split) supplies a
//! [`KindExtension`] to resolve it. The extension seam is deliberately
//! minimal for now — this PR only wires the scope/scoping resolution
//! described above; folding the relay's per-kind authorization/validation
//! residue and the `P_GATED_KINDS`/`AUTHOR_ONLY_KINDS`/`RESULT_GATED_KINDS`
//! read-gate slices into this seam is a natural, separable follow-up (see the
//! PR description).

#![forbid(unsafe_code)]

mod descriptor;
mod extension;
mod registry;

pub use descriptor::{Authorship, KindDescriptor, ReadGate, RequiredScope, Scoping};
pub use extension::KindExtension;
pub use registry::{register_builtin, KindRegistry};
