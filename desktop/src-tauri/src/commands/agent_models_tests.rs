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

fn saved_discovery_record() -> crate::managed_agents::ManagedAgentRecord {
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
    .expect("sample managed agent record")
}

fn saved_discovery_persona(
    runtime: &str,
    model: Option<&str>,
) -> crate::managed_agents::AgentDefinition {
    serde_json::from_value(serde_json::json!({
        "id": "live-persona",
        "display_name": "Live Persona",
        "system_prompt": "",
        "runtime": runtime,
        "model": model,
        "provider": "ignored-for-locked-runtime",
        "env_vars": {
            "SAFE_PERSONA_SETTING": "live",
            "LM_STUDIO_API_TOKEN": "must-not-leak"
        },
        "created_at": "",
        "updated_at": ""
    }))
    .expect("sample persona")
}

#[test]
fn saved_agent_model_discovery_uses_live_persona_then_global_projection() {
    let mut record = saved_discovery_record();
    record.persona_id = Some("live-persona".to_string());
    record.agent_command_override = None;
    record.model = None;
    record.provider = None;
    let persona = saved_discovery_persona("buzz-lmstudio-agent", Some("persona-current"));
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-current".to_string()),
        provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let config = saved_agent_model_discovery_config(&record, &[persona], &global);

    assert_eq!(config.model.as_deref(), Some("persona-current"));
    assert_eq!(
        config.provider.as_deref(),
        Some("ignored-for-locked-runtime")
    );
    assert_eq!(
        config.env.get("LM_STUDIO_MODEL").map(String::as_str),
        Some("persona-current")
    );
    assert_eq!(
        config.env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("lmstudio-native")
    );
    assert_eq!(
        config.env.get("SAFE_PERSONA_SETTING").map(String::as_str),
        Some("live")
    );
    assert!(!config.env.contains_key("LM_STUDIO_API_TOKEN"));
}

#[test]
fn saved_agent_model_discovery_reflects_global_edit_and_agent_override() {
    let mut record = saved_discovery_record();
    record.persona_id = Some("live-persona".to_string());
    record.agent_command_override = None;
    record.model = None;
    record.provider = None;
    let persona = saved_discovery_persona("buzz-lmstudio-agent", None);
    let mut global = crate::managed_agents::GlobalAgentConfig {
        model: Some("global-before".to_string()),
        ..Default::default()
    };

    global.model = Some("global-after".to_string());
    let inherited =
        saved_agent_model_discovery_config(&record, std::slice::from_ref(&persona), &global);
    assert_eq!(inherited.model.as_deref(), Some("global-after"));
    assert_eq!(
        inherited.env.get("LM_STUDIO_MODEL").map(String::as_str),
        Some("global-after")
    );

    record.model = Some("agent-override".to_string());
    let overridden = saved_agent_model_discovery_config(&record, &[persona], &global);
    assert_eq!(overridden.model.as_deref(), Some("agent-override"));
    assert_eq!(
        overridden.env.get("LM_STUDIO_MODEL").map(String::as_str),
        Some("agent-override")
    );
}

// ---------------------------------------------------------------------------
// Databricks provider detection
// ---------------------------------------------------------------------------
//
// Parse/filter/pagination tests live in crates/buzz-agent/src/catalog.rs

#[test]
fn lmstudio_models_filter_non_llm_and_preserve_loaded_facts() {
    let value = serde_json::json!({
        "models": [
            {
                "type": "llm",
                "key": "qwen/qwen3.6-27b",
                "display_name": "Qwen3.6 27B",
                "loaded_instances": [{"id": "qwen/qwen3.6-27b"}],
                "max_context_length": 262144,
                "capabilities": {
                    "vision": true,
                    "trained_for_tool_use": true,
                    "reasoning": {"allowed_options": ["off", "on"], "default": "on"}
                }
            },
            {
                "type": "embedding",
                "key": "nomic/embed",
                "display_name": "Embed",
                "loaded_instances": []
            }
        ]
    });

    let models = super::normalize_lmstudio_models(value).expect("valid native catalog");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "qwen/qwen3.6-27b");
    assert!(models[0].is_loaded);
    assert_eq!(models[0].loaded_instance_ids, ["qwen/qwen3.6-27b"]);
    assert_eq!(models[0].max_context_length, Some(262_144));
    assert_eq!(
        models[0]
            .capabilities
            .as_ref()
            .and_then(|value| value.get("trained_for_tool_use"))
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn lmstudio_empty_and_non_llm_catalogs_are_successful_empty_discovery() {
    let empty = super::normalize_lmstudio_models(serde_json::json!({"models": []}))
        .expect("an empty native catalog is a valid response");
    assert!(empty.is_empty());

    let non_llm = super::normalize_lmstudio_models(serde_json::json!({
        "models": [{
            "type": "embedding",
            "key": "nomic/embed",
            "loaded_instances": []
        }]
    }))
    .expect("a catalog with no LLM entries is still a valid response");
    assert!(non_llm.is_empty());
}

#[test]
fn lmstudio_catalog_rejects_model_count_over_limit() {
    let at_limit = (0..256)
        .map(|index| {
            serde_json::json!({
                "type": "llm",
                "key": format!("model-{index}"),
                "loaded_instances": []
            })
        })
        .collect::<Vec<_>>();
    let normalized = super::normalize_lmstudio_models(serde_json::json!({"models": at_limit}))
        .expect("boundary catalog");
    assert_eq!(normalized.len(), 256);
    assert_eq!(
        normalized.first().map(|model| model.id.as_str()),
        Some("model-0")
    );
    assert_eq!(
        normalized.last().map(|model| model.id.as_str()),
        Some("model-255")
    );

    let one_past = (0..257)
        .map(|index| {
            serde_json::json!({
                "type": "llm",
                "key": format!("model-{index}"),
                "loaded_instances": []
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        super::normalize_lmstudio_models(serde_json::json!({"models": one_past}))
            .expect_err("oversized catalog must be rejected"),
        "LM Studio model catalog exceeds the maximum model count"
    );
}

#[test]
fn lmstudio_catalog_rejects_control_identifiers_and_oversized_metadata() {
    let control = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe\nforged",
            "loaded_instances": []
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(control)
            .expect_err("control-bearing identifiers must be rejected"),
        "LM Studio model catalog contains an invalid model identifier"
    );

    let oversized_description = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe",
            "description": "x".repeat(4097),
            "loaded_instances": []
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(oversized_description)
            .expect_err("oversized descriptions must be rejected"),
        "LM Studio model catalog contains an oversized description"
    );
}

#[test]
fn lmstudio_catalog_bounds_nested_capabilities_and_context_length() {
    let boundary = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "m".repeat(256),
            "display_name": "n".repeat(512),
            "description": "d".repeat(4096),
            "loaded_instances": (0..32)
                .map(|index| serde_json::json!({"id": format!("instance-{index}")}))
                .collect::<Vec<_>>(),
            "max_context_length": 16777216_u64,
            "capabilities": {"nested":{"level":{"enabled":true}}}
        }]
    });
    let normalized = super::normalize_lmstudio_models(boundary).expect("metadata boundary values");
    assert_eq!(normalized[0].loaded_instance_ids.len(), 32);
    assert_eq!(normalized[0].max_context_length, Some(16_777_216));

    let too_long_id = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "m".repeat(257),
            "loaded_instances": []
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(too_long_id)
            .expect_err("one-past model identifier must fail"),
        "LM Studio model catalog contains an invalid model identifier"
    );

    let too_many_instances = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe",
            "loaded_instances": (0..33)
                .map(|index| serde_json::json!({"id": format!("instance-{index}")}))
                .collect::<Vec<_>>()
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(too_many_instances)
            .expect_err("one-past loaded instance count must fail"),
        "LM Studio model catalog contains too many loaded instances"
    );

    let too_large_context = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe",
            "loaded_instances": [],
            "max_context_length": 16777217_u64
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(too_large_context)
            .expect_err("unreasonable context length must be rejected"),
        "LM Studio model catalog contains an invalid context length"
    );

    let too_deep = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe",
            "loaded_instances": [],
            "capabilities": {"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":true}}}}}}}}}
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(too_deep)
            .expect_err("overly nested capabilities must fail"),
        "LM Studio model catalog contains overly complex capabilities metadata"
    );

    let oversized_capabilities = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe",
            "loaded_instances": [],
            "capabilities": (0..17)
                .map(|index| (format!("field_{index}"), serde_json::json!("x".repeat(1000))))
                .collect::<serde_json::Map<String, serde_json::Value>>()
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(oversized_capabilities)
            .expect_err("oversized capabilities must fail"),
        "LM Studio model catalog contains oversized capabilities metadata"
    );

    let invalid_shape = serde_json::json!({
        "models": [{
            "type": "llm",
            "key": "safe",
            "loaded_instances": [],
            "capabilities": ["unexpected", "top-level", "array"]
        }]
    });
    assert_eq!(
        super::normalize_lmstudio_models(invalid_shape)
            .expect_err("capabilities must be a bounded object"),
        "LM Studio model catalog contains invalid capabilities metadata"
    );
}

#[test]
fn lmstudio_catalog_duplicate_handling_is_deterministic_first_wins() {
    let value = serde_json::json!({
        "models": [
            {
                "type": "llm",
                "key": "duplicate",
                "display_name": "First",
                "loaded_instances": []
            },
            {
                "type": "llm",
                "key": "duplicate",
                "display_name": "Second",
                "loaded_instances": [{"id": "second"}]
            }
        ]
    });

    let models = super::normalize_lmstudio_models(value).expect("duplicate catalog");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name.as_deref(), Some("First"));
    assert!(!models[0].is_loaded);
}

#[test]
fn lmstudio_readiness_distinguishes_no_loaded_mismatch_and_ready() {
    let unloaded = vec![crate::managed_agents::AgentModelInfo {
        id: "installed".to_string(),
        name: Some("Installed".to_string()),
        description: None,
        loaded_instance_ids: Vec::new(),
        is_loaded: false,
        max_context_length: Some(8_192),
        capabilities: None,
    }];
    assert_eq!(
        super::lmstudio_readiness_from_models(true, None, unloaded, false).status,
        super::LmStudioReadinessState::NoLoadedModel
    );

    let loaded = vec![crate::managed_agents::AgentModelInfo {
        id: "loaded".to_string(),
        name: Some("Loaded".to_string()),
        description: None,
        loaded_instance_ids: vec!["loaded".to_string()],
        is_loaded: true,
        max_context_length: Some(262_144),
        capabilities: None,
    }];
    assert_eq!(
        super::lmstudio_readiness_from_models(
            true,
            Some("different".to_string()),
            loaded.clone(),
            true,
        )
        .status,
        super::LmStudioReadinessState::ConfiguredModelUnavailable
    );
    let ready =
        super::lmstudio_readiness_from_models(true, Some("loaded".to_string()), loaded, false);
    assert_eq!(ready.status, super::LmStudioReadinessState::Ready);
    assert_eq!(
        ready.security_warnings,
        [
            "LM Studio API authentication is not enabled.",
            "LM Studio listener exposure is unverified."
        ]
    );
    assert_eq!(ready.bind_exposure, "unknown");
}

#[test]
fn lmstudio_authenticated_ready_still_warns_when_listener_exposure_is_unknown() {
    let loaded = vec![crate::managed_agents::AgentModelInfo {
        id: "loaded".to_string(),
        name: Some("Loaded".to_string()),
        description: None,
        loaded_instance_ids: vec!["loaded".to_string()],
        is_loaded: true,
        max_context_length: Some(262_144),
        capabilities: None,
    }];

    let ready =
        super::lmstudio_readiness_from_models(true, Some("loaded".to_string()), loaded, true);

    assert_eq!(ready.status, super::LmStudioReadinessState::Ready);
    assert_eq!(ready.bind_exposure, "unknown");
    assert_eq!(
        ready.security_warnings,
        ["LM Studio listener exposure is unverified."]
    );
}
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
