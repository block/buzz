use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::scheduled::{
    cancel_by_id, enqueue, load_queue, next_due, parse_scheduled_at, reenqueue, take_due,
    ScheduleMessageRequest, ScheduledMessage,
};

/// List all pending scheduled messages, newest first.
#[tauri::command]
pub async fn scheduled_list(app: AppHandle) -> Result<Vec<ScheduledMessage>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .scheduled_messages_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut queue = load_queue(&app)?;
        queue.sort_by_key(|msg| std::cmp::Reverse(msg.created_at));
        Ok(queue)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Enqueue a scheduled delivery for later.
///
/// Validates the timestamp (RFC 3339, must be in the future), channel id, and
/// content up front so a bad request fails before anything is persisted.
#[tauri::command]
pub async fn scheduled_enqueue(
    input: ScheduleMessageRequest,
    app: AppHandle,
) -> Result<ScheduledMessage, String> {
    tokio::task::spawn_blocking(move || {
        let channel_id = input.channel_id.trim().to_string();
        if channel_id.is_empty() {
            return Err("channel is required".into());
        }
        let content = input.content.trim().to_string();
        if content.is_empty() {
            return Err("message content is required".into());
        }
        let scheduled_at = parse_scheduled_at(&input.scheduled_at)?;

        let state = app.state::<AppState>();
        let _store_guard = state
            .scheduled_messages_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let msg = ScheduledMessage {
            id: uuid::Uuid::new_v4().to_string(),
            channel_id,
            content,
            kind: None,
            reply_to: input.reply_to,
            broadcast: None,
            mentions: input.mentions,
            scheduled_at,
            created_at: chrono::Utc::now().timestamp(),
        };
        enqueue(&app, msg.clone())?;
        Ok(msg)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Cancel a pending scheduled message by id, returning the removed entry.
#[tauri::command]
pub async fn scheduled_cancel(id: String, app: AppHandle) -> Result<ScheduledMessage, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .scheduled_messages_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        cancel_by_id(&app, &id)?
            .ok_or_else(|| format!("no pending scheduled message with id '{id}'"))
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Re-persist an entry the delivery loop took but failed to deliver.
///
/// The entry is stored verbatim (past `scheduled_at` included) so the next
/// sweep retries it, matching the CLI's transient-failure re-enqueue.
#[tauri::command]
pub async fn scheduled_reenqueue(
    message: ScheduledMessage,
    app: AppHandle,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .scheduled_messages_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        reenqueue(&app, message)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Atomically remove and return every scheduled message that is due now.
///
/// The delivery loop calls this once per sweep; entries it fails to deliver
/// may be re-enqueued by the caller so a later sweep retries them.
#[tauri::command]
pub async fn scheduled_take_due(app: AppHandle) -> Result<Vec<ScheduledMessage>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .scheduled_messages_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        take_due(&app, chrono::Utc::now().timestamp())
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Earliest scheduled timestamp still pending, if any.
#[tauri::command]
pub async fn scheduled_next_due(app: AppHandle) -> Result<Option<i64>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .scheduled_messages_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        next_due(&app)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}