use super::deploy::DeployPayloadContext;
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
        reply_placement: None,
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
        definition_reply_placement: None,
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
        reply_placement: None,
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

    let launch = crate::managed_agents::EffectiveHarnessDescriptor {
        command: "goose".to_string(),
        args: vec!["--proof".to_string()],
        env: std::collections::BTreeMap::from([(
            "FAKE_LAUNCH_MARKER".to_string(),
            "from-descriptor".to_string(),
        )]),
    };
    let payload = deploy_payload_json(
        &record,
        DeployPayloadContext {
            relay_url: "wss://relay.example".to_string(),
            effective_model: Some("gpt-x".to_string()),
            effective_provider: Some("openai".to_string()),
            effective_prompt: None,
            reply_placement: crate::managed_agents::ReplyPlacement::Thread,
            merged_env: std::collections::BTreeMap::new(),
            launch,
            policy_env: std::collections::BTreeMap::from([(
                "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
                "policy-prompt".to_string(),
            )]),
            owner_pubkey: Some("owner-proof".to_string()),
        },
    );

    assert_eq!(payload["parallelism"], 4);
    assert_eq!(payload["respond_to"], "allowlist");
    assert_eq!(payload["respond_to_allowlist"][0], "a".repeat(64));
    assert_eq!(payload["model"], "gpt-x");
    assert_eq!(payload["provider"], "openai");
    assert_eq!(payload["relay_url"], "wss://relay.example");
    assert_eq!(payload["launch"]["command"], "goose");
    assert_eq!(payload["launch"]["args"][0], "--proof");
    assert_eq!(
        payload["launch"]["env"]["FAKE_LAUNCH_MARKER"],
        "from-descriptor"
    );
    assert_eq!(payload["launch"]["owner_pubkey"], "owner-proof");
    assert_eq!(
        payload["launch"]["policy_env"]["BUZZ_ACP_SYSTEM_PROMPT"],
        "policy-prompt"
    );
}

#[test]
fn deploy_payload_carries_each_reply_placement_to_provider_launch_env() {
    use crate::managed_agents::ReplyPlacement;

    for (mode, wire) in [
        (ReplyPlacement::Thread, "thread"),
        (ReplyPlacement::TopLevel, "top-level"),
        (ReplyPlacement::FollowScope, "follow-scope"),
    ] {
        let record = bare_agent_record(None, None, None);
        let global = crate::managed_agents::GlobalAgentConfig {
            reply_placement: Some(mode),
            ..Default::default()
        };
        let (_, resolved_mode) = resolve_deploy_config(&record, &[], Ok(global))
            .expect("provider deploy mode should resolve through the shared helper");
        let payload = deploy_payload_json(
            &record,
            DeployPayloadContext {
                relay_url: "wss://relay.example".to_string(),
                effective_model: None,
                effective_provider: None,
                effective_prompt: None,
                reply_placement: resolved_mode,
                merged_env: std::collections::BTreeMap::new(),
                launch: crate::managed_agents::EffectiveHarnessDescriptor {
                    command: "goose".to_string(),
                    args: vec![],
                    env: std::collections::BTreeMap::new(),
                },
                policy_env: std::collections::BTreeMap::new(),
                owner_pubkey: Some("owner-proof".to_string()),
            },
        );

        assert_eq!(payload["reply_placement"], wire);
        assert_eq!(
            payload["launch"]["policy_env"]["BUZZ_ACP_REPLY_PLACEMENT"],
            wire
        );
    }
}

#[test]
fn provider_deploy_refuses_malformed_global_config() {
    let record = bare_agent_record(None, None, None);
    let err = resolve_deploy_config(
        &record,
        &[],
        Err("failed to parse global agent config: invalid json".to_string()),
    )
    .expect_err("provider deploy must not substitute the thread default");

    assert!(err.contains("failed to parse global agent config"));
}

#[test]
fn provider_policy_env_contains_all_desktop_resolved_launch_defaults() {
    use crate::managed_agents::{ReplyPlacement, TeamRecord};

    let mut record = bare_agent_record(None, None, None);
    record.display_name = Some("Policy Display".to_string());
    record.team_id = Some("team-1".to_string());
    record.idle_timeout_seconds = Some(11);
    record.max_turn_duration_seconds = Some(22);
    record.parallelism = 4;

    let teams = vec![TeamRecord {
        id: "team-1".to_string(),
        name: "Team".to_string(),
        description: None,
        instructions: Some("shared team instructions".to_string()),
        persona_ids: vec![],
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: String::new(),
        updated_at: String::new(),
    }];

    let policy = crate::managed_agents::resolve_effective_launch_policy_env(
        &record,
        "goose",
        &teams,
        Some("desktop prompt"),
        Some("desktop model"),
        ReplyPlacement::FollowScope,
        true,
    );

    for (key, expected) in [
        ("GOOSE_MODE", "auto"),
        ("BUZZ_ACP_LAZY_POOL", "true"),
        ("BUZZ_ACP_RELAY_OBSERVER", "true"),
        ("BUZZ_ACP_SYSTEM_PROMPT", "desktop prompt"),
        ("BUZZ_ACP_MODEL", "desktop model"),
        ("BUZZ_ACP_IDLE_TIMEOUT", "11"),
        ("BUZZ_ACP_MAX_TURN_DURATION", "22"),
        ("BUZZ_ACP_AGENTS", "4"),
        ("BUZZ_ACP_SESSION_TITLE", "Policy Display"),
        ("BUZZ_ACP_TEAM_INSTRUCTIONS", "shared team instructions"),
        ("BUZZ_ACP_REPLY_PLACEMENT", "follow-scope"),
    ] {
        assert_eq!(policy.get(key).map(String::as_str), Some(expected), "{key}");
    }

    let hooks = crate::managed_agents::resolve_effective_launch_policy_env(
        &record,
        "buzz-agent",
        &teams,
        None,
        None,
        ReplyPlacement::Thread,
        true,
    );
    assert_eq!(hooks.get("MCP_HOOK_SERVERS").map(String::as_str), Some("*"));
}

/// The provider boundary must be executable, not just JSON-shaped: a fake
/// provider applies the launch contract to a fake harness and the harness
/// records the environment it received. The reserved reply-placement key is
/// deliberately poisoned in the user env to prove the policy value wins.
#[cfg(unix)]
#[test]
fn provider_deploy_executes_launch_contract_and_preserves_reply_placement() {
    use crate::managed_agents::provider_deploy;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn shell_quote(path: &Path) -> String {
        let value = path.to_string_lossy().replace('\'', "'\\\"'\\\"'");
        format!("'{value}'")
    }

    fn make_executable(path: &Path) {
        let mut permissions = fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).expect("make script executable");
    }

    let dir = tempfile::tempdir().expect("temporary provider directory");
    let harness_path = dir.path().join("fake-harness");
    let observed_path = dir.path().join("observed-env");
    let provider_path = dir.path().join("fake-provider");

    fs::write(
        &harness_path,
        format!(
            "#!/bin/sh\nset -eu\nprintf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s' \\
\"$FAKE_LAUNCH_MARKER\" \\
\"$BUZZ_ACP_REPLY_PLACEMENT\" \\
\"$BUZZ_ACP_SYSTEM_PROMPT\" \\
\"$BUZZ_ACP_IDLE_TIMEOUT\" \\
\"$BUZZ_ACP_MAX_TURN_DURATION\" \\
\"$BUZZ_ACP_AGENTS\" \\
\"$GOOSE_MODE\" \\
\"$BUZZ_ACP_LAZY_POOL\" \\
\"$BUZZ_ACP_RELAY_OBSERVER\" \\
\"$BUZZ_ACP_SESSION_TITLE\" \\
\"$BUZZ_ACP_TEAM_INSTRUCTIONS\" \\
\"$1:$2\" > {}\n",
            shell_quote(&observed_path)
        ),
    )
    .expect("write fake harness");
    make_executable(&harness_path);

    fs::write(
        &provider_path,
        r#"#!/usr/bin/env python3
import json
import os
import subprocess
import sys

request = json.load(sys.stdin)
agent = request["agent"]
launch = agent["launch"]
assert isinstance(launch["command"], str) and launch["command"]
assert isinstance(launch["args"], list)
assert isinstance(launch["env"], dict)
assert isinstance(launch["policy_env"], dict)
assert launch["owner_pubkey"]
policy_env = launch["policy_env"]
launch_env = launch["env"]
env = os.environ.copy()
env["BUZZ_RELAY_URL"] = agent["relay_url"]
env["BUZZ_PRIVATE_KEY"] = agent["private_key_nsec"]
if agent.get("auth_tag"):
    env["BUZZ_AUTH_TAG"] = agent["auth_tag"]
else:
    env["BUZZ_ACP_AGENT_OWNER"] = launch["owner_pubkey"]
env.update(policy_env)
env.update(launch_env)
# The policy key is reserved: it wins even if launch.env contained a stale value.
env["BUZZ_ACP_REPLY_PLACEMENT"] = policy_env["BUZZ_ACP_REPLY_PLACEMENT"]
subprocess.run(
    [launch["command"], *launch["args"]],
    check=True,
    env=env,
)
print(json.dumps({"ok": True, "agent_id": "fake-provider"}))
"#,
    )
    .expect("write fake provider");
    make_executable(&provider_path);

    let _path_guard = crate::managed_agents::lock_path_mutex();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let provider_path_value = format!(
        "{}:{}",
        dir.path().display(),
        original_path.to_string_lossy()
    );
    std::env::set_var("PATH", provider_path_value);

    for mode in [
        crate::managed_agents::ReplyPlacement::Thread,
        crate::managed_agents::ReplyPlacement::TopLevel,
        crate::managed_agents::ReplyPlacement::FollowScope,
    ] {
        let launch = crate::managed_agents::EffectiveHarnessDescriptor {
            command: "fake-harness".to_string(),
            args: vec!["--contract".to_string(), "v1".to_string()],
            env: std::collections::BTreeMap::from([
                ("BUZZ_ACP_REPLY_PLACEMENT".to_string(), "thread".to_string()),
                (
                    "FAKE_LAUNCH_MARKER".to_string(),
                    "from-descriptor".to_string(),
                ),
                (
                    "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
                    "user-prompt".to_string(),
                ),
                ("BUZZ_ACP_IDLE_TIMEOUT".to_string(), "99".to_string()),
            ]),
        };
        let policy_env = std::collections::BTreeMap::from([
            (
                "BUZZ_ACP_SYSTEM_PROMPT".to_string(),
                "desktop-prompt".to_string(),
            ),
            ("BUZZ_ACP_IDLE_TIMEOUT".to_string(), "11".to_string()),
            ("BUZZ_ACP_MAX_TURN_DURATION".to_string(), "22".to_string()),
            ("BUZZ_ACP_AGENTS".to_string(), "4".to_string()),
            ("GOOSE_MODE".to_string(), "auto".to_string()),
            ("BUZZ_ACP_LAZY_POOL".to_string(), "true".to_string()),
            ("BUZZ_ACP_RELAY_OBSERVER".to_string(), "true".to_string()),
            (
                "BUZZ_ACP_SESSION_TITLE".to_string(),
                "Policy Display".to_string(),
            ),
            (
                "BUZZ_ACP_TEAM_INSTRUCTIONS".to_string(),
                "shared team instructions".to_string(),
            ),
            (
                "BUZZ_ACP_REPLY_PLACEMENT".to_string(),
                mode.as_str().to_string(),
            ),
        ]);
        let payload = deploy_payload_json(
            &bare_agent_record(None, None, None),
            DeployPayloadContext {
                relay_url: "wss://relay.example".to_string(),
                effective_model: None,
                effective_provider: None,
                effective_prompt: None,
                reply_placement: mode,
                merged_env: std::collections::BTreeMap::from([(
                    "BUZZ_ACP_REPLY_PLACEMENT".to_string(),
                    "thread".to_string(),
                )]),
                launch,
                policy_env,
                owner_pubkey: Some("owner-proof".to_string()),
            },
        );

        let agent_id = provider_deploy(&provider_path, &payload, &serde_json::json!({}))
            .expect("fake provider deploy");
        assert_eq!(agent_id, "fake-provider");
        assert!(payload["launch"]["env"]
            .get("BUZZ_ACP_REPLY_PLACEMENT")
            .is_none());
        let observed = fs::read_to_string(&observed_path).expect("fake harness output");
        assert_eq!(
            observed,
            format!(
                "from-descriptor|{}|user-prompt|99|22|4|auto|true|true|Policy Display|shared team instructions|--contract:v1",
                mode.as_str()
            )
        );
    }

    std::env::set_var("PATH", original_path);
}
