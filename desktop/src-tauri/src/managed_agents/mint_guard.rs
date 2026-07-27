//! Mint guard for managed-agent instances (#2515).

use super::ManagedAgentRecord;

/// Refuse minting a record whose name matches an existing instance in the
/// same relay scope (same pin, or both unpinned). A second keypair for one
/// @Name on a relay is exactly the duplicate-identity state from #2515:
/// mentions resolve to one pubkey while the other instance answers. The same
/// name on a different relay stays allowed — one identity per agent per
/// community.
pub(crate) fn ensure_no_duplicate_instance(
    records: &[ManagedAgentRecord],
    name: &str,
    relay_url: &str,
) -> Result<(), String> {
    let relay = relay_url.trim();
    let Some(existing) = records.iter().find(|record| {
        !record.pubkey.is_empty()
            && record.name.trim().eq_ignore_ascii_case(name)
            && record.relay_url.trim() == relay
    }) else {
        return Ok(());
    };
    let scope = if relay.is_empty() {
        "the workspace relay"
    } else {
        relay
    };
    Err(format!(
        "agent '{name}' already has an instance on {scope} (pubkey {}) — start that instance instead, or create the new one under a different name or relay",
        existing.pubkey
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, relay_url: &str) -> ManagedAgentRecord {
        serde_json::from_str(&format!(
            r#"{{
                "pubkey": "{}",
                "name": "{name}",
                "relay_url": "{relay_url}",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": "",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }}"#,
            "aa".repeat(32)
        ))
        .unwrap()
    }

    #[test]
    fn same_name_same_relay_is_refused() {
        let records = [record("Neo", "wss://one.example")];
        assert!(ensure_no_duplicate_instance(&records, "Neo", "wss://one.example").is_err());
    }

    #[test]
    fn name_match_is_case_insensitive_and_trimmed() {
        let records = [record(" Neo ", "wss://one.example")];
        assert!(ensure_no_duplicate_instance(&records, "neo", "wss://one.example").is_err());
    }

    #[test]
    fn same_name_different_relay_is_allowed() {
        // One identity per agent per community: a second instance on another
        // relay is the intended multi-community shape.
        let records = [record("Neo", "wss://one.example")];
        assert!(ensure_no_duplicate_instance(&records, "Neo", "wss://two.example").is_ok());
    }

    #[test]
    fn both_unpinned_share_the_workspace_scope() {
        let records = [record("Neo", "")];
        let error = ensure_no_duplicate_instance(&records, "Neo", "").unwrap_err();
        assert!(error.contains("the workspace relay"));
    }

    #[test]
    fn key_less_definitions_do_not_block_minting() {
        let mut definition = record("Neo", "wss://one.example");
        definition.pubkey = String::new();
        assert!(ensure_no_duplicate_instance(&[definition], "Neo", "wss://one.example").is_ok());
    }

    #[test]
    fn different_name_is_allowed() {
        let records = [record("Neo", "wss://one.example")];
        assert!(ensure_no_duplicate_instance(&records, "Trinity", "wss://one.example").is_ok());
    }
}
