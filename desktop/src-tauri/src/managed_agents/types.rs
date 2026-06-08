use std::{collections::BTreeMap, path::PathBuf, process::Child};

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
    /// Preferred ACP runtime ID (e.g., 'goose', 'claude', 'codex'). Determines which agent binary
    /// Sprout spawns. When deploying from this persona, this runtime is pre-selected in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Opaque, harness-specific model identifier string. Format depends on the runtime and its LLM
    /// provider (e.g., 'goose-claude-4-6-opus' for Databricks, 'claude-opus-4-7' for Anthropic
    /// direct). Sprout stores and passes through without interpretation.
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
    /// Harness-level configuration passed to the agent subprocess as environment variables.
    /// Opaque to Sprout — keys and values are runtime-specific.
    ///
    /// Stored as a BTreeMap for deterministic on-disk ordering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
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
    /// Environment variables injected at spawn time. Layered as: desktop
    /// parent env < persona `env_vars` < this agent's `env_vars` (last wins).
    ///
    /// To "override" a persona env var: set the same key here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
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
    /// Inbound author gate mode. Translates to `SPROUT_ACP_RESPOND_TO`.
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Allowlist used when `respond_to == Allowlist`. Stored normalized
    /// (64-char lowercase hex, deduped). Empty when mode is not Allowlist.
    /// Preserved across mode toggles so users don't lose state.
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
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
    pub respond_to: RespondTo,
    pub respond_to_allowlist: Vec<String>,
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
    /// Environment variables for this agent. Layered on top of persona env.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub spawn_after_create: bool,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Raw allowlist as received from the frontend. Validated and normalized
    /// before being written to the record.
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
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
    pub runtime: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    /// Environment variables for agents created from this persona.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersonaRequest {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    /// Environment variables for agents created from this persona.
    ///
    /// Absent (`None`) = don't touch the stored value (caller didn't include
    /// the field). `Some(map)` = replace entirely (empty map clears all).
    /// Defaulting an omitted field to an empty map would silently erase
    /// stored credentials when an unrelated field is edited.
    #[serde(default)]
    pub env_vars: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct ManagedAgentLogResponse {
    pub content: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpAvailabilityStatus {
    Available,
    AdapterMissing,
    CliMissing,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpRuntimeCatalogEntry {
    pub id: String,
    pub label: String,
    pub avatar_url: String,
    pub availability: AcpAvailabilityStatus,
    pub command: Option<String>,
    pub binary_path: Option<String>,
    pub default_args: Vec<String>,
    pub mcp_command: Option<String>,
    pub install_hint: String,
    pub install_instructions_url: String,
    /// true when at least one automated install step is available
    pub can_auto_install: bool,
    pub underlying_cli_path: Option<String>,
}

/// Result of a single install step (CLI or adapter).
#[derive(Debug, Clone, Serialize)]
pub struct InstallStepResult {
    pub step: String,
    pub command: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

/// Aggregate result of installing a runtime (may include CLI + adapter steps).
#[derive(Debug, Clone, Serialize)]
pub struct InstallRuntimeResult {
    pub success: bool,
    pub steps: Vec<InstallStepResult>,
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
    /// Absent = don't touch. Present = replace the env_vars map entirely.
    #[serde(default)]
    pub env_vars: Option<BTreeMap<String, String>>,
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
    /// Absent = don't touch. Present = set mode.
    #[serde(default)]
    pub respond_to: Option<RespondTo>,
    /// Absent = don't touch. Present = replace the allowlist (validated &
    /// normalized server-side).
    #[serde(default)]
    pub respond_to_allowlist: Option<Vec<String>>,
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
/// ~5 min (320s) — matches the CLI harness default (SPROUT_ACP_IDLE_TIMEOUT).
pub const DEFAULT_AGENT_TURN_TIMEOUT_SECONDS: u64 = 320;
/// 1 hour — absolute wall-clock safety cap per turn.
pub const DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS: u64 = 3600;
pub const DEFAULT_AGENT_PARALLELISM: u32 = 24;

fn default_agent_parallelism() -> u32 {
    DEFAULT_AGENT_PARALLELISM
}

fn default_start_on_app_launch() -> bool {
    true
}

fn default_record_active() -> bool {
    true
}

// ── Inbound author gate ──────────────────────────────────────────────────────
//
// Mirrors `sprout-acp`'s `--respond-to` CLI flag and the related
// `--respond-to-allowlist` option. Persisted per agent so the desktop can
// translate the user's choice into `SPROUT_ACP_RESPOND_TO` /
// `SPROUT_ACP_RESPOND_TO_ALLOWLIST` env vars at spawn time.
//
// Wire format is kebab-case (`owner-only`, `allowlist`, `anyone`) to match
// the harness CLI vocabulary and the strings the GUI emits.
//
// `nobody` is intentionally NOT exposed here. The harness supports it, but
// it's a heartbeat-only mode and the desktop has no surface for it.

/// Who the agent should respond to. Defaults to `OwnerOnly`, which matches
/// the harness default → existing agents behave identically.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RespondTo {
    #[default]
    OwnerOnly,
    Allowlist,
    Anyone,
}

impl RespondTo {
    /// CLI/env wire string (matches `sprout-acp`'s `--respond-to`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnerOnly => "owner-only",
            Self::Allowlist => "allowlist",
            Self::Anyone => "anyone",
        }
    }
}

/// Validate and normalize a respond-to allowlist.
///
/// Rules mirror `sprout-acp/src/config.rs::validate_allowlist`:
/// - Each entry is exactly 64 hex chars (any case in, lowercase out).
/// - Duplicates removed, insertion order preserved.
///
/// Empty input is allowed here — the boundary check (allowlist mode requires
/// at least one entry) is the caller's job, because an `UpdateManagedAgentRequest`
/// may want to validate a list without yet knowing the final mode.
pub fn validate_respond_to_allowlist(input: &[String]) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(input.len());
    for entry in input {
        let trimmed = entry.trim();
        if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid pubkey in respond-to allowlist: '{trimmed}' (must be 64 hex chars)"
            ));
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    Ok(out)
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
        assert_eq!(record.runtime, None);
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
                "mcp_command": "",
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
            "mcp_command": "",
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

    // ── Inbound author gate tests ────────────────────────────────────────

    use super::{validate_respond_to_allowlist, RespondTo};

    #[test]
    fn respond_to_default_is_owner_only() {
        assert_eq!(RespondTo::default(), RespondTo::OwnerOnly);
    }

    #[test]
    fn respond_to_serde_is_kebab_case() {
        assert_eq!(
            serde_json::to_string(&RespondTo::OwnerOnly).unwrap(),
            "\"owner-only\""
        );
        assert_eq!(
            serde_json::to_string(&RespondTo::Allowlist).unwrap(),
            "\"allowlist\""
        );
        assert_eq!(
            serde_json::to_string(&RespondTo::Anyone).unwrap(),
            "\"anyone\""
        );
        let parsed: RespondTo = serde_json::from_str("\"owner-only\"").unwrap();
        assert_eq!(parsed, RespondTo::OwnerOnly);
        let parsed: RespondTo = serde_json::from_str("\"allowlist\"").unwrap();
        assert_eq!(parsed, RespondTo::Allowlist);
        let parsed: RespondTo = serde_json::from_str("\"anyone\"").unwrap();
        assert_eq!(parsed, RespondTo::Anyone);
    }

    #[test]
    fn respond_to_rejects_unknown_modes() {
        // `nobody` is a valid harness mode but intentionally not exposed
        // through the desktop request types.
        assert!(serde_json::from_str::<RespondTo>("\"nobody\"").is_err());
        assert!(serde_json::from_str::<RespondTo>("\"OwnerOnly\"").is_err());
    }

    /// Records persisted before this feature must continue to load,
    /// defaulting to OwnerOnly (the safe, matches-harness-default value).
    #[test]
    fn managed_agent_record_without_respond_to_fields_defaults_to_owner_only() {
        let record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "abcd1234",
                "name": "legacy-agent",
                "private_key_nsec": "nsec1fake",
                "relay_url": "wss://localhost:3000",
                "acp_command": "sprout-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
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
        .expect("legacy record without respond_to fields should deserialize");
        assert_eq!(record.respond_to, RespondTo::OwnerOnly);
        assert!(record.respond_to_allowlist.is_empty());
    }

    #[test]
    fn validate_respond_to_allowlist_accepts_valid_hex_and_lowercases() {
        let upper = "A".repeat(64);
        let lower = "a".repeat(64);
        let result = validate_respond_to_allowlist(std::slice::from_ref(&upper)).unwrap();
        assert_eq!(result, vec![lower.clone()]);
    }

    #[test]
    fn validate_respond_to_allowlist_dedups_preserving_order() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let a_upper = "A".repeat(64);
        let input = vec![a.clone(), b.clone(), a_upper];
        let result = validate_respond_to_allowlist(&input).unwrap();
        assert_eq!(result, vec![a, b]);
    }

    #[test]
    fn validate_respond_to_allowlist_rejects_wrong_length() {
        let too_short = "a".repeat(63);
        assert!(validate_respond_to_allowlist(&[too_short]).is_err());
        let too_long = "a".repeat(65);
        assert!(validate_respond_to_allowlist(&[too_long]).is_err());
    }

    #[test]
    fn validate_respond_to_allowlist_rejects_non_hex() {
        let bad = "z".repeat(64);
        assert!(validate_respond_to_allowlist(&[bad]).is_err());
        // npub-style strings should not slip through.
        let npub = format!("npub1{}", "a".repeat(59));
        assert!(validate_respond_to_allowlist(&[npub]).is_err());
    }

    #[test]
    fn validate_respond_to_allowlist_trims_whitespace() {
        let padded = format!("  {}  ", "a".repeat(64));
        let result = validate_respond_to_allowlist(&[padded]).unwrap();
        assert_eq!(result, vec!["a".repeat(64)]);
    }

    #[test]
    fn validate_respond_to_allowlist_accepts_empty() {
        // Empty is allowed at this layer; the boundary check
        // (Allowlist mode requires ≥1 entry) is the caller's job.
        let result = validate_respond_to_allowlist(&[]).unwrap();
        assert!(result.is_empty());
    }
}
