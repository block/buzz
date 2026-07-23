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

pub(crate) fn validate_effective_local_agent_relay(
    backend: &BackendKind,
    relay_pin: &str,
    workspace_relay_url: &str,
) -> Result<(), String> {
    validate_effective_local_agent_relay_with_policy(
        backend,
        relay_pin,
        workspace_relay_url,
        super::internal_build(),
        baked_allowlist,
    )
}

fn validate_effective_local_agent_relay_with_policy<F>(
    backend: &BackendKind,
    relay_pin: &str,
    workspace_relay_url: &str,
    internal: bool,
    allowlist: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<Vec<String>, String>,
{
    let effective = crate::relay::effective_agent_relay_url(relay_pin, workspace_relay_url);
    validate_local_agent_relay_with_policy(backend, &effective, internal, allowlist)
}

/// Validate a legacy explicit record pin at persistence boundaries. Empty pins
/// are workspace-relative and are checked when their effective relay is known.
pub(crate) fn validate_managed_agent_relay_pin(record: &ManagedAgentRecord) -> Result<(), String> {
    if record.relay_url.trim().is_empty() {
        return Ok(());
    }
    validate_local_agent_relay(&record.backend, &record.relay_url)
}

/// Load and validate local members only when the internal-build policy applies.
/// OSS builds must not make ordinary membership depend on managed-agent store health.
pub(crate) fn validate_local_agent_members_from_store<F>(
    pubkeys: &[String],
    relay_url: &str,
    load: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<Vec<ManagedAgentRecord>, String>,
{
    validate_local_agent_members_from_store_with_policy(
        pubkeys,
        relay_url,
        super::internal_build(),
        load,
    )
}

fn validate_local_agent_members_from_store_with_policy<F>(
    pubkeys: &[String],
    relay_url: &str,
    internal: bool,
    load: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<Vec<ManagedAgentRecord>, String>,
{
    if !internal {
        return Ok(());
    }
    validate_local_agent_members(&load()?, pubkeys, relay_url)
}

/// Reject attachment of locally managed agents to a disallowed effective relay.
/// Unknown pubkeys and provider-backed records are outside this policy.
pub(crate) fn validate_local_agent_members(
    records: &[ManagedAgentRecord],
    pubkeys: &[String],
    relay_url: &str,
) -> Result<(), String> {
    validate_local_agent_members_with(records, pubkeys, |backend| {
        validate_local_agent_relay(backend, relay_url)
    })
}

fn validate_local_agent_members_with<F>(
    records: &[ManagedAgentRecord],
    pubkeys: &[String],
    validate: F,
) -> Result<(), String>
where
    F: Fn(&BackendKind) -> Result<(), String>,
{
    for pubkey in pubkeys {
        if let Some(record) = records.iter().find(|record| {
            record.pubkey.eq_ignore_ascii_case(pubkey) && record.backend == BackendKind::Local
        }) {
            validate(&record.backend)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowlist() -> Result<Vec<String>, String> {
        parse_allowlist(" WSS://Buzz.Block.Builderlab.XYZ:443/\n")
    }

    fn record(pubkey: &str, backend: BackendKind) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": pubkey,
            "name": "test-agent",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
        .expect("sample record");
        record.backend = backend;
        record
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
    fn empty_pin_validates_the_effective_workspace_relay() {
        assert!(validate_effective_local_agent_relay_with_policy(
            &BackendKind::Local,
            "",
            "wss://public.example",
            true,
            allowlist,
        )
        .is_err());
        assert!(validate_effective_local_agent_relay_with_policy(
            &BackendKind::Local,
            "",
            "wss://buzz.block.builderlab.xyz",
            true,
            allowlist,
        )
        .is_ok());
    }

    #[test]
    fn effective_relay_validation_skips_oss_and_provider_backends() {
        let provider = BackendKind::Provider {
            id: "provider".into(),
            config: serde_json::json!({}),
        };
        for (backend, internal) in [(&BackendKind::Local, false), (&provider, true)] {
            assert!(validate_effective_local_agent_relay_with_policy(
                backend,
                "",
                "not-even-a-relay",
                internal,
                || Err("allowlist must not load".into()),
            )
            .is_ok());
        }
    }

    #[test]
    fn member_enrollment_validates_only_matching_local_agents() {
        let provider = BackendKind::Provider {
            id: "provider".into(),
            config: serde_json::json!({}),
        };
        let records = vec![
            record("local", BackendKind::Local),
            record("provider", provider),
        ];
        let calls = std::cell::Cell::new(0);

        assert!(
            validate_local_agent_members_with(&records, &["LOCAL".into()], |_| {
                calls.set(calls.get() + 1);
                Err("blocked".to_string())
            })
            .is_err()
        );
        assert_eq!(calls.get(), 1);
        assert!(validate_local_agent_members_with(
            &records,
            &["provider".into(), "unknown".into()],
            |_| {
                calls.set(calls.get() + 1);
                Err("blocked".to_string())
            },
        )
        .is_ok());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn role_change_rejects_matching_local_agent_before_membership_emit() {
        let records = vec![record("local", BackendKind::Local)];
        let result = validate_local_agent_members_with(&records, &["LOCAL".into()], |backend| {
            validate_local_agent_relay_with_policy(backend, "wss://public.example", true, allowlist)
        });
        assert!(result.is_err());
    }

    #[test]
    fn oss_member_enrollment_does_not_load_the_agent_store() {
        let loads = std::cell::Cell::new(0);
        assert!(validate_local_agent_members_from_store_with_policy(
            &["human".into()],
            "not-even-a-relay",
            false,
            || {
                loads.set(loads.get() + 1);
                Err("broken store".into())
            },
        )
        .is_ok());
        assert_eq!(loads.get(), 0);
    }

    #[test]
    fn internal_member_enrollment_fails_loudly_on_broken_store() {
        assert!(validate_local_agent_members_from_store_with_policy(
            &["human".into()],
            "wss://buzz.block.builderlab.xyz",
            true,
            || Err("broken store".into()),
        )
        .is_err());
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
