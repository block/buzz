//! Shared IPC authority metadata; interpreted only by the identity boundary.

pub(crate) fn invocation_generation(
    body: &tauri::ipc::InvokeBody,
    headers: &tauri::http::HeaderMap,
) -> Option<u64> {
    match body {
        tauri::ipc::InvokeBody::Json(body) => body
            .get("expectedGeneration")
            .and_then(serde_json::Value::as_u64),
        tauri::ipc::InvokeBody::Raw(_) => headers
            .get("x-buzz-identity-generation")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok()),
    }
}
