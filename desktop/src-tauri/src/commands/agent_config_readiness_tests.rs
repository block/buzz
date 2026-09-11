use super::*;
use crate::managed_agents::{BackendKind, RespondTo};

fn record() -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": "a".repeat(64),
        "name": "Agent",
        "private_key_nsec": "",
        "relay_url": "",
        "acp_command": "buzz-acp",
        "agent_command": "buzz-agent",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 300,
        "parallelism": 1,
        "system_prompt": null,
        "model": "claude-opus-4-5",
        "provider": "anthropic",
        "env_vars": {},
        "created_at": "",
        "updated_at": "",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null,
        "backend": { "type": "local" },
        "respond_to": "owner-only"
    }))
    .expect("record fixture")
}

fn definition() -> AgentDefinition {
    AgentDefinition {
        id: "persona-1".to_string(),
        display_name: "Persona".to_string(),
        description: None,
        avatar_url: None,
        system_prompt: "Instructions".to_string(),
        runtime: Some("buzz-agent".to_string()),
        model: Some("openrouter/model".to_string()),
        provider: Some("openrouter".to_string()),
        env_vars: BTreeMap::from([("OPENROUTER_API_KEY".to_string(), "sk-test".to_string())]),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn existing(pubkey: String) -> AgentReadinessDraft {
    AgentReadinessDraft::Existing {
        config: ExistingAgentReadinessDraft {
            pubkey,
            agent_command: None,
            harness_override: false,
            model: None,
            provider: None,
            env_vars: None,
        },
    }
}

#[test]
fn existing_draft_uses_production_update_projection() {
    let saved = record();
    let mut config = match existing(saved.pubkey.clone()) {
        AgentReadinessDraft::Existing { config } => config,
        AgentReadinessDraft::New { .. } => unreachable!(),
    };
    config.env_vars = Some(BTreeMap::from([(
        "ANTHROPIC_API_KEY".to_string(),
        "sk-test".to_string(),
    )]));

    let result = evaluate_draft(
        AgentReadinessDraft::Existing { config },
        std::slice::from_ref(&saved),
        &[],
        &GlobalAgentConfig::default(),
    )
    .expect("draft evaluates");

    assert!(result.ready);
    assert!(
        saved.env_vars.is_empty(),
        "saved baseline remains unchanged"
    );
}

#[test]
fn unknown_existing_target_is_not_reinterpreted_as_create() {
    let error = evaluate_draft(
        existing("missing".to_string()),
        &[],
        &[],
        &GlobalAgentConfig::default(),
    )
    .expect_err("unknown edit target must fail");

    assert!(error.contains("agent missing not found"));
}

#[test]
fn new_draft_uses_production_create_harness_projection() {
    let draft = AgentReadinessDraft::New {
        config: NewAgentReadinessDraft {
            agent_command: Some("buzz-agent".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            provider: Some("anthropic".to_string()),
            env_vars: BTreeMap::new(),
        },
    };

    let result = evaluate_draft(draft, &[], &[], &GlobalAgentConfig::default())
        .expect("new draft evaluates");
    assert_eq!(
        result.requirements,
        vec![Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string(),
        }]
    );
}

#[cfg(feature = "mesh-llm")]
#[test]
fn new_relay_mesh_draft_uses_create_default_model_projection() {
    let draft = AgentReadinessDraft::New {
        config: NewAgentReadinessDraft {
            agent_command: Some("buzz-agent".to_string()),
            model: None,
            provider: Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID.to_string()),
            env_vars: BTreeMap::new(),
        },
    };

    let result = evaluate_draft(draft, &[], &[], &GlobalAgentConfig::default())
        .expect("relay mesh draft evaluates");
    assert!(
        !result.requirements.contains(&Requirement::NormalizedField {
            field: "model".to_string(),
        }),
        "Create projects an omitted relay-mesh model to auto before readiness"
    );
}

#[test]
fn missing_custom_command_uses_same_binary_check_as_startup() {
    let draft = AgentReadinessDraft::New {
        config: NewAgentReadinessDraft {
            agent_command: Some("buzz-readiness-missing-command".to_string()),
            model: None,
            provider: None,
            env_vars: BTreeMap::new(),
        },
    };

    let result = evaluate_draft(draft, &[], &[], &GlobalAgentConfig::default())
        .expect("custom command readiness evaluates");
    assert_eq!(
        result.requirements,
        vec![Requirement::MissingBinary {
            command: "buzz-readiness-missing-command".to_string(),
        }]
    );
}

#[test]
fn linked_edit_uses_definition_owned_config() {
    let mut saved = record();
    saved.persona_id = Some("persona-1".to_string());
    saved.provider = Some("anthropic".to_string());
    saved.model = Some("stale".to_string());
    let definition = definition();

    let result = evaluate_draft(
        existing(saved.pubkey.clone()),
        std::slice::from_ref(&saved),
        std::slice::from_ref(&definition),
        &GlobalAgentConfig::default(),
    )
    .expect("linked draft evaluates");

    assert!(
        result.ready,
        "definition provider/model/env are authoritative"
    );
}

#[test]
fn orphaned_link_is_refused_like_spawn() {
    let mut saved = record();
    saved.persona_id = Some("missing-persona".to_string());

    let error = evaluate_draft(
        existing(saved.pubkey.clone()),
        std::slice::from_ref(&saved),
        &[],
        &GlobalAgentConfig::default(),
    )
    .expect_err("orphan must not report readiness");

    assert_eq!(
        error,
        crate::managed_agents::effective_config::ORPHANED_INSTANCE_ERROR
    );
}

#[test]
fn request_shape_preserves_new_vs_existing_and_patch_nulls() {
    let new: AgentReadinessDraft = serde_json::from_value(serde_json::json!({
        "kind": "new",
        "config": {
            "agentCommand": "buzz-agent",
            "provider": "anthropic",
            "model": "claude-opus-4-5"
        }
    }))
    .expect("new wire shape");
    assert!(matches!(new, AgentReadinessDraft::New { .. }));

    let existing: AgentReadinessDraft = serde_json::from_value(serde_json::json!({
        "kind": "existing",
        "config": {
            "pubkey": "agent",
            "provider": null,
            "model": null
        }
    }))
    .expect("existing wire shape");
    let AgentReadinessDraft::Existing { config } = existing else {
        panic!("expected existing draft");
    };
    assert_eq!(config.provider, Some(None));
    assert_eq!(config.model, Some(None));
}

#[test]
fn fixture_defaults_match_local_managed_agent_shape() {
    let saved = record();
    assert_eq!(saved.backend, BackendKind::Local);
    assert_eq!(saved.respond_to, RespondTo::OwnerOnly);
}
