use super::{AgentDefinition, CatalogSource, ManagedAgentRecord};
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
    assert!(
        !record.provider_policy_pending,
        "pre-pending stores must deserialize as acknowledged"
    );
}

#[test]
fn pending_provider_policy_round_trips() {
    let mut record = sample_agent_record();
    record.provider_policy_pending = true;

    let json = serde_json::to_string(&record).expect("serialize pending policy");
    let reloaded: ManagedAgentRecord = serde_json::from_str(&json).expect("reload pending policy");

    assert!(reloaded.provider_policy_pending);
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
        description: None,
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
        shared: false,
        source_team: Some("team-1".to_string()),
        source_team_persona_slug: Some("helper".to_string()),
        catalog_source: None,
        team_catalog_source: None,
        env_vars: [("K".to_string(), "v".to_string())].into_iter().collect(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-02T00:00:00Z".to_string(),
    }
}

#[test]
fn persona_record_without_catalog_source_deserializes_and_omits_it() {
    // Every persona already on disk predates the field — an old record must
    // load as "not a catalog copy" and must not gain a null key on save.
    let record: AgentDefinition = serde_json::from_str(
        r#"{
            "id": "persona-1",
            "display_name": "Test",
            "avatar_url": null,
            "system_prompt": "Prompt",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#,
    )
    .expect("pre-catalog-source persona should deserialize");

    assert_eq!(record.catalog_source, None);
    let json = serde_json::to_string(&record).unwrap();
    assert!(
        !json.contains("catalog_source"),
        "absent provenance must stay absent on disk: {json}"
    );
}

#[test]
fn persona_catalog_source_survives_the_agent_store_fold() {
    // Provenance is only useful if it is still there on the next launch, and
    // `save_personas` funnels every definition through `into_agent_record`.
    let mut persona = sample_persona();
    persona.catalog_source = Some(CatalogSource {
        owner_pubkey: "a".repeat(64),
        persona_id: "helper".to_string(),
    });

    let view = persona
        .clone()
        .into_agent_record()
        .to_definition_view()
        .expect("slugged record must present a persona view");

    assert_eq!(view.catalog_source, persona.catalog_source);
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

// ── Restart-diff wire shape ─────────────────────────────────────────────────

fn summary_fixture(
    restart_diff: Vec<crate::managed_agents::spawn_snapshot::RestartDiffEntry>,
) -> super::ManagedAgentSummary {
    super::ManagedAgentSummary {
        pubkey: "aa".repeat(32),
        name: "test".into(),
        persona_id: None,
        runtime: None,
        team_id: None,
        relay_url: String::new(),
        acp_command: "buzz-acp".into(),
        agent_command: "goose".into(),
        agent_command_override: None,
        agent_args: Vec::new(),
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        avatar_url: None,
        model: None,
        model_source: None,
        provider: None,
        persona_out_of_date: false,
        persona_orphaned: false,
        // Both fields derive from one vector in `build_managed_agent_summary`;
        // the fixture reproduces that rule rather than letting them disagree.
        needs_restart: !restart_diff.is_empty(),
        restart_diff,
        env_vars: Default::default(),
        backend: super::BackendKind::Local,
        backend_agent_id: None,
        status: "running".into(),
        pid: Some(4242),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        start_on_app_launch: false,
        auto_restart_on_config_change: false,
        log_path: String::new(),
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: Vec::new(),
    }
}

#[test]
fn summary_without_drift_omits_restart_diff_from_the_wire() {
    // An adopted `runtime_pid`-only process is never stamped, so its summary
    // carries an empty vector. `skip_serializing_if` must then drop the key
    // entirely — the frontend normalizes omission to `[]`, and emitting an
    // empty array on every stopped agent would bloat every list response.
    let wire = serde_json::to_value(summary_fixture(Vec::new())).expect("summary serializes");
    assert_eq!(wire.get("needs_restart"), Some(&serde_json::json!(false)));
    assert!(
        wire.get("restart_diff").is_none(),
        "empty restart_diff must be omitted, got: {wire}"
    );
}

#[test]
fn summary_with_drift_serializes_restart_diff_entries() {
    // The other side of the same rule: a present entry must reach the wire
    // under its snake_case key with the tagged change payload intact.
    let wire = serde_json::to_value(summary_fixture(vec![
        crate::managed_agents::spawn_snapshot::RestartDiffEntry {
            field: "model".into(),
            change: crate::managed_agents::spawn_snapshot::diff::RestartChange::Value {
                before: serde_json::json!("gpt-5"),
                after: serde_json::json!("claude-4"),
            },
        },
    ]))
    .expect("summary serializes");
    assert_eq!(wire.get("needs_restart"), Some(&serde_json::json!(true)));
    assert_eq!(
        wire.get("restart_diff"),
        Some(&serde_json::json!([{
            "field": "model",
            "change": { "kind": "value", "before": "gpt-5", "after": "claude-4" },
        }]))
    );
}
