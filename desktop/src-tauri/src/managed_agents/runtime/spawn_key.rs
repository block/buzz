//! Production spawn-key derivation — split from `runtime.rs` (file-size
//! guard). The regression tests live beside the function so they exercise
//! the exact seam production spawn keys on.

use crate::managed_agents::types::ManagedAgentRecord;
use crate::managed_agents::ManagedAgentRuntimeKey;

/// The one production derivation from a caller-bound workspace relay to the
/// runtime-pair key `start_managed_agent_process` spawns and persists under.
/// Extracted so the regression suite exercises the exact seam production
/// uses: a mutation that keys the spawn to anything but the bound value now
/// fails the tests below, instead of leaving them green while a painted
/// guard watches the door.
pub(crate) fn bound_runtime_key(
    record: &ManagedAgentRecord,
    workspace_relay: &crate::relay::ScopedWorkspaceRelay,
) -> Result<ManagedAgentRuntimeKey, String> {
    let relay_url =
        crate::relay::effective_agent_relay_url(&record.relay_url, workspace_relay.as_str());
    ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url)
}

#[cfg(test)]
mod tests {
    use super::bound_runtime_key;
    use crate::managed_agents::types::ManagedAgentRecord;

    fn record(pubkey: &str, relay_url: &str) -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": pubkey,
            "name": "test",
            "private_key_nsec": "nsec1fake",
            "relay_url": relay_url,
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "",
            "updated_at": ""
        }))
        .expect("record fixture")
    }

    #[test]
    fn production_spawn_key_derives_from_the_bound_relay_not_the_post_switch_workspace() {
        // Round-8 regression: the previous test reconstructed the key
        // derivation by hand, so hard-coding a wrong tenant inside production
        // spawn stayed green. This calls `bound_runtime_key` — the exact
        // function `start_managed_agent_process` keys its spawn, receipt, and
        // runtimes-map insert on — so that mutation now fails here.
        let record = record(&"aa".repeat(32), ""); // never-pinned record
        let mut workspace = "wss://tenant-a.example".to_string();
        let bound = crate::relay::bind_expected_relay_scope(
            Some("wss://tenant-a.example"),
            workspace.clone(),
        )
        .expect("scope matches at bind time");
        workspace = "wss://tenant-b.example".to_string(); // the switch lands post-check

        let key = bound_runtime_key(&record, &bound).expect("keyable record and relay");
        assert_eq!(key.relay_url, "wss://tenant-a.example");
        assert_eq!(key.pubkey, "aa".repeat(32));
        assert_ne!(
            key.relay_url, workspace,
            "the production spawn key must be unrepresentable for the post-switch tenant"
        );
    }

    #[test]
    fn production_spawn_key_ignores_a_legacy_record_pin() {
        // agents-everywhere (#2122): the stored per-record pin never
        // contributes; the bound workspace relay is the only input. Pins the
        // same contract at the production seam so a regression re-honoring
        // the pin fails loudly.
        let record = record(&"bb".repeat(32), "wss://stale-pin.example");
        let bound =
            crate::relay::bind_expected_relay_scope(None, "wss://tenant-a.example".to_string())
                .expect("unscoped bind");

        let key = bound_runtime_key(&record, &bound).expect("keyable record and relay");
        assert_eq!(key.relay_url, "wss://tenant-a.example");
    }
}
