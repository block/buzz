use super::*;

#[test]
fn normalize_relay_mesh_rejects_empty_model_ref() {
    let config = RelayMeshConfig {
        model_ref: "  \t ".to_string(),
    };

    assert_eq!(
        normalize_relay_mesh(Some(&config), &BackendKind::Local).unwrap_err(),
        "relay mesh modelRef is required"
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
        "relay mesh agents must use the local backend"
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
            "mcp_toolsets": "developer,search",
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
    assert_eq!(payload["mcp_toolsets"], "developer,search");
    assert_eq!(payload["respond_to"], "allowlist");
    assert_eq!(payload["respond_to_allowlist"][0], "a".repeat(64));
    assert_eq!(payload["model"], "gpt-x");
    assert_eq!(payload["provider"], "openai");
    assert_eq!(payload["relay_url"], "wss://relay.example");
}
