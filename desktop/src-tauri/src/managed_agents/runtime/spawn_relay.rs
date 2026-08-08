use crate::managed_agents::ManagedAgentRuntimeKey;

/// Split a spawn's relay input into its two roles: the canonical pair key
/// (identity) and the URL the child actually connects with.
///
/// The runtime key canonicalizes via `buzz_core::relay::normalize_relay_url`,
/// which folds every loopback spelling to `127.0.0.1`. That is correct for
/// identity, receipts, log paths, and dedup — and explicitly NOT for
/// connections: the normalizer's own contract says "Connection code may
/// retain the configured URL; this canonical form is for identity, receipts,
/// status and deduplication." On a per-host multi-tenant relay,
/// `ws://localhost:3100` and `ws://127.0.0.1:3100` resolve to *different
/// communities*, so handing the canonical form to the child connects the
/// agent to a different (typically empty) community than the desktop's own
/// traffic — the harness logs "discovered 0 channel(s)" and idles while the
/// UI writes memberships to a tenant the agent never sees. The child
/// therefore gets the configured URL byte-for-byte.
pub(super) fn spawn_relay_roles(
    pubkey: String,
    configured_relay_url: &str,
) -> Result<(ManagedAgentRuntimeKey, String), String> {
    let key = ManagedAgentRuntimeKey::new(pubkey, configured_relay_url)?;
    Ok((key, configured_relay_url.to_string()))
}

#[cfg(test)]
mod tests {
    use super::spawn_relay_roles;

    #[test]
    fn spawn_relay_roles_keeps_configured_url_for_the_child() {
        // Identity canonicalizes loopback spellings to 127.0.0.1; the connection
        // URL handed to the child must stay exactly as configured. On a per-host
        // multi-tenant relay `ws://localhost:3100` and `ws://127.0.0.1:3100` are
        // different communities, so folding the child's URL strands the agent in
        // an empty parallel tenant ("discovered 0 channel(s)", idles) while the
        // desktop's own traffic — and the memberships the user creates in the UI —
        // land under the configured host.
        let (key, connection) =
            spawn_relay_roles("a".repeat(64), "ws://localhost:3100").expect("valid relay URL");
        assert_eq!(key.relay_url, "ws://127.0.0.1:3100");
        assert_eq!(connection, "ws://localhost:3100");
    }

    #[test]
    fn spawn_relay_roles_agree_for_non_loopback_hosts() {
        // For real deployments the two roles agree (modulo canonical lowercasing),
        // which is why this bug was invisible against hosted relays.
        let (key, connection) =
            spawn_relay_roles("b".repeat(64), "wss://relay.example.com").expect("valid relay URL");
        assert_eq!(key.relay_url, "wss://relay.example.com");
        assert_eq!(connection, "wss://relay.example.com");
    }
}
