use super::trusted_lan::{TrustedLanConfig, TrustedLanEndpoint};

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
