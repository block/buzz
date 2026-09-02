//! Canonical relay identities shared by runtime components.

use thiserror::Error;
use url::{Host, Url};

/// Errors returned while canonicalizing a relay URL for runtime identity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NormalizeRelayUrlError {
    /// The input is not a valid URL.
    #[error("invalid relay URL: {0}")]
    InvalidUrl(String),
    /// Relay sockets must use WebSocket schemes.
    #[error("relay URL scheme must be ws or wss")]
    InvalidScheme,
    /// Relay identity never includes user credentials.
    #[error("relay URL must not contain credentials")]
    Credentials,
    /// Relay identity never includes a fragment.
    #[error("relay URL must not contain a fragment")]
    Fragment,
    /// A relay URL requires a host.
    #[error("relay URL must contain a host")]
    MissingHost,
}

/// Canonicalize a WebSocket relay URL for use as a runtime identity key.
///
/// This is the sole normalizer for `(agent, relay)` process identity. It keeps
/// the WebSocket scheme, lowercases DNS hosts, folds all loopback spellings to
/// `127.0.0.1`, removes default ports and a root slash, and preserves non-root
/// paths and queries. It deliberately is **not** the NIP-42 AUTH comparison
/// helper in `buzz-auth`: AUTH validation is a security boundary with narrower
/// equivalence rules and must not be widened by runtime-key canonicalization.
///
/// Connection code may retain the configured URL; this canonical form is for
/// identity, receipts, status and deduplication.
pub fn normalize_relay_url(raw: &str) -> Result<String, NormalizeRelayUrlError> {
    let mut url = Url::parse(raw.trim())
        .map_err(|error| NormalizeRelayUrlError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err(NormalizeRelayUrlError::InvalidScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NormalizeRelayUrlError::Credentials);
    }
    if url.fragment().is_some() {
        return Err(NormalizeRelayUrlError::Fragment);
    }

    let host = url.host().ok_or(NormalizeRelayUrlError::MissingHost)?;
    let loopback = match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    if loopback {
        url.set_host(Some("127.0.0.1"))
            .map_err(|_| NormalizeRelayUrlError::MissingHost)?;
    } else if let Host::Domain(domain) = host {
        let lowercase = domain.to_ascii_lowercase();
        url.set_host(Some(&lowercase))
            .map_err(|_| NormalizeRelayUrlError::MissingHost)?;
    }

    let default_port = match url.scheme() {
        "ws" => Some(80),
        "wss" => Some(443),
        _ => None,
    };
    if url.port() == default_port {
        url.set_port(None)
            .map_err(|_| NormalizeRelayUrlError::InvalidScheme)?;
    }
    if url.path() == "/" {
        url.set_path("");
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

/// Errors from [`resolve_agent_dial_relay_url`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResolveAgentDialRelayUrlError {
    /// Either URL failed identity normalization.
    #[error(transparent)]
    Normalize(#[from] NormalizeRelayUrlError),
    /// Workspace and pair identity normalize to different communities.
    #[error(
        "refusing to dial workspace relay `{workspace_url}` for pair identity `{identity_url}`: \
         they normalize to different communities (`{workspace_identity}` vs `{pair_identity}`)"
    )]
    CommunityMismatch {
        /// Configured workspace relay spelling.
        workspace_url: String,
        /// Canonical pair identity URL.
        identity_url: String,
        /// Normalized form of `workspace_url`.
        workspace_identity: String,
        /// Normalized form of `identity_url`.
        pair_identity: String,
    },
}

/// Resolve the WebSocket URL a managed-agent child must dial.
///
/// `identity_url` is the canonical pair key from [`normalize_relay_url`]
/// (loopback spellings fold to `127.0.0.1` for dedup). That form is correct
/// for identity/receipts, but relays select communities from the literal HTTP
/// `Host` / authority — so dialing the canonical spelling while the desktop
/// joined as `localhost` lands the agent in a different (often empty) tenant
/// (#4888).
///
/// When the workspace URL normalizes to the same identity, return the
/// workspace spelling verbatim so Host-bound tenancy matches the UI. If the
/// two normalize differently, fail closed rather than silently attaching the
/// child to another community.
pub fn resolve_agent_dial_relay_url(
    workspace_url: &str,
    identity_url: &str,
) -> Result<String, ResolveAgentDialRelayUrlError> {
    let workspace_identity = normalize_relay_url(workspace_url)?;
    let pair_identity = normalize_relay_url(identity_url)?;
    if workspace_identity != pair_identity {
        return Err(ResolveAgentDialRelayUrlError::CommunityMismatch {
            workspace_url: workspace_url.to_string(),
            identity_url: identity_url.to_string(),
            workspace_identity,
            pair_identity,
        });
    }
    Ok(workspace_url.trim().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_spellings_have_one_identity() {
        let ipv6 = normalize_relay_url("wss://[::1]/").unwrap();
        let ipv4 = normalize_relay_url("wss://127.0.0.1/").unwrap();
        let localhost = normalize_relay_url("wss://localhost/").unwrap();
        assert_eq!(ipv6, ipv4);
        assert_eq!(ipv4, localhost);
        assert_eq!(localhost, "wss://127.0.0.1");
    }

    #[test]
    fn dial_preserves_localhost_when_identity_folded_to_loopback_ip() {
        // Regression for #4888: runtime identity stores 127.0.0.1, but the
        // child must dial the workspace's localhost authority.
        let identity = normalize_relay_url("ws://localhost:3000").unwrap();
        assert_eq!(identity, "ws://127.0.0.1:3000");
        assert_eq!(
            resolve_agent_dial_relay_url("ws://localhost:3000", &identity).unwrap(),
            "ws://localhost:3000"
        );
    }

    #[test]
    fn dial_preserves_ipv4_loopback_when_workspace_uses_it() {
        let identity = normalize_relay_url("ws://127.0.0.1:3000").unwrap();
        assert_eq!(
            resolve_agent_dial_relay_url("ws://127.0.0.1:3000", &identity).unwrap(),
            "ws://127.0.0.1:3000"
        );
    }

    #[test]
    fn dial_strips_trailing_slash_but_keeps_authority() {
        let identity = normalize_relay_url("ws://localhost:3000/").unwrap();
        assert_eq!(
            resolve_agent_dial_relay_url("ws://localhost:3000/", &identity).unwrap(),
            "ws://localhost:3000"
        );
    }

    #[test]
    fn dial_rejects_workspace_that_normalizes_to_a_different_community() {
        let identity = normalize_relay_url("wss://one.example").unwrap();
        let err = resolve_agent_dial_relay_url("wss://two.example", &identity).unwrap_err();
        assert!(
            matches!(err, ResolveAgentDialRelayUrlError::CommunityMismatch { .. }),
            "expected fail-closed mismatch, got: {err}"
        );
    }

    #[test]
    fn removing_authority_preservation_would_break_localhost_dial() {
        // Falsifiable guard: dialing the identity URL is exactly the #4888 bug.
        let workspace = "ws://localhost:3000";
        let identity = normalize_relay_url(workspace).unwrap();
        let dial = resolve_agent_dial_relay_url(workspace, &identity).unwrap();
        assert_ne!(
            dial, identity,
            "dial must not collapse to the canonical identity spelling for localhost"
        );
        assert_eq!(dial, workspace);
    }

    #[test]
    fn canonicalizes_only_identity_equivalences() {
        assert_eq!(
            normalize_relay_url(" WSS://Relay.Example:443/ ").unwrap(),
            "wss://relay.example"
        );
        assert_eq!(
            normalize_relay_url("ws://relay.example:8080/community/?x=1").unwrap(),
            "ws://relay.example:8080/community/?x=1"
        );
    }

    #[test]
    fn rejects_non_relay_and_ambiguous_urls() {
        assert_eq!(
            normalize_relay_url("https://relay.example").unwrap_err(),
            NormalizeRelayUrlError::InvalidScheme
        );
        assert_eq!(
            normalize_relay_url("wss://user@relay.example").unwrap_err(),
            NormalizeRelayUrlError::Credentials
        );
        assert_eq!(
            normalize_relay_url("wss://relay.example/#x").unwrap_err(),
            NormalizeRelayUrlError::Fragment
        );
    }
}
