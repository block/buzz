use super::{
    merge_mcp_servers, replace_mcp_servers, validate_effective_mcp_cap,
    validate_effective_mcp_cap_for_records, validate_mcp_servers, AgentDefinition,
    ManagedAgentRecord, McpServerConfig, McpServerEnvVar, MAX_USER_MCP_SERVERS,
};
use std::path::PathBuf;

#[test]
fn persona_record_defaults_active_when_field_is_missing() {
    let record: AgentDefinition = serde_json::from_str(
        r#"{
            "id": "builtin:fizz",
            "display_name": "Fizz",
            "avatar_url": null,
            "system_prompt": "Prompt",
            "created_at": "2026-03-19T00:00:00Z",
            "updated_at": "2026-03-19T00:00:00Z"
        }"#,
    )
    .expect("legacy persona payload should deserialize");

    assert!(record.is_active);
    assert!(!record.is_builtin);
    assert_eq!(record.runtime, None);
    assert_eq!(record.model, None);
    assert!(record.name_pool.is_empty());
}

/// Legacy agent records (created before NIP-OA) lack the `auth_tag` field.
/// `#[serde(default)]` must ensure they deserialize with `auth_tag: None`.
#[test]
fn managed_agent_record_without_auth_tag_deserializes() {
    let record: ManagedAgentRecord = serde_json::from_str(
        r#"{
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
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("legacy agent record without auth_tag should deserialize");

    assert_eq!(record.auth_tag, None);
    assert_eq!(record.avatar_url, None);
    assert_eq!(record.pubkey, "abcd1234");
}

/// Agent records WITH an auth_tag round-trip correctly through serde.
#[test]
fn managed_agent_record_with_auth_tag_round_trips() {
    let json = r#"{
        "pubkey": "abcd1234",
        "name": "test-agent",
        "private_key_nsec": "nsec1fake",
        "auth_tag": "[\"auth\",\"deadbeef\",\"\",\"cafebabe\"]",
        "relay_url": "wss://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "system_prompt": null,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    }"#;

    let record: ManagedAgentRecord =
        serde_json::from_str(json).expect("record with auth_tag should deserialize");

    assert_eq!(
        record.auth_tag.as_deref(),
        Some(r#"["auth","deadbeef","","cafebabe"]"#)
    );

    // Round-trip: serialize and deserialize again.
    let serialized = serde_json::to_string(&record).expect("should serialize");
    let record2: ManagedAgentRecord =
        serde_json::from_str(&serialized).expect("round-trip should deserialize");
    assert_eq!(record.auth_tag, record2.auth_tag);
}

// ── Inbound author gate tests ────────────────────────────────────────

use super::{validate_respond_to_allowlist, RespondTo};

#[test]
fn respond_to_default_is_owner_only() {
    assert_eq!(RespondTo::default(), RespondTo::OwnerOnly);
}

#[test]
fn respond_to_serde_is_kebab_case() {
    assert_eq!(
        serde_json::to_string(&RespondTo::OwnerOnly).unwrap(),
        "\"owner-only\""
    );
    assert_eq!(
        serde_json::to_string(&RespondTo::Allowlist).unwrap(),
        "\"allowlist\""
    );
    assert_eq!(
        serde_json::to_string(&RespondTo::Anyone).unwrap(),
        "\"anyone\""
    );
    let parsed: RespondTo = serde_json::from_str("\"owner-only\"").unwrap();
    assert_eq!(parsed, RespondTo::OwnerOnly);
    let parsed: RespondTo = serde_json::from_str("\"allowlist\"").unwrap();
    assert_eq!(parsed, RespondTo::Allowlist);
    let parsed: RespondTo = serde_json::from_str("\"anyone\"").unwrap();
    assert_eq!(parsed, RespondTo::Anyone);
}

#[test]
fn respond_to_rejects_unknown_modes() {
    // `nobody` is a valid harness mode but intentionally not exposed
    // through the desktop request types.
    assert!(serde_json::from_str::<RespondTo>("\"nobody\"").is_err());
    assert!(serde_json::from_str::<RespondTo>("\"OwnerOnly\"").is_err());
}

/// Records persisted before this feature must continue to load,
/// defaulting to OwnerOnly (the safe, matches-harness-default value).
#[test]
fn managed_agent_record_without_respond_to_fields_defaults_to_owner_only() {
    let record: ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "abcd1234",
            "name": "legacy-agent",
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("legacy record without respond_to fields should deserialize");
    assert_eq!(record.respond_to, RespondTo::OwnerOnly);
    assert!(record.respond_to_allowlist.is_empty());
}

#[test]
fn validate_respond_to_allowlist_accepts_valid_hex_and_lowercases() {
    let upper = "A".repeat(64);
    let lower = "a".repeat(64);
    let result = validate_respond_to_allowlist(std::slice::from_ref(&upper)).unwrap();
    assert_eq!(result, vec![lower.clone()]);
}

#[test]
fn validate_respond_to_allowlist_dedups_preserving_order() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let a_upper = "A".repeat(64);
    let input = vec![a.clone(), b.clone(), a_upper];
    let result = validate_respond_to_allowlist(&input).unwrap();
    assert_eq!(result, vec![a, b]);
}

#[test]
fn validate_respond_to_allowlist_rejects_wrong_length() {
    let too_short = "a".repeat(63);
    assert!(validate_respond_to_allowlist(&[too_short]).is_err());
    let too_long = "a".repeat(65);
    assert!(validate_respond_to_allowlist(&[too_long]).is_err());
}

#[test]
fn validate_respond_to_allowlist_rejects_non_hex() {
    let bad = "z".repeat(64);
    assert!(validate_respond_to_allowlist(&[bad]).is_err());
    // npub-style strings should not slip through.
    let npub = format!("npub1{}", "a".repeat(59));
    assert!(validate_respond_to_allowlist(&[npub]).is_err());
}

#[test]
fn validate_respond_to_allowlist_trims_whitespace() {
    let padded = format!("  {}  ", "a".repeat(64));
    let result = validate_respond_to_allowlist(&[padded]).unwrap();
    assert_eq!(result, vec!["a".repeat(64)]);
}

#[test]
fn validate_respond_to_allowlist_accepts_empty() {
    // Empty is allowed at this layer; the boundary check
    // (Allowlist mode requires ≥1 entry) is the caller's job.
    let result = validate_respond_to_allowlist(&[]).unwrap();
    assert!(result.is_empty());
}

#[test]
fn update_request_provider_tristate_absent_means_no_touch() {
    // A JSON payload with no "provider" key deserialized with `None` —
    // the backend must leave the record's existing provider unchanged.
    let request: super::UpdateManagedAgentRequest =
        serde_json::from_str(r#"{"pubkey": "abcd1234"}"#)
            .expect("minimal update request should deserialize");
    assert!(
        request.provider.is_none(),
        "absent provider must deserialize to None (don't touch)"
    );
}

#[test]
fn update_request_provider_tristate_null_means_clear() {
    // A JSON payload with `"provider": null` deserialized with `Some(None)` —
    // the backend must clear the record's provider back to the runtime default.
    let request: super::UpdateManagedAgentRequest =
        serde_json::from_str(r#"{"pubkey": "abcd1234", "provider": null}"#)
            .expect("null provider request should deserialize");
    assert_eq!(
        request.provider,
        Some(None),
        "explicit null must deserialize to Some(None) (clear)"
    );
}

#[test]
fn update_request_provider_tristate_value_means_set() {
    // A JSON payload with a provider string deserialized with `Some(Some(…))`.
    let request: super::UpdateManagedAgentRequest =
        serde_json::from_str(r#"{"pubkey": "abcd1234", "provider": "databricks_v2"}"#)
            .expect("provider value request should deserialize");
    assert_eq!(
        request.provider,
        Some(Some("databricks_v2".to_string())),
        "provider value must deserialize to Some(Some(value)) (set)"
    );
}

use super::{CreateManagedAgentRequest, RelayMeshConfig};

/// Wire-shape test: the create request arrives from TS as camelCase
/// (`relayMesh: { modelRef }`). `rename_all = "camelCase"` on
/// `CreateManagedAgentRequest` does NOT recurse into nested structs, so
/// `RelayMeshConfig` needs its own `alias = "modelRef"`. This test pins
/// the exact JSON the frontend sends; if the alias is dropped, creating
/// a relay-mesh agent fails to deserialize at the Tauri boundary.
#[test]
fn create_request_deserializes_camel_case_relay_mesh() {
    let request: CreateManagedAgentRequest = serde_json::from_str(
        r#"{
            "name": "mesh-agent",
            "relayMesh": { "modelRef": "Qwen3" }
        }"#,
    )
    .expect("camelCase relayMesh payload from TS should deserialize");
    assert_eq!(
        request.relay_mesh,
        Some(RelayMeshConfig {
            model_ref: "Qwen3".to_string()
        })
    );
}

/// Persisted records use snake_case; the camelCase alias must not break
/// the stored-record round trip.
#[test]
fn relay_mesh_config_round_trips_snake_case() {
    let config = RelayMeshConfig {
        model_ref: "Qwen3".to_string(),
    };
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(json, r#"{"model_ref":"Qwen3"}"#);
    let back: RelayMeshConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back, config);
}

// ── Packs → Teams serde alias backward compatibility ────────────────

#[test]
fn persona_record_deserializes_old_source_pack_fields_via_alias() {
    let record: AgentDefinition = serde_json::from_str(
        r#"{
            "id": "persona-1",
            "display_name": "Test",
            "avatar_url": null,
            "system_prompt": "Prompt",
            "source_pack": "com.example.my-pack",
            "source_pack_persona_slug": "agent-one",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#,
    )
    .expect("old-format persona with source_pack should deserialize via alias");

    assert_eq!(record.source_team.as_deref(), Some("com.example.my-pack"));
    assert_eq!(
        record.source_team_persona_slug.as_deref(),
        Some("agent-one")
    );
}

#[test]
fn persona_record_serializes_new_field_names() {
    let record: AgentDefinition = serde_json::from_str(
        r#"{
            "id": "persona-1",
            "display_name": "Test",
            "avatar_url": null,
            "system_prompt": "Prompt",
            "source_team": "com.example.my-team",
            "source_team_persona_slug": "agent-one",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#,
    )
    .unwrap();

    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("source_team"));
    assert!(json.contains("source_team_persona_slug"));
    assert!(!json.contains("source_pack"));
}

#[test]
fn managed_agent_record_deserializes_old_pack_path_fields_via_alias() {
    let record: ManagedAgentRecord = serde_json::from_str(
        r#"{
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
            "persona_pack_path": "/path/to/agents/packs/my-pack",
            "persona_name_in_pack": "agent-one",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("old-format agent with persona_pack_path should deserialize via alias");

    assert_eq!(
        record.persona_team_dir,
        Some(PathBuf::from("/path/to/agents/packs/my-pack"))
    );
    assert_eq!(record.persona_name_in_team.as_deref(), Some("agent-one"));
}

#[test]
fn team_record_deserializes_without_new_fields() {
    let record: super::TeamRecord = serde_json::from_str(
        r#"{
            "id": "team-1",
            "name": "My Team",
            "description": null,
            "persona_ids": ["p1", "p2"],
            "is_builtin": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#,
    )
    .expect("team record without new fields should deserialize with defaults");

    assert_eq!(record.source_dir, None);
    assert!(!record.is_symlink);
    assert_eq!(record.symlink_target, None);
    assert_eq!(record.version, None);
}

/// A record whose in-memory key was blanked (because it lives in the
/// keyring) must NOT serialize `private_key_nsec` into JSON.
#[test]
fn managed_agent_record_omits_empty_key_from_json() {
    let mut record = sample_agent_record();
    record.private_key_nsec = String::new();

    let json = serde_json::to_string(&record).expect("serialize");
    assert!(
        !json.contains("private_key_nsec"),
        "blanked key must be skipped from JSON, got: {json}"
    );
}

/// A record with an inline key (the keyringless `0o600` JSON fallback)
/// serializes the key and round-trips it back.
#[test]
fn managed_agent_record_serializes_inline_key_for_fallback() {
    let mut record = sample_agent_record();
    record.private_key_nsec = "nsec1fallback".to_string();

    let json = serde_json::to_string(&record).expect("serialize");
    assert!(json.contains("nsec1fallback"));

    let back: ManagedAgentRecord = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.private_key_nsec, "nsec1fallback");
}

/// A keyring-backed record on disk lacks `private_key_nsec`; it must
/// deserialize with an empty key (to be hydrated from the keyring).
#[test]
fn managed_agent_record_without_key_deserializes_empty() {
    let record: ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "abcd1234",
            "name": "test-agent",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("keyring-backed record without inline key should deserialize");

    assert_eq!(record.private_key_nsec, "");
}

fn sample_agent_record() -> ManagedAgentRecord {
    serde_json::from_str(
        r#"{
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
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("sample record")
}

// ── AgentDefinition ↔ ManagedAgentRecord fold mapping (Phase 1A) ─────────────────────

fn sample_persona() -> AgentDefinition {
    AgentDefinition {
        mcp_servers: vec![],
        id: "custom:helper".to_string(),
        display_name: "Helper".to_string(),
        avatar_url: Some("https://example.com/a.png".to_string()),
        system_prompt: "You help.".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("gpt-x".to_string()),
        provider: Some("openai".to_string()),
        name_pool: vec!["Nimble".to_string()],
        is_builtin: false,
        is_active: true,
        source_team: Some("team-1".to_string()),
        source_team_persona_slug: Some("helper".to_string()),
        env_vars: [("K".to_string(), "v".to_string())].into_iter().collect(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    }
}

#[test]
fn persona_into_agent_record_is_keyless_and_slugged() {
    let record = sample_persona().into_agent_record();
    assert!(record.pubkey.is_empty(), "fold must not mint identity");
    assert!(record.private_key_nsec.is_empty());
    assert_eq!(record.slug.as_deref(), Some("custom:helper"));
    assert_eq!(record.display_name.as_deref(), Some("Helper"));
    assert_eq!(record.system_prompt.as_deref(), Some("You help."));
    assert_eq!(record.runtime.as_deref(), Some("goose"));
    assert_eq!(record.source_team.as_deref(), Some("team-1"));
    assert_eq!(record.env_vars.get("K").map(String::as_str), Some("v"));
    assert!(
        record.mcp_servers.is_empty(),
        "definition MCP servers remain inherited"
    );
}

#[test]
fn persona_view_round_trips_through_agent_record() {
    let persona = sample_persona();
    let view = persona
        .clone()
        .into_agent_record()
        .to_definition_view()
        .expect("slugged record must present a persona view");
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        serde_json::to_value(&persona).unwrap(),
        "fold + view must round-trip every persona field"
    );
}

#[test]
fn keyed_record_without_slug_has_no_persona_view() {
    let mut record = sample_persona().into_agent_record();
    record.slug = None;
    assert!(
        record.to_definition_view().is_none(),
        "instances (no slug) are not definitions"
    );
}

#[test]
fn empty_prompt_folds_to_none() {
    let mut persona = sample_persona();
    persona.system_prompt = String::new();
    assert_eq!(persona.into_agent_record().system_prompt, None);
}

// ── Mint-time behavioral defaults (B5 quad activation) ──────────────────────

use super::resolve_mint_behavioral_defaults;

fn quad_definition(respond_to: &str, allowlist: Vec<&str>) -> AgentDefinition {
    let mut persona = sample_persona();
    persona.respond_to = Some(respond_to.to_string());
    persona.respond_to_allowlist = allowlist.into_iter().map(str::to_string).collect();
    persona.parallelism = Some(8);
    persona
}

#[test]
fn mint_explicit_input_wins_over_definition() {
    let definition = quad_definition("anyone", vec![]);
    let minted = resolve_mint_behavioral_defaults(
        Some(RespondTo::OwnerOnly),
        Vec::new(),
        Some(2),
        Some(&definition),
    )
    .unwrap();
    assert_eq!(minted.respond_to, RespondTo::OwnerOnly);
    assert_eq!(minted.parallelism, Some(2));
}

#[test]
fn mint_copies_definition_quad_when_input_silent() {
    let allow = "a".repeat(64);
    let definition = quad_definition("allowlist", vec![&allow]);
    let minted =
        resolve_mint_behavioral_defaults(None, Vec::new(), None, Some(&definition)).unwrap();
    assert_eq!(minted.respond_to, RespondTo::Allowlist);
    assert_eq!(minted.respond_to_allowlist, vec![allow]);
    assert_eq!(minted.parallelism, Some(8));
}

#[test]
fn mint_without_definition_or_input_uses_client_defaults() {
    let minted = resolve_mint_behavioral_defaults(None, Vec::new(), None, None).unwrap();
    assert_eq!(minted.respond_to, RespondTo::default());
    assert!(minted.respond_to_allowlist.is_empty());
    assert_eq!(minted.parallelism, None);
}

#[test]
fn mint_fails_loudly_on_unknown_definition_respond_to() {
    // A typo'd mode must never silently become owner-only — the definition
    // author intended SOMETHING, and guessing which thing is the one wrong
    // move. The error must carry the offending string.
    let definition = quad_definition("allowlst", vec![]);
    let err =
        resolve_mint_behavioral_defaults(None, Vec::new(), None, Some(&definition)).unwrap_err();
    assert!(
        err.contains("allowlst"),
        "error must name the bad mode: {err}"
    );
}

#[test]
fn mint_fails_loudly_on_empty_definition_allowlist() {
    // Inbound definitions bypass the dialog guard entirely — the mint
    // boundary is the backstop against a crash-looping instance.
    let definition = quad_definition("allowlist", vec![]);
    let err =
        resolve_mint_behavioral_defaults(None, Vec::new(), None, Some(&definition)).unwrap_err();
    assert!(
        err.contains("at least one pubkey"),
        "unexpected error: {err}"
    );
}

#[test]
fn mint_fails_loudly_on_out_of_range_definition_parallelism() {
    let mut definition = quad_definition("anyone", vec![]);
    definition.parallelism = Some(64);
    let err =
        resolve_mint_behavioral_defaults(None, Vec::new(), None, Some(&definition)).unwrap_err();
    assert!(err.contains("64"), "error must name the bad value: {err}");
}

#[test]
fn mint_normalizes_definition_allowlist_from_wire() {
    let upper = "A".repeat(64);
    let definition = quad_definition("allowlist", vec![&upper]);
    let minted =
        resolve_mint_behavioral_defaults(None, Vec::new(), None, Some(&definition)).unwrap();
    assert_eq!(minted.respond_to_allowlist, vec!["a".repeat(64)]);
}

#[test]
fn mint_resolves_each_behavioral_field_independently() {
    // PR #1667 review (convergent): the input-wins rule is per-FIELD, not
    let definition = quad_definition("anyone", vec![]);
    let minted =
        resolve_mint_behavioral_defaults(None, Vec::new(), None, Some(&definition)).unwrap();
    assert_eq!(minted.respond_to, RespondTo::Anyone, "inherited");
    assert_eq!(minted.parallelism, Some(8), "inherited");
}

#[test]
fn mint_rejects_out_of_range_input_parallelism() {
    // The "validated when present" contract on MintBehavioralDefaults holds
    // for the INPUT branch too, not just definition values.
    let err = resolve_mint_behavioral_defaults(None, Vec::new(), Some(64), None).unwrap_err();
    assert!(err.contains("64"), "error must name the bad value: {err}");
    assert!(
        !err.contains("definition"),
        "input-branch error must not blame the definition: {err}"
    );
}

fn mcp_server(name: &str, command: &str, enabled: bool) -> McpServerConfig {
    McpServerConfig {
        name: name.into(),
        command: command.into(),
        args: vec![],
        env: vec![],
        enabled,
    }
}

#[test]
fn merge_mcp_servers_higher_layer_replaces_same_name() {
    let merged = merge_mcp_servers(
        &[mcp_server("shared", "global", true)],
        &[mcp_server("shared", "persona", true)],
        &[mcp_server("shared", "agent", true)],
    )
    .unwrap();

    assert_eq!(merged, vec![mcp_server("shared", "agent", true)]);
}

#[test]
fn merge_mcp_servers_disabled_override_masks_lower_server() {
    let merged = merge_mcp_servers(
        &[mcp_server("shared", "global", true)],
        &[],
        &[mcp_server("shared", "agent", false)],
    )
    .unwrap();

    assert!(merged.is_empty());
}

#[test]
fn disabled_mask_requires_only_a_name() {
    let mask = McpServerConfig {
        name: "shared".into(),
        command: String::new(),
        args: vec![],
        env: vec![],
        enabled: false,
    };
    validate_mcp_servers(std::slice::from_ref(&mask)).unwrap();
    assert!(
        merge_mcp_servers(&[mcp_server("shared", "global", true)], &[], &[mask])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn replace_mcp_servers_preserves_absent_and_replaces_present_empty_layer() {
    let mut current = vec![mcp_server("existing", "mcp", true)];
    replace_mcp_servers(&mut current, &None).unwrap();
    assert_eq!(current, vec![mcp_server("existing", "mcp", true)]);

    replace_mcp_servers(&mut current, &Some(vec![])).unwrap();
    assert!(current.is_empty());
}

#[test]
fn merge_mcp_servers_rejects_more_than_fifteen_enabled_servers_after_merge() {
    let global: Vec<_> = (0..MAX_USER_MCP_SERVERS)
        .map(|index| mcp_server(&format!("global-{index}"), "mcp", true))
        .collect();
    let error = merge_mcp_servers(&global, &[mcp_server("persona", "mcp", true)], &[])
        .expect_err("post-merge cap must include servers from all layers");

    assert!(error.contains("effective MCP server count is 16"));
}

#[test]
fn validate_mcp_servers_rejects_empty_duplicate_and_reserved_env_inputs() {
    assert!(validate_mcp_servers(&[mcp_server("", "mcp", true)]).is_err());
    assert!(validate_mcp_servers(&[
        mcp_server("same", "mcp", true),
        mcp_server("same", "other", true),
    ])
    .is_err());
    let mut server = mcp_server("server", "mcp", true);
    server.env.push(McpServerEnvVar {
        name: "BUZZ_ACP_MCP_SERVERS".into(),
        value: "spoof".into(),
    });
    assert!(validate_mcp_servers(&[server]).is_err());
}

#[test]
fn validate_mcp_servers_rejects_invalid_name_characters() {
    // Space, slash, dot — not in the buzz-agent grammar.
    for bad_name in &["bad name", "bad/name", "bad.name", "bad@name"] {
        assert!(
            validate_mcp_servers(&[mcp_server(bad_name, "cmd", true)]).is_err(),
            "expected error for name {bad_name:?}"
        );
    }
}

#[test]
fn validate_mcp_servers_rejects_double_underscore_in_name() {
    assert!(validate_mcp_servers(&[mcp_server("bad__name", "cmd", true)]).is_err());
}

#[test]
fn validate_mcp_servers_rejects_reserved_name() {
    assert!(validate_mcp_servers(&[mcp_server("buzz-dev-mcp", "cmd", true)]).is_err());
}

#[test]
fn validate_mcp_servers_accepts_valid_grammar() {
    // Alphanumeric, hyphens, underscores (but not double) are all OK.
    assert!(validate_mcp_servers(&[mcp_server("github", "cmd", true)]).is_ok());
    assert!(validate_mcp_servers(&[mcp_server("my-server", "cmd", true)]).is_ok());
    assert!(validate_mcp_servers(&[mcp_server("my_server", "cmd", true)]).is_ok());
    assert!(validate_mcp_servers(&[mcp_server("Server123", "cmd", true)]).is_ok());
}

#[test]
fn validate_mcp_servers_rejects_more_than_fifteen_enabled_in_one_layer() {
    let servers: Vec<_> = (0..=MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("server-{i}"), "cmd", true))
        .collect();
    let err = validate_mcp_servers(&servers).expect_err("layer cap must fire at 16 enabled");
    assert!(err.contains("enabled servers"), "error: {err}");
}

#[test]
fn validate_mcp_servers_allows_fifteen_enabled_in_one_layer() {
    let servers: Vec<_> = (0..MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("server-{i}"), "cmd", true))
        .collect();
    assert!(validate_mcp_servers(&servers).is_ok());
}

// ── validate_effective_mcp_cap — save-time effective merge cap ──────────

/// Build a minimal buzz-agent ManagedAgentRecord with the given mcp_servers
/// layer. `agent_command = "buzz-agent"` so the effective check fires.
fn buzz_agent_record(mcp_servers: Vec<McpServerConfig>) -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": "aabbccdd",
        "name": "test",
        "relay_url": "",
        "acp_command": "buzz-acp",
        "agent_command": "buzz-agent",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "system_prompt": null,
        "mcp_servers": mcp_servers,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .expect("minimal record should deserialize")
}

#[test]
fn effective_cap_rejects_15_global_plus_1_local() {
    let global: Vec<_> = (0..MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    let record = buzz_agent_record(vec![mcp_server("local-0", "cmd", true)]);
    let err = validate_effective_mcp_cap(&record, &[], &global, "buzz-agent")
        .expect_err("effective cap must reject 16 enabled");
    assert!(err.contains("effective MCP server count"), "error: {err}");
}

#[test]
fn effective_cap_allows_at_cap() {
    let global: Vec<_> = (0..MAX_USER_MCP_SERVERS - 1)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    let record = buzz_agent_record(vec![mcp_server("local-0", "cmd", true)]);
    validate_effective_mcp_cap(&record, &[], &global, "buzz-agent")
        .expect("15 effective should pass");
}

#[test]
fn effective_cap_rejects_rename_unmask() {
    // Global has 15 enabled. The agent has a same-name override that masks
    // one of them. If the override is "renamed" (simulated by changing the
    // name to something unique), the masked global server is un-masked and
    // the effective count goes to 16.
    let global: Vec<_> = (0..MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    // Agent overrides global-0 (masks it) — effective = 15.
    let record_masked = buzz_agent_record(vec![mcp_server("global-0", "", false)]);
    validate_effective_mcp_cap(&record_masked, &[], &global, "buzz-agent")
        .expect("masked state should be at-cap and valid");

    // Agent renamed its override → unique name, un-masks global-0 → effective = 16.
    let record_unmasked = buzz_agent_record(vec![mcp_server("unique-name", "cmd", true)]);
    let err = validate_effective_mcp_cap(&record_unmasked, &[], &global, "buzz-agent")
        .expect_err("rename-unmask must reject");
    assert!(err.contains("effective MCP server count"), "error: {err}");
}

#[test]
fn effective_cap_skips_non_buzz_agent_runtime() {
    let global: Vec<_> = (0..=MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    let record = buzz_agent_record(vec![mcp_server("local-0", "cmd", true)]);
    // Non-buzz-agent runtime → effective check returns Ok (skip).
    validate_effective_mcp_cap(&record, &[], &global, "goose")
        .expect("non-buzz-agent runtime should skip the cap");
}

// ── validate_effective_mcp_cap_for_records — inherited-layer gates ───────

/// Build a buzz-agent record with a persona reference and custom mcp_servers.
fn buzz_agent_record_with_persona(
    name: &str,
    persona_id: &str,
    mcp_servers: Vec<McpServerConfig>,
) -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": format!("pk-{name}"),
        "name": name,
        "persona_id": persona_id,
        "relay_url": "",
        "acp_command": "buzz-acp",
        "agent_command": "buzz-agent",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "system_prompt": null,
        "mcp_servers": mcp_servers,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .expect("minimal record should deserialize")
}

/// Build a minimal AgentDefinition (persona) with the given mcp_servers.
fn persona_with_mcp(id: &str, mcp_servers: Vec<McpServerConfig>) -> AgentDefinition {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "display_name": format!("Persona {id}"),
        "system_prompt": "",
        "mcp_servers": mcp_servers,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .expect("minimal persona should deserialize")
}

#[test]
fn inherited_gate_rejects_global_14_to_15_with_1_local_agent() {
    // Agent has 1 local enabled server. Global goes from 14 → 15. Effective
    // would be 16 — the gate must reject with the agent's name.
    let record = buzz_agent_record(vec![mcp_server("local-0", "cmd", true)]);
    let prospective_global: Vec<_> = (0..MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    let err = validate_effective_mcp_cap_for_records(&[record], &[], &prospective_global)
        .expect_err("global 14→15 with 1 local must reject");
    assert!(
        err.contains("test"),
        "error must name the offending agent: {err}"
    );
    assert!(
        err.contains("saving would push agent"),
        "error must use the required phrasing: {err}"
    );
}

#[test]
fn inherited_gate_allows_global_14_with_1_local_agent() {
    // Agent has 1 local. Global stays at 14. Effective = 15 (at cap) — OK.
    let record = buzz_agent_record(vec![mcp_server("local-0", "cmd", true)]);
    let prospective_global: Vec<_> = (0..MAX_USER_MCP_SERVERS - 1)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    validate_effective_mcp_cap_for_records(&[record], &[], &prospective_global)
        .expect("15 effective should pass");
}

#[test]
fn inherited_gate_rejects_persona_unmask_pushing_agent_over_cap() {
    // Agent has 1 local enabled. Global has 14. Persona goes from 0 → 1
    // unique enabled server. Effective = 16 — reject.
    let persona_id = "p1";
    let record = buzz_agent_record_with_persona(
        "agent-a",
        persona_id,
        vec![mcp_server("local-0", "cmd", true)],
    );
    let global: Vec<_> = (0..MAX_USER_MCP_SERVERS - 1)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    let persona = persona_with_mcp(persona_id, vec![mcp_server("persona-0", "cmd", true)]);
    let err = validate_effective_mcp_cap_for_records(&[record], &[persona], &global)
        .expect_err("persona add pushing over cap must reject");
    assert!(
        err.contains("agent-a"),
        "error must name the offending agent: {err}"
    );
}

#[test]
fn inherited_gate_skips_non_buzz_agent_records() {
    // A goose-runtime agent should be unaffected by the cap.
    let mut record = buzz_agent_record(vec![mcp_server("local-0", "cmd", true)]);
    // record_agent_command resolves via agent_command_override first.
    record.agent_command_override = Some("goose".to_string());
    let prospective_global: Vec<_> = (0..=MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    validate_effective_mcp_cap_for_records(&[record], &[], &prospective_global)
        .expect("non-buzz-agent runtime should skip the cap");
}

#[test]
fn inherited_gate_allows_unaffected_agent() {
    // Agent with no local servers. Global at 15 → effective = 15. OK.
    let record = buzz_agent_record(vec![]);
    let prospective_global: Vec<_> = (0..MAX_USER_MCP_SERVERS)
        .map(|i| mcp_server(&format!("global-{i}"), "cmd", true))
        .collect();
    validate_effective_mcp_cap_for_records(&[record], &[], &prospective_global)
        .expect("agent with no local servers at cap should pass");
}
