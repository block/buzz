use super::*;
use crate::managed_agents::AgentDefinition;

fn bare_agent_record(
    persona_id: Option<&str>,
    model: Option<&str>,
    provider: Option<&str>,
) -> ManagedAgentRecord {
    use crate::managed_agents::{BackendKind, RespondTo};
    use std::collections::BTreeMap;
    ManagedAgentRecord {
        pubkey: "agent".to_string(),
        name: "Agent".to_string(),
        persona_id: persona_id.map(str::to_string),
        private_key_nsec: "".to_string(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".to_string(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "goose".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: "".to_string(),
        turn_timeout_seconds: 300,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        relay_mesh: None,
        auto_restart_on_config_change: false,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
    }
}
fn persona_record(id: &str, model: Option<&str>, provider: Option<&str>) -> AgentDefinition {
    use std::collections::BTreeMap;
    AgentDefinition {
        id: id.to_string(),
        display_name: "Test Persona".to_string(),
        avatar_url: None,
        system_prompt: "".to_string(),
        runtime: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: vec![],
        parallelism: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

/// Auto-archive uses the same NIP-IA wire builder as the explicit GUI action,
/// attaches owner consent, and marks a deliberate delete as `retired`.
#[test]
fn build_agent_archive_request_attaches_owner_auth_and_retired_reason() {
    use nostr::JsonUtil;

    let owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let event = build_agent_archive_request(&owner, &agent.public_key().to_hex())
        .expect("build archive request");
    let json: serde_json::Value = serde_json::from_str(&event.as_json()).unwrap();
    let tags = json["tags"].as_array().unwrap();

    assert_eq!(event.kind.as_u16(), 9035);
    assert_eq!(event.pubkey, owner.public_key());
    assert!(event.verify_id());
    assert!(event.verify_signature());
    assert!(tags.iter().any(|tag| {
        tag.as_array().is_some_and(|parts| {
            parts.first().and_then(serde_json::Value::as_str) == Some("p")
                && parts.get(1).and_then(serde_json::Value::as_str)
                    == Some(agent.public_key().to_hex().as_str())
        })
    }));
    assert!(tags.iter().any(|tag| {
        tag.as_array().is_some_and(|parts| {
            parts.first().and_then(serde_json::Value::as_str) == Some("reason")
                && parts.get(1).and_then(serde_json::Value::as_str) == Some("retired")
        })
    }));
    assert!(tags.iter().any(|tag| {
        tag.as_array().is_some_and(|parts| {
            parts.first().and_then(serde_json::Value::as_str) == Some("auth")
                && parts.get(1).and_then(serde_json::Value::as_str)
                    == Some(owner.public_key().to_hex().as_str())
                && parts.len() == 4
        })
    }));
}

/// Deploy-path regression for Fix 1 of Thufir pass-2: a persona-linked
/// provider agent with a stale record snapshot must use the live persona
/// model/provider in the deploy payload, not the stale record values.
///
/// Scenario: agent was created with persona at model="old-model"/provider="old-prov".
/// The persona was subsequently updated to "new-model"/"new-prov" but the record
/// was NOT re-snapshotted (provider start skips re-snapshot; local spawn does it).
/// The deploy resolver must use the current persona values.
///
/// Fails against `resolve_effective_model_provider` (record-first precedence),
/// which would return "old-model"/"old-prov" from the stale record.
#[test]
fn deploy_resolver_uses_live_persona_over_stale_record_snapshot() {
    // Record holds the stale snapshot (created when persona had old values).
    let record = bare_agent_record(Some("p1"), Some("old-model"), Some("old-prov"));
    // Live persona has been updated since the record was snapshotted.
    let personas = vec![persona_record("p1", Some("new-model"), Some("new-prov"))];
    let global = crate::managed_agents::GlobalAgentConfig::default();

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert_eq!(
        model,
        Some("new-model"),
        "deploy must use live persona model, not stale record snapshot"
    );
    assert_eq!(
        provider,
        Some("new-prov"),
        "deploy must use live persona provider, not stale record snapshot"
    );
}

/// Deploy resolver falls back to record when persona has no model/provider
/// (persona without structured model — fallback to record snapshot).
#[test]
fn deploy_resolver_falls_back_to_record_when_persona_has_none() {
    let record = bare_agent_record(Some("p1"), Some("record-model"), Some("record-prov"));
    // Persona exists but has no model/provider.
    let personas = vec![persona_record("p1", None, None)];
    let global = crate::managed_agents::GlobalAgentConfig::default();

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert_eq!(model, Some("record-model"));
    assert_eq!(provider, Some("record-prov"));
}

/// Deploy resolver falls back to global when both persona and record have none.
#[test]
fn deploy_resolver_falls_back_to_global_when_persona_and_record_have_none() {
    let record = bare_agent_record(Some("p1"), None, None);
    let personas = vec![persona_record("p1", None, None)];
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-prov".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert_eq!(model, Some("global-model"));
    assert_eq!(provider, Some("global-prov"));
}

#[test]
fn normalize_relay_mesh_rejects_empty_model_ref() {
    let config = RelayMeshConfig {
        model_ref: "  \t ".to_string(),
    };

    assert_eq!(
        normalize_relay_mesh(Some(&config), &BackendKind::Local).unwrap_err(),
        "Buzz shared compute model is required"
    );
}

#[test]
fn normalize_relay_mesh_rejects_non_local_backend() {
    let config = RelayMeshConfig {
        model_ref: "Qwen3".to_string(),
    };
    let backend = BackendKind::Provider {
        id: "blox".to_string(),
        config: serde_json::json!({}),
    };

    assert_eq!(
        normalize_relay_mesh(Some(&config), &backend).unwrap_err(),
        "Buzz shared compute agents must use the local backend"
    );
}

#[test]
fn normalize_relay_mesh_trims_and_preserves_valid_config() {
    let config = RelayMeshConfig {
        model_ref: "  Qwen3  ".to_string(),
    };

    assert_eq!(
        normalize_relay_mesh(Some(&config), &BackendKind::Local).unwrap(),
        Some(RelayMeshConfig {
            model_ref: "Qwen3".to_string(),
        })
    );
}

#[test]
fn created_avatar_prefers_explicit_input() {
    let resolved = resolve_created_avatar_url(
        Some(" https://x/input.png "),
        Some("https://x/persona.png".to_string()),
        "goose",
    );

    assert_eq!(resolved.as_deref(), Some("https://x/input.png"));
}

#[test]
fn created_avatar_uses_persona_before_command_fallback() {
    let resolved =
        resolve_created_avatar_url(None, Some(" https://x/persona.png ".to_string()), "goose");

    assert_eq!(resolved.as_deref(), Some("https://x/persona.png"));
}

#[test]
fn created_avatar_uses_command_fallback_without_input_or_persona() {
    use crate::managed_agents::managed_agent_avatar_url;

    let resolved = resolve_created_avatar_url(None, None, "goose");

    assert_eq!(resolved, managed_agent_avatar_url("goose"));
}

fn profile(name: Option<&str>, picture: Option<&str>) -> crate::relay::AgentProfileInfo {
    crate::relay::AgentProfileInfo {
        display_name: name.map(str::to_string),
        picture: picture.map(str::to_string),
    }
}

#[test]
fn profile_needs_sync_when_missing() {
    assert!(profile_needs_sync(None, "Duncan", Some("https://x/a.png")));
}

#[test]
fn profile_needs_sync_when_missing_even_without_expected_avatar() {
    assert!(profile_needs_sync(None, "Duncan", None));
}

#[test]
fn profile_needs_sync_when_name_diverges() {
    let existing = profile(Some("Stilgar"), Some("https://x/a.png"));
    assert!(profile_needs_sync(
        Some(&existing),
        "Duncan",
        Some("https://x/a.png")
    ));
}

#[test]
fn profile_needs_sync_when_picture_diverges() {
    let existing = profile(Some("Duncan"), Some("https://x/old.png"));
    assert!(profile_needs_sync(
        Some(&existing),
        "Duncan",
        Some("https://x/new.png")
    ));
}

#[test]
fn profile_in_sync_when_name_and_picture_match() {
    let existing = profile(Some("Duncan"), Some("https://x/a.png"));
    assert!(!profile_needs_sync(
        Some(&existing),
        "Duncan",
        Some("https://x/a.png")
    ));
}

#[test]
fn profile_in_sync_when_both_avatars_absent() {
    let existing = profile(Some("Duncan"), None);
    assert!(!profile_needs_sync(Some(&existing), "Duncan", None));
}

#[test]
fn profile_needs_sync_when_existing_name_is_none() {
    let existing = profile(None, Some("https://x/a.png"));
    assert!(profile_needs_sync(
        Some(&existing),
        "Duncan",
        Some("https://x/a.png"),
    ));
}

#[test]
fn profile_needs_sync_when_expected_avatar_absent_but_published() {
    let existing = profile(Some("Duncan"), Some("https://x/a.png"));
    assert!(profile_needs_sync(Some(&existing), "Duncan", None));
}

#[test]
fn legacy_avatar_prefers_persona_over_corrupted_relay_picture() {
    // The regression: the relay picture was overwritten with the command
    // default. The persona avatar must win so the correct avatar is restored.
    let resolved = resolve_legacy_avatar(
        Some("https://x/persona.png".to_string()),
        Some("https://x/default-icon.png".to_string()),
        "goose",
    );

    assert_eq!(resolved, "https://x/persona.png");
}

#[test]
fn legacy_avatar_falls_back_to_relay_picture_without_persona() {
    let resolved = resolve_legacy_avatar(None, Some("https://x/relay.png".to_string()), "goose");

    assert_eq!(resolved, "https://x/relay.png");
}

#[test]
fn legacy_avatar_falls_back_to_command_icon_when_no_persona_or_relay() {
    use crate::managed_agents::managed_agent_avatar_url;

    let resolved = resolve_legacy_avatar(None, None, "goose");

    assert_eq!(resolved, managed_agent_avatar_url("goose").unwrap());
}

#[test]
fn legacy_avatar_empty_when_nothing_resolves() {
    let resolved = resolve_legacy_avatar(None, None, "totally-unknown-command");

    assert!(resolved.is_empty());
}

// ── Provider deploy payload completeness ─────────────────────────────────────

/// Regression (PR #1667 review, Thufir): the provider deploy payload must
/// carry every behavioral field the local spawn path applies — a field
/// missing here silently strips it from provider-backed agents.
#[test]
fn deploy_payload_carries_the_full_behavioral_quad() {
    let allow = "a".repeat(64);
    let record: ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{
            "pubkey": "abcd1234",
            "name": "test-agent",
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "parallelism": 4,
            "respond_to": "allowlist",
            "respond_to_allowlist": ["{allow}"],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }}"#
    ))
    .expect("sample record");

    let payload = deploy_payload_json(
        &record,
        "wss://relay.example".to_string(),
        Some("gpt-x".to_string()),
        Some("openai".to_string()),
        std::collections::BTreeMap::new(),
    );

    assert_eq!(payload["parallelism"], 4);
    assert_eq!(payload["respond_to"], "allowlist");
    assert_eq!(payload["respond_to_allowlist"][0], "a".repeat(64));
    assert_eq!(payload["model"], "gpt-x");
    assert_eq!(payload["provider"], "openai");
    assert_eq!(payload["relay_url"], "wss://relay.example");
}
