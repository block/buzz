use std::io::Write;

/// Assign an experimental canvas task and notify it through the dedicated Monitor.
#[tauri::command]
pub async fn assign_task_channel(
    channel_id: String,
    assignee_pubkey: String,
    expected_canvas: String,
    canvas_content: String,
    completed_canvas: String,
    operation_id: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, String> {
    uuid::Uuid::parse_str(&channel_id).map_err(|_| "Invalid channel")?;
    uuid::Uuid::parse_str(&operation_id).map_err(|_| "Invalid assignment operation")?;
    if assignee_pubkey.len() != 64 || !assignee_pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid agent public key".into());
    }
    if [&expected_canvas, &canvas_content, &completed_canvas]
        .iter()
        .any(|canvas| canvas.len() > 256 * 1024)
    {
        return Err("Task canvas exceeds assignment limit".into());
    }
    let python = crate::managed_agents::resolve_command("python3").ok_or("Install Python 3")?;
    let buzz = crate::managed_agents::resolve_command("buzz").ok_or("Install the Buzz CLI")?;
    let key = state
        .keys
        .lock()
        .map_err(|_| "Identity unavailable")?
        .secret_key()
        .to_secret_hex();
    let relay = crate::relay::relay_ws_url_with_override(&state);
    let payload = serde_json::to_vec(&serde_json::json!({
        "channel_id": channel_id, "assignee_pubkey": assignee_pubkey,
        "expected_canvas": expected_canvas, "canvas_content": canvas_content,
        "completed_canvas": completed_canvas, "operation_id": operation_id,
    }))
    .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        // Private temporary input keeps canvas content off argv and within exec-size limits.
        let mut input = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
        input
            .write_all(&payload)
            .map_err(|error| error.to_string())?;
        let mut command = std::process::Command::new(python);
        command
            .args([
                "-c",
                include_str!("../../../../scripts/assign-task-channel.py"),
            ])
            .env("BUZZ_BIN", buzz)
            .env("BUZZ_TASK_ASSIGNMENT_INPUT", input.path())
            .env("BUZZ_PRIVATE_KEY", key)
            .env("BUZZ_RELAY_URL", relay)
            .env_remove("BUZZ_AUTH_TAG");
        let output = crate::managed_agents::output_with_timeout(
            command,
            std::time::Duration::from_secs(300),
        )
        .ok_or("Assignment failed or timed out; refresh the task before retrying")?;
        serde_json::from_slice(&output.stdout)
            .map_err(|_| "Assignment helper did not return a result".into())
    })
    .await
    .map_err(|error| error.to_string())?
}
