//! NIP-IA archive-request construction for deleted agents, split from
//! `agents.rs` (file-size guard). Pure given the keys, so the auth-tag and
//! reason-code wiring stays unit-testable without an `AppHandle`.

/// Build and sign the NIP-IA `kind:9035` archive request enqueued when an
/// agent is deleted. Pure given the keys — unit-testable without an
/// `AppHandle`. Reuses the same wire builder as the GUI's Archive action
/// (`events::build_archive_identity_request`); the machine-readable reason is
/// `retired` (NIP-IA suggested code for a deliberately decommissioned key).
///
/// The owner auth tag is minted locally from the same keys used to sign the
/// request, avoiding a network fetch while the managed-agent store lock is
/// held. The relay still independently verifies it against the agent's live
/// kind:0.
pub(super) fn build_agent_archive_request(
    keys: &nostr::Keys,
    agent_pubkey: &str,
) -> Result<nostr::Event, String> {
    let auth_tag = if keys
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(agent_pubkey)
    {
        None
    } else {
        let agent = nostr::PublicKey::from_hex(agent_pubkey)
            .map_err(|e| format!("invalid agent pubkey: {e}"))?;
        let tag_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(keys, &agent, "")
            .map_err(|e| format!("failed to build owner auth tag: {e}"))?;
        let parts: Vec<String> = serde_json::from_str(&tag_json)
            .map_err(|e| format!("failed to parse owner auth tag: {e}"))?;
        Some(
            <[String; 4]>::try_from(parts)
                .map_err(|_| "owner auth tag must have four elements".to_string())?,
        )
    };
    crate::events::build_archive_identity_request(
        agent_pubkey,
        "",
        Some("retired"),
        None,
        auth_tag.as_ref(),
    )?
    .sign_with_keys(keys)
    .map_err(|e| format!("failed to sign archive request: {e}"))
}
