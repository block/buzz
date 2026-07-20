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

// ---------------------------------------------------------------------------
// Dead-knob guards: mcp_command and turn_timeout_seconds
// ---------------------------------------------------------------------------

#[test]
fn update_request_mcp_command_parses_for_wire_compat() {
    // UpdateManagedAgentRequest accepts mcpCommand for backward-compatibility
    // with frontends that still send it: the deprecated field must keep
    // parsing cleanly. Nothing consumes it — the patching loop in
    // update_managed_agent has no mcp_command arm (the effective MCP command
    // is always catalog-derived at spawn). That absent-arm invariant lives in
    // the code, not in this test: it only guards the wire shape.
    let req: crate::managed_agents::UpdateManagedAgentRequest =
        serde_json::from_str(r#"{"pubkey": "abc", "mcpCommand": "user-override"}"#)
            .expect("request with deprecated mcpCommand parses");
    assert_eq!(req.mcp_command.as_deref(), Some("user-override"));
}

#[test]
fn update_request_turn_timeout_parses_for_wire_compat() {
    // UpdateManagedAgentRequest accepts turnTimeoutSeconds for
    // backward-compatibility with frontends that still send it: the deprecated
    // field must keep parsing cleanly. Nothing consumes it — the patching loop
    // in update_managed_agent has no turn_timeout_seconds arm
    // (BUZZ_ACP_TURN_TIMEOUT is deprecated and ignored by the harness). That
    // absent-arm invariant lives in the code, not in this test: it only
    // guards the wire shape.
    let req: crate::managed_agents::UpdateManagedAgentRequest =
        serde_json::from_str(r#"{"pubkey": "abc", "turnTimeoutSeconds": 9999}"#)
            .expect("request with deprecated turnTimeoutSeconds parses");
    assert_eq!(req.turn_timeout_seconds, Some(9999));
}

#[test]
fn is_databricks_provider_matches_both_variants() {
    assert!(is_databricks_provider(Some("databricks")));
    assert!(is_databricks_provider(Some("databricks_v2")));
    assert!(is_databricks_provider(Some("  DATABRICKS  ")));
    assert!(!is_databricks_provider(Some("anthropic")));
    assert!(!is_databricks_provider(None));
}

// ---------------------------------------------------------------------------
// OpenRouter provider
// ---------------------------------------------------------------------------

#[test]
fn is_openrouter_provider_matches() {
    assert!(is_openrouter_provider(Some("openrouter")));
    assert!(is_openrouter_provider(Some("  OpenRouter  ")));
    assert!(!is_openrouter_provider(Some("openai")));
    assert!(!is_openrouter_provider(Some("anthropic")));
    assert!(!is_openrouter_provider(None));
}

#[test]
fn openrouter_models_url_uses_default_base_url() {
    assert_eq!(
        openrouter_models_url(&BTreeMap::new()),
        "https://openrouter.ai/api/v1/models"
    );
}

#[test]
fn openrouter_models_url_respects_custom_base_url() {
    let env = BTreeMap::from([(
        "OPENROUTER_BASE_URL".to_string(),
        "https://eu.openrouter.ai/api/v1".to_string(),
    )]);
    assert_eq!(
        openrouter_models_url(&env),
        "https://eu.openrouter.ai/api/v1/models"
    );
}

#[test]
fn openrouter_models_url_strips_trailing_slash() {
    let env = BTreeMap::from([(
        "OPENROUTER_BASE_URL".to_string(),
        "https://proxy.example.com/api/v1/".to_string(),
    )]);
    assert_eq!(
        openrouter_models_url(&env),
        "https://proxy.example.com/api/v1/models"
    );
}

#[test]
fn openrouter_filter_keeps_tools_capable_models() {
    let response = OpenRouterModelListResponse {
        data: vec![
            OpenRouterModelListItem {
                id: "anthropic/claude-opus-4-7".to_string(),
                supported_parameters: vec!["tools".to_string(), "reasoning".to_string()],
            },
            OpenRouterModelListItem {
                id: "openai/gpt-5.5-pro".to_string(),
                supported_parameters: vec!["tools".to_string()],
            },
            OpenRouterModelListItem {
                id: "meta-llama/llama-no-tools".to_string(),
                supported_parameters: vec!["temperature".to_string()],
            },
        ],
    };
    let result = filter_openrouter_models(response, None).unwrap().unwrap();
    let ids: Vec<_> = result.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["anthropic/claude-opus-4-7", "openai/gpt-5.5-pro"]);
}

#[test]
fn openrouter_filter_excludes_absent_supported_parameters() {
    let response: OpenRouterModelListResponse =
        serde_json::from_str(r#"{"data": [{"id": "model-no-params"}]}"#).unwrap();
    assert!(
        response.data[0].supported_parameters.is_empty(),
        "absent supported_parameters must default to empty vec"
    );
    let result = filter_openrouter_models(response, None);
    assert!(
        result.is_err(),
        "models with no supported_parameters must be excluded"
    );
    assert!(
        result.unwrap_err().contains("no tools-capable models"),
        "error must indicate no tools-capable models"
    );
}

#[test]
fn openrouter_filter_excludes_empty_supported_parameters() {
    let response = OpenRouterModelListResponse {
        data: vec![OpenRouterModelListItem {
            id: "model-empty-params".to_string(),
            supported_parameters: Vec::new(),
        }],
    };
    let result = filter_openrouter_models(response, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no tools-capable models"));
}

#[test]
fn openrouter_filter_empty_result_returns_error() {
    let response = OpenRouterModelListResponse { data: Vec::new() };
    let result = filter_openrouter_models(response, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no tools-capable models"));
}

#[test]
fn openrouter_filter_preserves_selected_model() {
    let response = OpenRouterModelListResponse {
        data: vec![OpenRouterModelListItem {
            id: "openai/gpt-5.5-pro".to_string(),
            supported_parameters: vec!["tools".to_string()],
        }],
    };
    let result = filter_openrouter_models(response, Some("openai/gpt-5.5-pro".to_string()))
        .unwrap()
        .unwrap();
    assert_eq!(result.selected_model.as_deref(), Some("openai/gpt-5.5-pro"));
}

#[test]
fn openrouter_credential_redaction_env_records_key() {
    let env = BTreeMap::from([(
        "OPENROUTER_API_KEY".to_string(),
        "sk-or-v1-secret-key-12345".to_string(),
    )]);
    let redaction =
        redaction_env_with_value(&env, "OPENROUTER_API_KEY", "sk-or-v1-secret-key-12345");
    assert_eq!(
        redaction.get("OPENROUTER_API_KEY").map(String::as_str),
        Some("sk-or-v1-secret-key-12345"),
        "redaction env must record the API key for error body redaction"
    );
}

#[test]
fn openrouter_saved_agent_model_discovery_resolves_provider() {
    let record: crate::managed_agents::ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "abcd1234",
            "name": "test-agent",
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "model": "anthropic/claude-sonnet-4",
            "provider": "openrouter",
            "env_vars": {
                "OPENROUTER_API_KEY": "sk-or-test-key",
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
    .expect("sample openrouter managed agent record");

    let config = saved_agent_model_discovery_config(&record, "buzz-agent");
    assert_eq!(config.provider.as_deref(), Some("openrouter"));
    assert_eq!(config.model.as_deref(), Some("anthropic/claude-sonnet-4"));
    assert_eq!(
        config.env.get("OPENROUTER_API_KEY").map(String::as_str),
        Some("sk-or-test-key")
    );
    assert!(!config.env.contains_key("BUZZ_PRIVATE_KEY"));
}

/// B5/T4: unsaved-agent ("draft") discovery mirrors the saved-agent path —
/// `draft_agent_model_discovery_env` must derive the provider env var from
/// form input the same way `saved_agent_model_discovery_config` derives it
/// from a persisted record, and preserve caller-supplied env (including the
/// OpenRouter API key) unmodified.
#[test]
fn openrouter_draft_agent_model_discovery_derives_provider_env() {
    let env_vars = BTreeMap::from([(
        "OPENROUTER_API_KEY".to_string(),
        "sk-or-draft-key".to_string(),
    )]);

    let merged = draft_agent_model_discovery_env("buzz-agent", Some("openrouter"), &env_vars);

    assert_eq!(
        merged.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("openrouter"),
        "provider env var must be derived from form input for a known ACP runtime"
    );
    assert_eq!(
        merged.get("OPENROUTER_API_KEY").map(String::as_str),
        Some("sk-or-draft-key"),
        "caller-supplied env vars must survive the merge"
    );
}

#[test]
fn draft_agent_model_discovery_env_omits_provider_when_absent() {
    let merged = draft_agent_model_discovery_env("buzz-agent", None, &BTreeMap::new());
    assert!(
        !merged.contains_key("BUZZ_AGENT_PROVIDER"),
        "no provider must be derived when the caller supplies none"
    );
}
