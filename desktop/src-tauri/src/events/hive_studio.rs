//! Buzz Hive event builders (Flow Studio + Agent Studio kinds).

use buzz_core_pkg::kind::{
    KIND_AGENT_SKILL_IMPORTED, KIND_FLOW_FILE_UPLOADED, KIND_FLOW_GRAPH_SAVED,
    KIND_FLOW_KB_DOCUMENT_INGESTED, KIND_FLOW_TABLE_ROW_CREATED, KIND_FLOW_TABLE_ROW_DELETED,
};
use nostr::{EventBuilder, Kind, Tag};

const MAX_CONTENT_BYTES: usize = 64 * 1024;

fn tag(parts: Vec<&str>) -> Result<Tag, String> {
    Tag::parse(parts).map_err(|e| format!("invalid tag: {e}"))
}

fn check_content(content: &str) -> Result<(), String> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(format!(
            "content exceeds maximum size of {} bytes (got {})",
            MAX_CONTENT_BYTES,
            content.len()
        ));
    }
    Ok(())
}

/// Kind 46200 — save a Flow Studio canvas graph (replaceable by `d` tag).
pub fn build_flow_graph_saved(flow_id: &str, graph_json: &str) -> Result<EventBuilder, String> {
    if flow_id.trim().is_empty() {
        return Err("flow_id is required".into());
    }
    check_content(graph_json)?;
    let payload = serde_json::json!({
        "flow_id": flow_id,
        "graph_json": graph_json,
    });
    let content = serde_json::to_string(&payload).map_err(|e| format!("serialize graph: {e}"))?;
    let tags = vec![tag(vec!["d", flow_id])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_FLOW_GRAPH_SAVED as u16), content).tags(tags))
}

/// Kind 47250 — record a skill import from GitHub.
pub fn build_agent_skill_imported(
    skill_id: &str,
    source_repo: Option<&str>,
    source_commit: Option<&str>,
) -> Result<EventBuilder, String> {
    if skill_id.trim().is_empty() {
        return Err("skill_id is required".into());
    }
    let payload = serde_json::json!({
        "skill_id": skill_id,
        "source_repo": source_repo,
        "source_commit": source_commit,
    });
    let content = serde_json::to_string(&payload).map_err(|e| format!("serialize skill: {e}"))?;
    let tags = vec![tag(vec!["d", skill_id])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_AGENT_SKILL_IMPORTED as u16), content).tags(tags))
}

/// Kind 46250 — ingest a knowledge-base document.
pub fn build_flow_kb_document_ingested(
    knowledge_base_id: &str,
    document_id: &str,
    filename: &str,
    mime_type: &str,
    content: Option<&str>,
) -> Result<EventBuilder, String> {
    if knowledge_base_id.trim().is_empty() || document_id.trim().is_empty() {
        return Err("knowledge_base_id and document_id are required".into());
    }
    let payload = serde_json::json!({
        "knowledge_base_id": knowledge_base_id,
        "document_id": document_id,
        "filename": filename,
        "mime_type": mime_type,
        "content": content,
    });
    let content_json =
        serde_json::to_string(&payload).map_err(|e| format!("serialize kb document: {e}"))?;
    let tags = vec![tag(vec!["d", document_id])?];
    Ok(EventBuilder::new(
        Kind::Custom(KIND_FLOW_KB_DOCUMENT_INGESTED as u16),
        content_json,
    )
    .tags(tags))
}

/// Kind 46300 — create or update a Flow Studio table row.
pub fn build_flow_table_row_created(
    table_id: &str,
    row_id: &str,
    row_json: &str,
) -> Result<EventBuilder, String> {
    if table_id.trim().is_empty() || row_id.trim().is_empty() {
        return Err("table_id and row_id are required".into());
    }
    check_content(row_json)?;
    let payload = serde_json::json!({
        "table_id": table_id,
        "row_id": row_id,
        "row_json": row_json,
    });
    let content =
        serde_json::to_string(&payload).map_err(|e| format!("serialize table row: {e}"))?;
    let tags = vec![tag(vec!["d", &format!("{table_id}:{row_id}")])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_FLOW_TABLE_ROW_CREATED as u16), content).tags(tags))
}

/// Kind 46302 — delete a Flow Studio table row.
pub fn build_flow_table_row_deleted(table_id: &str, row_id: &str) -> Result<EventBuilder, String> {
    if table_id.trim().is_empty() || row_id.trim().is_empty() {
        return Err("table_id and row_id are required".into());
    }
    let payload = serde_json::json!({
        "table_id": table_id,
        "row_id": row_id,
    });
    let content =
        serde_json::to_string(&payload).map_err(|e| format!("serialize table delete: {e}"))?;
    let tags = vec![tag(vec!["d", &format!("{table_id}:{row_id}")])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_FLOW_TABLE_ROW_DELETED as u16), content).tags(tags))
}

/// Kind 46350 — register Flow Studio file metadata (bytes via Buzz media).
pub fn build_flow_file_uploaded(
    file_id: &str,
    filename: &str,
    media_url: Option<&str>,
) -> Result<EventBuilder, String> {
    if file_id.trim().is_empty() || filename.trim().is_empty() {
        return Err("file_id and filename are required".into());
    }
    let payload = serde_json::json!({
        "file_id": file_id,
        "filename": filename,
        "media_url": media_url,
    });
    let content =
        serde_json::to_string(&payload).map_err(|e| format!("serialize file metadata: {e}"))?;
    let tags = vec![tag(vec!["d", file_id])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_FLOW_FILE_UPLOADED as u16), content).tags(tags))
}
