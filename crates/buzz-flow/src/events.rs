//! Nostr event kinds and payloads for Buzz Flow Studio (Sim merge).
//!
//! Kind numbers are defined in `buzz_core::kind` (range 46200–46399).

pub use buzz_core::kind::{
    is_flow_studio_kind, KIND_FLOW_BLOCK_EXECUTED, KIND_FLOW_BLOCK_FAILED, KIND_FLOW_FILE_DELETED,
    KIND_FLOW_FILE_UPLOADED, KIND_FLOW_FILE_VERSIONED, KIND_FLOW_GRAPH_SAVED,
    KIND_FLOW_KB_DOCUMENT_INGESTED, KIND_FLOW_KB_EMBEDDING_INDEXED, KIND_FLOW_KB_SEMANTIC_QUERY,
    KIND_FLOW_TABLE_ROW_CREATED, KIND_FLOW_TABLE_ROW_DELETED, KIND_FLOW_TABLE_ROW_UPDATED,
};

/// Payload for [`KIND_FLOW_GRAPH_SAVED`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlowGraphSaved {
    /// Stable flow identifier (`d` tag).
    pub flow_id: String,
    /// Serialized canvas graph (nodes + edges).
    pub graph_json: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_graph_saved_json_roundtrip() {
        let event = FlowGraphSaved {
            flow_id: "onboarding".into(),
            graph_json: r#"{"nodes":[],"edges":[]}"#.into(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let back: FlowGraphSaved = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event);
    }

    #[test]
    fn kind_range_covers_flow_studio() {
        assert!(is_flow_studio_kind(KIND_FLOW_GRAPH_SAVED));
        assert!(is_flow_studio_kind(KIND_FLOW_FILE_DELETED));
        assert!(is_flow_studio_kind(46399));
        assert!(!is_flow_studio_kind(46199));
        assert!(!is_flow_studio_kind(46400));
    }
}
