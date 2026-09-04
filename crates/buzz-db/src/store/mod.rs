//! Domain-owned persistence implementations.

/// Explicit deployment-global admin report reads.
pub mod admin_moderation;
/// Community-scoped authentication allowlist persistence.
pub mod allowlist;
/// API token storage and lookup.
pub mod api_token;
/// Relay-scoped archived identity persistence (NIP-IA).
pub mod archived_identities;
/// Channel lifecycle and metadata persistence.
pub mod channel;
/// Channel membership and roster persistence.
pub mod channel_members;
/// Community lifecycle and host-map persistence.
pub mod community;
/// Durable whole-community deletion lifecycle and PostgreSQL adapter.
pub mod deletion;
/// Direct message channel persistence.
pub mod dm;
/// Event storage and retrieval.
pub mod event;
/// Home feed queries.
pub mod feed;
/// Git repository name registry (NIP-34 kind:30617).
pub mod git_repo;
/// Community moderation: reports, bans/timeouts, audit actions.
pub mod moderation;
/// Monthly table partition management.
pub mod partition;
/// Buzz product-feedback sidecar persistence.
pub mod product_feedback;
/// Community-scoped push lease and durable wake-outbox persistence.
pub mod push;
/// Reaction persistence.
pub mod reaction;
/// HTTP report-resolution enforcement state machine persistence.
pub mod relay_admin_actions;
/// Use-limited relay invite persistence (v2 opaque tokens).
pub mod relay_invite;
/// Relay-level membership persistence (NIP-43).
pub mod relay_members;
/// Deployment-global relay operator/moderator roster persistence.
pub mod relay_operators;
/// Event-reminder delivery query, claim, and release persistence.
pub mod reminder;
/// Replaceable-event persistence and coordinate locking.
pub mod replaceable;
/// Thread metadata persistence.
pub mod thread;
/// Per-community usage rollup queries for Prometheus gauges.
pub mod usage;
/// User profile persistence.
pub mod user;
/// Workflow, run, and approval persistence.
pub mod workflow;
