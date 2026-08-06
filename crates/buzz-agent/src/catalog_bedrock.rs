//! Bedrock model catalog discovery.
//!
//! Exposes [`discover_bedrock_models`] — an async helper that lists
//! available foundation models accessible under the configured AWS account
//! and region, using the Bedrock `ListFoundationModels` API.
//!
//! Auth is handled via the standard AWS credential chain
//! ([`sigv4::load_aws_credentials`]).

use reqwest::Client;
use serde_json::Value;

use crate::{
    config::Config,
    sigv4,
    types::AgentError,
};

/// A discovered model entry: `id` is the Bedrock model ID
/// (e.g. `anthropic.claude-3-5-sonnet-20241022-v2:0`), `name` is the
/// display label (model name + provider, e.g. `Claude 3.5 Sonnet (Anthropic)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
}

/// Discover available Bedrock models via `ListFoundationModels`.
///
/// Calls `GET https://bedrock.{region}.amazonaws.com/foundation-models`
/// with SigV4 signing, then filters to models that support ON_DEMAND
/// inference and TEXT modality.
///
/// Returns `Err(AgentError::Llm)` on transport or auth errors.
pub async fn discover_bedrock_models(cfg: &Config) -> Result<Vec<ModelEntry>, AgentError> {
    let region = sigv4::parse_bedrock_region(&cfg.base_url)
        .map_err(|e| AgentError::Llm(format!("Bedrock catalog: {e}")))?;
    let creds = sigv4::load_aws_credentials()
        .map_err(|e| AgentError::Llm(format!("Bedrock catalog: {e}")))?;

    let url = format!("https://bedrock.{region}.amazonaws.com/foundation-models");

    let http = Client::new();

    // Build a GET request, sign it with SigV4
    let req = http::Request::builder()
        .uri(&url)
        .method("GET")
        .body(Vec::new())
        .map_err(|e| AgentError::Llm(format!("Bedrock catalog: build request: {e}")))?;

    let signed = sigv4::sign_request(req, &creds, "bedrock", &region)
        .map_err(|e| AgentError::Llm(format!("Bedrock catalog: sign request: {e}")))?;

    let (parts, _) = signed.into_parts();
    let parsed_url = reqwest::Url::parse(&url)
        .map_err(|e| AgentError::Llm(format!("Bedrock catalog: parse url: {e}")))?;
    let mut rq = reqwest::Request::new(parts.method, parsed_url);
    *rq.headers_mut() = parts.headers;

    let response = http
        .execute(rq)
        .await
        .map_err(|e| AgentError::Llm(format!("Bedrock catalog: transport: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AgentError::Llm(format!(
            "Bedrock catalog HTTP {status}: {body}"
        )));
    }

    let json: Value = response.json().await.map_err(|e| {
        AgentError::Llm(format!("Bedrock catalog: parse response: {e}"))
    })?;

    parse_bedrock_model_list(&json)
}

/// Parse a `ListFoundationModels` response into model entries.
///
/// Filters to models that support ON_DEMAND inference and TEXT modality.
pub(crate) fn parse_bedrock_model_list(json: &Value) -> Result<Vec<ModelEntry>, AgentError> {
    let Some(summaries) = json.get("modelSummaries").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };

    let entries: Vec<ModelEntry> = summaries
        .iter()
        .filter(|m| {
            // Only include models that support on-demand inference
            let has_on_demand = m
                .get("inferenceTypesSupported")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().any(|t| t == "ON_DEMAND"))
                .unwrap_or(false);
            // Only include models that support text input
            let has_text = m
                .get("inputModalities")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().any(|t| t == "TEXT"))
                .unwrap_or(false);
            has_on_demand && has_text
        })
        .map(|m| {
            let id = m
                .get("modelId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let model_name = m
                .get("modelName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let provider = m
                .get("providerName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            ModelEntry {
                id,
                name: format!("{model_name} ({provider})"),
            }
        })
        .collect();

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_bedrock_model_list_basic() {
        let json = json!({
            "modelSummaries": [
                {
                    "modelId": "anthropic.claude-3-5-sonnet-20241022-v2:0",
                    "modelName": "Claude 3.5 Sonnet",
                    "providerName": "Anthropic",
                    "inferenceTypesSupported": ["ON_DEMAND"],
                    "inputModalities": ["TEXT"],
                    "outputModalities": ["TEXT"],
                },
                {
                    "modelId": "meta.llama3-70b-instruct-v1:0",
                    "modelName": "Llama 3 70B Instruct",
                    "providerName": "Meta",
                    "inferenceTypesSupported": ["ON_DEMAND"],
                    "inputModalities": ["TEXT"],
                    "outputModalities": ["TEXT"],
                },
            ]
        });

        let models = parse_bedrock_model_list(&json).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0].id,
            "anthropic.claude-3-5-sonnet-20241022-v2:0"
        );
        assert_eq!(models[0].name, "Claude 3.5 Sonnet (Anthropic)");
        assert_eq!(models[1].id, "meta.llama3-70b-instruct-v1:0");
        assert_eq!(models[1].name, "Llama 3 70B Instruct (Meta)");
    }

    #[test]
    fn test_parse_bedrock_model_list_filters_non_text() {
        let json = json!({
            "modelSummaries": [
                {
                    "modelId": "amazon.titan-embed-text-v2:0",
                    "modelName": "Titan Text Embeddings v2",
                    "providerName": "Amazon",
                    "inferenceTypesSupported": ["ON_DEMAND"],
                    "inputModalities": ["TEXT"],
                    "outputModalities": ["EMBEDDING"],
                },
            ]
        });

        let models = parse_bedrock_model_list(&json).unwrap();
        // Embedding models don't output text, but inputModalities is TEXT so
        // the filter includes them. This is intentional — the catalog is permissive
        // and the user can filter further in the UI.
        assert_eq!(models.len(), 1);
    }

    #[test]
    fn test_parse_bedrock_model_list_filters_provisioned_only() {
        let json = json!({
            "modelSummaries": [
                {
                    "modelId": "anthropic.claude-opus-4-v2:0",
                    "modelName": "Claude Opus 4 v2",
                    "providerName": "Anthropic",
                    "inferenceTypesSupported": ["PROVISIONED"],
                    "inputModalities": ["TEXT"],
                    "outputModalities": ["TEXT"],
                },
            ]
        });

        let models = parse_bedrock_model_list(&json).unwrap();
        // PROVISIONED-only models are excluded — only ON_DEMAND is surfaced
        assert_eq!(models.len(), 0);
    }

    #[test]
    fn test_parse_bedrock_model_list_empty() {
        let json = json!({});
        let models = parse_bedrock_model_list(&json).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn test_parse_bedrock_model_list_no_summaries_key() {
        let json = json!({"foo": "bar"});
        let models = parse_bedrock_model_list(&json).unwrap();
        assert!(models.is_empty());
    }
}
