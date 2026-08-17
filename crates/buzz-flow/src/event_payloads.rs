//! Additional Flow Studio event payloads (kinds 46200–46399).

use serde::{Deserialize, Serialize};

/// Payload for [`KIND_FLOW_BLOCK_EXECUTED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowBlockExecuted {
    /// Flow identifier (`d` tag).
    pub flow_id: String,
    /// Block instance id on the canvas.
    pub block_id: String,
    /// Block registry type (e.g. `http`).
    pub block_type: String,
    /// Redacted output summary.
    pub output_json: String,
}

/// Payload for [`KIND_FLOW_BLOCK_FAILED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowBlockFailed {
    /// Flow identifier (`d` tag).
    pub flow_id: String,
    /// Block instance id on the canvas.
    pub block_id: String,
    /// Block registry type (e.g. `http`).
    pub block_type: String,
    /// Human-readable failure reason.
    pub error: String,
}

/// Payload for [`KIND_FLOW_KB_DOCUMENT_INGESTED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowKbDocumentIngested {
    /// Knowledge base identifier.
    pub knowledge_base_id: String,
    /// Document identifier (`d` tag).
    pub document_id: String,
    /// Original filename.
    pub filename: String,
    /// MIME type of the ingested document.
    pub mime_type: String,
    /// Optional plain-text body indexed as a single embedding chunk (MVP keyword search).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Payload for [`KIND_FLOW_KB_EMBEDDING_INDEXED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowKbEmbeddingIndexed {
    /// Parent document identifier.
    pub document_id: String,
    /// Unique embedding row identifier.
    pub embedding_id: String,
    /// Chunk index within the document.
    pub chunk_index: i32,
}

/// Payload for [`KIND_FLOW_FILE_UPLOADED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowFileUploaded {
    /// File identifier (`d` tag).
    pub file_id: String,
    /// Original filename.
    pub filename: String,
    /// Blossom media URL when bytes were uploaded.
    pub media_url: Option<String>,
}

/// Payload for [`KIND_FLOW_FILE_DELETED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowFileDeleted {
    /// File identifier (`d` tag).
    pub file_id: String,
}

/// Payload for [`KIND_FLOW_KB_SEMANTIC_QUERY`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowKbSemanticQuery {
    /// Knowledge base to search.
    pub knowledge_base_id: String,
    /// Natural-language query string.
    pub query: String,
    /// Maximum hits to return.
    pub top_k: u32,
}

/// Payload for [`KIND_FLOW_TABLE_ROW_CREATED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowTableRowCreated {
    /// Table identifier.
    pub table_id: String,
    /// Row identifier within the table.
    pub row_id: String,
    /// Serialized row JSON.
    pub row_json: String,
}

/// Payload for [`KIND_FLOW_TABLE_ROW_UPDATED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowTableRowUpdated {
    /// Table identifier.
    pub table_id: String,
    /// Row identifier within the table.
    pub row_id: String,
    /// Serialized row JSON.
    pub row_json: String,
}

/// Payload for [`KIND_FLOW_TABLE_ROW_DELETED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowTableRowDeleted {
    /// Table identifier.
    pub table_id: String,
    /// Row identifier within the table.
    pub row_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_block_executed_roundtrip() {
        let payload = FlowBlockExecuted {
            flow_id: "onboarding".into(),
            block_id: "step-1".into(),
            block_type: "http".into(),
            output_json: r#"{"status":200}"#.into(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: FlowBlockExecuted = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, payload);
    }
}
