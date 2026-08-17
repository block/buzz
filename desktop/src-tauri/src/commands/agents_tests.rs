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
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
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
    let event = build_agent_archive_request(
        &owner,
        &agent.public_key().to_hex(),
        Some("persona-reviewer"),
    )
    .expect("build archive request");
    let json: serde_json::Value = serde_json::from_str(&event.as_json()).unwrap();
    let tags = json["tags"].as_array().unwrap();

    assert_eq!(event.kind.as_u16(), 9035);
    assert_eq!(event.pubkey, owner.public_key());
    assert!(event.verify_id());
    assert!(event.verify_signature());
    assert_eq!(event.content, r#"{"persona_id":"persona-reviewer"}"#);
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

/// Deploy resolver uses definition model/provider, ignoring stale record.
#[test]
fn deploy_resolver_uses_definition_over_stale_record() {
    let record = bare_agent_record(Some("p1"), Some("old-model"), Some("old-prov"));
    let personas = vec![persona_record("p1", Some("new-model"), Some("new-prov"))];
    let global = crate::managed_agents::GlobalAgentConfig::default();

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert_eq!(
        model.as_deref(),
        Some("new-model"),
        "deploy must use definition model, not stale record snapshot"
    );
    assert_eq!(
        provider.as_deref(),
        Some("new-prov"),
        "deploy must use definition provider, not stale record snapshot"
    );
}

/// When a linked definition has blank model/provider (inherit), the deploy
/// resolver must fall through to global — stale record bytes are inert.
#[test]
fn deploy_resolver_inherits_global_when_definition_blank() {
    let record = bare_agent_record(Some("p1"), Some("stale-model"), Some("stale-prov"));
    let personas = vec![persona_record("p1", None, None)];
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-prov".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert_eq!(
        model.as_deref(),
        Some("global-model"),
        "definition blank → global; stale record ignored"
    );
    assert_eq!(
        provider.as_deref(),
        Some("global-prov"),
        "definition blank → global; stale record ignored"
    );
}

/// Deploy resolver falls back to global when both definition and record have none.
#[test]
fn deploy_resolver_falls_back_to_global_when_definition_and_record_have_none() {
    let record = bare_agent_record(Some("p1"), None, None);
    let personas = vec![persona_record("p1", None, None)];
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-prov".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert_eq!(model.as_deref(), Some("global-model"));
    assert_eq!(provider.as_deref(), Some("global-prov"));
}

/// Orphan: linked record with missing definition → the pure model/provider
/// pair resolver returns `(None, None)`. This is NOT the deploy refusal
/// boundary — `build_deploy_payload` refuses an orphan outright via
/// `.require_resolved()?` before this pair is ever computed. This test pins
/// the resolver's own orphan behavior, which readiness/hash also depend on.
#[test]
fn deploy_resolver_returns_none_for_orphaned_instance() {
    let record = bare_agent_record(Some("missing-def"), Some("stale-model"), Some("stale-prov"));
    let personas: Vec<AgentDefinition> = vec![];
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-model".to_string()),
        provider: Some("global-prov".to_string()),
        ..Default::default()
    };

    let (model, provider) = resolve_deploy_model_provider(&record, &personas, &global);

    assert!(
        model.is_none(),
        "orphaned instance must not resolve to any model"
    );
    assert!(
        provider.is_none(),
        "orphaned instance must not resolve to any provider"
    );
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
fn deploy_refuses_resolved_relay_mesh_provider_with_padding() {
    let record = bare_agent_record(Some("p1"), None, None);
    let personas = vec![persona_record("p1", None, Some("  relay-mesh  "))];
    let global = crate::managed_agents::GlobalAgentConfig::default();

    let (_, provider) = resolve_deploy_model_provider(&record, &personas, &global);
    let error = ensure_remote_provider_supported(provider.as_deref())
        .expect_err("resolved shared-compute provider must not deploy remotely");

    assert!(error.contains("cannot be deployed remotely"), "{error}");
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

fn deploy_payload_for_policy(
    record: &ManagedAgentRecord,
    owner_only_access: bool,
) -> serde_json::Value {
    deploy_payload_json(
        record,
        "wss://relay.example".to_string(),
        DeployProjections {
            effective_model: Some("gpt-x".to_string()),
            effective_provider: Some("openai".to_string()),
            effective_prompt: None,
            effective_parallelism: record.parallelism,
            owner_only_access,
        },
        std::collections::BTreeMap::new(),
        // Access projection is the subject here; the launch block is exercised
        // by the shared provider fixture test below.
        serde_json::Value::Null,
    )
}

/// The shared provider fixture is the contract arbiter: it must be the exact
/// richest deploy request produced by the real desktop serializers.
#[test]
fn deploy_payload_matches_the_shared_full_launch_fixture() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../crates/buzz-backend-kubernetes/tests/fixtures/provider-wire/deploy-full-launch.request.json",
    );
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display())),
    )
    .expect("parse shared provider fixture");
    let record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "abcd1234",
        "name": "worker",
        "private_key_nsec": "nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5",
        "relay_url": "wss://localhost:3000",
        "auth_tag": "tag-1",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "runtime": "goose",
        "model": "gpt-5",
        "provider": "openai",
        "env_vars": {"USER_KEY": "user-value"},
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 300,
        "system_prompt": null,
        "idle_timeout_seconds": null,
        "max_turn_duration_seconds": null,
        "parallelism": 10,
        "respond_to": "allowlist",
        "respond_to_allowlist": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z"
    }))
    .expect("fixture source record");
    let descriptor = crate::managed_agents::resolve_effective_harness_descriptor(
        &record,
        &[],
        &crate::managed_agents::GlobalAgentConfig::default(),
    )
    .expect("resolve fixture source record descriptor");
    let launch = super::deploy::build_launch_block(
        &record,
        &descriptor,
        &[],
        None,
        Some("gpt-5"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let agent = deploy_payload_json(
        &record,
        "wss://relay.example".into(),
        DeployProjections {
            effective_model: Some("gpt-5".into()),
            effective_provider: Some("openai".into()),
            effective_prompt: None,
            effective_parallelism: crate::managed_agents::effective_parallelism(
                &descriptor.command,
                record.parallelism,
            ),
            // Fixture asserts the record's own access fields survive.
            owner_only_access: false,
        },
        std::collections::BTreeMap::from([("USER_KEY".into(), "user-value".into())]),
        launch,
    );

    assert_eq!(
        agent, fixture["agent"],
        "desktop payload drifted from the shared provider fixture"
    );
}

#[test]
fn tauri_platform_configs_bundle_kubernetes_only_on_supported_hosts() {
    use tauri_utils::{config::parse::read_from, platform::Target};

    let config_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (target, expected) in [
        (Target::MacOS, true),
        (Target::Linux, true),
        (Target::Windows, false),
    ] {
        let (config, paths) = read_from(target, config_root).expect("read Tauri config");
        let external_bins = config["bundle"]["externalBin"]
            .as_array()
            .expect("bundle.externalBin array");
        let has_kubernetes = external_bins
            .iter()
            .any(|value| value == "binaries/buzz-backend-kubernetes");
        assert_eq!(
            has_kubernetes, expected,
            "unexpected Kubernetes externalBin for {target}; merged {paths:?}"
        );
    }
}

#[test]
fn current_build_deploy_payload_forwards_compiled_policy() {
    use crate::managed_agents::{BackendKind, RespondTo};

    let expected_owner_only = match std::env::var("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY") {
        Ok(value) => value
            .parse::<bool>()
            .expect("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be true or false"),
        Err(std::env::VarError::NotPresent)
            if !crate::managed_agents::owner_only_access_build() =>
        {
            false
        }
        Err(std::env::VarError::NotPresent) => {
            panic!(
                "BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be set for owner-only-access-build tests"
            )
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be valid UTF-8")
        }
    };
    let mut record = bare_agent_record(None, None, None);
    record.backend = BackendKind::Provider {
        id: "provider".to_string(),
        config: serde_json::json!({}),
    };
    record.respond_to = RespondTo::Anyone;
    record.respond_to_allowlist = vec!["a".repeat(64)];

    let payload = deploy_payload_json(
        &record,
        "wss://relay.example".to_string(),
        DeployProjections {
            effective_model: None,
            effective_provider: None,
            effective_prompt: None,
            effective_parallelism: record.parallelism,
            owner_only_access: crate::managed_agents::owner_only_access_build(),
        },
        std::collections::BTreeMap::new(),
        // The compiled access policy is the subject here; the launch block is
        // exercised by the shared provider fixture test above.
        serde_json::Value::Null,
    );
    let expected_mode = if expected_owner_only {
        "owner-only"
    } else {
        "anyone"
    };

    assert_eq!(
        payload["respond_to"], expected_mode,
        "current-build deploy payload did not forward the compiled policy",
    );
    let expected_allowlist = if expected_owner_only {
        serde_json::json!([])
    } else {
        serde_json::json!(["a".repeat(64)])
    };
    assert_eq!(
        payload["respond_to_allowlist"], expected_allowlist,
        "current-build deploy payload did not apply the compiled policy to the stale allowlist",
    );
}

#[test]
fn provider_upgrade_reconciliation_targets_existing_deployments_only_in_marked_builds() {
    use crate::managed_agents::BackendKind;

    let mut record = bare_agent_record(None, None, None);
    record.backend = BackendKind::Provider {
        id: "provider".to_string(),
        config: serde_json::json!({}),
    };
    record.backend_agent_id = Some("existing-provider-agent".to_string());
    record.respond_to = crate::managed_agents::RespondTo::Anyone;
    record.respond_to_allowlist = vec!["a".repeat(64)];

    assert!(provider_access::needs_reconciliation_with_policy(
        &record, true
    ));
    let payload = deploy_payload_for_policy(&record, true);
    assert_eq!(payload["respond_to"], "owner-only");
    assert_eq!(payload["respond_to_allowlist"], serde_json::json!([]));
    assert!(!provider_access::needs_reconciliation_with_policy(
        &record, false
    ));

    record.backend_agent_id = None;
    assert!(!provider_access::needs_reconciliation_with_policy(
        &record, true
    ));

    record.backend = BackendKind::Local;
    record.backend_agent_id = Some("stale-provider-id".to_string());
    assert!(!provider_access::needs_reconciliation_with_policy(
        &record, true
    ));
}

#[test]
fn owner_only_access_deploy_payload_clamps_stale_access() {
    use crate::managed_agents::{BackendKind, RespondTo};

    let mut record = bare_agent_record(None, None, None);
    record.backend = BackendKind::Provider {
        id: "provider".to_string(),
        config: serde_json::json!({}),
    };
    record.respond_to = RespondTo::Anyone;
    record.respond_to_allowlist = vec!["a".repeat(64)];

    let payload = deploy_payload_for_policy(&record, true);

    assert_eq!(
        payload["respond_to"], "owner-only",
        "owner-only-access deploy payload widened stale access"
    );
    assert_eq!(
        payload["respond_to_allowlist"],
        serde_json::json!([]),
        "owner-only-access deploy payload retained a stale allowlist"
    );
}

// ── kind:10100 agent directory record ───────────────────────────────────────
//
// The record is world-readable, so its content is an explicit allowlist of
// publishable fields rather than a dump of the agent record. These tests pin
// both halves of that contract: the fields a remote client needs to decide
// mentionability, and the fields that must never leave the host.

fn directory_agent_record() -> ManagedAgentRecord {
    use crate::managed_agents::RespondTo;
    let mut record = bare_agent_record(None, Some("gpt-5"), Some("openai"));
    record.pubkey = "a".repeat(64);
    record.name = "Tester".to_string();
    record.display_name = Some("Tester".to_string());
    record.respond_to = RespondTo::Anyone;
    record.respond_to_allowlist = vec![];
    record.private_key_nsec = "nsec1supersecret".to_string();
    record.auth_tag = Some(r#"{"tag":"secret-attestation"}"#.to_string());
    record.system_prompt = Some("you are a helpful agent".to_string());
    record
        .env_vars
        .insert("OPENAI_API_KEY".to_string(), "sk-secret".to_string());
    record
}

#[test]
fn directory_record_carries_the_fields_a_remote_client_needs() {
    let record = directory_agent_record();
    let content = build_agent_directory_content(
        &record,
        &["chan-1".to_string(), "chan-2".to_string()],
        &["general".to_string(), "agents".to_string()],
        "online",
        "anyone",
    );

    assert_eq!(content["name"], "Tester");
    assert_eq!(content["status"], "online");
    assert_eq!(content["respond_to"], "anyone");
    assert_eq!(content["channel_add_policy"], "anyone");
    assert_eq!(
        content["channel_ids"],
        serde_json::json!(["chan-1", "chan-2"])
    );
    assert_eq!(
        content["channels"],
        serde_json::json!(["general", "agents"])
    );
    assert!(content["agent_type"].is_string());
    assert!(content["capabilities"].is_array());
    assert!(content["respond_to_allowlist"].is_array());
}

#[test]
fn directory_record_carries_the_allowlist_when_respond_to_is_allowlist() {
    use crate::managed_agents::RespondTo;
    let mut record = directory_agent_record();
    record.respond_to = RespondTo::Allowlist;
    record.respond_to_allowlist = vec!["b".repeat(64)];

    let content = build_agent_directory_content(&record, &[], &[], "online", "anyone");

    assert_eq!(content["respond_to"], "allowlist");
    assert_eq!(
        content["respond_to_allowlist"],
        serde_json::json!([&"b".repeat(64)])
    );
}

#[test]
fn directory_record_reports_a_stopped_agent_as_offline() {
    let record = directory_agent_record();
    let content = build_agent_directory_content(&record, &[], &[], "offline", "anyone");

    assert_eq!(content["status"], "offline");
}

#[test]
fn directory_record_never_leaks_the_agents_secret_key() {
    let record = directory_agent_record();
    let serialized =
        build_agent_directory_content(&record, &[], &[], "online", "anyone").to_string();

    assert!(!serialized.contains("nsec1supersecret"));
    assert!(!serialized.contains("private_key"));
}

#[test]
fn directory_record_never_leaks_the_auth_attestation() {
    let record = directory_agent_record();
    let serialized =
        build_agent_directory_content(&record, &[], &[], "online", "anyone").to_string();

    assert!(!serialized.contains("secret-attestation"));
    assert!(!serialized.contains("auth_tag"));
}

#[test]
fn directory_record_never_leaks_environment_variables() {
    let record = directory_agent_record();
    let serialized =
        build_agent_directory_content(&record, &[], &[], "online", "anyone").to_string();

    assert!(!serialized.contains("OPENAI_API_KEY"));
    assert!(!serialized.contains("sk-secret"));
}

#[test]
fn directory_record_never_leaks_runtime_configuration() {
    let record = directory_agent_record();
    let serialized =
        build_agent_directory_content(&record, &[], &[], "online", "anyone").to_string();

    assert!(!serialized.contains("you are a helpful agent"));
    assert!(!serialized.contains("system_prompt"));
    assert!(!serialized.contains("acp_command"));
    assert!(!serialized.contains("agent_args"));
}

#[test]
fn channel_ids_come_from_the_d_tags_of_membership_events() {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    let keys = Keys::generate();
    let build = |d: &str| {
        EventBuilder::new(Kind::Custom(39002), "")
            .tags([Tag::parse(["d", d]).expect("d tag")])
            .sign_with_keys(&keys)
            .expect("sign")
    };

    // Two distinct channels, one duplicated, and one event with no `d` tag at
    // all — a malformed record must be skipped, not abort the whole publish.
    let untagged = EventBuilder::new(Kind::Custom(39002), "")
        .sign_with_keys(&keys)
        .expect("sign");
    let events = vec![build("chan-b"), build("chan-a"), build("chan-b"), untagged];

    assert_eq!(
        channel_ids_from_member_events(&events),
        vec!["chan-a".to_string(), "chan-b".to_string()]
    );
}

#[test]
fn existing_channel_add_policy_is_preserved_from_the_agents_prior_record() {
    use nostr::{EventBuilder, Keys, Kind};

    let keys = Keys::generate();
    let prior = EventBuilder::new(
        Kind::Custom(10100),
        r#"{"name":"Tester","channel_add_policy":"owner_only"}"#,
    )
    .sign_with_keys(&keys)
    .expect("sign");

    assert_eq!(existing_channel_add_policy(&[prior]), "owner_only");
}

#[test]
fn channel_add_policy_falls_back_to_the_schema_default_when_never_published() {
    // The relay column defaults to 'anyone'; sending that for an agent with no
    // prior record leaves the stored policy exactly as it already was.
    assert_eq!(existing_channel_add_policy(&[]), "anyone");
}

#[test]
fn channel_add_policy_falls_back_when_the_prior_record_is_malformed() {
    use nostr::{EventBuilder, Keys, Kind};

    let keys = Keys::generate();
    let junk = EventBuilder::new(Kind::Custom(10100), "not json")
        .sign_with_keys(&keys)
        .expect("sign");

    assert_eq!(existing_channel_add_policy(&[junk]), "anyone");
}

#[test]
fn channel_names_come_from_the_name_tag_of_metadata_events() {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    let keys = Keys::generate();
    // kind:39000 carries the channel name in a `name` tag; `content` is empty.
    let meta = EventBuilder::new(Kind::Custom(39000), "")
        .tags([
            Tag::parse(["d", "chan-a"]).expect("d"),
            Tag::parse(["name", "general"]).expect("name"),
        ])
        .sign_with_keys(&keys)
        .expect("sign");

    assert_eq!(
        channel_names_from_meta_events(&[meta], &["chan-a".to_string()]),
        vec!["general".to_string()]
    );
}

#[test]
fn channel_name_falls_back_to_the_id_when_metadata_is_missing() {
    assert_eq!(
        channel_names_from_meta_events(&[], &["chan-zz".to_string()]),
        vec!["chan-zz".to_string()]
    );
}
