use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryKnowledgeStatus {
    status: String,
    server_identity: Option<String>,
    node_id: Option<String>,
    revision_count: u64,
    conflict_count: u64,
    replication_cursor: Option<u64>,
    last_successful_sync: Option<String>,
    freshness: &'static str,
    validation: &'static str,
    tool_allowlist: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RagKnowledgeStatus {
    status: String,
    server_identity: Option<String>,
    active_snapshot_id: Option<String>,
    signature_fingerprint: Option<String>,
    snapshot_time: Option<String>,
    last_successful_activation: Option<String>,
    freshness: String,
    validation: String,
    tool_allowlist: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppleKnowledgeStatus {
    source: String,
    permission: String,
    observed_at: String,
    record_count: usize,
    truncated: bool,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandKnowledgeStatus {
    kind: &'static str,
    version: u32,
    classification: &'static str,
    observed_at: String,
    memory: MemoryKnowledgeStatus,
    rag: RagKnowledgeStatus,
    apple_inputs: Vec<AppleKnowledgeStatus>,
    degraded_sections: Vec<String>,
}

fn value_text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn value_texts(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

async fn memory_knowledge_status(app: tauri::AppHandle) -> (MemoryKnowledgeStatus, Vec<String>) {
    let readiness =
        crate::command_services::memory::get_memory_service_readiness(app.clone()).await;
    let value = serde_json::to_value(readiness).unwrap_or(Value::Null);
    let mut status = value_text(&value, "status").unwrap_or_else(|| "unavailable".to_string());
    let node_id = value_text(&value, "nodeId");
    let mut error = value_text(&value, "error");
    let mut validation = if status == "ready" {
        "verified"
    } else if status == "not_configured" {
        "unknown"
    } else {
        "failed"
    };
    let mut freshness = if status == "ready" {
        "fresh"
    } else {
        "unknown"
    };
    let mut server_identity = None;
    let mut tool_allowlist = Vec::new();
    if status == "ready" {
        let admission_app = app.clone();
        let admission_node = node_id.clone().unwrap_or_default();
        let observed_at = Utc::now().to_rfc3339();
        let admission = tauri::async_runtime::spawn_blocking(move || {
            admit_memory_for_catalog(&admission_app, &admission_node, &observed_at)
        })
        .await;
        match admission {
            Ok(Ok(service)) => {
                let adviser_tools = build_catalog_integrations(
                    std::slice::from_ref(&service),
                    CommandKnowledgeWorkflow::Adviser,
                )
                .ok()
                .and_then(|integrations| {
                    integrations
                        .into_iter()
                        .next()
                        .map(|integration| integration.allowed_tools)
                });
                if let Some(adviser_tools) = adviser_tools {
                    server_identity = Some(service.server_identity.clone());
                    tool_allowlist = adviser_tools;
                    cache_verified_service(service);
                } else {
                    clear_cached_service(KnowledgeServiceKind::Memory);
                    status = "unavailable".to_string();
                    freshness = "unknown";
                    validation = "failed";
                    error = Some("admission_failed".to_string());
                }
            }
            _ => {
                clear_cached_service(KnowledgeServiceKind::Memory);
                status = "unavailable".to_string();
                freshness = "unknown";
                validation = "failed";
                error = Some("admission_failed".to_string());
            }
        }
    } else {
        clear_cached_service(KnowledgeServiceKind::Memory);
    }
    let conflict_count = value
        .get("conflictCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut degraded = Vec::new();
    if status != "ready" {
        degraded.push("memory-readiness".to_string());
    }
    if conflict_count > 0 {
        degraded.push("memory-conflicts".to_string());
    }
    (
        MemoryKnowledgeStatus {
            status,
            server_identity,
            node_id,
            revision_count: value
                .get("revisionCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            conflict_count,
            replication_cursor: None,
            last_successful_sync: None,
            freshness,
            validation,
            tool_allowlist,
            error,
        },
        degraded,
    )
}

async fn rag_knowledge_status(app: tauri::AppHandle) -> (RagKnowledgeStatus, Vec<String>) {
    let readiness = crate::command_services::rag::get_rag_service_readiness(app).await;
    let value = serde_json::to_value(readiness).unwrap_or(Value::Null);
    let status = value_text(&value, "status").unwrap_or_else(|| "unavailable".to_string());
    let degraded = if status == "ready" {
        Vec::new()
    } else {
        vec!["rag-readiness".to_string()]
    };
    (
        RagKnowledgeStatus {
            status,
            server_identity: value_text(&value, "serverIdentity"),
            active_snapshot_id: value_text(&value, "activeSnapshotId"),
            signature_fingerprint: value_text(&value, "signatureFingerprint"),
            snapshot_time: value_text(&value, "snapshotTime"),
            last_successful_activation: value_text(&value, "lastSuccessfulActivation"),
            freshness: value_text(&value, "freshness").unwrap_or_else(|| "unknown".to_string()),
            validation: value_text(&value, "validation").unwrap_or_else(|| "unknown".to_string()),
            tool_allowlist: value_texts(&value, "toolAllowlist"),
            error: value_text(&value, "error"),
        },
        degraded,
    )
}

pub(crate) async fn refresh_knowledge_admissions(app: tauri::AppHandle) {
    let _ = tokio::join!(
        memory_knowledge_status(app.clone()),
        rag_knowledge_status(app),
    );
}

async fn apple_knowledge_status() -> (Vec<AppleKnowledgeStatus>, Vec<String>) {
    let sources = ["calendar", "reminders", "notes", "files"];
    let futures = sources.into_iter().map(|source| async move {
        let request = serde_json::from_value::<
            crate::command_services::apple_inputs::AppleInputRequest,
        >(serde_json::json!({
            "operation": "permission_status",
            "arguments": {"source": source},
        }));
        let value = match request {
            Ok(request) => serde_json::to_value(
                crate::command_services::apple_inputs::read_apple_inputs(request).await,
            )
            .unwrap_or(Value::Null),
            Err(_) => Value::Null,
        };
        let permission =
            value_text(&value, "permission").unwrap_or_else(|| "unavailable".to_string());
        let response_source = value_text(&value, "source").unwrap_or_else(|| source.to_string());
        let records = value
            .get("records")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        AppleKnowledgeStatus {
            source: response_source,
            permission,
            observed_at: value_text(&value, "observedAt")
                .unwrap_or_else(|| Utc::now().to_rfc3339()),
            record_count: records,
            truncated: value
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            error: value_text(&value, "error"),
        }
    });
    let statuses = futures_util::future::join_all(futures).await;
    let degraded = statuses
        .iter()
        .filter(|status| status.permission != "authorized" || status.error.is_some())
        .map(|status| format!("apple-{}", status.source))
        .collect();
    (statuses, degraded)
}

#[tauri::command]
pub(crate) async fn get_command_knowledge_status(app: tauri::AppHandle) -> CommandKnowledgeStatus {
    let (memory, rag, apple) = tokio::join!(
        memory_knowledge_status(app.clone()),
        rag_knowledge_status(app),
        apple_knowledge_status(),
    );
    let mut degraded_sections = memory.1;
    degraded_sections.extend(rag.1);
    degraded_sections.extend(apple.1);
    degraded_sections.sort();
    degraded_sections.dedup();
    CommandKnowledgeStatus {
        kind: "command-knowledge-status",
        version: 1,
        classification: "OFFICIAL",
        observed_at: Utc::now().to_rfc3339(),
        memory: memory.0,
        rag: rag.0,
        apple_inputs: apple.0,
        degraded_sections,
    }
}
