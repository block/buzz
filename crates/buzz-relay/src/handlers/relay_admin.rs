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

use std::sync::Arc;

use nostr::Event;
use tracing::{info, warn};

use sprout_core::kind::{
    RELAY_ADMIN_ADD_MEMBER, RELAY_ADMIN_CHANGE_ROLE, RELAY_ADMIN_REMOVE_MEMBER,
};
use sprout_db::relay_members::RemoveResult;

use crate::handlers::side_effects::{
    publish_nip43_member_added, publish_nip43_member_removed, publish_nip43_membership_list,
};
use crate::state::AppState;

// ── Tag extraction helpers ────────────────────────────────────────────────────

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

// ── Public handler ────────────────────────────────────────────────────────────

/// Validate and execute a relay admin command (kinds 9030–9032).
///
/// The handler:
/// 1. Extracts the target pubkey from the `["p", ...]` tag.
/// 2. Extracts the role from the `["role", ...]` tag (kinds 9030 and 9032).
/// 3. Looks up the sender's current role in `relay_members`.
/// 4. Enforces the permission matrix.
/// 5. Applies the change via the DB.
///
/// Returns `Ok(())` on success.  Returns `Err(msg)` — where `msg` is a
/// human-readable rejection reason — on any validation failure.
pub async fn handle_relay_admin_event(state: &Arc<AppState>, event: &Event) -> Result<(), String> {
    let kind = event.kind.as_u16() as u32;
    let sender_hex = event.pubkey.to_hex();

    // ── Replay protection: reject events outside ±120s of now ────────────
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

    // ── Extract target pubkey ─────────────────────────────────────────────
    let target_hex = extract_p_tag_hex(event)
        .ok_or_else(|| "missing or invalid p tag".to_string())?
        .to_ascii_lowercase();

    // ── Look up sender's relay role ───────────────────────────────────────
    let sender_member = state
        .db
        .get_relay_member(&sender_hex)
        .await
        .map_err(|e| format!("database error: {e}"))?;

    let sender_role = sender_member
        .as_ref()
        .map(|m| m.role.as_str())
        .unwrap_or("");

    // ── Dispatch by kind ──────────────────────────────────────────────────
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
                .add_relay_member(&target_hex, &role, Some(&sender_hex))
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
                if let Err(e) = publish_nip43_member_added(state, &target_hex).await {
                    warn!(error = %e, "failed to publish NIP-43 member added event");
                }
                if let Err(e) = publish_nip43_membership_list(state).await {
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
                    .remove_relay_member_if_role(&target_hex, "member")
                    .await
                    .map_err(|e| format!("database error: {e}"))?
            } else {
                // Owner path — atomic delete that refuses to remove other owners.
                state
                    .db
                    .remove_relay_member(&target_hex)
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

            if let Err(e) = publish_nip43_member_removed(state, &target_hex).await {
                warn!(error = %e, "failed to publish NIP-43 member removed event");
            }
            if let Err(e) = publish_nip43_membership_list(state).await {
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
                .update_relay_member_role(&target_hex, &new_role)
                .await
                .map_err(|e| format!("database error: {e}"))?;

            if !updated {
                // Distinguish "owner (protected)" from "doesn't exist"
                let exists = state
                    .db
                    .get_relay_member(&target_hex)
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

            if let Err(e) = publish_nip43_membership_list(state).await {
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

    // ── extract_p_tag_hex ─────────────────────────────────────────────────

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

    // ── extract_tag_value ─────────────────────────────────────────────────

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
}
