use super::import::decode_snapshot_from_bytes;
use super::*;
use crate::managed_agents::{
    agent_snapshot::{
        AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotProfile,
        FORMAT_DISCRIMINATOR, FORMAT_VERSION,
    },
    BackendKind, ManagedAgentRecord, RespondTo,
};
use std::collections::BTreeMap;

fn make_definition(slug: &str) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: String::new(),
        slug: Some(slug.to_string()),
        name: slug.to_string(),
        display_name: None,
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::default(),
        respond_to_allowlist: vec![],
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: false,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
        relay_mesh: None,
    }
}

/// Build a minimal valid AgentSnapshot for import tests.
fn make_snapshot(
    memory_level: MemoryLevel,
    entries: Vec<AgentSnapshotMemoryEntry>,
) -> AgentSnapshot {
    AgentSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        definition: AgentSnapshotDefinition {
            name: "Test Agent".to_string(),
            source_is_builtin: false,
            system_prompt: Some("You are helpful.".to_string()),
            runtime: None,
            model: None,
            provider: None,
            parallelism: None,
            respond_to: None,
            respond_to_allowlist: vec![],
            name_pool: vec![],
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
        },
        profile: AgentSnapshotProfile {
            display_name: "Test Agent".to_string(),
            about: None,
            avatar_data_url: None,
            avatar_url: None,
        },
        memory: AgentSnapshotMemory {
            level: memory_level,
            entries,
        },
    }
}

// ── Portable effective configuration ─────────────────────────────────────

#[test]
fn inherited_runtime_provider_and_model_are_materialized_for_export() {
    let mut record = make_definition("wren");
    let global = crate::managed_agents::GlobalAgentConfig {
        preferred_runtime: Some("goose".to_string()),
        provider: Some("databricks_v2".to_string()),
        model: Some("databricks-gpt-5-6-sol".to_string()),
        ..Default::default()
    };

    materialize_portable_runtime_defaults(&mut record, &global);

    assert_eq!(record.runtime.as_deref(), Some("goose"));
    assert_eq!(record.provider.as_deref(), Some("databricks_v2"));
    assert_eq!(record.model.as_deref(), Some("databricks-gpt-5-6-sol"));
}

#[test]
fn explicit_runtime_provider_and_model_win_over_global_defaults() {
    let mut record = make_definition("wren");
    record.runtime = Some("claude".to_string());
    record.provider = Some("anthropic".to_string());
    record.model = Some("claude-opus-5".to_string());
    let global = crate::managed_agents::GlobalAgentConfig {
        preferred_runtime: Some("goose".to_string()),
        provider: Some("databricks_v2".to_string()),
        model: Some("databricks-gpt-5-6-sol".to_string()),
        ..Default::default()
    };

    materialize_portable_runtime_defaults(&mut record, &global);

    assert_eq!(record.runtime.as_deref(), Some("claude"));
    assert_eq!(record.provider.as_deref(), Some("anthropic"));
    assert_eq!(record.model.as_deref(), Some("claude-opus-5"));
}

/// PNG image-body avatar overrides manifest avatar fields and all definition
/// config survives the exact production decoder.
#[test]
fn import_png_body_avatar_and_full_model_round_trip() {
    use crate::managed_agents::agent_snapshot::{decode_avatar_data_url, encode_snapshot_png};

    let mut snapshot = make_snapshot(MemoryLevel::None, vec![]);
    snapshot.definition.runtime = Some("goose".to_string());
    snapshot.definition.model = Some("databricks-gpt-5-6-sol".to_string());
    snapshot.definition.provider = Some("databricks_v2".to_string());
    snapshot.profile.avatar_data_url = None;
    snapshot.profile.avatar_url = Some("https://sender.invalid/avatar.png".to_string());

    let avatar = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        4,
        3,
        image::Rgba([23, 91, 177, 255]),
    ));
    let mut avatar_png = std::io::Cursor::new(Vec::new());
    avatar
        .write_to(&mut avatar_png, image::ImageFormat::Png)
        .unwrap();
    let png_bytes = encode_snapshot_png(&snapshot, Some(avatar_png.get_ref())).unwrap();

    let decoded = decode_snapshot_from_bytes(&png_bytes).unwrap();
    assert_eq!(decoded.definition.runtime.as_deref(), Some("goose"));
    assert_eq!(
        decoded.definition.model.as_deref(),
        Some("databricks-gpt-5-6-sol")
    );
    assert_eq!(
        decoded.definition.provider.as_deref(),
        Some("databricks_v2")
    );
    assert_eq!(
        decoded.profile.avatar_url.as_deref(),
        Some("https://sender.invalid/avatar.png")
    );

    let avatar_data_url = decoded
        .profile
        .avatar_data_url
        .as_deref()
        .expect("PNG image body must become the effective portable avatar");
    let avatar_bytes = decode_avatar_data_url(avatar_data_url).unwrap();
    let imported_avatar = image::load_from_memory(&avatar_bytes).unwrap();
    assert_eq!((imported_avatar.width(), imported_avatar.height()), (4, 3));
    assert_eq!(
        imported_avatar.to_rgba8().get_pixel(0, 0).0,
        [23, 91, 177, 255]
    );
}

/// The transparent 1×1 no-avatar card must not override a manifest fallback.
#[test]
fn import_png_placeholder_keeps_manifest_avatar_fallback() {
    use crate::managed_agents::agent_snapshot::encode_snapshot_png;

    let mut snapshot = make_snapshot(MemoryLevel::None, vec![]);
    snapshot.profile.avatar_data_url = None;
    snapshot.profile.avatar_url = Some("https://example.com/avatar.png".to_string());
    let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();

    let decoded = decode_snapshot_from_bytes(&png_bytes).unwrap();
    assert!(decoded.profile.avatar_data_url.is_none());
    assert_eq!(decoded.profile.avatar_url, snapshot.profile.avatar_url);
}

/// An unlocked trading card imports the agent's REAL avatar, never the card.
///
/// Mint-shaped input: the PNG body is the generated card artwork, while the
/// manifest inlines the source avatar (`manifest_avatar_bytes` in `card.rs`).
/// The #3578 body-wins override must not fire when the manifest already
/// carries inline avatar bytes — otherwise the imported agent publishes the
/// 1500-wide card as its kind:0 picture.
#[test]
fn import_unlocked_card_uses_manifest_avatar_not_card_artwork() {
    use crate::managed_agents::agent_snapshot::{decode_avatar_data_url, encode_snapshot_png};
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    // The real avatar: 4×3 solid blue, inlined in the manifest at mint time.
    let real_avatar = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        4,
        3,
        image::Rgba([23, 91, 177, 255]),
    ));
    let mut real_avatar_png = std::io::Cursor::new(Vec::new());
    real_avatar
        .write_to(&mut real_avatar_png, image::ImageFormat::Png)
        .unwrap();

    let mut snapshot = make_snapshot(MemoryLevel::None, vec![]);
    snapshot.profile.avatar_data_url = Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(real_avatar_png.get_ref())
    ));
    snapshot.profile.avatar_url = Some("https://relay.example/media/live-kind0.png".to_string());

    // The card artwork: a distinct 1500×2250 solid red "trading card" as the
    // PNG body — the exact dimensions the minter encodes for unlocked cards.
    // Size matters: 2250px exceeds `snapshot_avatar`'s 2048px decode limit,
    // so reaching the body override here wouldn't just import the wrong
    // face — it would fail the import outright.
    let card_art = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1500,
        2250,
        image::Rgba([200, 16, 16, 255]),
    ));
    let mut card_png = std::io::Cursor::new(Vec::new());
    card_art
        .write_to(&mut card_png, image::ImageFormat::Png)
        .unwrap();
    let file_bytes = encode_snapshot_png(&snapshot, Some(card_png.get_ref())).unwrap();

    // Production import decode: the effective avatar must be the real one.
    let decoded = decode_snapshot_from_bytes(&file_bytes).unwrap();
    let avatar_bytes =
        decode_avatar_data_url(decoded.profile.avatar_data_url.as_deref().unwrap()).unwrap();
    let imported = image::load_from_memory(&avatar_bytes).unwrap();
    assert_eq!(
        (imported.width(), imported.height()),
        (4, 3),
        "imported avatar must be the source avatar, not the card artwork"
    );
    assert_eq!(imported.to_rgba8().get_pixel(0, 0).0, [23, 91, 177, 255]);
    // The live kind:0 URL fallback survives untouched.
    assert_eq!(decoded.profile.avatar_url, snapshot.profile.avatar_url);
}
