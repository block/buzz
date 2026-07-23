//! Distribution policy for relays used by local managed agents.

use base64::Engine as _;

use super::{BackendKind, ManagedAgentRecord};

const MISSING_ALLOWLIST_ERROR: &str =
    "internal build has no valid local-agent relay allowlist; contact your Buzz administrator";

fn normalized_origin(raw: &str) -> Result<String, String> {
    let normalized =
        buzz_core_pkg::relay::normalize_relay_url(raw).map_err(|error| error.to_string())?;
    let url =
        url::Url::parse(&normalized).map_err(|error| format!("invalid relay URL: {error}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "relay URL must contain a host".to_string())?;
    let mut origin = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Ok(origin)
}

fn baked_allowlist() -> Result<Vec<String>, String> {
    let encoded = option_env!("BUZZ_DESKTOP_BUILD_LOCAL_AGENT_RELAY_ALLOWLIST")
        .ok_or_else(|| MISSING_ALLOWLIST_ERROR.to_string())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| MISSING_ALLOWLIST_ERROR.to_string())?;
    let raw = String::from_utf8(decoded).map_err(|_| MISSING_ALLOWLIST_ERROR.to_string())?;
    parse_allowlist(&raw)
}

fn parse_allowlist(raw: &str) -> Result<Vec<String>, String> {
    let entries: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(normalized_origin)
        .collect::<Result<_, _>>()?;
    if entries.is_empty() {
        return Err(MISSING_ALLOWLIST_ERROR.to_string());
    }
    Ok(entries)
}

pub(crate) fn validate_local_agent_relay(
    backend: &BackendKind,
    relay_url: &str,
) -> Result<(), String> {
    validate_local_agent_relay_with_policy(
        backend,
        relay_url,
        super::internal_build(),
        baked_allowlist,
    )
}

fn validate_local_agent_relay_with_policy<F>(
    backend: &BackendKind,
    relay_url: &str,
    internal: bool,
    allowlist: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<Vec<String>, String>,
{
    if !internal || *backend != BackendKind::Local {
        return Ok(());
    }
    let requested = normalized_origin(relay_url)
        .map_err(|error| format!("local agent relay is invalid: {error}"))?;
    if allowlist()?.iter().any(|allowed| allowed == &requested) {
        return Ok(());
    }
    Err(format!(
        "local agents in this internal build cannot use relay {requested}"
    ))
}

/// Validate a legacy explicit record pin at persistence boundaries. Empty pins
/// are workspace-relative and are checked when their effective relay is known.
pub(crate) fn validate_managed_agent_relay_pin(record: &ManagedAgentRecord) -> Result<(), String> {
    if record.relay_url.trim().is_empty() {
        return Ok(());
    }
    validate_local_agent_relay(&record.backend, &record.relay_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist() -> Result<Vec<String>, String> {
        parse_allowlist(" WSS://Buzz.Block.Builderlab.XYZ:443/\n")
    }

    #[test]
    fn internal_local_policy_matches_exact_normalized_origin() {
        assert!(validate_local_agent_relay_with_policy(
            &BackendKind::Local,
            "wss://buzz.block.builderlab.xyz/channels",
            true,
            allowlist,
        )
        .is_ok());
        assert!(validate_local_agent_relay_with_policy(
            &BackendKind::Local,
            "wss://public.example",
            true,
            allowlist,
        )
        .is_err());
    }

    #[test]
    fn provider_and_oss_relays_remain_configurable() {
        let provider = BackendKind::Provider {
            id: "provider".into(),
            config: serde_json::json!({}),
        };
        let unavailable = || Err("allowlist must not be read".into());
        assert!(validate_local_agent_relay_with_policy(
            &BackendKind::Local,
            "wss://public.example",
            false,
            unavailable,
        )
        .is_ok());
        assert!(validate_local_agent_relay_with_policy(
            &provider,
            "wss://public.example",
            true,
            unavailable,
        )
        .is_ok());
    }

    #[test]
    fn internal_policy_fails_closed_on_missing_empty_or_malformed_allowlist() {
        for allowlist in [
            || Err(MISSING_ALLOWLIST_ERROR.into()),
            || parse_allowlist(" \n"),
            || parse_allowlist("https://not-a-relay.example"),
        ] {
            assert!(validate_local_agent_relay_with_policy(
                &BackendKind::Local,
                "wss://buzz.block.builderlab.xyz",
                true,
                allowlist,
            )
            .is_err());
        }
    }
}
