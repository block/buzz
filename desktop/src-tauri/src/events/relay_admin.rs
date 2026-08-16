use nostr::{EventBuilder, Kind};

use super::{check_pubkey, tag};

/// Allowed relay member roles for NIP-43 admin commands.
const VALID_RELAY_ROLES: &[&str] = &["owner", "admin", "member"];

fn check_relay_role(role: &str) -> Result<(), String> {
    if !VALID_RELAY_ROLES.contains(&role) {
        return Err(format!(
            "invalid relay role \"{role}\" (expected one of: {})",
            VALID_RELAY_ROLES.join(", ")
        ));
    }
    Ok(())
}

/// Kind 9030 — add a pubkey to the relay member list.
pub fn build_relay_admin_add(target_pubkey: &str, role: &str) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    check_relay_role(role)?;
    let tags = vec![
        tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?,
        tag(vec!["role", role])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9030), "").tags(tags))
}

/// Kind 9031 — remove a pubkey from the relay member list.
pub fn build_relay_admin_remove(target_pubkey: &str) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    let tags = vec![tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?];
    Ok(EventBuilder::new(Kind::Custom(9031), "").tags(tags))
}

/// Kind 9032 — change the role of an existing relay member.
pub fn build_relay_admin_change_role(
    target_pubkey: &str,
    new_role: &str,
) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    check_relay_role(new_role)?;
    let tags = vec![
        tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?,
        tag(vec!["role", new_role])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9032), "").tags(tags))
}
