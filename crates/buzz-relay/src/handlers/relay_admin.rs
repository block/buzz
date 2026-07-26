//! Relay-level admin command handler (kinds 9030–9034).
//!
//! These events are processed directly — they mutate normalized relay state
//! and return without being stored as regular Nostr events.
//!
//! ## Permission matrix
//!
//! | Kind | Operation       | Required sender role |
//! |------|-----------------|----------------------|
//! | 9030 | Add member      | admin or owner       |
//! | 9031 | Remove member   | admin or owner       |
//! | 9032 | Change role     | owner only           |
//! | 9033 | Set workspace profile (icon) | admin or owner |
//! | 9034 | Curate a sticker pack revision | admin or owner |

use std::sync::Arc;

use nostr::Event;
use tracing::{info, warn};

use buzz_core::kind::{
    RELAY_ADMIN_ADD_MEMBER, RELAY_ADMIN_CHANGE_ROLE, RELAY_ADMIN_CURATE_STICKER_PACK,
    RELAY_ADMIN_REMOVE_MEMBER, RELAY_ADMIN_SET_WORKSPACE_PROFILE,
};
use buzz_core::tenant::TenantContext;
use buzz_db::relay_members::RemoveResult;

use crate::handlers::side_effects::{
    publish_nip43_member_added, publish_nip43_member_removed, publish_nip43_membership_list,
    publish_sticker_catalog_mutation,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StickerCatalogCommand {
    coordinate: String,
    pack_author: [u8; 32],
    identifier: String,
    approved_event_id: Option<[u8; 32]>,
}

fn decode_lower_hex_32(value: &str, label: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{label} must be 64 lowercase hex characters"));
    }
    let decoded = hex::decode(value).map_err(|_| format!("invalid {label}"))?;
    decoded
        .try_into()
        .map_err(|_| format!("{label} must decode to 32 bytes"))
}

/// Parse kind:9034. Approval deliberately uses a three-field `a` tag so the
/// admin pins a reviewed revision; removal uses exactly `["a", coordinate]`.
fn parse_sticker_catalog_command(event: &Event) -> Result<StickerCatalogCommand, String> {
    let action_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|part| part == "action"))
        .collect();
    if action_tags.len() != 1 {
        return Err("expected exactly one action tag".to_string());
    }
    let action = action_tags[0].as_slice();
    if action.len() != 2 || !matches!(action[1].as_str(), "approve" | "remove") {
        return Err("action tag must be exactly [action, approve|remove]".to_string());
    }

    let address_tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|part| part == "a"))
        .collect();
    if address_tags.len() != 1 {
        return Err("expected exactly one a tag".to_string());
    }
    let address = address_tags[0].as_slice();
    let expected_len = if action[1] == "approve" { 3 } else { 2 };
    if address.len() != expected_len {
        return Err(if action[1] == "approve" {
            "approve requires exactly [a, coordinate, event_id]".to_string()
        } else {
            "remove requires exactly [a, coordinate]".to_string()
        });
    }

    let coordinate = address[1].to_string();
    if coordinate.len() > 512 {
        return Err("sticker pack coordinate is too long".to_string());
    }
    let mut parts = coordinate.splitn(3, ':');
    if parts.next() != Some("30031") {
        return Err("sticker pack coordinate must use kind 30031".to_string());
    }
    let author_hex = parts
        .next()
        .ok_or_else(|| "sticker pack coordinate is missing author".to_string())?;
    let identifier = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "sticker pack coordinate is missing identifier".to_string())?
        .to_string();
    if identifier.len() > 80
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "sticker pack identifier must be 1..80 ASCII alnum, dot, underscore, or dash"
                .to_string(),
        );
    }
    let pack_author = decode_lower_hex_32(author_hex, "sticker pack author")?;
    // Rebuilding catches extra/missing delimiters and non-canonical spellings.
    if coordinate != format!("30031:{author_hex}:{identifier}") {
        return Err("sticker pack coordinate is not canonical".to_string());
    }

    let approved_event_id = if action[1] == "approve" {
        Some(decode_lower_hex_32(
            address[2].as_str(),
            "approved event id",
        )?)
    } else {
        None
    };

    Ok(StickerCatalogCommand {
        coordinate,
        pack_author,
        identifier,
        approved_event_id,
    })
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

/// Validate and execute a relay admin command (kinds 9030–9034).
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
pub async fn handle_relay_admin_event(
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
        if sender_role != "admin" && sender_role != "owner" {
            return Err("actor not authorized: must be admin or owner".to_string());
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

    // kind:9034 — Curate an exact sticker pack revision. Handled before
    // p-tag extraction because it targets an addressable event, not a member.
    if kind == RELAY_ADMIN_CURATE_STICKER_PACK {
        if sender_role != "admin" && sender_role != "owner" {
            return Err("actor not authorized: must be admin or owner".to_string());
        }
        let command = parse_sticker_catalog_command(event)?;
        let actor = event.pubkey.to_bytes();
        let (changed, approval_count) = publish_sticker_catalog_mutation(
            tenant,
            state,
            &command.coordinate,
            command.pack_author.as_slice(),
            &command.identifier,
            command.approved_event_id.as_ref().map(<[u8; 32]>::as_slice),
            actor.as_slice(),
        )
        .await
        .map_err(|error| format!("failed to update sticker catalog: {error}"))?;

        info!(
            sender = %sender_hex,
            coordinate = %command.coordinate,
            changed,
            approval_count,
            "workspace sticker catalog updated"
        );
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

    #[test]
    fn sticker_catalog_approve_pins_exact_revision() {
        let author = "a".repeat(64);
        let event_id = "b".repeat(64);
        let coordinate = format!("30031:{author}:animals");
        let event = make_test_event(
            9034,
            vec![
                vec!["action", "approve"],
                vec![
                    "a",
                    Box::leak(coordinate.clone().into_boxed_str()),
                    Box::leak(event_id.into_boxed_str()),
                ],
            ],
        );
        let parsed = parse_sticker_catalog_command(&event).expect("valid command");
        assert_eq!(parsed.coordinate, coordinate);
        assert_eq!(parsed.pack_author, [0xaa; 32]);
        assert_eq!(parsed.approved_event_id, Some([0xbb; 32]));
    }

    #[test]
    fn sticker_catalog_remove_requires_coordinate_only() {
        let coordinate = format!("30031:{}:animals", "a".repeat(64));
        let valid = make_test_event(
            9034,
            vec![
                vec!["action", "remove"],
                vec!["a", Box::leak(coordinate.into_boxed_str())],
            ],
        );
        assert!(parse_sticker_catalog_command(&valid).is_ok());

        let invalid = make_test_event(
            9034,
            vec![
                vec!["action", "remove"],
                vec!["a", "30031:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:animals", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
            ],
        );
        assert!(parse_sticker_catalog_command(&invalid).is_err());
    }

    #[test]
    fn sticker_catalog_rejects_uppercase_author_and_duplicate_tags() {
        let uppercase = make_test_event(
            9034,
            vec![
                vec!["action", "remove"],
                vec!["a", "30031:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:animals"],
            ],
        );
        assert!(parse_sticker_catalog_command(&uppercase).is_err());

        let duplicate = make_test_event(
            9034,
            vec![
                vec!["action", "remove"],
                vec!["action", "remove"],
                vec!["a", "30031:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:animals"],
            ],
        );
        assert!(parse_sticker_catalog_command(&duplicate).is_err());
    }
}
