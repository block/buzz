//! NIP-56 report (kind:1984) validation + persistence (Phase 1 contract).
//!
//! Reports are signals, never triggers (NIP-56): the relay persists them to
//! the tenant-scoped moderation queue and **never** auto-actions or fans them
//! out publicly.
//!
//! ## The pinned invariant (MOD, `docs/spec/MultiTenantRelay.tla`)
//! Report targets resolve under `tenant.community()` **only**:
//! - `e` target → event row looked up in this tenant; infer `channel_id`
//!   from it. Not found in-tenant ⇒ reject (never search other tenants).
//! - `x` blob target → tenant-scoped media reference `(community_id, sha256)`.
//!   A bare SHA-256 is shared across tenants and must not grant cross-tenant
//!   visibility.
//! - `p`-only target → community-local report about that pubkey in this
//!   tenant; implies nothing platform/global.
//!
//! Lane ownership: L3 (Perci) — including the `required_scope_for_kind` /
//! storage-suppression wiring in `ingest.rs`.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use nostr::Event;

use crate::state::AppState;

/// NIP-56 report types accepted at ingest.
pub const REPORT_TYPES: &[&str] = &[
    "illegal",
    "nudity",
    "malware",
    "spam",
    "impersonation",
    "profanity",
    "other",
];

/// Validate a kind:1984 report and persist it to `moderation_reports`.
///
/// Rejections use client-safe `invalid:`/`restricted:` reasons. On success
/// the report is queued (idempotently, keyed by the signed event id) and the
/// event itself is **not** stored or broadcast as a regular event.
pub async fn handle_report_event(
    _tenant: &TenantContext,
    _event: &Event,
    _state: &Arc<AppState>,
) -> Result<(), String> {
    todo!("L3 (Perci): validate report-type tag, tenant-fenced target resolution, insert_report")
}
