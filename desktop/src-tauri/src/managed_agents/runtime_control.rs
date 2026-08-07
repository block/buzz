//! Signed supervisor -> Desktop managed-runtime control queue.

use atomic_write_file::AtomicWriteFile;
use nostr::JsonUtil;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const CONTROL_KIND: u16 = 29_110;
const SUPERVISOR_NAME: &str = "Buzz Management";
const MAX_REQUEST_BYTES: u64 = 128 * 1024;
const MAX_AGE_SECS: u64 = 120;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeControlPayload {
    action: String,
    target_pubkey: String,
    relay_url: String,
    requested_at: u64,
    nonce: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeControlReceipt {
    ok: bool,
    action: String,
    target_pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(crate) fn spawn_runtime_control_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(400));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = process_pending(&app).await {
                eprintln!("buzz-desktop: runtime-control watcher: {error}");
            }
        }
    });
}

async fn process_pending(app: &AppHandle) -> Result<(), String> {
    let root = control_root(app)?;
    let inbox = root.join("inbox");
    let receipts = root.join("receipts");
    tokio::fs::create_dir_all(&inbox)
        .await
        .map_err(|error| format!("create {}: {error}", inbox.display()))?;
    tokio::fs::create_dir_all(&receipts)
        .await
        .map_err(|error| format!("create {}: {error}", receipts.display()))?;
    let mut entries = tokio::fs::read_dir(&inbox)
        .await
        .map_err(|error| format!("read {}: {error}", inbox.display()))?;
    let mut processed = 0usize;
    while processed < 16 {
        let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| error.to_string())?
        else {
            break;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        processed += 1;
        let event_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("invalid")
            .to_owned();
        let receipt_path = receipts.join(format!("{event_id}.json"));
        let app_for_request = app.clone();
        let request_path = path.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            process_one(&app_for_request, &request_path)
        })
        .await
        .map_err(|error| format!("runtime-control worker join: {error}"))?;
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => RuntimeControlReceipt {
                ok: false,
                action: "unknown".into(),
                target_pubkey: "unknown".into(),
                error: Some(error),
            },
        };
        atomic_write_json(&receipt_path, &receipt)?;
        if let Err(error) = tokio::fs::remove_file(&path).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "buzz-desktop: runtime-control could not remove {}: {error}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn process_one(app: &AppHandle, path: &Path) -> Result<RuntimeControlReceipt, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("inspect {}: {error}", path.display()))?;
    if metadata.len() > MAX_REQUEST_BYTES {
        return Err("runtime-control request exceeds size limit".into());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let event =
        nostr::Event::from_json(raw).map_err(|error| format!("parse signed event: {error}"))?;
    event
        .verify()
        .map_err(|error| format!("verify signed event: {error}"))?;
    if event.kind.as_u16() != CONTROL_KIND {
        return Err(format!(
            "unexpected runtime-control kind {}",
            event.kind.as_u16()
        ));
    }
    let payload: RuntimeControlPayload = serde_json::from_str(&event.content)
        .map_err(|error| format!("parse control payload: {error}"))?;
    if payload.nonce.trim().is_empty() {
        return Err("runtime-control nonce is empty".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if payload.requested_at > now.saturating_add(10)
        || now.saturating_sub(payload.requested_at) > MAX_AGE_SECS
        || now.saturating_sub(event.created_at.as_secs()) > MAX_AGE_SECS
    {
        return Err("runtime-control request is stale or future-dated".into());
    }
    let requester_pubkey = event.pubkey.to_hex();
    let target_pubkey = payload.target_pubkey.trim().to_ascii_lowercase();
    if requester_pubkey == target_pubkey {
        return Err("supervisor cannot lifecycle-control itself".into());
    }
    let signed_target = event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(String::as_str) == Some("p")
            && values
                .get(1)
                .is_some_and(|value| value.eq_ignore_ascii_case(&target_pubkey))
    });
    if !signed_target {
        return Err("signed p tag does not match target_pubkey".into());
    }
    if !(payload.relay_url.starts_with("ws://") || payload.relay_url.starts_with("wss://")) {
        return Err("relay_url must use ws:// or wss://".into());
    }

    let records = super::load_managed_agents(app)?;
    let requester = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&requester_pubkey))
        .ok_or_else(|| "requester is not a saved managed agent".to_string())?;
    if requester.name.trim() != SUPERVISOR_NAME || requester.backend != super::BackendKind::Local {
        return Err("requester is not the local Buzz Management supervisor".into());
    }
    let target = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&target_pubkey))
        .ok_or_else(|| "target is not a saved managed agent".to_string())?;
    if target.backend != super::BackendKind::Local {
        return Err("target is not a local managed agent".into());
    }

    // The signed request is still bound to the active workspace relay. This
    // prevents a valid supervisor identity from using the local queue to
    // start/stop the same agent against an arbitrary relay pair.
    let state = app.state::<crate::app_state::AppState>();
    let active_relay = crate::relay::relay_ws_url_with_override(&state);
    if normalize_relay_url(&payload.relay_url) != normalize_relay_url(&active_relay) {
        return Err("runtime-control relay is not the active workspace relay".into());
    }

    let action = payload.action.trim().to_ascii_lowercase();
    let outcome = match action.as_str() {
        "start" => super::start_managed_agent_runtime(
            target_pubkey.clone(),
            payload.relay_url.clone(),
            app.clone(),
        )
        .map(|_| ()),
        "stop" => super::stop_managed_agent_runtime(
            target_pubkey.clone(),
            payload.relay_url.clone(),
            app.clone(),
        )
        .map(|_| ()),
        _ => return Err("action must be start or stop".into()),
    };
    Ok(RuntimeControlReceipt {
        ok: outcome.is_ok(),
        action,
        target_pubkey,
        error: outcome.err(),
    })
}

fn normalize_relay_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn control_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("agents").join("runtime-control"))
        .map_err(|error| format!("resolve app-data runtime-control path: {error}"))
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("receipt path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create {}: {error}", parent.display()))?;
    let payload = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let mut file =
        AtomicWriteFile::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    file.write_all(&payload)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", path.display()))
}
