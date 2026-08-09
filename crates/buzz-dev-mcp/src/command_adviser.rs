use std::{path::PathBuf, sync::Arc};

use buzz_command_sources::{
    mcp_http::McpHttpClient,
    oauth::WorldMonitorOAuthStore,
    usage::{UsageAdmission, UsagePool, WorldMonitorUsageLedger},
    world_monitor::{NormalizedWorldMonitorEvidence, WorldMonitorRequest, WorldMonitorTool},
};
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};

const N2_PERSONA: &str = "builtin:command-intelligence";

#[derive(Clone)]
pub struct CommandAdviserTools {
    inner: Arc<Inner>,
}

struct Inner {
    persona_id: String,
    rag: Option<McpHttpClient>,
    world_monitor: Option<McpHttpClient>,
    usage: Option<WorldMonitorUsageLedger>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DoctrineParams {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KnowledgeParams {
    pub query: String,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountryParams {
    pub country_code: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountryListParams {
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegionalParams {
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewsParams {
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChokepointParams {
    #[serde(default)]
    pub chokepoint: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

fn default_top_k() -> u32 {
    5
}

impl CommandAdviserTools {
    pub fn from_env() -> Option<Self> {
        let persona_id = std::env::var("COMMAND_ADVISER_PERSONA_ID").ok()?;
        let rag = std::env::var("COMMAND_ADVISER_RAG_URL")
            .ok()
            .and_then(|endpoint| url::Url::parse(&endpoint).ok())
            .and_then(|endpoint| McpHttpClient::new(endpoint).ok());

        let endpoint = std::env::var("COMMAND_ADVISER_WORLD_MONITOR_ENDPOINT").ok();
        let world_monitor = endpoint
            .as_deref()
            .zip(std::env::var_os("COMMAND_ADVISER_WORLD_MONITOR_OAUTH_PATH"))
            .and_then(|(endpoint, path)| {
                WorldMonitorOAuthStore::new(PathBuf::from(path))
                    .ok()
                    .map(|store| (endpoint, store))
            })
            .and_then(|(endpoint, store)| McpHttpClient::world_monitor(endpoint, store).ok());
        let usage = std::env::var_os("COMMAND_ADVISER_WORLD_MONITOR_USAGE_PATH")
            .map(PathBuf::from)
            .map(WorldMonitorUsageLedger::new);
        Some(Self {
            inner: Arc::new(Inner {
                persona_id,
                rag,
                world_monitor,
                usage,
            }),
        })
    }

    pub async fn search_doctrine(&self, params: DoctrineParams) -> CallToolResult {
        self.search_knowledge(KnowledgeParams {
            query: params.query,
            collections: vec!["ADF Doctrine".to_string()],
            top_k: params.top_k,
        })
        .await
    }

    pub async fn search_knowledge(&self, params: KnowledgeParams) -> CallToolResult {
        if !valid_query(&params.query)
            || !(1..=12).contains(&params.top_k)
            || params.collections.len() > 12
            || params.collections.iter().any(|item| !valid_text(item, 160))
        {
            return error_result("Command knowledge request was invalid.");
        }
        let Some(rag) = &self.inner.rag else {
            return error_result(
                "Command knowledge is unavailable; continue with available evidence.",
            );
        };
        match rag
            .call_tool(
                "search_knowledge_base",
                json!({
                    "query": params.query,
                    "collections": params.collections,
                    "top_k": params.top_k,
                }),
            )
            .await
        {
            Ok(result) => json_result(&result),
            Err(_) => {
                error_result("Command knowledge is unavailable; continue with available evidence.")
            }
        }
    }

    pub async fn world_monitor(&self, tool: WorldMonitorTool, arguments: Value) -> CallToolResult {
        if self.inner.persona_id != N2_PERSONA {
            return error_result("World Monitor tools are available only to the Maritime N2.");
        }
        let request = match WorldMonitorRequest::new(tool, arguments) {
            Ok(request) => request,
            Err(_) => return error_result("World Monitor request was invalid."),
        };
        let (Some(client), Some(usage)) = (&self.inner.world_monitor, &self.inner.usage) else {
            return error_result(
                "World Monitor is not connected; continue with available evidence.",
            );
        };
        let admission = match usage.admit(UsagePool::Direct, &request, chrono::Local::now()) {
            Ok(admission) => admission,
            Err(_) => {
                return error_result(
                    "World Monitor direct-question allowance is unavailable or exhausted.",
                );
            }
        };
        let cache_key = match admission {
            UsageAdmission::Cached(evidence) => return json_result(&json!(evidence)),
            UsageAdmission::Reserved { cache_key, .. } => cache_key,
        };
        let payload = match client
            .call_tool(request.tool.as_str(), request.arguments.clone())
            .await
        {
            Ok(payload) => payload,
            Err(_) => {
                return error_result(
                    "World Monitor is unavailable; continue with available evidence.",
                );
            }
        };
        let evidence = NormalizedWorldMonitorEvidence::new(request, payload, chrono::Utc::now());
        let _ = usage.store_success(&cache_key, &evidence, chrono::Local::now());
        json_result(&json!(evidence))
    }
}

pub fn country_arguments(params: CountryParams) -> Value {
    json!({"country_code": params.country_code})
}

pub fn country_list_arguments(params: CountryListParams) -> Value {
    compact_object([
        ("country_code", params.country_code.map(Value::String)),
        ("limit", params.limit.map(|value| json!(value))),
        ("days", params.days.map(|value| json!(value))),
    ])
}

pub fn regional_arguments(params: RegionalParams) -> Value {
    compact_object([
        ("country_code", params.country_code.map(Value::String)),
        ("region", params.region.map(Value::String)),
        ("limit", params.limit.map(|value| json!(value))),
    ])
}

pub fn news_arguments(params: NewsParams) -> Value {
    compact_object([
        ("country_code", params.country_code.map(Value::String)),
        ("topic", params.topic.map(Value::String)),
        ("limit", params.limit.map(|value| json!(value))),
        ("days", params.days.map(|value| json!(value))),
    ])
}

pub fn chokepoint_arguments(params: ChokepointParams) -> Value {
    compact_object([
        ("chokepoint", params.chokepoint.map(Value::String)),
        ("region", params.region.map(Value::String)),
        ("limit", params.limit.map(|value| json!(value))),
    ])
}

fn compact_object<const N: usize>(entries: [(&str, Option<Value>); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key.to_string(), value)))
            .collect::<Map<_, _>>(),
    )
}

fn valid_query(value: &str) -> bool {
    valid_text(value, 4096)
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}

fn json_result(value: &Value) -> CallToolResult {
    match serde_json::to_string(value) {
        Ok(text) => CallToolResult::success(vec![Content::text(text)]),
        Err(_) => error_result("Command source response was invalid."),
    }
}

fn error_result(message: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapters_emit_only_approved_argument_fields() {
        assert_eq!(
            country_arguments(CountryParams {
                country_code: "PH".to_string()
            }),
            json!({"country_code":"PH"})
        );
        assert_eq!(
            news_arguments(NewsParams {
                country_code: Some("PH".to_string()),
                topic: Some("maritime".to_string()),
                limit: Some(10),
                days: None,
            }),
            json!({"country_code":"PH","topic":"maritime","limit":10})
        );
        assert_eq!(
            chokepoint_arguments(ChokepointParams {
                chokepoint: Some("Luzon Strait".to_string()),
                region: None,
                limit: None,
            }),
            json!({"chokepoint":"Luzon Strait"})
        );
    }
}
