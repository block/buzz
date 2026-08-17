//! Event → Postgres projector (P3 read-model).

use buzz_core::tenant::CommunityId;

use crate::event_payloads::{
    FlowFileDeleted, FlowFileUploaded, FlowKbDocumentIngested, FlowTableRowCreated,
    FlowTableRowDeleted,
};
use crate::events::{
    KIND_FLOW_FILE_DELETED, KIND_FLOW_FILE_UPLOADED, KIND_FLOW_KB_DOCUMENT_INGESTED,
    KIND_FLOW_TABLE_ROW_CREATED, KIND_FLOW_TABLE_ROW_DELETED, KIND_FLOW_TABLE_ROW_UPDATED,
};

/// Projector instruction derived from a stored Nostr event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectorAction {
    /// Upsert a knowledge document row.
    UpsertKnowledgeDocument {
        /// Community that owns the document row.
        community_id: CommunityId,
        /// Parsed ingest event payload.
        payload: FlowKbDocumentIngested,
    },
    /// Upsert a table row.
    UpsertTableRow {
        /// Community that owns the table row.
        community_id: CommunityId,
        /// Table identifier.
        table_id: String,
        /// Row identifier within the table.
        row_id: String,
        /// Serialized row JSON.
        row_json: String,
    },
    /// Soft-delete a table row.
    DeleteTableRow {
        /// Community that owns the table row.
        community_id: CommunityId,
        /// Table identifier.
        table_id: String,
        /// Row identifier within the table.
        row_id: String,
    },
    /// Upsert file metadata (content lives in Buzz media).
    UpsertFile {
        /// Community that owns the file row.
        community_id: CommunityId,
        /// File identifier.
        file_id: String,
        /// Original filename.
        filename: String,
        /// Blossom media URL when uploaded.
        media_url: Option<String>,
    },
    /// Soft-delete file metadata.
    DeleteFile {
        /// Community that owns the file row.
        community_id: CommunityId,
        /// File identifier.
        file_id: String,
    },
}

/// Map a Flow Studio kind + JSON content to a projector action (MVP).
pub fn project_event(
    community_id: CommunityId,
    kind: u32,
    content: &str,
) -> Result<Option<ProjectorAction>, serde_json::Error> {
    match kind {
        KIND_FLOW_KB_DOCUMENT_INGESTED => {
            let payload: FlowKbDocumentIngested = serde_json::from_str(content)?;
            Ok(Some(ProjectorAction::UpsertKnowledgeDocument {
                community_id,
                payload,
            }))
        }
        KIND_FLOW_TABLE_ROW_CREATED | KIND_FLOW_TABLE_ROW_UPDATED => {
            let payload: FlowTableRowCreated = serde_json::from_str(content)?;
            Ok(Some(ProjectorAction::UpsertTableRow {
                community_id,
                table_id: payload.table_id,
                row_id: payload.row_id,
                row_json: payload.row_json,
            }))
        }
        KIND_FLOW_TABLE_ROW_DELETED => {
            let payload: FlowTableRowDeleted = serde_json::from_str(content)?;
            Ok(Some(ProjectorAction::DeleteTableRow {
                community_id,
                table_id: payload.table_id,
                row_id: payload.row_id,
            }))
        }
        KIND_FLOW_FILE_UPLOADED => {
            let payload: FlowFileUploaded = serde_json::from_str(content)?;
            Ok(Some(ProjectorAction::UpsertFile {
                community_id,
                file_id: payload.file_id,
                filename: payload.filename,
                media_url: payload.media_url,
            }))
        }
        KIND_FLOW_FILE_DELETED => {
            let payload: FlowFileDeleted = serde_json::from_str(content)?;
            Ok(Some(ProjectorAction::DeleteFile {
                community_id,
                file_id: payload.file_id,
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn projects_kb_ingest() {
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        let content = r#"{"knowledge_base_id":"kb","document_id":"d1","filename":"a.txt","mime_type":"text/plain"}"#;
        let action = project_event(community_id, KIND_FLOW_KB_DOCUMENT_INGESTED, content)
            .expect("project")
            .expect("some");
        assert!(matches!(
            action,
            ProjectorAction::UpsertKnowledgeDocument { .. }
        ));
    }
}
