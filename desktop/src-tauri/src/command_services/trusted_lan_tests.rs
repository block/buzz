use super::trusted_lan::{
    catalogue_fingerprint, load_optional, mcp_tool_result, save_routing_preference,
    ModelRoutingPreference, TrustedLanConfig, TrustedLanEndpoint,
};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

#[test]
fn accepts_the_approved_literal_private_endpoints() {
    let memory =
        TrustedLanEndpoint::parse_memory("http://192.168.1.26:8006/mcp").expect("memory endpoint");
    let rag =
        TrustedLanEndpoint::parse_rag("http://192.168.1.107:8005/mcp/").expect("RAG endpoint");

    assert_eq!(memory.as_str(), "http://192.168.1.26:8006/mcp");
    assert_eq!(rag.as_str(), "http://192.168.1.107:8005/mcp/");
}

#[test]
fn optional_loader_selects_only_a_valid_protected_trusted_lan_config() {
    let directory = tempfile::tempdir_in(std::env::current_dir().expect("working directory"))
        .expect("temporary protected config directory");
    let absent = directory.path().join("absent.json");
    assert!(load_optional(&absent).expect("absent config").is_none());

    let path = directory.path().join("trusted-lan-sources.json");
    std::fs::write(
        &path,
        include_bytes!("../../trusted-lan-sources.example.json"),
    )
    .expect("write trusted LAN config");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("protect trusted LAN config");

    assert!(load_optional(&path).expect("protected config").is_some());
}

#[test]
fn routing_preference_defaults_local_and_accepts_cloud_first() {
    let example =
        String::from_utf8(include_bytes!("../../trusted-lan-sources.example.json").to_vec())
            .expect("fixture utf8");
    let legacy_fixture = example.replace("  \"model_routing_preference\": \"cloud_first\",\n", "");
    let legacy =
        TrustedLanConfig::parse(legacy_fixture.as_bytes()).expect("legacy trusted LAN config");
    assert_eq!(
        legacy.routing_preference(),
        ModelRoutingPreference::LocalFirst
    );

    let cloud_first = example;
    let parsed =
        TrustedLanConfig::parse(cloud_first.as_bytes()).expect("cloud-first trusted LAN config");
    assert_eq!(
        parsed.routing_preference(),
        ModelRoutingPreference::CloudFirst
    );
    assert_ne!(
        legacy.configuration_identity(),
        parsed.configuration_identity()
    );
}

#[test]
fn routing_preference_save_is_atomic_protected_and_preserves_routes() {
    let directory = tempfile::tempdir_in(std::env::current_dir().expect("working directory"))
        .expect("temporary protected config directory");
    let path = directory.path().join("trusted-lan-sources.json");
    std::fs::write(
        &path,
        include_bytes!("../../trusted-lan-sources.example.json"),
    )
    .expect("write trusted LAN config");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("protect trusted LAN config");

    let before = TrustedLanConfig::load(&path).expect("load before save");
    save_routing_preference(&path, ModelRoutingPreference::CloudFirst)
        .expect("save cloud-first preference");
    let after = TrustedLanConfig::load(&path).expect("load after save");

    assert_eq!(
        after.routing_preference(),
        ModelRoutingPreference::CloudFirst
    );
    assert_eq!(after.memory_url(), before.memory_url());
    assert_eq!(after.rag_url(), before.rag_url());
    assert_eq!(after.litellm().endpoint(), before.litellm().endpoint());
    assert_eq!(after.litellm().model(), before.litellm().model());
    assert_eq!(
        after.litellm().keychain_key(),
        before.litellm().keychain_key()
    );
    assert_eq!(after.openai().endpoint(), before.openai().endpoint());
    assert_eq!(after.openai().model(), before.openai().model());
    assert_eq!(
        after.openai().keychain_key(),
        before.openai().keychain_key()
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("saved config metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn rejects_routes_outside_the_trusted_lan_contract() {
    for rejected in [
        "https://192.168.1.26:8006/mcp",
        "http://memory.home.arpa:8006/mcp",
        "http://127.0.0.1:8006/mcp",
        "http://8.8.8.8:8006/mcp",
        "http://192.168.1.26:8006/mcp?x=1",
        "http://user:secret@192.168.1.26:8006/mcp",
        "http://192.168.1.26/mcp",
        "http://192.168.1.26:8006/other",
    ] {
        assert!(
            TrustedLanEndpoint::parse_memory(rejected).is_err(),
            "accepted {rejected}"
        );
    }

    assert!(TrustedLanEndpoint::parse_rag("http://192.168.1.107:8005/mcp").is_err());
}

#[test]
fn parses_the_closed_cloud_fallback_configuration() {
    let config = TrustedLanConfig::parse(
        br#"{
          "schema_version": 1,
          "mode": "OFFICIAL_TRUSTED_LAN",
          "memory_url": "http://192.168.1.26:8006/mcp",
          "rag_url": "http://192.168.1.107:8005/mcp/",
          "automatic_cloud_fallback_acknowledged": true,
          "litellm": {
            "enabled": true,
            "endpoint": "http://192.168.1.31:4000/v1/chat/completions",
            "model": "openai/gpt-5.4",
            "keychain_key": "command.cloud.litellm"
          },
          "openai": {
            "enabled": true,
            "endpoint": "https://api.openai.com/v1/responses",
            "model": "gpt-5.4",
            "keychain_key": "command.cloud.openai"
          }
        }"#,
    )
    .expect("trusted LAN config");

    assert_eq!(config.memory_url().as_str(), "http://192.168.1.26:8006/mcp");
    assert_eq!(config.rag_url().as_str(), "http://192.168.1.107:8005/mcp/");
    assert_eq!(config.litellm().model(), "openai/gpt-5.4");
    assert_eq!(config.openai().model(), "gpt-5.4");
}

#[test]
fn rejects_unacknowledged_or_ambiguous_cloud_configuration() {
    let unacknowledged = br#"{
      "schema_version": 1,
      "mode": "OFFICIAL_TRUSTED_LAN",
      "memory_url": "http://192.168.1.26:8006/mcp",
      "rag_url": "http://192.168.1.107:8005/mcp/",
      "automatic_cloud_fallback_acknowledged": false,
      "litellm": {
        "enabled": true,
        "endpoint": "http://192.168.1.31:4000/v1/chat/completions",
        "model": "openai/gpt-5.4",
        "keychain_key": "command.cloud.litellm"
      },
      "openai": {
        "enabled": true,
        "endpoint": "https://api.openai.com/v1/responses",
        "model": "gpt-5.4",
        "keychain_key": "command.cloud.openai"
      }
    }"#;
    assert!(TrustedLanConfig::parse(unacknowledged).is_err());

    let unknown_field = String::from_utf8(unacknowledged.to_vec())
        .expect("fixture utf8")
        .replace(
            "\"schema_version\": 1,",
            "\"schema_version\": 1, \"surprise\": true,",
        )
        .replace(
            "\"automatic_cloud_fallback_acknowledged\": false",
            "\"automatic_cloud_fallback_acknowledged\": true",
        );
    assert!(TrustedLanConfig::parse(unknown_field.as_bytes()).is_err());
}

#[test]
fn unwraps_legacy_fastmcp_text_without_synthesizing_evidence_fields() {
    let value = mcp_tool_result(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": {
            "content": [{
                "type": "text",
                "text": "{\"query\":\"bridge safety\",\"total\":1,\"results\":[{\"point_id\":\"point-1\",\"text\":\"evidence\"}]}"
            }],
            "isError": false
        }
    }))
    .expect("legacy MCP result");

    assert_eq!(value["results"][0]["point_id"], "point-1");
    let encoded = serde_json::to_string(&value).expect("fixture serialization");
    assert!(!encoded.contains("signature"));
    assert!(!encoded.contains("immutable"));
    assert!(!encoded.contains("revision_hash"));
}

#[test]
fn catalogue_fingerprint_is_stable_but_changes_with_the_observed_catalogue() {
    let first = json!({
        "collections": [
            {"name": "navy-publications", "chunks": 3579},
            {"name": "marine-navigation", "chunks": 11162}
        ],
        "total_chunks": 14741
    });
    let reordered = json!({
        "total_chunks": 14741,
        "collections": [
            {"chunks": 3579, "name": "navy-publications"},
            {"chunks": 11162, "name": "marine-navigation"}
        ]
    });
    let changed = json!({
        "collections": [
            {"name": "navy-publications", "chunks": 3580},
            {"name": "marine-navigation", "chunks": 11162}
        ],
        "total_chunks": 14742
    });

    assert_eq!(
        catalogue_fingerprint(&first).expect("first"),
        catalogue_fingerprint(&reordered).expect("reordered")
    );
    assert_ne!(
        catalogue_fingerprint(&first).expect("first"),
        catalogue_fingerprint(&changed).expect("changed")
    );
}
