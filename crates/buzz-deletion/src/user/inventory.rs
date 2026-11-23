//! Inventory preview and shared subject parsing for preview and submission.
use anyhow::Result;

pub(super) async fn generate(
    db: &buzz_db::Db,
    pubkey: &str,
    relay_pubkey: Option<&str>,
) -> Result<serde_json::Value> {
    Ok(db
        .user_deletion_store()
        .generate_inventory_with_relay(pubkey, relay_pubkey)
        .await?)
}

pub(super) fn parse_subject(input: &str) -> std::result::Result<String, String> {
    if input == "demo" {
        return Ok(input.to_owned());
    }
    buzz_db::user_deletion::normalize_user_deletion_pubkey(input).map_err(|error| error.to_string())
}

// Filtered discovery and execution must use the same attribution boundary.
// Older all-community manifests retain their existing runtime configuration.
pub(super) fn saved_relay_key(
    manifest: Option<&serde_json::Value>,
    supplied: Option<&str>,
) -> Result<Option<String>> {
    Ok(buzz_db::user_deletion::UserDeletionInventory::resolve_relay_pubkey(manifest, supplied)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filtered_inventory_freezes_attribution_but_legacy_requests_keep_runtime_choice() {
        let key = nostr::Keys::generate().public_key().to_hex();
        let other = nostr::Keys::generate().public_key().to_hex();
        let saved = json!({"scope":"affected_communities", "discovery_relay_pubkey":key});
        assert_eq!(
            saved_relay_key(Some(&saved), None).unwrap(),
            Some(key.clone())
        );
        assert_eq!(
            saved_relay_key(Some(&saved), Some(&key.to_uppercase())).unwrap(),
            Some(key.clone())
        );
        assert!(saved_relay_key(Some(&saved), Some(&other)).is_err());
        let keyless = json!({"scope":"affected_communities"});
        assert_eq!(saved_relay_key(Some(&keyless), None).unwrap(), None);
        assert!(saved_relay_key(Some(&keyless), Some(&key)).is_err());
        let legacy = json!({"scope":"all_stored_communities"});
        assert_eq!(
            saved_relay_key(Some(&legacy), Some(&key)).unwrap(),
            Some(key)
        );
    }
}
