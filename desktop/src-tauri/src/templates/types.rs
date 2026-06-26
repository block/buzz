use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelTemplateRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_channel_type")]
    pub channel_type: String,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_template: Option<String>,
    #[serde(default)]
    pub agents: TemplateAgentRoster,
    #[serde(default)]
    pub is_builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateAgentRoster {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personas: Vec<TemplateAgentEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TemplateTeamEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateAgentEntry {
    pub persona_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "provider")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<TemplateBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateTeamEntry {
    pub team_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "provider")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<TemplateBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TemplateBackend {
    Local,
    Provider { id: String },
}

fn default_channel_type() -> String {
    "stream".to_string()
}

fn default_visibility() -> String {
    "open".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelTemplateRequest {
    pub name: String,
    pub description: Option<String>,
    pub channel_type: Option<String>,
    pub visibility: Option<String>,
    pub canvas_template: Option<String>,
    #[serde(default)]
    pub agents: TemplateAgentRoster,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelTemplateRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channel_type: Option<String>,
    pub visibility: Option<String>,
    pub canvas_template: Option<String>,
    #[serde(default)]
    pub agents: TemplateAgentRoster,
}
