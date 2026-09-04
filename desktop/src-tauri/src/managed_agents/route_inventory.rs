use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::{
    discovery::dangling_harness_id,
    effective_config::{resolve_effective_config, ConfigSource, EffectiveConfigResult},
    known_acp_runtime, resolve_effective_harness_descriptor, AgentDefinition, GlobalAgentConfig,
    ManagedAgentRecord,
};
use crate::app_state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RouteSource {
    Definition,
    Global,
    InstanceLegacy,
    RecordRuntime,
    LinkedPersonaRuntime,
    CommandMapping,
    CommandOverrideUnmapped,
    DefaultCommandUnmapped,
    NotInSpawnEffectiveConfig,
    NoSafeToolCatalog,
    OrphanedInstance,
    DanglingHarness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RouteStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteField {
    value: Option<String>,
    source: RouteSource,
    status: RouteStatus,
}

impl RouteField {
    fn available(value: String, source: RouteSource) -> Self {
        Self {
            value: Some(value),
            source,
            status: RouteStatus::Available,
        }
    }

    fn unavailable(source: RouteSource) -> Self {
        Self {
            value: None,
            source,
            status: RouteStatus::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteListField {
    value: Option<Vec<String>>,
    source: RouteSource,
    status: RouteStatus,
}

impl RouteListField {
    fn unavailable(source: RouteSource) -> Self {
        Self {
            value: None,
            source,
            status: RouteStatus::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedAgentRouteInventoryEntry {
    identity: String,
    pubkey: String,
    runtime: RouteField,
    provider: RouteField,
    model: RouteField,
    effort: RouteField,
    tools: RouteListField,
}

fn config_source(source: ConfigSource) -> RouteSource {
    match source {
        ConfigSource::Definition => RouteSource::Definition,
        ConfigSource::Global => RouteSource::Global,
        ConfigSource::InstanceLegacy => RouteSource::InstanceLegacy,
    }
}

fn resolved_field(field: super::effective_config::ResolvedField<String>) -> RouteField {
    match field.value {
        Some(value) => RouteField::available(value, config_source(field.source)),
        None => RouteField::unavailable(config_source(field.source)),
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn runtime_field(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    effective_command: &str,
) -> RouteField {
    if non_blank(record.agent_command_override.as_deref()).is_some() {
        return known_acp_runtime(effective_command).map_or_else(
            || RouteField::unavailable(RouteSource::CommandOverrideUnmapped),
            |runtime| RouteField::available(runtime.id.to_string(), RouteSource::CommandMapping),
        );
    }

    if let Some(runtime) = non_blank(record.runtime.as_deref()) {
        return RouteField::available(runtime.to_string(), RouteSource::RecordRuntime);
    }

    if let Some(runtime) = record
        .persona_id
        .as_deref()
        .and_then(|persona_id| personas.iter().find(|persona| persona.id == persona_id))
        .and_then(|persona| non_blank(persona.runtime.as_deref()))
    {
        return RouteField::available(runtime.to_string(), RouteSource::LinkedPersonaRuntime);
    }

    known_acp_runtime(effective_command).map_or_else(
        || RouteField::unavailable(RouteSource::DefaultCommandUnmapped),
        |runtime| RouteField::available(runtime.id.to_string(), RouteSource::CommandMapping),
    )
}

pub(crate) fn build_managed_agent_route_inventory(
    records: &[ManagedAgentRecord],
    personas: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Result<Vec<ManagedAgentRouteInventoryEntry>, String> {
    records
        .iter()
        .map(|record| {
            let config = match resolve_effective_config(record, personas, global) {
                EffectiveConfigResult::Resolved(config) => config,
                EffectiveConfigResult::OrphanedInstance { .. } => {
                    return Ok(ManagedAgentRouteInventoryEntry {
                        identity: record.name.clone(),
                        pubkey: record.pubkey.clone(),
                        runtime: RouteField::unavailable(RouteSource::OrphanedInstance),
                        provider: RouteField::unavailable(RouteSource::OrphanedInstance),
                        model: RouteField::unavailable(RouteSource::OrphanedInstance),
                        effort: RouteField::unavailable(RouteSource::OrphanedInstance),
                        tools: RouteListField::unavailable(RouteSource::OrphanedInstance),
                    });
                }
            };
            let descriptor = match resolve_effective_harness_descriptor(record, personas, global) {
                Ok(descriptor) => descriptor,
                Err(error) if dangling_harness_id(&error).is_some() => {
                    return Ok(ManagedAgentRouteInventoryEntry {
                        identity: record.name.clone(),
                        pubkey: record.pubkey.clone(),
                        runtime: RouteField::unavailable(RouteSource::DanglingHarness),
                        provider: RouteField::unavailable(RouteSource::DanglingHarness),
                        model: RouteField::unavailable(RouteSource::DanglingHarness),
                        effort: RouteField::unavailable(RouteSource::DanglingHarness),
                        tools: RouteListField::unavailable(RouteSource::DanglingHarness),
                    });
                }
                Err(error) => return Err(error),
            };
            Ok(ManagedAgentRouteInventoryEntry {
                identity: record.name.clone(),
                pubkey: record.pubkey.clone(),
                runtime: runtime_field(record, personas, &descriptor.command),
                provider: resolved_field(config.provider),
                model: resolved_field(config.model),
                effort: RouteField::unavailable(RouteSource::NotInSpawnEffectiveConfig),
                tools: RouteListField::unavailable(RouteSource::NoSafeToolCatalog),
            })
        })
        .collect()
}

pub(crate) fn require_signing_identity_available<T>(
    identity: Result<T, String>,
) -> Result<(), String> {
    identity
        .map(|_| ())
        .map_err(|_| "route inventory requires an available signing identity".to_string())
}

#[tauri::command]
pub(crate) async fn export_managed_agent_route_inventory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ManagedAgentRouteInventoryEntry>, String> {
    require_signing_identity_available(state.signing_keys().map(|keys| keys.public_key()))?;

    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = super::load_managed_agents(&app)?;
        let personas = super::load_personas(&app)?;
        let global = super::load_global_agent_config(&app)?;
        build_managed_agent_route_inventory(&records, &personas, &global)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::managed_agents::{BackendKind, RespondTo};

    fn record(name: &str, pubkey: &str, persona_id: Option<&str>) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: pubkey.to_string(),
            name: name.to_string(),
            persona_id: persona_id.map(str::to_string),
            private_key_nsec: "nsec-secret-sentinel".into(),
            auth_tag: Some("BUZZ_AUTH_TAG-secret".into()),
            relay_url: "wss://secret-relay.example".into(),
            avatar_url: None,
            acp_command: "secret-acp-command".into(),
            agent_command: "stale-command".into(),
            agent_command_override: None,
            agent_args: vec!["--secret-arg".into()],
            mcp_command: "secret-mcp-command".into(),
            turn_timeout_seconds: 300,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: Some("secret prompt and memory".into()),
            model: Some("stale-model".into()),
            provider: Some("stale-provider".into()),
            persona_source_version: None,
            env_vars: BTreeMap::from([("TOKEN".into(), "token-secret".into())]),
            start_on_app_launch: false,
            runtime_pid: Some(4242),
            backend: BackendKind::Provider {
                id: "secret-account".into(),
                config: serde_json::json!({"credential":"secret"}),
            },
            backend_agent_id: None,
            provider_binary_path: Some("/secret/path".into()),
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: Some("secret-log".into()),
            last_error_code: None,
            respond_to: RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            relay_mesh: None,
            auto_restart_on_config_change: false,
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_parallelism: None,
        }
    }

    fn persona(
        id: &str,
        runtime: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            display_name: id.into(),
            avatar_url: None,
            system_prompt: "definition-secret".into(),
            runtime: runtime.map(str::to_string),
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn effective_precedence_ignores_linked_record_snapshots() {
        let records = vec![record("Agent", "pk", Some("persona"))];
        let personas = vec![persona(
            "persona",
            Some("goose"),
            Some("definition-model"),
            None,
        )];
        let global = GlobalAgentConfig {
            model: Some("global-model".into()),
            provider: Some("global-provider".into()),
            ..Default::default()
        };
        let entry = build_managed_agent_route_inventory(&records, &personas, &global)
            .unwrap()
            .remove(0);
        assert_eq!(
            entry.model,
            RouteField::available("definition-model".into(), RouteSource::Definition)
        );
        assert_eq!(
            entry.provider,
            RouteField::available("global-provider".into(), RouteSource::Global)
        );
    }

    #[test]
    fn runtime_matches_spawn_precedence_and_unknown_override_fails_closed() {
        let mut inherited = record("Inherited", "pk1", Some("persona"));
        let personas = vec![persona("persona", Some("goose"), None, None)];
        let global = GlobalAgentConfig::default();
        let entry = build_managed_agent_route_inventory(&[inherited.clone()], &personas, &global)
            .unwrap()
            .remove(0);
        assert_eq!(
            entry.runtime,
            RouteField::available("goose".into(), RouteSource::LinkedPersonaRuntime)
        );
        inherited.runtime = Some("codex".into());
        inherited.agent_command_override = Some("/custom/unknown-agent".into());
        let entry = build_managed_agent_route_inventory(&[inherited], &personas, &global)
            .unwrap()
            .remove(0);
        assert_eq!(
            entry.runtime,
            RouteField::unavailable(RouteSource::CommandOverrideUnmapped)
        );

        let bare = record("Bare", "pk2", None);
        assert_eq!(
            runtime_field(&bare, &[], "/custom/unknown-default"),
            RouteField::unavailable(RouteSource::DefaultCommandUnmapped)
        );
    }

    #[test]
    fn serialized_output_is_exact_allowlist_and_excludes_source_secrets() {
        let records = vec![record("Agent", "pk", Some("persona"))];
        let personas = vec![persona(
            "persona",
            Some("goose"),
            Some("safe-model"),
            Some("safe-provider"),
        )];
        let value = serde_json::to_value(
            build_managed_agent_route_inventory(&records, &personas, &GlobalAgentConfig::default())
                .unwrap(),
        )
        .unwrap();
        let object = value[0].as_object().unwrap();
        assert_eq!(
            object.keys().cloned().collect::<BTreeSet<_>>(),
            ["effort", "identity", "model", "provider", "pubkey", "runtime", "tools"]
                .map(str::to_string)
                .into_iter()
                .collect()
        );
        let json = value.to_string();
        for forbidden in [
            "prompt",
            "memory",
            "BUZZ_AUTH_TAG",
            "nsec-secret",
            "token-secret",
            "secret-relay",
            "TOKEN",
            "/secret/path",
            "secret-command",
            "secret-log",
            "4242",
            "secret-account",
            "credential",
            "prior_rollback_pin",
        ] {
            assert!(!json.contains(forbidden), "leaked {forbidden}: {json}");
        }
    }

    // Fixture used to verify identity pass-through and lack of deduplication.
    const EXPECTED_IDENTITIES: [&str; 58] = [
        "Ad Performance Analyst",
        "Analytics Lead",
        "Backend Engineer",
        "Brand Designer",
        "Bumble",
        "Calibration Engineer",
        "Churn Analyst",
        "Community Manager",
        "Compliance Officer",
        "Content Manager",
        "Content Strategist",
        "Customer Success & Onboarding Liaison",
        "Customer Success Manager",
        "Customer Support Agent",
        "Data Engineer",
        "Developer Advocate",
        "Edge Detector",
        "Engineering Lead",
        "Feature Engineer",
        "Finance Lead",
        "Fizz",
        "Frontend Engineer",
        "Graph Engineer",
        "Growth Hacker",
        "Harness Engineer",
        "Head of Business Operations",
        "Honey",
        "Infrastructure Engineer",
        "Keanu",
        "Legal & Finance Officer",
        "Loop Engineer",
        "Marketing Manager",
        "ML Tech Lead",
        "Newsletter Editor",
        "Nostradamus",
        "Onboarding Guide",
        "Partnerships Lead",
        "People Operations Lead",
        "Pipeline Health Monitor",
        "PR & Communications",
        "Prediction Analyst",
        "Product Analyst",
        "Product Designer",
        "Product Manager",
        "QA Engineer",
        "Reel Producer",
        "Research Scout",
        "Sales Lead",
        "Security Engineer",
        "Social Media Manager",
        "Sports Writer",
        "Support Lead",
        "Technical Writer",
        "The Raven",
        "Threat Hunter",
        "Training Engineer",
        "Upsell Detector",
        "Web Designer",
    ];

    #[test]
    fn identity_fixture_passes_through_without_deduplication() {
        let records = EXPECTED_IDENTITIES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut record = record(name, &format!("pubkey-{index}"), None);
                record.runtime = Some("goose".into());
                record
            })
            .collect::<Vec<_>>();
        let entries =
            build_managed_agent_route_inventory(&records, &[], &GlobalAgentConfig::default())
                .unwrap();
        assert_eq!(entries.len(), 58);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.identity.as_str())
                .collect::<BTreeSet<_>>(),
            EXPECTED_IDENTITIES.into_iter().collect()
        );
        assert!(entries.iter().all(|entry| !entry.pubkey.is_empty()));
        assert_eq!(
            entries
                .iter()
                .map(|entry| &entry.pubkey)
                .collect::<BTreeSet<_>>()
                .len(),
            58
        );
    }

    #[test]
    fn effort_and_tools_are_explicitly_unavailable() {
        let mut record = record("Agent", "pk", None);
        record.runtime = Some("goose".into());
        let entry =
            build_managed_agent_route_inventory(&[record], &[], &GlobalAgentConfig::default())
                .unwrap()
                .remove(0);
        assert_eq!(
            entry.effort,
            RouteField::unavailable(RouteSource::NotInSpawnEffectiveConfig)
        );
        assert_eq!(
            entry.tools,
            RouteListField::unavailable(RouteSource::NoSafeToolCatalog)
        );
    }

    #[test]
    fn unavailable_signing_identity_fails_before_building_a_payload() {
        let build_calls = std::cell::Cell::new(0);
        let result =
            require_signing_identity_available::<()>(Err("identity_lost".into())).map(|_| {
                build_calls.set(build_calls.get() + 1);
                vec!["payload"]
            });
        assert_eq!(
            result.unwrap_err(),
            "route inventory requires an available signing identity"
        );
        assert_eq!(build_calls.get(), 0);
        assert!(require_signing_identity_available(Ok("signing-pubkey")).is_ok());
    }

    #[test]
    fn broken_references_degrade_only_their_own_entries() {
        let mut healthy = record("Healthy", "healthy-pk", None);
        healthy.runtime = Some("goose".into());
        let mut dangling = record("Dangling", "dangling-pk", None);
        dangling.runtime = Some("deleted-custom-harness".into());
        let orphan = record("Orphan", "orphan-pk", Some("deleted-persona"));

        let entries = build_managed_agent_route_inventory(
            &[healthy, dangling, orphan],
            &[],
            &GlobalAgentConfig::default(),
        )
        .unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].identity, "Healthy");
        assert_eq!(entries[1].identity, "Dangling");
        assert_eq!(entries[2].identity, "Orphan");
        assert_eq!(
            entries[1].runtime,
            RouteField::unavailable(RouteSource::DanglingHarness)
        );
        assert_eq!(
            entries[1].provider,
            RouteField::unavailable(RouteSource::DanglingHarness)
        );
        assert_eq!(
            entries[1].model,
            RouteField::unavailable(RouteSource::DanglingHarness)
        );
        assert_eq!(
            entries[1].effort,
            RouteField::unavailable(RouteSource::DanglingHarness)
        );
        assert_eq!(
            entries[1].tools,
            RouteListField::unavailable(RouteSource::DanglingHarness)
        );
        assert_eq!(
            entries[2].runtime,
            RouteField::unavailable(RouteSource::OrphanedInstance)
        );
        assert_eq!(
            entries[2].provider,
            RouteField::unavailable(RouteSource::OrphanedInstance)
        );
        assert_eq!(
            entries[2].model,
            RouteField::unavailable(RouteSource::OrphanedInstance)
        );
        assert_eq!(
            entries[2].effort,
            RouteField::unavailable(RouteSource::OrphanedInstance)
        );
        assert_eq!(
            entries[2].tools,
            RouteListField::unavailable(RouteSource::OrphanedInstance)
        );
    }
}
