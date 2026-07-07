//! Community moderation command handler (kinds 9040–9044, Phase 1 contract).
//!
//! Mirrors the NIP-43 relay-admin pattern (`relay_admin.rs`, 9030-series):
//! commands are validated + executed directly and are **never** stored as
//! regular events. Authorization goes through
//! [`crate::handlers::moderation_authz::authorize_moderation_action`] —
//! never inline role checks.
//!
//! | Kind | Operation      | Side effects (all mandatory)                       |
//! |------|----------------|----------------------------------------------------|
//! | 9040 | Ban            | `community_bans` upsert, audit row, live disconnect (L4 fanout), restriction notice DM (L5) |
//! | 9041 | Unban          | ban lift, audit row                                |
//! | 9042 | Timeout        | `muted_until` upsert, audit row, notice DM         |
//! | 9043 | Untimeout      | mute clear, audit row                              |
//! | 9044 | Resolve report | report status update, audit row, reporter notice DM; `delete`/`kick`/`ban` resolutions fan out through the existing 9005/9001 + 9040 paths |
//!
//! Targets (`p` tag pubkey, `report` tag row id) are resolved under the
//! request's `TenantContext` only.
//!
//! ## Routing (pinned — Wren contract review, 2026-07-07)
//! 9040–9044 are **community-global direct commands**, exactly like the
//! relay-admin 9030-series: route via
//! [`buzz_core::kind::is_moderation_command_kind`], list them in
//! `is_global_only_kind` so a stray `h` tag can never channel-scope them
//! (no channel membership/archive gates apply), require a fresh timestamp,
//! never store them, and reject channel-scoped API tokens.
//!
//! ## Tag vocabulary (pinned — CLI and relay must agree)
//! - 9040 ban: `["p", <hex pubkey>]` required; optional
//!   `["expiration", <unix secs>]` (absent ⇒ permanent), `["reason", <text>]`.
//! - 9041 unban: `["p", <hex pubkey>]`.
//! - 9042 timeout: `["p", <hex pubkey>]` + required `["expiration", <unix secs>]`;
//!   optional `["reason", <text>]`.
//! - 9043 untimeout: `["p", <hex pubkey>]`.
//! - 9044 resolve: `["report", <row uuid>]` + `["status", resolved|dismissed|escalated]`
//!   + `["resolution", dismiss|delete|kick|ban|timeout|escalate]`; optional
//!   `["reason", <text>]` (private) and `["public_reason", <text>]` (tombstone-safe).
//!   `delete`/`kick`/`ban`/`timeout` resolutions fan out through the existing
//!   9005/9001 paths and the 9040/9042 handlers — no second implementation.
//!
//! Lane ownership: L6 (Quinn) — plus `buzz-cli` `moderation` command group.
//! The `ingest.rs` routing entries (scope map + `is_global_only_kind` +
//! direct-processing dispatch) for 9040–9044 belong to L3 (Perci):
//! coordinate, don't edit ingest.rs.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use nostr::Event;

use crate::state::AppState;

/// Validate and execute a moderation command (kinds 9040–9044).
///
/// Returns a client-safe error string for `OK false` on rejection.
pub async fn handle_moderation_command(
    _tenant: &TenantContext,
    _state: &Arc<AppState>,
    _event: &Event,
) -> Result<(), String> {
    todo!("L6 (Quinn): dispatch 9040–9044 through moderation_authz + buzz_db::moderation")
}
