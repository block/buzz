//! Nostr event kinds and payloads for Buzz Agent Studio (claude-code-cli-ui merge).
//!
//! Kind numbers are defined in `buzz_core::kind` (range 47200–47399).

pub use buzz_core::kind::{
    is_agent_studio_kind, KIND_AGENT_CONFIG_CREATED, KIND_AGENT_CONFIG_UPDATED,
    KIND_AGENT_GRAPH_EDGE, KIND_AGENT_SESSION_TELEMETRY, KIND_AGENT_SKILL_IMPORTED,
};

/// Payload for [`KIND_AGENT_CONFIG_CREATED`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentConfigCreated {
    /// Persona / agent slug (`d` tag).
    pub agent_id: String,
    /// Serialized persona frontmatter or JSON config.
    pub config_json: String,
}

/// Payload for [`KIND_AGENT_CONFIG_UPDATED`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentConfigUpdated {
    /// Persona / agent slug (`d` tag).
    pub agent_id: String,
    /// Serialized persona frontmatter or JSON config.
    pub config_json: String,
}

/// Payload for [`KIND_AGENT_SESSION_TELEMETRY`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentSessionTelemetry {
    /// Session identifier.
    pub session_id: String,
    /// Agent / persona slug when known.
    pub agent_id: Option<String>,
    /// Cumulative input tokens.
    pub input_tokens: u64,
    /// Cumulative output tokens.
    pub output_tokens: u64,
    /// Estimated USD cost.
    pub cost_usd: f64,
    /// Tool invocation count.
    pub tool_calls: u32,
}

/// Payload for [`KIND_AGENT_GRAPH_EDGE`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentGraphEdge {
    /// Source node kind label.
    pub source_type: String,
    /// Source slug.
    pub source_slug: String,
    /// Target node kind label.
    pub target_type: String,
    /// Target slug.
    pub target_slug: String,
    /// Edge semantics.
    pub relationship_type: String,
    /// Detection evidence.
    pub evidence: String,
}

/// Payload for [`KIND_AGENT_SKILL_IMPORTED`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentSkillImported {
    /// Skill identifier.
    pub skill_id: String,
    /// Source repository URL when imported from GitHub.
    pub source_repo: Option<String>,
    /// Commit SHA at import time.
    pub source_commit: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_config_json_roundtrip() {
        let event = AgentConfigCreated {
            agent_id: "reviewer".into(),
            config_json: r#"{"model":"claude-sonnet"}"#.into(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let back: AgentConfigCreated = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event);
    }

    #[test]
    fn skill_import_json_roundtrip() {
        let event = AgentSkillImported {
            skill_id: "lint-rust".into(),
            source_repo: Some("https://github.com/example/skills".into()),
            source_commit: Some("abc123".into()),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let back: AgentSkillImported = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event);
    }

    #[test]
    fn kind_range_covers_agent_studio() {
        assert!(is_agent_studio_kind(KIND_AGENT_CONFIG_CREATED));
        assert!(is_agent_studio_kind(KIND_AGENT_GRAPH_EDGE));
        assert!(is_agent_studio_kind(47399));
        assert!(!is_agent_studio_kind(47199));
        assert!(!is_agent_studio_kind(47400));
        assert!(!is_agent_studio_kind(48001));
        assert!(!is_agent_studio_kind(48100));
    }
}
