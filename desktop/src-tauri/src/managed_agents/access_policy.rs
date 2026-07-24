//! Distribution policy for managed-agent inbound author access.

use super::{
    validate_respond_to_allowlist, AgentDefinition, BackendKind, ManagedAgentRecord, RespondTo,
};

/// Internal packaging sets `BUZZ_BUILD_INTERNAL`; OSS/custom builds do not.
pub(crate) fn internal_build() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_INTERNAL").is_some()
}

pub(crate) fn owner_only_for_backend(backend: &BackendKind) -> bool {
    owner_only_for_backend_with_policy(backend, internal_build())
}

pub(crate) fn owner_only_for_backend_with_policy(backend: &BackendKind, internal: bool) -> bool {
    internal && *backend == BackendKind::Local
}

/// Normalize a persisted/projected instance. Provider instances remain
/// configurable because the internal policy applies only to local execution.
pub(crate) fn normalize_managed_agent_access(record: &mut ManagedAgentRecord) -> bool {
    normalize_managed_agent_access_with_policy(record, internal_build())
}

pub(crate) fn normalize_managed_agent_access_with_policy(
    record: &mut ManagedAgentRecord,
    internal: bool,
) -> bool {
    if !owner_only_for_backend_with_policy(&record.backend, internal) {
        return false;
    }
    let changed =
        record.respond_to != RespondTo::OwnerOnly || !record.respond_to_allowlist.is_empty();
    record.respond_to = RespondTo::OwnerOnly;
    record.respond_to_allowlist.clear();
    changed
}

/// Definitions are backend-neutral defaults. Internal builds store owner-only
/// defaults so every later local mint starts safe; provider instance policy is
/// still decided from its own backend at create/update/deploy time.
pub(crate) fn normalize_definition_access(record: &mut AgentDefinition) -> bool {
    normalize_definition_access_with_policy(record, internal_build())
}

pub(crate) fn normalize_definition_access_with_policy(
    record: &mut AgentDefinition,
    internal: bool,
) -> bool {
    if !internal {
        return false;
    }
    let changed = record.respond_to.is_some() || !record.respond_to_allowlist.is_empty();
    record.respond_to = None;
    record.respond_to_allowlist.clear();
    changed
}

pub(crate) fn resolve_create_access(
    backend: &BackendKind,
    requested_mode: Option<RespondTo>,
    requested_allowlist: &[String],
) -> Result<(Option<RespondTo>, Vec<String>), String> {
    resolve_create_access_with_policy(
        backend,
        requested_mode,
        requested_allowlist,
        internal_build(),
    )
}

pub(crate) fn resolve_create_access_with_policy(
    backend: &BackendKind,
    requested_mode: Option<RespondTo>,
    requested_allowlist: &[String],
    internal: bool,
) -> Result<(Option<RespondTo>, Vec<String>), String> {
    if owner_only_for_backend_with_policy(backend, internal) {
        return Ok((Some(RespondTo::OwnerOnly), Vec::new()));
    }
    let allowlist = validate_respond_to_allowlist(requested_allowlist)?;
    if requested_mode == Some(RespondTo::Allowlist) && allowlist.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".into(),
        );
    }
    Ok((requested_mode, allowlist))
}

pub(crate) fn apply_update_access(
    record: &mut ManagedAgentRecord,
    requested_mode: Option<RespondTo>,
    requested_allowlist: Option<&[String]>,
) -> Result<(), String> {
    apply_update_access_with_policy(
        record,
        requested_mode,
        requested_allowlist,
        internal_build(),
    )
}

pub(crate) fn apply_update_access_with_policy(
    record: &mut ManagedAgentRecord,
    requested_mode: Option<RespondTo>,
    requested_allowlist: Option<&[String]>,
    internal: bool,
) -> Result<(), String> {
    if owner_only_for_backend_with_policy(&record.backend, internal) {
        record.respond_to = RespondTo::OwnerOnly;
        record.respond_to_allowlist.clear();
        return Ok(());
    }

    let mode = requested_mode.unwrap_or(record.respond_to);
    let allowlist = match requested_allowlist {
        Some(list) => validate_respond_to_allowlist(list)?,
        None => record.respond_to_allowlist.clone(),
    };
    if mode == RespondTo::Allowlist && allowlist.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".into(),
        );
    }
    record.respond_to = mode;
    if requested_allowlist.is_some() {
        record.respond_to_allowlist = allowlist;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(backend: BackendKind) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
            "agent_command": "", "agent_args": [], "mcp_command": "",
            "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
            "updated_at": "", "last_started_at": null, "last_stopped_at": null,
            "last_exit_code": null, "last_error": null
        }))
        .unwrap();
        record.backend = backend;
        record.respond_to = RespondTo::Anyone;
        record.respond_to_allowlist = vec!["stale".into()];
        record
    }

    #[test]
    fn internal_create_clamps_local_but_not_provider() {
        let local = resolve_create_access_with_policy(
            &BackendKind::Local,
            Some(RespondTo::Anyone),
            &["bad".into()],
            true,
        )
        .unwrap();
        assert_eq!(local, (Some(RespondTo::OwnerOnly), Vec::new()));

        let provider = BackendKind::Provider {
            id: "p".into(),
            config: serde_json::json!({}),
        };
        let result =
            resolve_create_access_with_policy(&provider, Some(RespondTo::Anyone), &[], true)
                .unwrap();
        assert_eq!(result, (Some(RespondTo::Anyone), Vec::new()));
    }

    #[test]
    fn internal_update_clamps_local_but_not_provider() {
        let mut local = record(BackendKind::Local);
        apply_update_access_with_policy(&mut local, None, None, true).unwrap();
        assert_eq!(local.respond_to, RespondTo::OwnerOnly);
        assert!(local.respond_to_allowlist.is_empty());

        let mut provider = record(BackendKind::Provider {
            id: "p".into(),
            config: serde_json::json!({}),
        });
        apply_update_access_with_policy(&mut provider, None, None, true).unwrap();
        assert_eq!(provider.respond_to, RespondTo::Anyone);
        assert_eq!(provider.respond_to_allowlist, vec!["stale"]);
    }
}
