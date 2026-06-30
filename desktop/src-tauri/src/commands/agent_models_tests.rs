use super::*;

#[test]
fn openai_model_normalization_keeps_agent_text_models() {
    let models = normalize_openai_compatible_models(
        OpenAiModelListResponse {
            data: vec![
                OpenAiModelListItem {
                    id: "text-embedding-3-large".to_string(),
                    created: Some(4),
                },
                OpenAiModelListItem {
                    id: "gpt-image-2".to_string(),
                    created: Some(5),
                },
                OpenAiModelListItem {
                    id: "chatgpt-5.5-pro-2026-04-23".to_string(),
                    created: Some(7),
                },
                OpenAiModelListItem {
                    id: "chatgpt-5.5-pro".to_string(),
                    created: Some(6),
                },
                OpenAiModelListItem {
                    id: "gpt-5.4-mini".to_string(),
                    created: Some(2),
                },
                OpenAiModelListItem {
                    id: "o4-mini".to_string(),
                    created: Some(3),
                },
                OpenAiModelListItem {
                    id: "gpt-5.4-mini".to_string(),
                    created: Some(1),
                },
            ],
        },
        Some("openai"),
    );

    let ids_and_names = models
        .into_iter()
        .map(|model| (model.id, model.name))
        .collect::<Vec<_>>();
    assert_eq!(
        ids_and_names,
        vec![
            (
                "chatgpt-5.5-pro".to_string(),
                Some("ChatGPT 5.5 Pro".to_string()),
            ),
            ("o4-mini".to_string(), Some("o4-mini".to_string())),
            ("gpt-5.4-mini".to_string(), Some("GPT-5.4 mini".to_string()),),
        ]
    );
}

#[test]
fn openai_compat_model_normalization_preserves_provider_specific_ids() {
    let models = normalize_openai_compatible_models(
        OpenAiModelListResponse {
            data: vec![
                OpenAiModelListItem {
                    id: "meta-llama/Llama-3.3-70B-Instruct".to_string(),
                    created: Some(5),
                },
                OpenAiModelListItem {
                    id: "mistral-large-latest".to_string(),
                    created: Some(4),
                },
                OpenAiModelListItem {
                    id: "anthropic/claude-sonnet-4-6".to_string(),
                    created: Some(3),
                },
                OpenAiModelListItem {
                    id: "text-embedding-compatible".to_string(),
                    created: Some(2),
                },
                OpenAiModelListItem {
                    id: "meta-llama/Llama-3.3-70B-Instruct".to_string(),
                    created: Some(1),
                },
            ],
        },
        Some("openai-compat"),
    );

    let ids = models.into_iter().map(|model| model.id).collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "meta-llama/Llama-3.3-70B-Instruct".to_string(),
            "mistral-large-latest".to_string(),
            "anthropic/claude-sonnet-4-6".to_string(),
            "text-embedding-compatible".to_string(),
        ]
    );
}

#[test]
fn openai_models_url_uses_openai_default_base_url() {
    assert_eq!(
        openai_compatible_models_url(&BTreeMap::new()),
        "https://api.openai.com/v1/models"
    );
}

#[test]
fn anthropic_models_url_uses_anthropic_default_base_url() {
    assert_eq!(
        anthropic_models_url(&BTreeMap::new()),
        "https://api.anthropic.com/v1/models"
    );
}

#[test]
fn anthropic_models_url_accepts_versioned_base_url() {
    let env = BTreeMap::from([(
        "ANTHROPIC_BASE_URL".to_string(),
        "https://proxy.example/v1/".to_string(),
    )]);

    assert_eq!(
        anthropic_models_url(&env),
        "https://proxy.example/v1/models"
    );
}

#[test]
fn anthropic_model_normalization_uses_display_names() {
    let models = normalize_anthropic_models(AnthropicModelListResponse {
        data: vec![
            AnthropicModelListItem {
                id: "claude-opus-4-6".to_string(),
                display_name: Some("Claude Opus 4.6".to_string()),
            },
            AnthropicModelListItem {
                id: "claude-opus-4-6".to_string(),
                display_name: Some("Duplicate".to_string()),
            },
        ],
        has_more: false,
        last_id: None,
    });

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "claude-opus-4-6");
    assert_eq!(models[0].name.as_deref(), Some("Claude Opus 4.6"));
}

#[test]
fn redaction_env_records_value_used_for_request() {
    let env = BTreeMap::from([("OPENAI_COMPAT_API_KEY".to_string(), "   ".to_string())]);

    let redaction_env =
        redaction_env_with_value(&env, "OPENAI_COMPAT_API_KEY", "inherited-process-key");

    assert_eq!(
        redaction_env
            .get("OPENAI_COMPAT_API_KEY")
            .map(String::as_str),
        Some("inherited-process-key")
    );
}

#[test]
fn saved_agent_model_discovery_uses_record_snapshot() {
    let record: crate::managed_agents::ManagedAgentRecord = serde_json::from_str(
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
            "model": "record-model",
            "provider": "databricks",
            "env_vars": {
                "OPENAI_API_KEY": "record-key",
                "BUZZ_PRIVATE_KEY": "must-not-leak"
            },
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#,
    )
    .expect("sample managed agent record");

    let config = saved_agent_model_discovery_config(&record, "goose");

    assert_eq!(config.model.as_deref(), Some("record-model"));
    assert_eq!(config.provider.as_deref(), Some("databricks"));
    assert_eq!(
        config.env.get("GOOSE_MODEL").map(String::as_str),
        Some("record-model")
    );
    assert_eq!(
        config.env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("databricks")
    );
    assert_eq!(
        config.env.get("OPENAI_API_KEY").map(String::as_str),
        Some("record-key")
    );
    assert!(!config.env.contains_key("BUZZ_PRIVATE_KEY"));
}

// ---------------------------------------------------------------------------
// Databricks provider detection
// ---------------------------------------------------------------------------
//
// Parse/filter/pagination tests live in crates/buzz-agent/src/catalog.rs
// (they moved there with the Option C refactor).

#[test]
fn is_databricks_provider_matches_both_variants() {
    assert!(is_databricks_provider(Some("databricks")));
    assert!(is_databricks_provider(Some("databricks_v2")));
    assert!(is_databricks_provider(Some("  DATABRICKS  ")));
    assert!(!is_databricks_provider(Some("anthropic")));
    assert!(!is_databricks_provider(None));
}
