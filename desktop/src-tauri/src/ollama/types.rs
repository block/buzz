use serde::{Deserialize, Serialize};

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434";

/// Defines which parts of an Ollama installation Buzz owns.
#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OllamaOwnershipMode {
    /// Buzz only connects to a daemon operated by the user.
    #[default]
    ConnectOnly,
    /// Buzz may pull and delete models, but never starts or stops the daemon.
    ExternalManagedModels,
    /// Buzz owns a private runtime process and private model directory.
    Managed,
}

/// Machine-wide Ollama connection and ownership settings.
#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaMachineConfig {
    pub endpoint: String,
    pub mode: OllamaOwnershipMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_model: Option<String>,
}

impl Default for OllamaMachineConfig {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_OLLAMA_ENDPOINT.to_string(),
            mode: OllamaOwnershipMode::ConnectOnly,
            selected_model: None,
        }
    }
}

/// One model returned by Ollama's native model inventory API.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModel {
    pub name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Live daemon and Buzz-managed runtime status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub config: OllamaMachineConfig,
    pub reachable: bool,
    pub version: Option<String>,
    pub models: Vec<OllamaModel>,
    pub error: Option<String>,
    pub managed_runtime_installed: bool,
    pub managed_runtime_running: bool,
    pub managed_install_supported: bool,
}

/// Parsed progress from Ollama's newline-delimited pull response.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaPullProgress {
    pub model: String,
    pub status: String,
    pub digest: Option<String>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub done: bool,
}

/// Model metadata relevant to agent compatibility.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelInfo {
    pub model: String,
    pub capabilities: Vec<String>,
    pub supports_tools: bool,
    pub details: serde_json::Value,
    pub model_info: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOllamaModelInput {
    pub model: String,
    pub confirmed: bool,
}
