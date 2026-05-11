use std::{path::PathBuf, process::Child};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendKind {
    #[default]
    Local,
    Provider {
        id: String,
        config: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaRecord {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    /// Preferred ACP provider ID (e.g. "goose", "claude", "codex").
    /// When deploying an agent from this persona, this provider is pre-selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Preferred model ID (e.g. "gpt-4o", "claude-sonnet-4-20250514").
    /// Passed to the agent at creation time when deploying from this persona.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Pool of short, thematic names for bot instances created from this persona.
    /// When a new copy is added to a channel, a random unused name is picked from this pool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(default = "default_record_active")]
    pub is_active: bool,
    /// Pack ID if this persona was imported from a persona pack.
    /// Pack personas are non-editable (system_prompt, model locked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_pack: Option<String>,
    /// Internal persona slug within the pack (e.g., "lep", "pip").
    /// Used by ACP's `resolve_persona_by_name()` to find the right persona.
    /// Validated: `[a-zA-Z0-9_-]+`, max 64 chars (safe for env vars and paths).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_pack_persona_slug: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayAgentInfo {
    pub pubkey: String,
    pub name: String,
    pub agent_type: String,
    pub channels: Vec<String>,
    #[serde(default)]
    pub channel_ids: Vec<String>,
    pub capabilities: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedAgentRecord {
    pub pubkey: String,
    pub name: String,
    #[serde(default)]
    pub persona_id: Option<String>,
    pub private_key_nsec: String,
    /// NIP-OA auth tag JSON. Computed at agent creation time.
    ///
    /// Pre-existing agents created before NIP-OA will have `None` here.
    /// This is intentional — they continue to work without attestation.
    /// Re-attestation requires agent recreation (v2 migration scope).
    #[serde(default)]
    pub auth_tag: Option<String>,
    pub relay_url: String,
    pub acp_command: String,
    pub agent_command: String,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub turn_timeout_seconds: u64,
    /// Idle timeout in seconds. If set, overrides turn_timeout_seconds.
    #[serde(default)]
    pub idle_timeout_seconds: Option<u64>,
    /// Absolute wall-clock cap per turn.
    #[serde(default)]
    pub max_turn_duration_seconds: Option<u64>,
    #[serde(default = "default_agent_parallelism")]
    pub parallelism: u32,
    pub system_prompt: Option<String>,
    /// Desired LLM model ID. Matches AgentModelInfo.id from discovery.
    /// The harness re-discovers the correct ACP switching metadata at session
    /// creation by matching this ID against the fresh session/new response.
    #[serde(default)]
    pub model: Option<String>,
    /// Comma-separated toolset string forwarded as SPROUT_TOOLSETS to the MCP subprocess.
    /// When None, the MCP server uses its own default ("default" toolset).
    #[serde(default)]
    pub mcp_toolsets: Option<String>,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    #[serde(default)]
    pub runtime_pid: Option<u32>,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default)]
    pub backend_agent_id: Option<String>,
    #[serde(default)]
    pub provider_binary_path: Option<String>,
    /// Installed pack path (absolute). Set when agent was created from a pack persona.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_pack_path: Option<PathBuf>,
    /// Persona name within the pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_name_in_pack: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_started_at: Option<String>,
    pub last_stopped_at: Option<String>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Debug)]
pub struct ManagedAgentProcess {
    pub child: Child,
    pub log_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedAgentSummary {
    pub pubkey: String,
    pub name: String,
    pub persona_id: Option<String>,
    pub relay_url: String,
    pub acp_command: String,
    pub agent_command: String,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub turn_timeout_seconds: u64,
    pub idle_timeout_seconds: Option<u64>,
    pub max_turn_duration_seconds: Option<u64>,
    pub parallelism: u32,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub mcp_toolsets: Option<String>,
    pub backend: BackendKind,
    pub backend_agent_id: Option<String>,
    pub status: String,
    pub pid: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
    pub last_started_at: Option<String>,
    pub last_stopped_at: Option<String>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
    pub start_on_app_launch: bool,
    pub log_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateManagedAgentRequest {
    pub name: String,
    #[serde(default)]
    pub persona_id: Option<String>,
    pub relay_url: Option<String>,
    pub acp_command: Option<String>,
    pub agent_command: Option<String>,
    #[serde(default)]
    pub agent_args: Vec<String>,
    pub mcp_command: Option<String>,
    pub turn_timeout_seconds: Option<u64>,
    pub idle_timeout_seconds: Option<u64>,
    pub max_turn_duration_seconds: Option<u64>,
    pub parallelism: Option<u32>,
    pub system_prompt: Option<String>,
    pub avatar_url: Option<String>,
    pub model: Option<String>,
    pub mcp_toolsets: Option<String>,
    #[serde(default)]
    pub spawn_after_create: bool,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    #[serde(default)]
    pub backend: BackendKind,
}

#[derive(Debug, Serialize)]
pub struct CreateManagedAgentResponse {
    pub agent: ManagedAgentSummary,
    pub private_key_nsec: String,
    pub profile_sync_error: Option<String>,
    pub spawn_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePersonaRequest {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersonaRequest {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ManagedAgentLogResponse {
    pub content: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpProviderInfo {
    pub id: String,
    pub label: String,
    pub command: String,
    pub binary_path: String,
    pub default_args: Vec<String>,
    /// MCP server binary override. `None` means use the default (`sprout-mcp-server`).
    pub mcp_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandAvailabilityInfo {
    pub command: String,
    pub resolved_path: Option<String>,
    pub available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverManagedAgentPrereqsRequest {
    pub acp_command: Option<String>,
    pub mcp_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedAgentPrereqsInfo {
    pub acp: CommandAvailabilityInfo,
    pub mcp: CommandAvailabilityInfo,
}

/// Patch request for updating a managed agent's mutable fields.
///
/// Tri-state nullable semantics via `Option<Option<T>>`:
/// - Field absent in JSON → `None` (don't touch)
/// - `"field": null` → `Some(None)` (clear to default)
/// - `"field": "value"` → `Some(Some("value"))` (set)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManagedAgentRequest {
    pub pubkey: String,
    /// Absent = don't touch. Present = rename the agent.
    #[serde(default)]
    pub name: Option<String>,
    /// Absent = don't touch. null = clear to agent default. "id" = set.
    #[serde(default)]
    pub model: Option<Option<String>>,
    #[serde(default)]
    pub system_prompt: Option<Option<String>>,
    #[serde(default)]
    pub mcp_toolsets: Option<Option<String>>,
    #[serde(default)]
    pub parallelism: Option<u32>,
    #[serde(default)]
    pub turn_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub acp_command: Option<String>,
    #[serde(default)]
    pub agent_command: Option<String>,
    #[serde(default)]
    pub agent_args: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_command: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdateManagedAgentResponse {
    pub agent: ManagedAgentSummary,
    pub profile_sync_error: Option<String>,
}

/// Response from `get_agent_models` — normalized model info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelsResponse {
    pub agent_name: String,
    pub agent_version: String,
    /// Unified model list (merged from both ACP paths, deduplicated by ID).
    pub models: Vec<AgentModelInfo>,
    /// The agent's default model for a fresh session.
    pub agent_default_model: Option<String>,
    /// The user's persisted model selection (from ManagedAgentRecord.model).
    pub selected_model: Option<String>,
    /// Whether this agent supports model switching.
    pub supports_switching: bool,
}

/// A single model available from an agent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelInfo {
    /// Canonical ID used for persistence and round-tripping.
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub persona_ids: Vec<String>,
    #[serde(default)]
    pub is_builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTeamRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}

pub const DEFAULT_ACP_COMMAND: &str = "sprout-acp";
pub const DEFAULT_AGENT_COMMAND: &str = "goose";
pub const DEFAULT_MCP_COMMAND: &str = "sprout-mcp-server";
/// ~5 min (320s) — matches the CLI harness default (SPROUT_ACP_IDLE_TIMEOUT).
pub const DEFAULT_AGENT_TURN_TIMEOUT_SECONDS: u64 = 320;
/// 1 hour — absolute wall-clock safety cap per turn.
pub const DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS: u64 = 3600;
pub const DEFAULT_AGENT_PARALLELISM: u32 = 3;

fn default_agent_parallelism() -> u32 {
    DEFAULT_AGENT_PARALLELISM
}

fn default_start_on_app_launch() -> bool {
    true
}

fn default_record_active() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{ManagedAgentRecord, PersonaRecord};

    #[test]
    fn persona_record_defaults_active_when_field_is_missing() {
        let record: PersonaRecord = serde_json::from_str(
            r#"{
                "id": "builtin:solo",
                "display_name": "Solo",
                "avatar_url": null,
                "system_prompt": "Prompt",
                "created_at": "2026-03-19T00:00:00Z",
                "updated_at": "2026-03-19T00:00:00Z"
            }"#,
        )
        .expect("legacy persona payload should deserialize");

        assert!(record.is_active);
        assert!(!record.is_builtin);
        assert_eq!(record.provider, None);
        assert_eq!(record.model, None);
        assert!(record.name_pool.is_empty());
    }

    /// Legacy agent records (created before NIP-OA) lack the `auth_tag` field.
    /// `#[serde(default)]` must ensure they deserialize with `auth_tag: None`.
    #[test]
    fn managed_agent_record_without_auth_tag_deserializes() {
        let record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "abcd1234",
                "name": "test-agent",
                "private_key_nsec": "nsec1fake",
                "relay_url": "wss://localhost:3000",
                "acp_command": "sprout-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "sprout-mcp-server",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            }"#,
        )
        .expect("legacy agent record without auth_tag should deserialize");

        assert_eq!(record.auth_tag, None);
        assert_eq!(record.pubkey, "abcd1234");
    }

    /// Agent records WITH an auth_tag round-trip correctly through serde.
    #[test]
    fn managed_agent_record_with_auth_tag_round_trips() {
        let json = r#"{
            "pubkey": "abcd1234",
            "name": "test-agent",
            "private_key_nsec": "nsec1fake",
            "auth_tag": "[\"auth\",\"deadbeef\",\"\",\"cafebabe\"]",
            "relay_url": "wss://localhost:3000",
            "acp_command": "sprout-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "sprout-mcp-server",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }"#;

        let record: ManagedAgentRecord =
            serde_json::from_str(json).expect("record with auth_tag should deserialize");

        assert_eq!(
            record.auth_tag.as_deref(),
            Some(r#"["auth","deadbeef","","cafebabe"]"#)
        );

        // Round-trip: serialize and deserialize again.
        let serialized = serde_json::to_string(&record).expect("should serialize");
        let record2: ManagedAgentRecord =
            serde_json::from_str(&serialized).expect("round-trip should deserialize");
        assert_eq!(record.auth_tag, record2.auth_tag);
    }
}
