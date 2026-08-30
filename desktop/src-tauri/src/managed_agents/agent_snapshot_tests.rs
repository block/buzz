//! Unit tests for `managed_agents/agent_snapshot.rs`.
//!
//! Kept in a sibling file so `agent_snapshot.rs` stays under the
//! 1000-line gate; `#[path]`-included from there.

use super::*;
use crate::managed_agents::types::{BackendKind, ManagedAgentRecord, RespondTo};
use std::collections::BTreeMap;

/// Build a minimal `ManagedAgentRecord` for testing. Only the fields
/// relevant to snapshot export are filled; the rest use defaults.
fn minimal_record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "deadbeef".to_string(),
        name: "Test Agent".to_string(),
        display_name: Some("Test Agent Display".to_string()),
        persona_id: Some("SENTINEL_PERSONA_ID".to_string()), // MUST NOT appear in snapshot
        team_id: Some("SENTINEL_TEAM_ID".to_string()),       // MUST NOT appear in snapshot
        private_key_nsec: "nsec1secret".to_string(),         // MUST NOT appear in snapshot
        auth_tag: Some("auth-tag-secret".to_string()),       // MUST NOT appear in snapshot
        relay_url: "wss://relay.example.com".to_string(),    // MUST NOT appear in snapshot
        avatar_url: Some("https://example.com/avatar.png".to_string()),
        acp_command: "/usr/local/bin/acp".to_string(), // MUST NOT appear in snapshot
        agent_command: "goose".to_string(),            // MUST NOT appear in snapshot
        agent_command_override: Some("goose-override".to_string()), // MUST NOT appear
        agent_args: vec!["--arg".to_string()],         // MUST NOT appear in snapshot
        mcp_command: "mcp-server".to_string(),         // MUST NOT appear in snapshot
        turn_timeout_seconds: 120,                     // deprecated, MUST NOT appear
        idle_timeout_seconds: Some(30),
        max_turn_duration_seconds: Some(600),
        parallelism: 2,
        system_prompt: Some("You are a test agent.".to_string()),
        model: Some("claude-opus-4".to_string()),
        provider: Some("anthropic".to_string()),
        persona_source_version: Some("v1.0".to_string()), // MUST NOT appear
        env_vars: {
            let mut m = BTreeMap::new();
            m.insert("API_KEY".to_string(), "secret123".to_string()); // MUST NOT appear
            m
        },
        start_on_app_launch: true,
        auto_restart_on_config_change: true,
        runtime_pid: Some(12345), // MUST NOT appear
        backend: BackendKind::Provider {
            // MUST NOT appear — carries a provider secret
            id: "SENTINEL_BACKEND_ID".to_string(),
            config: serde_json::json!({"api_key": "SENTINEL_BACKEND_SECRET"}),
        },
        backend_agent_id: Some("SENTINEL_BACKEND_AGENT_ID".to_string()), // MUST NOT appear
        provider_policy_pending: false,
        provider_binary_path: Some("/usr/bin/SENTINEL_PROVIDER_BINARY".to_string()), // MUST NOT appear
        persona_team_dir: Some(std::path::PathBuf::from("SENTINEL_TEAM_DIR")), // MUST NOT appear
        persona_name_in_team: Some("SENTINEL_NAME_IN_TEAM".to_string()),       // MUST NOT appear
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
        last_started_at: Some("2024-01-03T00:00:00Z".to_string()), // MUST NOT appear
        last_stopped_at: None,
        last_exit_code: Some(0), // MUST NOT appear
        last_error: Some("SENTINEL_LAST_ERROR".to_string()), // MUST NOT appear
        last_error_code: Some(42), // MUST NOT appear
        respond_to: RespondTo::default(),
        respond_to_allowlist: vec!["pubkey1hex".to_string()],
        slug: Some("test-agent".to_string()),
        runtime: Some("goose".to_string()),
        name_pool: vec!["Alice".to_string(), "Bob".to_string()],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: Some("team-id-123".to_string()), // MUST NOT appear
        source_team_persona_slug: Some("lep".to_string()), // MUST NOT appear
        definition_respond_to: Some("allowlist".to_string()),
        catalog_source: None,
        team_catalog_source: None,
        definition_respond_to_allowlist: vec!["abc123def".to_string()],
        definition_parallelism: Some(4),
        relay_mesh: None,
        effort_level: None,
    }
}

// ── Round-trip tests ──────────────────────────────────────────────────────

#[test]
fn json_round_trip_config_only() {
    let record = minimal_record();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
    let bytes = encode_snapshot_json(&snapshot).unwrap();
    let parsed = decode_snapshot_json(&bytes).unwrap();
    assert_eq!(parsed, snapshot);
}

#[test]
fn json_round_trip_with_memory() {
    let record = minimal_record();
    let entries = vec![
        AgentSnapshotMemoryEntry {
            slug: "core".to_string(),
            body: "I am a test agent.".to_string(),
        },
        AgentSnapshotMemoryEntry {
            slug: "mem/research".to_string(),
            body: "Some research notes.".to_string(),
        },
    ];
    let snapshot = build_snapshot(&record, MemoryLevel::Everything, entries, None);
    let bytes = encode_snapshot_json(&snapshot).unwrap();
    let parsed = decode_snapshot_json(&bytes).unwrap();
    assert_eq!(parsed, snapshot);
}

#[test]
fn png_round_trip_no_memory() {
    let record = minimal_record();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
    let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();
    let parsed = decode_snapshot_png(&png_bytes).unwrap();
    assert_eq!(parsed.definition.name, snapshot.definition.name);
    assert_eq!(parsed.profile.display_name, snapshot.profile.display_name);
    assert_eq!(parsed.memory.level, MemoryLevel::None);
}

#[test]
fn png_round_trip_with_avatar_png() {
    // Build a minimal PNG avatar.
    let avatar = make_png_with_text("dummy", "value").unwrap();
    let record = minimal_record();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&avatar));
    // Avatar should be inlined as a data URL.
    assert!(snapshot
        .profile
        .avatar_data_url
        .as_deref()
        .unwrap_or("")
        .starts_with("data:image/png;base64,"));

    let png_bytes = encode_snapshot_png(&snapshot, Some(&avatar)).unwrap();
    let parsed = decode_snapshot_png(&png_bytes).unwrap();
    assert_eq!(parsed.definition.name, snapshot.definition.name);
}

/// Plain-card byte compatibility: `encode_snapshot_png` was refactored
/// through the shared `encode_chunk_payload_png` when locked cards were
/// added. Plain cards must emit byte-identical PNGs to the pre-envelope
/// encoder. This vector reimplements the legacy encoder body verbatim and
/// asserts equality on all three composition paths: placeholder (no avatar),
/// PNG-avatar (where tEXt chunk injection ordering matters), and
/// JPEG-avatar transcode.
#[test]
fn plain_encoder_bytes_identical_to_pre_envelope_encoder() {
    // Verbatim pre-refactor `encode_snapshot_png` body (post memory guard).
    fn legacy_encode(
        snapshot: &AgentSnapshot,
        avatar_bytes: Option<&[u8]>,
    ) -> Result<Vec<u8>, String> {
        let json_bytes = encode_snapshot_json(snapshot)?;
        let chunk_text = STANDARD.encode(&json_bytes);
        let png_bytes = match avatar_bytes.filter(|bytes| !bytes.is_empty()) {
            Some(bytes) => {
                let encoded_avatar = if bytes.starts_with(b"\x89PNG") {
                    inject_text_chunk(bytes, PNG_CHUNK_KEYWORD, &chunk_text).or_else(|_| {
                        transcode_avatar_to_png_with_text(bytes, PNG_CHUNK_KEYWORD, &chunk_text)
                    })
                } else {
                    transcode_avatar_to_png_with_text(bytes, PNG_CHUNK_KEYWORD, &chunk_text)
                };
                match encoded_avatar {
                    Ok(png_bytes) => png_bytes,
                    Err(_) => make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)?,
                }
            }
            None => make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)?,
        };
        Ok(png_bytes)
    }

    let record = minimal_record();

    // Placeholder path (no avatar).
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
    assert_eq!(
        encode_snapshot_png(&snapshot, None).unwrap(),
        legacy_encode(&snapshot, None).unwrap(),
        "placeholder-path plain PNG bytes must match the pre-envelope encoder"
    );

    // PNG-avatar path: chunk injected into the avatar image body.
    let avatar = make_png_with_text("dummy", "value").unwrap();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&avatar));
    assert_eq!(
        encode_snapshot_png(&snapshot, Some(&avatar)).unwrap(),
        legacy_encode(&snapshot, Some(&avatar)).unwrap(),
        "avatar-path plain PNG bytes must match the pre-envelope encoder"
    );

    // JPEG-avatar path: transcode-to-PNG composition.
    let jpeg_avatar = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        4,
        4,
        image::Rgb([0x10, 0x20, 0x30]),
    ));
    let mut jpeg_bytes = Vec::new();
    jpeg_avatar
        .write_to(&mut Cursor::new(&mut jpeg_bytes), image::ImageFormat::Jpeg)
        .unwrap();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&jpeg_bytes));
    assert_eq!(
        encode_snapshot_png(&snapshot, Some(&jpeg_bytes)).unwrap(),
        legacy_encode(&snapshot, Some(&jpeg_bytes)).unwrap(),
        "transcode-path plain PNG bytes must match the pre-envelope encoder"
    );
}

#[test]
fn png_snapshot_transcodes_jpeg_avatar_into_image_body() {
    let avatar = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        3,
        2,
        image::Rgb([0x12, 0x34, 0x56]),
    ));
    let mut jpeg_bytes = Vec::new();
    avatar
        .write_to(&mut Cursor::new(&mut jpeg_bytes), image::ImageFormat::Jpeg)
        .unwrap();

    let snapshot = build_snapshot(
        &minimal_record(),
        MemoryLevel::None,
        vec![],
        Some(&jpeg_bytes),
    );
    let png_bytes = encode_snapshot_png(&snapshot, Some(&jpeg_bytes)).unwrap();
    let decoder = Decoder::new(Cursor::new(png_bytes));
    let reader = decoder.read_info().unwrap();

    assert_eq!((reader.info().width, reader.info().height), (3, 2));
}

#[test]
fn png_snapshot_downscales_oversize_avatar_under_cap() {
    // A large avatar (mirrors Gurney's 2764×4096 image that encoded to ~26 MB)
    // must be downscaled for the PNG body so the snapshot stays under the
    // 10 MiB cap — while the manifest keeps the untouched source reference.
    // An already-PNG oversize avatar exercises the `png_within_body_cap` guard
    // that routes it through the downscaling transcode path.
    let avatar = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(2764, 4096, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    }));
    let mut source_bytes = Vec::new();
    avatar
        .write_to(&mut Cursor::new(&mut source_bytes), image::ImageFormat::Png)
        .unwrap();

    let snapshot = build_snapshot(
        &minimal_record(),
        MemoryLevel::None,
        vec![],
        Some(&source_bytes),
    );
    let png_bytes = encode_snapshot_png(&snapshot, Some(&source_bytes)).unwrap();

    assert!(
        png_bytes.len()
            <= super::MAX_PNG_BODY_EDGE as usize * super::MAX_PNG_BODY_EDGE as usize * 4,
        "downscaled snapshot ({} bytes) must be far under the 10 MiB cap",
        png_bytes.len()
    );

    let reader = Decoder::new(Cursor::new(png_bytes)).read_info().unwrap();
    let (width, height) = (reader.info().width, reader.info().height);
    assert!(
        width <= 512 && height <= 512,
        "body dimensions {width}×{height} must fit the 512px cap"
    );
    // Aspect ratio preserved: the longest edge (height) is clamped to the cap.
    assert_eq!(height, 512, "longest edge should hit the 512px cap");

    // The manifest keeps the untouched full-resolution source reference — only
    // the PNG body is downscaled. The oversize source bytes exceed the inline
    // cap, so the manifest falls back to the record's `avatar_url`.
    let manifest =
        decode_snapshot_png(&encode_snapshot_png(&snapshot, Some(&source_bytes)).unwrap()).unwrap();
    assert_eq!(
        manifest.profile.avatar_url.as_deref(),
        Some("https://example.com/avatar.png"),
        "manifest must preserve the untouched source avatar reference"
    );
    assert!(
        manifest.profile.avatar_data_url.is_none(),
        "oversize source bytes must not be inlined into the manifest"
    );
}

#[test]
fn png_round_trip_with_core_memory() {
    let record = minimal_record();
    let entries = vec![AgentSnapshotMemoryEntry {
        slug: "core".to_string(),
        body: "remember this".to_string(),
    }];
    let snapshot = build_snapshot(&record, MemoryLevel::Core, entries, None);

    let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();
    let parsed = decode_snapshot_png(&png_bytes).unwrap();

    assert_eq!(parsed.memory, snapshot.memory);
}

#[test]
fn png_round_trip_with_everything_memory() {
    let record = minimal_record();
    let entries = vec![
        AgentSnapshotMemoryEntry {
            slug: "core".to_string(),
            body: "remember this".to_string(),
        },
        AgentSnapshotMemoryEntry {
            slug: "mem/notes".to_string(),
            body: "private notes".to_string(),
        },
    ];
    let snapshot = build_snapshot(&record, MemoryLevel::Everything, entries, None);

    let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();
    let parsed = decode_snapshot_png(&png_bytes).unwrap();

    assert_eq!(parsed.memory, snapshot.memory);
}

#[test]
fn png_export_with_no_memory_succeeds() {
    let record = minimal_record();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
    assert!(encode_snapshot_png(&snapshot, None).is_ok());
}

#[test]
fn png_export_rejects_none_level_with_nonempty_entries() {
    // Inconsistent state: level == None but entries is non-empty.
    // The encoder must reject this to prevent a memory-leak bypass.
    let record = minimal_record();
    let entries = vec![AgentSnapshotMemoryEntry {
        slug: "core".to_string(),
        body: "leaked memory".to_string(),
    }];
    // Build with entries, then override level to None in the struct.
    let mut snapshot = build_snapshot(&record, MemoryLevel::Core, entries, None);
    snapshot.memory.level = MemoryLevel::None; // force inconsistency
    let result = encode_snapshot_png(&snapshot, None);
    assert!(
        result.is_err(),
        "PNG encoder must reject level=None with non-empty entries"
    );
    assert!(
        result
            .unwrap_err()
            .contains("memory.level 'none' and non-empty memory entries"),
        "Error must explain the malformed memory state"
    );
}

// ── Secret exclusion tests ────────────────────────────────────────────────
//
// These tests assert that every field in the exclusion list is absent from
// the serialized snapshot. We serialize to JSON and assert the key is NOT
// present.

fn snapshot_json_string(record: &ManagedAgentRecord) -> String {
    let snapshot = build_snapshot(record, MemoryLevel::None, vec![], None);
    let bytes = encode_snapshot_json(&snapshot).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn secret_exclusion_private_key_nsec_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("nsec1secret"),
        "nsec must not appear in snapshot"
    );
    assert!(
        !json.contains("privateKeyNsec") && !json.contains("private_key_nsec"),
        "privateKeyNsec field must not appear in snapshot"
    );
}

#[test]
fn secret_exclusion_auth_tag_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("auth-tag-secret"),
        "auth_tag value must not appear in snapshot"
    );
    assert!(
        !json.contains("authTag") && !json.contains("auth_tag"),
        "authTag field must not appear in snapshot"
    );
}

#[test]
fn secret_exclusion_env_vars_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("API_KEY") && !json.contains("secret123"),
        "env_vars content must not appear in snapshot"
    );
    assert!(
        !json.contains("envVars") && !json.contains("env_vars"),
        "envVars field must not appear in snapshot"
    );
}

#[test]
fn secret_exclusion_relay_url_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("wss://relay.example.com"),
        "relay_url value must not appear in snapshot"
    );
    assert!(
        !json.contains("relayUrl") && !json.contains("relay_url"),
        "relayUrl field must not appear in snapshot"
    );
}

#[test]
fn snapshot_omits_removed_mcp_toolsets_config() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("mcpToolsets") && !json.contains("mcp_toolsets"),
        "removed MCP toolsets config must not re-enter snapshots"
    );
}

#[test]
fn secret_exclusion_machine_commands_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    // acp_command / agent_command / agent_command_override / agent_args / mcp_command
    assert!(
        !json.contains("/usr/local/bin/acp"),
        "acp_command path must not appear"
    );
    assert!(
        !json.contains("acpCommand") && !json.contains("acp_command"),
        "acpCommand field must not appear"
    );
    assert!(
        !json.contains("agentCommand") && !json.contains("agent_command"),
        "agentCommand field must not appear"
    );
    assert!(
        !json.contains("mcpCommand") && !json.contains("mcp_command"),
        "mcpCommand field must not appear"
    );
}

#[test]
fn secret_exclusion_runtime_state_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("runtimePid") && !json.contains("runtime_pid"),
        "runtimePid must not appear"
    );
    assert!(
        !json.contains("backendAgentId") && !json.contains("backend_agent_id"),
        "backendAgentId must not appear"
    );
    assert!(
        !json.contains("SENTINEL_BACKEND_AGENT_ID"),
        "backendAgentId value must not appear"
    );
    assert!(
        !json.contains("providerBinaryPath") && !json.contains("provider_binary_path"),
        "providerBinaryPath must not appear"
    );
    assert!(
        !json.contains("SENTINEL_PROVIDER_BINARY"),
        "providerBinaryPath value must not appear"
    );
    assert!(
        !json.contains("lastStartedAt") && !json.contains("last_started_at"),
        "lastStartedAt must not appear"
    );
    assert!(
        !json.contains("lastExitCode") && !json.contains("last_exit_code"),
        "lastExitCode must not appear"
    );
    // backend blob — neither the type tag nor provider secret must leak.
    assert!(
        !json.contains("\"backend\"") && !json.contains("backend"),
        "backend field must not appear"
    );
    assert!(
        !json.contains("SENTINEL_BACKEND_ID") && !json.contains("SENTINEL_BACKEND_SECRET"),
        "backend config values must not appear"
    );
    // last_error / last_error_code
    assert!(
        !json.contains("lastError") && !json.contains("last_error"),
        "lastError must not appear"
    );
    assert!(
        !json.contains("SENTINEL_LAST_ERROR"),
        "lastError value must not appear"
    );
    assert!(
        !json.contains("lastErrorCode") && !json.contains("last_error_code"),
        "lastErrorCode must not appear"
    );
}

#[test]
fn secret_exclusion_lineage_ids_absent() {
    let record = minimal_record();
    let json = snapshot_json_string(&record);
    assert!(
        !json.contains("team-id-123"),
        "source_team value must not appear"
    );
    assert!(
        !json.contains("sourceTeam") && !json.contains("source_team"),
        "sourceTeam field must not appear"
    );
    assert!(
        !json.contains("sourceTeamPersonaSlug"),
        "sourceTeamPersonaSlug must not appear"
    );
    assert!(
        !json.contains("personaSourceVersion") && !json.contains("persona_source_version"),
        "personaSourceVersion must not appear"
    );
    // personaId
    assert!(
        !json.contains("personaId") && !json.contains("persona_id"),
        "personaId field must not appear"
    );
    assert!(
        !json.contains("SENTINEL_PERSONA_ID"),
        "personaId value must not appear"
    );
    // teamId
    assert!(
        !json.contains("teamId") && !json.contains("team_id"),
        "teamId field must not appear"
    );
    assert!(
        !json.contains("SENTINEL_TEAM_ID"),
        "teamId value must not appear"
    );
    // personaTeamDir
    assert!(
        !json.contains("personaTeamDir") && !json.contains("persona_team_dir"),
        "personaTeamDir field must not appear"
    );
    assert!(
        !json.contains("SENTINEL_TEAM_DIR"),
        "personaTeamDir value must not appear"
    );
    // personaNameInTeam
    assert!(
        !json.contains("personaNameInTeam") && !json.contains("persona_name_in_team"),
        "personaNameInTeam field must not appear"
    );
    assert!(
        !json.contains("SENTINEL_NAME_IN_TEAM"),
        "personaNameInTeam value must not appear"
    );
}

// ── Definition field presence tests ──────────────────────────────────────

#[test]
fn definition_fields_present_in_snapshot() {
    let record = minimal_record();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);

    assert_eq!(snapshot.definition.name, "Test Agent Display");
    assert!(!snapshot.definition.source_is_builtin);
    assert_eq!(
        snapshot.definition.system_prompt.as_deref(),
        Some("You are a test agent.")
    );
    assert_eq!(snapshot.definition.runtime.as_deref(), Some("goose"));
    assert_eq!(snapshot.definition.model.as_deref(), Some("claude-opus-4"));
    assert_eq!(snapshot.definition.provider.as_deref(), Some("anthropic"));
    assert_eq!(snapshot.definition.name_pool, vec!["Alice", "Bob"]);
    // definition_respond_to maps to respond_to in the snapshot definition
    assert_eq!(snapshot.definition.respond_to.as_deref(), Some("allowlist"));
    // definition_respond_to_allowlist should be included
    assert!(!snapshot.definition.respond_to_allowlist.is_empty());
}

#[test]
fn profile_fields_present_in_snapshot() {
    let record = minimal_record();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], None);
    assert_eq!(snapshot.profile.display_name, "Test Agent Display");
    // No bytes → should fall back to avatar_url
    assert_eq!(
        snapshot.profile.avatar_url.as_deref(),
        Some("https://example.com/avatar.png")
    );
    assert!(snapshot.profile.avatar_data_url.is_none());
}

#[test]
fn avatar_inlined_when_under_size_limit() {
    let record = minimal_record();
    let small_png = make_png_with_text("k", "v").unwrap();
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&small_png));
    assert!(snapshot.profile.avatar_data_url.is_some());
    assert!(snapshot.profile.avatar_url.is_none());
}

#[test]
fn avatar_url_fallback_when_over_size_limit() {
    let mut record = minimal_record();
    record.avatar_url = Some("https://example.com/big.png".to_string());
    // Synthesize oversized avatar bytes (> 2 MB) — just a large zeroed vec.
    let big_bytes = vec![0u8; MAX_AVATAR_INLINE_BYTES + 1];
    let snapshot = build_snapshot(&record, MemoryLevel::None, vec![], Some(&big_bytes));
    assert!(snapshot.profile.avatar_data_url.is_none());
    assert_eq!(
        snapshot.profile.avatar_url.as_deref(),
        Some("https://example.com/big.png")
    );
}

// ── Format/version validation ─────────────────────────────────────────────

#[test]
fn invalid_format_discriminator_is_rejected() {
    let mut snapshot = build_snapshot(&minimal_record(), MemoryLevel::None, vec![], None);
    snapshot.format = "not-a-buzz-snapshot".to_string();
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let result = decode_snapshot_json(&bytes);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unsupported snapshot format"));
}

#[test]
fn unsupported_version_is_rejected() {
    let mut snapshot = build_snapshot(&minimal_record(), MemoryLevel::None, vec![], None);
    snapshot.version = 99;
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let result = decode_snapshot_json(&bytes);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unsupported snapshot version"));
}
