use super::*;

fn databricks_family_fixture() -> AgentModelsResponse {
    AgentModelsResponse {
        agent_name: "goose".to_string(),
        agent_version: "test".to_string(),
        models: [
            "databricks-claude-opus-4-7",
            "databricks-gpt-5-6-sol",
            "databricks-llama-4-maverick",
        ]
        .into_iter()
        .map(|id| AgentModelInfo {
            id: id.to_string(),
            name: None,
            description: None,
        })
        .collect(),
        agent_default_model: Some("databricks-claude-opus-4-7".to_string()),
        selected_model: Some("databricks-gpt-5-6-sol".to_string()),
        supports_switching: true,
    }
}

#[test]
fn databricks_transport_filters_models_to_anthropic_family() {
    let filtered = filter_databricks_models_for_family(
        databricks_family_fixture(),
        Some("anthropic"),
        Some("databricks_v2"),
    );

    assert_eq!(
        filtered
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["databricks-claude-opus-4-7"]
    );
    assert_eq!(
        filtered.agent_default_model.as_deref(),
        Some("databricks-claude-opus-4-7")
    );
    assert_eq!(filtered.selected_model, None);
}

#[test]
fn databricks_transport_filters_models_to_openai_family() {
    let filtered = filter_databricks_models_for_family(
        databricks_family_fixture(),
        Some("openai"),
        Some("databricks-v2"),
    );

    assert_eq!(
        filtered
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["databricks-gpt-5-6-sol"]
    );
    assert_eq!(filtered.agent_default_model, None);
    assert_eq!(
        filtered.selected_model.as_deref(),
        Some("databricks-gpt-5-6-sol")
    );
}

#[test]
fn direct_provider_discovery_is_not_family_filtered() {
    let response = filter_databricks_models_for_family(
        databricks_family_fixture(),
        Some("anthropic"),
        Some("anthropic"),
    );

    assert_eq!(response.models.len(), 3);
}
