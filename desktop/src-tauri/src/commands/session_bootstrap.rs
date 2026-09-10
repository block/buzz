use std::{fs, io::Write, path::PathBuf};

use nostr::{Event, JsonUtil};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    events,
    relay::{relay_api_base_url_with_override, submit_signed_event_at_with_keys},
};

const SESSION_NAMESPACE: Uuid = uuid::uuid!("3f8ec32d-7af5-51de-b136-2484a4588013");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BootstrapPhase {
    Prepared,
    ChannelAccepted,
    MessageAccepted,
    LinkAccepted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionBootstrapJournal {
    operation_id: String,
    relay_url: String,
    signer_pubkey: String,
    channel_id: String,
    parent_channel_id: String,
    create_event_json: String,
    message_event_json: String,
    link_event_json: String,
    content: String,
    phase: BootstrapPhase,
}

#[derive(Debug, Serialize)]
pub struct PendingSessionBootstrap {
    pub operation_id: String,
    pub relay_url: String,
    pub signer_pubkey: String,
    pub channel_id: String,
    pub parent_channel_id: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct SessionBootstrapResult {
    pub operation_id: String,
    pub channel_id: String,
    pub message_event_id: String,
    pub created_at: i64,
    pub recovered: bool,
}

fn validate_operation_id(value: &str) -> Result<(), String> {
    if value.len() < 8
        || value.len() > 96
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    {
        return Err("operation_id must be 8-96 URL-safe characters".into());
    }
    Ok(())
}

fn journal_path(app: &AppHandle, operation_id: &str) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("resolve app data directory failed: {error}"))?
        .join("desktop-next")
        .join("session-bootstrap");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create Session recovery directory failed: {error}"))?;
    Ok(directory.join(format!("{operation_id}.json")))
}

fn write_journal(path: &PathBuf, journal: &SessionBootstrapJournal) -> Result<(), String> {
    let bytes = serde_json::to_vec(journal)
        .map_err(|error| format!("serialize Session recovery failed: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("open Session recovery file failed: {error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("write Session recovery failed: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("commit Session recovery failed: {error}"))
}

fn read_journal(path: &PathBuf) -> Result<Option<SessionBootstrapJournal>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("read Session recovery failed: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("read Session recovery failed: {error}")),
    }
}

fn generated_session_name(content: &str) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = normalized.chars().take(56).collect::<String>();
    if normalized.chars().count() > 56 {
        title.push('…');
    }
    if title.is_empty() {
        "New session".to_string()
    } else {
        title
    }
}

fn prepared_journal(
    operation_id: &str,
    content: &str,
    parent_channel_id: &str,
    relay_url: &str,
    keys: &nostr::Keys,
) -> Result<SessionBootstrapJournal, String> {
    let signer_pubkey = keys.public_key().to_hex();
    let deterministic_name = format!("{relay_url}\0{signer_pubkey}\0{operation_id}");
    let channel_id = Uuid::new_v5(&SESSION_NAMESPACE, deterministic_name.as_bytes());
    let session_name = generated_session_name(content);
    let create_event =
        events::build_create_channel(channel_id, &session_name, "private", "stream", None, None)?
            .sign_with_keys(keys)
            .map_err(|error| format!("sign Session create failed: {error}"))?;
    let message_event = events::build_message(
        channel_id,
        content.trim(),
        None,
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        relay_url,
    )?
    .sign_with_keys(keys)
    .map_err(|error| format!("sign first Session message failed: {error}"))?;
    let link_event = events::build_session_link(parent_channel_id, &channel_id.to_string())?
        .sign_with_keys(keys)
        .map_err(|error| format!("sign Session link failed: {error}"))?;
    Ok(SessionBootstrapJournal {
        operation_id: operation_id.to_string(),
        relay_url: relay_url.to_string(),
        signer_pubkey,
        channel_id: channel_id.to_string(),
        parent_channel_id: parent_channel_id.to_string(),
        create_event_json: create_event.as_json(),
        message_event_json: message_event.as_json(),
        link_event_json: link_event.as_json(),
        content: content.to_string(),
        phase: BootstrapPhase::Prepared,
    })
}

#[tauri::command]
pub fn list_pending_session_bootstraps(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<PendingSessionBootstrap>, String> {
    let active_relay = relay_api_base_url_with_override(&state);
    let active_signer = state.signing_keys()?.public_key().to_hex();
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("resolve app data directory failed: {error}"))?
        .join("desktop-next")
        .join("session-bootstrap");
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read Session recovery directory failed: {error}")),
    };
    let mut pending = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("read Session recovery entry failed: {error}"))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        if let Some(journal) = read_journal(&path)? {
            if journal.relay_url.trim_end_matches('/') != active_relay.trim_end_matches('/')
                || journal.signer_pubkey != active_signer
            {
                continue;
            }
            pending.push(PendingSessionBootstrap {
                operation_id: journal.operation_id,
                relay_url: journal.relay_url,
                signer_pubkey: journal.signer_pubkey,
                channel_id: journal.channel_id,
                parent_channel_id: journal.parent_channel_id,
                content: journal.content,
            });
        }
    }
    Ok(pending)
}

#[tauri::command]
pub async fn bootstrap_session(
    operation_id: String,
    content: String,
    parent_channel_id: String,
    expected_relay_url: String,
    expected_signer_pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SessionBootstrapResult, String> {
    validate_operation_id(&operation_id)?;
    if content.trim().is_empty() {
        return Err("first Session message is required".into());
    }
    Uuid::parse_str(&parent_channel_id)
        .map_err(|_| "parent_channel_id must be a UUID".to_string())?;
    let active_relay = relay_api_base_url_with_override(&state);
    if active_relay.trim_end_matches('/') != expected_relay_url.trim_end_matches('/') {
        return Err("active community changed before Session creation".into());
    }
    let keys = state.signing_keys()?;
    if keys.public_key().to_hex() != expected_signer_pubkey {
        return Err("active identity changed before Session creation".into());
    }
    let path = journal_path(&app, &operation_id)?;
    let existing = read_journal(&path)?;
    let recovered = existing.is_some();
    let mut journal = match existing {
        Some(journal) => {
            if journal.content != content
                || journal.parent_channel_id != parent_channel_id
                || journal.relay_url.trim_end_matches('/')
                    != expected_relay_url.trim_end_matches('/')
                || journal.signer_pubkey != expected_signer_pubkey
            {
                return Err("Session recovery operation does not match this request".into());
            }
            journal
        }
        None => {
            let journal = prepared_journal(
                &operation_id,
                &content,
                &parent_channel_id,
                &active_relay,
                &keys,
            )?;
            write_journal(&path, &journal)?;
            journal
        }
    };

    let create_event = Event::from_json(&journal.create_event_json)
        .map_err(|error| format!("invalid recovered Session create event: {error}"))?;
    let message_event = Event::from_json(&journal.message_event_json)
        .map_err(|error| format!("invalid recovered Session message event: {error}"))?;
    let link_event = Event::from_json(&journal.link_event_json)
        .map_err(|error| format!("invalid recovered Session link event: {error}"))?;

    if matches!(journal.phase, BootstrapPhase::Prepared) {
        match submit_signed_event_at_with_keys(&create_event, &state, &active_relay, &keys).await {
            Ok(_) => {}
            Err(error) if error.contains("duplicate: channel already exists") => {}
            Err(error) => return Err(error),
        }
        state.mark_pending_owned_channel(&journal.signer_pubkey, &journal.channel_id);
        journal.phase = BootstrapPhase::ChannelAccepted;
        write_journal(&path, &journal)?;
    }

    if matches!(journal.phase, BootstrapPhase::ChannelAccepted) {
        match submit_signed_event_at_with_keys(&message_event, &state, &active_relay, &keys).await {
            Ok(_) => {}
            Err(error) if error.contains("duplicate") => {}
            Err(error) => return Err(error),
        }
        journal.phase = BootstrapPhase::MessageAccepted;
        write_journal(&path, &journal)?;
    }

    if matches!(journal.phase, BootstrapPhase::MessageAccepted) {
        match submit_signed_event_at_with_keys(&link_event, &state, &active_relay, &keys).await {
            Ok(_) => {}
            Err(error) if error.contains("duplicate") => {}
            Err(error) => return Err(error),
        }
        journal.phase = BootstrapPhase::LinkAccepted;
        write_journal(&path, &journal)?;
    }

    fs::remove_file(&path).map_err(|error| format!("retire Session recovery failed: {error}"))?;
    Ok(SessionBootstrapResult {
        operation_id,
        channel_id: journal.channel_id,
        message_event_id: message_event.id.to_hex(),
        created_at: message_event.created_at.as_secs() as i64,
        recovered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_are_bounded_and_path_safe() {
        assert!(validate_operation_id("session-12345678").is_ok());
        assert!(validate_operation_id("../../escape").is_err());
        assert!(validate_operation_id("short").is_err());
    }

    #[test]
    fn retry_prepares_the_same_channel_and_events() {
        let keys = nostr::Keys::generate();
        let first = prepared_journal(
            "session-12345678",
            "hello",
            "00000000-0000-0000-0000-000000000001",
            "http://relay",
            &keys,
        )
        .expect("prepare first journal");
        let second = prepared_journal(
            "session-12345678",
            "hello",
            "00000000-0000-0000-0000-000000000001",
            "http://relay",
            &keys,
        )
        .expect("prepare retry journal");
        assert_eq!(first.channel_id, second.channel_id);
        let first_create = Event::from_json(first.create_event_json).expect("parse first create");
        let second_create =
            Event::from_json(second.create_event_json).expect("parse second create");
        let first_message =
            Event::from_json(first.message_event_json).expect("parse first message");
        let second_message =
            Event::from_json(second.message_event_json).expect("parse second message");
        assert_eq!(first_create.id, second_create.id);
        assert_eq!(first_message.id, second_message.id);
    }
}
