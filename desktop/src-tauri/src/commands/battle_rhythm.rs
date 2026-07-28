use crate::command_services::planning_import::{
    extract_planning_document, ExtractedPlanningDocument,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri_plugin_dialog::DialogExt;
use tokio_util::sync::CancellationToken;

const IMPORT_SYSTEM_PROMPT: &str = r#"You extract proposed calendar entries from a naval planning document. Return JSON only with exactly: schemaVersion (1), sourceType ("fas", "longcast", or "shortcast"), proposedCoverage ({start,end}), events, and uncertainties. Each event must contain exactly title, type, start, end, allDay, location, responsibleOwner, participants, remarks, and sourceLocation. Every event requires a precise sourceLocation copied from an extracted block. Use RFC3339 timestamps and never invent events not supported by the document. Use uncertainties as objects with exactly location and message. Do not add prose or markdown."#;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterpretPlanningDocumentRequest {
    document: ExtractedPlanningDocument,
    source_type: String,
    proposed_coverage: PlanningCoverage,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningCoverage {
    start: String,
    end: String,
}

#[tauri::command]
pub async fn pick_battle_rhythm_document(
    app: tauri::AppHandle,
) -> Result<Option<ExtractedPlanningDocument>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Planning documents", &["docx", "xlsx", "pdf"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let selected = receiver
        .await
        .map_err(|_| "planning document dialog was cancelled".to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .as_path()
        .ok_or_else(|| "planning document picker returned an invalid path".to_string())?
        .to_path_buf();
    tauri::async_runtime::spawn_blocking(move || extract_planning_document(&path))
        .await
        .map_err(|_| "planning document extraction task failed".to_string())?
        .map(Some)
}

#[tauri::command]
pub async fn interpret_battle_rhythm_document(
    app: tauri::AppHandle,
    request: InterpretPlanningDocumentRequest,
) -> Result<Option<Value>, String> {
    if !matches!(
        request.source_type.as_str(),
        "fas" | "longcast" | "shortcast"
    ) {
        return Err("planning source type is invalid".into());
    }
    let start = chrono::DateTime::parse_from_rfc3339(&request.proposed_coverage.start)
        .map_err(|_| "planning coverage is invalid".to_string())?;
    let end = chrono::DateTime::parse_from_rfc3339(&request.proposed_coverage.end)
        .map_err(|_| "planning coverage is invalid".to_string())?;
    if start >= end {
        return Err("planning coverage is invalid".into());
    }
    let mut document = request.document;
    while serde_json::to_vec(&document)
        .map_err(|_| "extracted planning document is invalid".to_string())?
        .len()
        > 700 * 1024
    {
        if document.blocks.pop().is_none() {
            return Err("extracted planning document cannot be bounded".into());
        }
        document.truncated = true;
    }
    let input = json!({
        "sourceType": request.source_type,
        "proposedCoverage": {
            "start": request.proposed_coverage.start,
            "end": request.proposed_coverage.end,
        },
        "document": document,
    });
    Ok(
        crate::command_services::structured_completion::complete_json(
            &app,
            IMPORT_SYSTEM_PROMPT,
            &input,
            "battle_rhythm_import_v1",
            CancellationToken::new(),
        )
        .await
        .ok(),
    )
}
