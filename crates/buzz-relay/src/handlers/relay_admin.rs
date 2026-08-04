//! NIP-43 relay membership admin command handler (kinds 9030–9032).
//!
//! These events are processed directly — they mutate the `relay_members` table
//! and return without being stored as regular Nostr events.
//!
//! ## Permission matrix
//!
//! | Kind | Operation       | Required sender role |
//! |------|-----------------|----------------------|
//! | 9030 | Add member      | admin or owner       |
//! | 9031 | Remove member   | admin or owner       |
//! | 9032 | Change role     | owner only           |
//! | 9033 | Set workspace profile (icon) | admin or owner; on an open relay whose community has no admin/owner row at all, any authenticated sender (see [`may_set_workspace_profile`]) |

use std::sync::Arc;

use nostr::Event;
use tracing::{info, warn};

use buzz_core::kind::{
    RELAY_ADMIN_ADD_MEMBER, RELAY_ADMIN_CHANGE_ROLE, RELAY_ADMIN_REMOVE_MEMBER,
    RELAY_ADMIN_SET_WORKSPACE_PROFILE,
};
use buzz_core::tenant::TenantContext;
use buzz_db::relay_members::RemoveResult;

use crate::handlers::side_effects::{
    publish_nip43_member_added, publish_nip43_member_removed, publish_nip43_membership_list,
};
use crate::state::AppState;

/// Extract the hex pubkey from the first `p` tag, returning it as a `String`.
fn extract_p_tag_hex(event: &Event) -> Option<String> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some("p") {
            if let Some(val) = parts.get(1).map(|s| s.as_str()) {
                // Must be exactly 64 hex chars (uncompressed pubkey representation).
                if val.len() == 64 && val.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// Extract the value of the first tag with the given name.
fn extract_tag_value(event: &Event, name: &str) -> Option<String> {
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some(name) {
            return parts.get(1).map(|s| s.to_string());
        }
    }
    None
}

/// Maximum accepted workspace icon https URL length.
const MAX_WORKSPACE_ICON_URL_LEN: usize = 2048;

/// Maximum accepted workspace icon data-URL length (~96 KB of base64 ≈ 72 KB
/// image — generous for a 128px icon).
const MAX_WORKSPACE_ICON_DATA_URL_LEN: usize = 98_304;

/// Validate a workspace icon: empty (clear), an http(s) URL, or an inline
/// `data:image/*` URL (what the desktop publishes — it renders across
/// workspaces without cross-relay media fetches).
fn validate_workspace_icon(icon: &str) -> Result<(), String> {
    if icon.is_empty() {
        return Ok(());
    }
    if icon.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("icon contains invalid characters".to_string());
    }
    if icon.starts_with("data:image/") {
        if icon.len() > MAX_WORKSPACE_ICON_DATA_URL_LEN {
            return Err(format!(
                "icon data URL too long: {} bytes (max {MAX_WORKSPACE_ICON_DATA_URL_LEN})",
                icon.len()
            ));
        }
        return Ok(());
    }
    if !icon.starts_with("https://") && !icon.starts_with("http://") {
        return Err("icon must be an http(s) URL or data:image/* URL".to_string());
    }
    if icon.len() > MAX_WORKSPACE_ICON_URL_LEN {
        return Err(format!(
            "icon URL too long: {} bytes (max {MAX_WORKSPACE_ICON_URL_LEN})",
            icon.len()
        ));
    }
    Ok(())
}

/// Whether `sender_role` may set the workspace profile (kind:9033).
///
/// Closed relays (`membership_enforced == true`) require an `admin`/`owner`
/// row in `relay_members` — the enforced roster is the authority. Open relays
/// don't *enforce* the roster, but the data can still exist: startup
/// bootstraps `RELAY_OWNER_PUBKEY` as `owner` regardless of the flag
/// (`main.rs`), as does operator provisioning. So the rule is steward-wins:
///
/// - a steward (any admin/owner row) exists → admin/owner only, exactly like
///   a closed relay. An open relay with a configured owner keeps its icon
///   owner-controlled instead of last-write-wins for every authenticated key.
/// - genuinely rosterless (e.g. a community created by
///   `ensure_configured_community`, which writes no owner row) → any
///   NIP-42-authenticated sender may set the icon, mirroring how open relays
///   gate every other write. Without this the icon is permanently unsettable:
///   the desktop deliberately shows the icon editor on open relays (see
///   `canEditIcon` in `EditCommunityDialog.tsx`, #2640) and defers to this
///   relay-side check, which used to always say no.
fn may_set_workspace_profile(
    sender_role: &str,
    membership_enforced: bool,
    community_has_steward: bool,
) -> bool {
    if !membership_enforced && !community_has_steward {
        return true;
    }
    sender_role == "admin" || sender_role == "owner"
}

/// A relay-admin command failure, carrying the *category* of the failure so
/// the ingest seam can map it to the right NIP-01 prefix and HTTP status.
///
/// The category is part of the security contract, not cosmetics: a ban must
/// surface as `blocked:` / HTTP 403 (like every other durable-restriction
/// refusal), and a restriction-lookup outage must surface as a 500 rather than
/// a client-side 400 that reads as "your request was malformed". Mirrors
/// [`super::push_lease::AcceptError`] and its `map_push_accept_error` seam.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RelayAdminError {
    /// Sender is under a durable community ban — `blocked:` / HTTP 403.
    Banned,
    /// Legacy command rejection — `invalid:` / HTTP 400. This is whatever
    /// [`execute_relay_admin_command`] returned as a `String`, which is mostly
    /// validation and authorization but also still includes that function's own
    /// DB failures. Categorizing those is deliberately out of scope for the ban
    /// fix, so this arm claims no stronger invariant than "the command body said
    /// no".
    Rejected(String),
    /// Admission could not be decided because the restriction lookup failed —
    /// `error:` / HTTP 500.
    Internal(String),
}

/// Decide whether a durable restriction state admits a relay-admin command.
///
/// Ban only, deliberately: a timeout is a write-block on *content*, and
/// `ingest_event` exempts relay-admin kinds from its durable write-path gate
/// precisely so restricted-but-not-banned admins retain their administrative
/// capability. Mirrors `moderation_commands::ensure_actor_not_banned`.
///
/// Split out as a pure function so the admission rule itself is unit-testable
/// without a live relay: the end-to-end HTTP test proves the transport, this
/// proves the decision.
fn admits_relay_admin_command(
    restriction: &buzz_db::moderation::RestrictionState,
) -> Result<(), RelayAdminError> {
    if restriction.banned {
        return Err(RelayAdminError::Banned);
    }
    Ok(())
}

/// Validate and execute a relay admin command (kinds 9030–9033).
///
/// Admission: rejects a sender under a durable community ban before any
/// command runs. The command itself is executed by
/// [`execute_relay_admin_command`].
///
/// Returns `Ok(())` on success, or a categorized [`RelayAdminError`].
pub(super) async fn handle_relay_admin_event(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(), RelayAdminError> {
    // A ban is an admission boundary, not only a WebSocket-auth check. HTTP
    // NIP-98 requests and already-authenticated sockets reach this handler
    // without passing through a fresh NIP-42 challenge, and `ingest_event`
    // exempts relay-admin kinds from its durable write-path gate so a *timed
    // out* admin can still administer the roster. That exemption is ban-blind,
    // so the ban must be enforced here or not at all: a banned admin otherwise
    // keeps mutating `relay_members` — the very table `moderation_authz`
    // derives moderator capability from — until someone manually deletes the
    // row. Mirrors `moderation_commands.rs`, which defends the same boundary
    // for 9040–9044, and holds that file's stated invariant that a direct
    // command handler rejects a banned actor on every transport.
    //
    // This gate wraps execution rather than opening it so no future early
    // return inside the command body can precede it.
    let restriction = state
        .db
        .moderation_restriction_state(tenant.community(), &event.pubkey.to_bytes())
        .await
        // Fail closed: a DB blip must never admit a banned admin.
        .map_err(|e| {
            RelayAdminError::Internal(format!("internal error checking restriction state: {e}"))
        })?;
    admits_relay_admin_command(&restriction)?;

    execute_relay_admin_command(tenant, state, event)
        .await
        .map_err(RelayAdminError::Rejected)
}

/// Execute an already-admitted relay admin command.
///
/// The handler:
/// 1. Extracts the target pubkey from the `["p", ...]` tag.
/// 2. Extracts the role from the `["role", ...]` tag (kinds 9030 and 9032).
/// 3. Looks up the sender's current role in `relay_members`.
/// 4. Enforces the permission matrix.
/// 5. Applies the change via the DB.
///
/// Returns `Ok(())` on success.  Returns `Err(msg)` — where `msg` is the
/// legacy rejection reason — on any failure. This body does not distinguish
/// validation failures from execution DB failures; both surface as `Err(msg)`
/// and are categorised by the caller as [`RelayAdminError::Rejected`].
async fn execute_relay_admin_command(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(), String> {
    let kind = event.kind.as_u16() as u32;
    let sender_hex = event.pubkey.to_hex();

    // This mirrors the NIP-42 auth event freshness check and prevents replay
    // of captured admin commands. The window is intentionally tight — admin
    // events should be freshly signed.
    {
        let event_ts = event.created_at.as_secs() as i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if (event_ts - now).abs() > 120 {
            return Err(format!(
                "event timestamp out of range: created_at={event_ts}, now={now}, delta={}s (max ±120s)",
                event_ts - now
            ));
        }
    }

    let sender_member = state
        .db
        .get_relay_member(tenant.community(), &sender_hex)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let sender_role = sender_member
        .as_ref()
        .map(|m| m.role.as_str())
        .unwrap_or("");

    // kind:9033 — Set workspace profile (icon). Handled before p-tag
    // extraction: it targets the relay itself, not a member pubkey.
    if kind == RELAY_ADMIN_SET_WORKSPACE_PROFILE {
        // Steward detection only matters on open relays (closed relays gate on
        // the sender's own role either way), so skip the extra query there.
        let community_has_steward = if state.config.require_relay_membership {
            true
        } else {
            state
                .db
                .has_admin_or_owner(tenant.community())
                .await
                .map_err(|e| format!("database error: {e}"))?
        };
        if !may_set_workspace_profile(
            sender_role,
            state.config.require_relay_membership,
            community_has_steward,
        ) {
            return Err("actor not authorized: must be admin or owner".to_string());
        }
        if sender_role != "admin" && sender_role != "owner" {
            // Rosterless-open-relay admit: 9033 writes no audit row and
            // publishes no announcement event (unlike 9030/9031), so this warn
            // is the only durable attribution of who changed the icon.
            warn!(
                sender = %sender_hex,
                "workspace profile change admitted without a roster role (open relay, no steward)"
            );
        }

        // Empty or missing icon tag clears the workspace icon.
        let icon = extract_tag_value(event, "icon").unwrap_or_default();
        validate_workspace_icon(&icon)?;

        state
            .db
            .set_community_icon(
                tenant.community(),
                (!icon.is_empty()).then_some(icon.as_str()),
            )
            .await
            .map_err(|e| format!("failed to store workspace icon: {e}"))?;

        info!(sender = %sender_hex, icon_len = icon.len(), "workspace profile updated");
        return Ok(());
    }

    let target_hex = extract_p_tag_hex(event)
        .ok_or_else(|| "missing or invalid p tag".to_string())?
        .to_ascii_lowercase();

    match kind {
        // kind:9030 — Add relay member
        k if k == RELAY_ADMIN_ADD_MEMBER => {
            // Sender must be admin or owner.
            if sender_role != "admin" && sender_role != "owner" {
                return Err("actor not authorized: must be admin or owner".to_string());
            }

            // Default role is "member" when no role tag is present.
            let role = extract_tag_value(event, "role").unwrap_or_else(|| "member".to_string());

            // Owners can add admins or members; admins can only add members.
            if role == "owner" {
                return Err("invalid role: use kind:9032 to promote to owner".to_string());
            }
            if role == "admin" && sender_role != "owner" {
                return Err("actor not authorized: only owner can grant admin role".to_string());
            }
            if role != "admin" && role != "member" {
                return Err(format!("invalid role: {role}"));
            }

            // Note: idempotent — if target already exists at any role, this is a
            // silent no-op. The existing role is NOT overwritten. Use kind:9032
            // to change an existing member's role.
            let was_inserted = state
                .db
                .add_relay_member(tenant.community(), &target_hex, &role, Some(&sender_hex))
                .await
                .map_err(|e| format!("database error: {e}"))?;

            info!(
                sender = %sender_hex,
                target = %target_hex,
                role = %role,
                was_inserted,
                "relay member add attempted"
            );

            // Only publish NIP-43 announcements when the row was actually inserted —
            // skip on no-op re-adds to avoid spurious kind:8000 events.
            if was_inserted {
                if let Err(e) = publish_nip43_member_added(tenant, state, &target_hex).await {
                    warn!(error = %e, "failed to publish NIP-43 member added event");
                }
                if let Err(e) = publish_nip43_membership_list(tenant, state).await {
                    warn!(error = %e, "failed to publish NIP-43 membership list");
                }
            }
        }

        // kind:9031 — Remove relay member
        k if k == RELAY_ADMIN_REMOVE_MEMBER => {
            // Sender must be admin or owner.
            if sender_role != "admin" && sender_role != "owner" {
                return Err("actor not authorized: must be admin or owner".to_string());
            }

            // Cannot remove yourself.
            if target_hex == sender_hex {
                return Err("cannot remove yourself".to_string());
            }

            // Dispatch removal by sender role:
            // - Admins: atomic conditional delete, only removes 'member' targets.
            //   This eliminates the TOCTOU race where the target could be promoted
            //   between a prior role read and the delete.
            // - Owners: can remove admins and members, not other owners.
            let remove_result = if sender_role == "admin" {
                state
                    .db
                    .remove_relay_member_if_role(tenant.community(), &target_hex, "member")
                    .await
                    .map_err(|e| format!("database error: {e}"))?
            } else {
                // Owner path — atomic delete that refuses to remove other owners.
                state
                    .db
                    .remove_relay_member(tenant.community(), &target_hex)
                    .await
                    .map_err(|e| format!("database error: {e}"))?
            };

            match remove_result {
                RemoveResult::Removed => {}
                RemoveResult::IsOwner => {
                    return Err("cannot remove the relay owner".to_string());
                }
                RemoveResult::NotFound => {
                    return Err(format!("member not found: {target_hex}"));
                }
                RemoveResult::RoleMismatch => {
                    return Err("actor not authorized: admins can only remove members".to_string());
                }
            }

            info!(
                sender = %sender_hex,
                target = %target_hex,
                "relay member removed"
            );

            if let Err(e) = publish_nip43_member_removed(tenant, state, &target_hex).await {
                warn!(error = %e, "failed to publish NIP-43 member removed event");
            }
            if let Err(e) = publish_nip43_membership_list(tenant, state).await {
                warn!(error = %e, "failed to publish NIP-43 membership list");
            }
        }

        // kind:9032 — Change relay member role
        k if k == RELAY_ADMIN_CHANGE_ROLE => {
            // Only owners may change roles.
            if sender_role != "owner" {
                return Err("actor not authorized: must be owner".to_string());
            }

            // Cannot change your own role.
            if target_hex == sender_hex {
                return Err("cannot change your own role".to_string());
            }

            let new_role =
                extract_tag_value(event, "role").ok_or_else(|| "missing role tag".to_string())?;

            // DESIGN: Ownership transfer via kind:9032 is intentionally blocked.
            // Transferring ownership is a high-risk operation that could permanently
            // lock out the current owner. Use RELAY_OWNER_PUBKEY config to change ownership.
            if new_role == "owner" {
                return Err("cannot set role to owner".to_string());
            }
            if new_role != "admin" && new_role != "member" {
                return Err(format!("invalid role: {new_role}"));
            }

            let updated = state
                .db
                .update_relay_member_role(tenant.community(), &target_hex, &new_role)
                .await
                .map_err(|e| format!("database error: {e}"))?;

            if !updated {
                // Distinguish "owner (protected)" from "doesn't exist"
                let exists = state
                    .db
                    .get_relay_member(tenant.community(), &target_hex)
                    .await
                    .map_err(|e| format!("database error: {e}"))?;
                return Err(if exists.is_some() {
                    "cannot change the relay owner's role".to_string()
                } else {
                    format!("member not found: {target_hex}")
                });
            }

            info!(
                sender = %sender_hex,
                target = %target_hex,
                new_role = %new_role,
                "relay member role changed"
            );

            if let Err(e) = publish_nip43_membership_list(tenant, state).await {
                warn!(error = %e, "failed to publish NIP-43 membership list");
            }
        }

        other => {
            return Err(format!("unexpected relay admin kind: {other}"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// The vulnerability this file's ban gate closes: `ingest_event` exempts
    /// relay-admin kinds 9030–9033 from its durable write-path restriction
    /// gate, so a **banned** admin could add/remove relay members and change
    /// the workspace icon over signed NIP-98 `POST /events`. Deleting the
    /// admission check must fail here, in the default (non-ignored) suite.
    #[test]
    fn banned_actor_is_not_admitted_to_a_relay_admin_command() {
        let banned = buzz_db::moderation::RestrictionState {
            banned: true,
            muted_until: None,
        };
        assert_eq!(
            admits_relay_admin_command(&banned),
            Err(RelayAdminError::Banned),
            "a durably banned admin must never reach a relay-admin command"
        );
    }

    /// The counter-invariant, and the reason the ingest exemption exists at
    /// all: a timeout restricts *content* writes, not administrative
    /// capability. Widening this gate to timeouts would silently change policy.
    #[test]
    fn timed_out_actor_is_still_admitted() {
        let timed_out = buzz_db::moderation::RestrictionState {
            banned: false,
            muted_until: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
        };
        assert!(
            admits_relay_admin_command(&timed_out).is_ok(),
            "a timed-out admin must still administer the roster"
        );
    }

    #[test]
    fn unrestricted_actor_is_admitted() {
        assert!(
            admits_relay_admin_command(&buzz_db::moderation::RestrictionState::default()).is_ok()
        );
    }

    /// Build a minimal signed Event with the given kind and tags.
    /// The pubkey will be randomly generated — sufficient for tag extraction tests.
    fn make_test_event(kind: u16, tags: Vec<Vec<&'static str>>) -> Event {
        let keys = Keys::generate();
        let nostr_tags: Vec<Tag> = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::from(kind), "")
            .tags(nostr_tags)
            .sign_with_keys(&keys)
            .expect("signing failed")
    }

    #[test]
    fn extract_p_tag_valid_hex() {
        let hex = "a".repeat(64);
        let event = make_test_event(
            9030,
            vec![vec!["p", Box::leak(hex.clone().into_boxed_str())]],
        );
        assert_eq!(extract_p_tag_hex(&event), Some(hex));
    }

    #[test]
    fn extract_p_tag_rejects_short_hex() {
        let event = make_test_event(9030, vec![vec!["p", "abcd"]]);
        assert_eq!(extract_p_tag_hex(&event), None);
    }

    #[test]
    fn extract_p_tag_rejects_non_hex() {
        // 'g' is not a hex digit
        let event = make_test_event(
            9030,
            vec![vec![
                "p",
                "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
            ]],
        );
        assert_eq!(extract_p_tag_hex(&event), None);
    }

    #[test]
    fn extract_p_tag_missing() {
        let event = make_test_event(9030, vec![]);
        assert_eq!(extract_p_tag_hex(&event), None);
    }

    #[test]
    fn extract_p_tag_ignores_non_p_tags() {
        let event = make_test_event(9030, vec![vec!["role", "admin"]]);
        assert_eq!(extract_p_tag_hex(&event), None);
    }

    #[test]
    fn extract_tag_value_found() {
        let event = make_test_event(9030, vec![vec!["role", "admin"]]);
        assert_eq!(extract_tag_value(&event, "role"), Some("admin".to_string()));
    }

    #[test]
    fn extract_tag_value_missing() {
        let event = make_test_event(9030, vec![]);
        assert_eq!(extract_tag_value(&event, "role"), None);
    }

    #[test]
    fn extract_tag_value_returns_first_match() {
        let event = make_test_event(9030, vec![vec!["role", "member"], vec!["role", "admin"]]);
        assert_eq!(
            extract_tag_value(&event, "role"),
            Some("member".to_string())
        );
    }

    #[test]
    fn extract_tag_value_wrong_name() {
        let event = make_test_event(9030, vec![vec!["role", "admin"]]);
        assert_eq!(extract_tag_value(&event, "p"), None);
    }

    #[test]
    fn workspace_icon_empty_ok() {
        assert!(validate_workspace_icon("").is_ok());
    }

    /// Closed relay (membership enforced): only an admin/owner row in
    /// `relay_members` may set the workspace profile — a plain member, or a
    /// pubkey with no row at all (empty role), must be refused. The steward
    /// flag is irrelevant when membership is enforced (call sites pass `true`,
    /// but the rule must not depend on it).
    #[test]
    fn closed_relay_requires_admin_or_owner_for_workspace_profile() {
        for steward in [true, false] {
            assert!(may_set_workspace_profile("owner", true, steward));
            assert!(may_set_workspace_profile("admin", true, steward));
            assert!(!may_set_workspace_profile("member", true, steward));
            assert!(!may_set_workspace_profile("", true, steward));
        }
    }

    /// Open relay with a steward: startup bootstraps `RELAY_OWNER_PUBKEY` as
    /// `owner` regardless of `require_relay_membership`, so an open relay's
    /// community can hold admin/owner rows. When one exists, the icon stays
    /// steward-only — the fix must not widen an owner-controlled icon to
    /// every authenticated key.
    #[test]
    fn open_relay_with_steward_keeps_workspace_profile_steward_only() {
        assert!(may_set_workspace_profile("owner", false, true));
        assert!(may_set_workspace_profile("admin", false, true));
        assert!(!may_set_workspace_profile("member", false, true));
        assert!(!may_set_workspace_profile("", false, true));
    }

    /// Open relay, genuinely rosterless (no admin/owner row anywhere): any
    /// authenticated sender may set the icon — including the roleless (empty
    /// role) case, which is *every* sender there. This is the bug being
    /// fixed: the desktop shows the icon editor on open relays (#2640) but
    /// the relay refused every 9033.
    #[test]
    fn rosterless_open_relay_admits_any_authenticated_sender_for_workspace_profile() {
        assert!(may_set_workspace_profile("", false, false));
        assert!(may_set_workspace_profile("member", false, false));
        assert!(may_set_workspace_profile("owner", false, false));
    }

    #[test]
    fn workspace_icon_https_ok() {
        assert!(validate_workspace_icon("https://example.com/icon.png").is_ok());
    }

    #[test]
    fn workspace_icon_data_url_ok() {
        assert!(validate_workspace_icon("data:image/webp;base64,UklGRg==").is_ok());
    }

    #[test]
    fn workspace_icon_rejects_non_url() {
        assert!(validate_workspace_icon("javascript:alert(1)").is_err());
        assert!(validate_workspace_icon("data:text/html;base64,PGI+").is_err());
    }

    #[test]
    fn workspace_icon_rejects_whitespace_and_control() {
        assert!(validate_workspace_icon("https://example.com/a b.png").is_err());
        assert!(validate_workspace_icon("https://example.com/a\nb.png").is_err());
    }

    #[test]
    fn workspace_icon_rejects_oversized() {
        let long_url = format!("https://example.com/{}.png", "a".repeat(2048));
        assert!(validate_workspace_icon(&long_url).is_err());
        let long_data = format!("data:image/png;base64,{}", "A".repeat(98_304));
        assert!(validate_workspace_icon(&long_data).is_err());
    }

    // ─── Call-site integration: the 9033 gate wired to real config + DB ────
    //
    // The unit tests above pin `may_set_workspace_profile`'s truth table, but
    // not its wiring: mutation-testing showed that inverting
    // `state.config.require_relay_membership` at the call site — an exact
    // inversion of the security contract — survives the default suite. These
    // tests drive `handle_relay_admin_event` with a real `AppState` against
    // Postgres, on both relay modes, so the wiring itself is pinned. Selected
    // explicitly in CI's Backend Integration job; requires local Postgres
    // (and hard-fails rather than skipping when it is unreachable).

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    /// Build a real `AppState` + tenant for a fresh community on `host`, with
    /// `require_relay_membership` set as given. Mirrors
    /// `api::invites::tests::invite_test_state`.
    async fn workspace_profile_test_state(
        host: &str,
        require_relay_membership: bool,
    ) -> (Arc<AppState>, TenantContext) {
        let mut config = crate::config::Config::from_env().expect("config from env");
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_string());
        config.database_url = database_url.clone();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_url = format!("wss://{host}");
        config.require_relay_membership = require_relay_membership;

        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("requires reachable Postgres");
        let db = buzz_db::Db::from_pool(pool.clone());
        let record = db
            .ensure_configured_community(host)
            .await
            .expect("ensure community");
        let tenant = TenantContext::resolved(record.id, host);

        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool config");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        (Arc::new(state), tenant)
    }

    /// Sign a fresh kind:9033 with `icon` and run it through the real
    /// admission + command path.
    async fn submit_9033(
        state: &Arc<AppState>,
        tenant: &TenantContext,
        keys: &Keys,
        icon: &str,
    ) -> Result<(), RelayAdminError> {
        let event = EventBuilder::new(Kind::Custom(9033), "")
            .tags(vec![Tag::parse(["icon", icon]).expect("icon tag")])
            .sign_with_keys(keys)
            .expect("sign 9033");
        handle_relay_admin_event(tenant, state, &event).await
    }

    async fn stored_icon(state: &Arc<AppState>, tenant: &TenantContext) -> Option<String> {
        state
            .db
            .get_community_icon(tenant.community())
            .await
            .expect("read icon")
    }

    /// Open relay (`require_relay_membership = false`): a rosterless
    /// community admits any authenticated sender, but the moment a steward
    /// (admin/owner row) exists the gate reverts to steward-only.
    ///
    /// Discriminating: fails if the call site inverts or drops
    /// `require_relay_membership`, or stops consulting `has_admin_or_owner`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn open_relay_9033_admits_roleless_only_until_a_steward_exists() {
        let host = format!("icon-gate-open-{}.example", uuid::Uuid::new_v4().simple());
        let (state, tenant) = workspace_profile_test_state(&host, false).await;
        let roleless = Keys::generate();
        let owner = Keys::generate();

        // Rosterless: the roleless sender may set the icon.
        submit_9033(&state, &tenant, &roleless, "https://example.com/open.png")
            .await
            .expect("rosterless open relay must admit an authenticated sender");
        assert_eq!(
            stored_icon(&state, &tenant).await.as_deref(),
            Some("https://example.com/open.png"),
            "icon must actually be stored"
        );

        // Seed a steward — the same roleless sender must now be refused, and
        // the previously stored icon must survive the refused attempt.
        state
            .db
            .add_relay_member(
                tenant.community(),
                &owner.public_key().to_hex(),
                "owner",
                None,
            )
            .await
            .expect("seed owner");
        let refused = submit_9033(&state, &tenant, &roleless, "https://evil.example/pwn.png").await;
        assert_eq!(
            refused,
            Err(RelayAdminError::Rejected(
                "actor not authorized: must be admin or owner".to_string()
            )),
            "an open relay with a steward must refuse a roleless sender"
        );
        assert_eq!(
            stored_icon(&state, &tenant).await.as_deref(),
            Some("https://example.com/open.png"),
            "refused attempt must not mutate the icon"
        );

        // The steward still can.
        submit_9033(&state, &tenant, &owner, "https://example.com/owner.png")
            .await
            .expect("the steward must retain icon control");
        assert_eq!(
            stored_icon(&state, &tenant).await.as_deref(),
            Some("https://example.com/owner.png")
        );
    }

    /// Closed relay (`require_relay_membership = true`): admin/owner only —
    /// a plain member and a roleless key are refused even though the
    /// community also *looks* rosterless-then-stewarded to the open-relay
    /// branch. Together with the open-relay test this kills the inverted-flag
    /// mutant: no assignment of the flag satisfies both.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn closed_relay_9033_still_requires_admin_or_owner() {
        let host = format!("icon-gate-closed-{}.example", uuid::Uuid::new_v4().simple());
        let (state, tenant) = workspace_profile_test_state(&host, true).await;
        let roleless = Keys::generate();
        let member = Keys::generate();
        let admin = Keys::generate();
        state
            .db
            .add_relay_member(
                tenant.community(),
                &member.public_key().to_hex(),
                "member",
                None,
            )
            .await
            .expect("seed member");
        state
            .db
            .add_relay_member(
                tenant.community(),
                &admin.public_key().to_hex(),
                "admin",
                None,
            )
            .await
            .expect("seed admin");

        for (keys, label) in [(&roleless, "roleless"), (&member, "member")] {
            let refused = submit_9033(&state, &tenant, keys, "https://evil.example/pwn.png").await;
            assert_eq!(
                refused,
                Err(RelayAdminError::Rejected(
                    "actor not authorized: must be admin or owner".to_string()
                )),
                "closed relay must refuse a {label} sender"
            );
        }
        assert_eq!(
            stored_icon(&state, &tenant).await,
            None,
            "refused attempts must not set an icon"
        );

        submit_9033(&state, &tenant, &admin, "https://example.com/closed.png")
            .await
            .expect("closed-relay admin must set the icon");
        assert_eq!(
            stored_icon(&state, &tenant).await.as_deref(),
            Some("https://example.com/closed.png")
        );
    }
}
