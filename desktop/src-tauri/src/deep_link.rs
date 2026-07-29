use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use url::Url;

use crate::{
    decode_snapshot_from_bytes, managed_agents::agent_snapshot::MemoryLevel, nostr_bind,
    MAX_SNAPSHOT_JSON_BYTES,
};

const AGENT_SNAPSHOT_HANDOFF_EVENT: &str = "agent-snapshot-import-available";
const AGENT_SNAPSHOT_HANDOFF_MAX_AGE: Duration = Duration::from_secs(10 * 60);
const AGENT_SNAPSHOT_HANDOFF_FUTURE_SKEW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingAgentSnapshotImport {
    id: String,
    file_bytes: Vec<u8>,
    file_name: String,
    snapshot_kind: String,
}

#[derive(Default)]
pub(crate) struct PendingAgentSnapshotImports(Mutex<VecDeque<PendingAgentSnapshotImport>>);

impl PendingAgentSnapshotImports {
    fn enqueue(&self, pending: PendingAgentSnapshotImport) {
        let mut queue = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if queue.iter().any(|item| item.id == pending.id) {
            return;
        }
        queue.push_back(pending);
    }

    fn first(&self) -> Option<PendingAgentSnapshotImport> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .front()
            .cloned()
    }

    fn acknowledge(&self, id: &str) -> bool {
        let mut queue = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if queue.front().is_some_and(|item| item.id == id) {
            queue.pop_front();
            true
        } else {
            false
        }
    }
}

#[tauri::command]
pub(crate) fn take_pending_agent_snapshot_import(
    pending: State<'_, PendingAgentSnapshotImports>,
) -> Option<PendingAgentSnapshotImport> {
    pending.first()
}

#[tauri::command]
pub(crate) fn acknowledge_pending_agent_snapshot_import(
    id: String,
    app: tauri::AppHandle,
    pending: State<'_, PendingAgentSnapshotImports>,
) -> bool {
    let acknowledged = pending.acknowledge(&id);
    if acknowledged && pending.first().is_some() {
        let _ = app.emit(AGENT_SNAPSHOT_HANDOFF_EVENT, ());
    }
    acknowledged
}

fn parse_agent_snapshot_handoff_id(url: &Url) -> Option<String> {
    let params = url.query_pairs().collect::<Vec<_>>();
    if params.len() != 1 || params[0].0 != "handoff" {
        return None;
    }
    let candidate = params[0].1.as_ref();
    let parsed = uuid::Uuid::parse_str(candidate).ok()?;
    (parsed.to_string() == candidate).then(|| candidate.to_owned())
}

fn agent_snapshot_handoff_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Co-Agent")
                .join("buzz-handoffs")
        })
        .ok_or_else(|| "cannot resolve the current user's home directory".to_string())
}

#[cfg(unix)]
fn validate_agent_snapshot_handoff_metadata(
    metadata: &std::fs::Metadata,
    expected_uid: u32,
    now: SystemTime,
) -> Result<(), String> {
    if !metadata.file_type().is_file() {
        return Err("handoff is not a regular file".to_string());
    }
    if metadata.uid() != expected_uid {
        return Err("handoff is not owned by the current user".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        return Err("handoff has group or world permissions".to_string());
    }
    if metadata.len() > MAX_SNAPSHOT_JSON_BYTES as u64 {
        return Err("handoff exceeds the agent JSON snapshot size limit".to_string());
    }
    let modified = metadata
        .modified()
        .map_err(|error| format!("cannot inspect handoff age: {error}"))?;
    match now.duration_since(modified) {
        Ok(age) if age > AGENT_SNAPSHOT_HANDOFF_MAX_AGE => {
            return Err("handoff has expired".to_string());
        }
        Ok(_) => {}
        Err(error) if error.duration() > AGENT_SNAPSHOT_HANDOFF_FUTURE_SKEW => {
            return Err("handoff modification time is too far in the future".to_string());
        }
        Err(_) => {}
    }
    Ok(())
}

#[cfg(unix)]
fn validate_agent_snapshot_handoff_directory_metadata(
    metadata: &std::fs::Metadata,
    expected_uid: u32,
) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("handoff directory is not a real directory".to_string());
    }
    if metadata.uid() != expected_uid {
        return Err("handoff directory is not owned by the current user".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        return Err("handoff directory has group or world permissions".to_string());
    }
    Ok(())
}

fn validate_agent_snapshot_handoff_shape(bytes: &[u8]) -> Result<(), String> {
    fn require_only_keys(
        value: &serde_json::Value,
        allowed: &[&str],
        section: &str,
    ) -> Result<(), String> {
        let object = value
            .as_object()
            .ok_or_else(|| format!("handoff {section} must be a JSON object"))?;
        if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(format!("handoff {section} contains forbidden field {key}"));
        }
        Ok(())
    }

    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid agent snapshot handoff JSON: {error}"))?;
    require_only_keys(
        &value,
        &["format", "version", "definition", "profile", "memory"],
        "root",
    )?;
    let root = value
        .as_object()
        .ok_or_else(|| "handoff root must be a JSON object".to_string())?;
    for (section, allowed) in [
        ("definition", &["name", "systemPrompt", "runtime"][..]),
        ("profile", &["displayName"][..]),
        ("memory", &["level"][..]),
    ] {
        let child = root
            .get(section)
            .ok_or_else(|| format!("handoff is missing {section}"))?;
        require_only_keys(child, allowed, section)?;
    }
    Ok(())
}

#[cfg(unix)]
fn read_agent_snapshot_handoff_from_dir(dir: &Path, handoff_id: &str) -> Result<Vec<u8>, String> {
    let parsed =
        uuid::Uuid::parse_str(handoff_id).map_err(|_| "handoff id is not a UUID".to_string())?;
    if parsed.to_string() != handoff_id {
        return Err("handoff id is not a canonical lowercase UUID".to_string());
    }

    let expected_uid = dirs::home_dir()
        .ok_or_else(|| "cannot resolve the current user's home directory".to_string())?
        .metadata()
        .map_err(|error| format!("cannot inspect the current user's home directory: {error}"))?
        .uid();
    let dir_metadata = std::fs::symlink_metadata(dir)
        .map_err(|error| format!("cannot inspect handoff directory: {error}"))?;
    validate_agent_snapshot_handoff_directory_metadata(&dir_metadata, expected_uid)?;

    let path = dir.join(format!("{handoff_id}.agent.json"));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| format!("cannot securely open handoff: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect open handoff: {error}"))?;
    validate_agent_snapshot_handoff_metadata(&opened_metadata, expected_uid, SystemTime::now())?;

    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.by_ref()
        .take(MAX_SNAPSHOT_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read handoff: {error}"))?;
    if bytes.len() > MAX_SNAPSHOT_JSON_BYTES {
        return Err("handoff exceeds the agent JSON snapshot size limit".to_string());
    }

    validate_agent_snapshot_handoff_shape(&bytes)?;
    let snapshot = decode_snapshot_from_bytes(&bytes)
        .map_err(|error| format!("invalid agent snapshot handoff: {error}"))?;
    if snapshot.definition.runtime.as_deref() != Some("hermes") {
        return Err("agent snapshot handoff runtime must be hermes".to_string());
    }
    if snapshot.memory.level != MemoryLevel::None || !snapshot.memory.entries.is_empty() {
        return Err("agent snapshot handoff must be config-only".to_string());
    }

    let current_metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot re-inspect handoff before deletion: {error}"))?;
    if current_metadata.file_type().is_symlink()
        || current_metadata.dev() != opened_metadata.dev()
        || current_metadata.ino() != opened_metadata.ino()
    {
        return Err("handoff changed while it was being read".to_string());
    }
    std::fs::remove_file(&path)
        .map_err(|error| format!("cannot delete accepted handoff: {error}"))?;
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_agent_snapshot_handoff_from_dir(_dir: &Path, _handoff_id: &str) -> Result<Vec<u8>, String> {
    Err("agent snapshot handoffs require POSIX file security".to_string())
}

fn queue_agent_snapshot_handoff(app: &tauri::AppHandle, handoff_id: String) -> Result<(), String> {
    let bytes = read_agent_snapshot_handoff_from_dir(&agent_snapshot_handoff_dir()?, &handoff_id)?;
    app.state::<PendingAgentSnapshotImports>()
        .enqueue(PendingAgentSnapshotImport {
            file_name: format!("{handoff_id}.agent.json"),
            id: handoff_id,
            file_bytes: bytes,
            snapshot_kind: "agent".to_string(),
        });
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingCommunityDeepLink {
    id: String,
    kind: String,
    relay_url: String,
    code: Option<String>,
    policy_receipt: Option<String>,
    name: Option<String>,
}

#[derive(Default)]
pub(crate) struct PendingCommunityDeepLinks(Mutex<VecDeque<PendingCommunityDeepLink>>);

impl PendingCommunityDeepLinks {
    fn enqueue(&self, pending: PendingCommunityDeepLink) {
        let mut queue = self.0.lock().expect("pending deep-link queue poisoned");
        if queue.iter().any(|item| {
            item.kind == pending.kind
                && item.relay_url == pending.relay_url
                && item.code == pending.code
                && item.policy_receipt == pending.policy_receipt
                && item.name == pending.name
        }) {
            return;
        }
        queue.push_back(pending);
    }

    fn first(&self) -> Option<PendingCommunityDeepLink> {
        self.0
            .lock()
            .expect("pending deep-link queue poisoned")
            .front()
            .cloned()
    }

    fn acknowledge(&self, id: &str) -> bool {
        let mut queue = self.0.lock().expect("pending deep-link queue poisoned");
        if queue.front().is_some_and(|item| item.id == id) {
            queue.pop_front();
            true
        } else {
            false
        }
    }
}

#[tauri::command]
pub(crate) fn take_pending_community_deep_link(
    pending: State<'_, PendingCommunityDeepLinks>,
) -> Option<PendingCommunityDeepLink> {
    pending.first()
}

#[tauri::command]
pub(crate) fn acknowledge_pending_community_deep_link(
    id: String,
    pending: State<'_, PendingCommunityDeepLinks>,
) -> bool {
    pending.acknowledge(&id)
}

fn queue_community_deep_link(
    app: &tauri::AppHandle,
    kind: &str,
    relay_url: String,
    code: Option<String>,
    policy_receipt: Option<String>,
    name: Option<String>,
) {
    app.state::<PendingCommunityDeepLinks>()
        .enqueue(PendingCommunityDeepLink {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.to_owned(),
            relay_url,
            code,
            policy_receipt,
            name,
        });
}

fn activate_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if let Err(error) = window.unminimize() {
        eprintln!("buzz-desktop: failed to unminimize main window for deep link: {error}");
    }
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to show main window for deep link: {error}");
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window for deep link: {error}");
    }
}

/// Parse the query string of a `buzz://message?…` URL into the JSON
/// payload emitted on `deep-link-message`. Returns `None` when a required
/// param (`channel`, `id`) is missing or empty — mirroring the validation
/// policy of the `connect` arm so the frontend never sees a half-formed
/// payload (e.g. `channelId: ""` from `channel=&id=foo`).
///
/// Pulled out of `handle_deep_link_url` so it can be unit-tested without
/// a live `tauri::AppHandle`.
fn parse_message_deep_link(url: &Url) -> Option<serde_json::Value> {
    let mut channel: Option<String> = None;
    let mut message_id: Option<String> = None;
    let mut thread: Option<String> = None;
    for (k, v) in url.query_pairs() {
        let v = v.into_owned();
        if v.is_empty() {
            continue;
        }
        match k.as_ref() {
            "channel" => channel = Some(v),
            "id" => message_id = Some(v),
            "thread" => thread = Some(v),
            _ => {}
        }
    }
    let (channel_id, message_id) = (channel?, message_id?);
    Some(serde_json::json!({
        "channelId": channel_id,
        "messageId": message_id,
        "threadRootId": thread,
    }))
}

/// Parse the query string of a `buzz://join?…` URL into the JSON payload
/// emitted on `deep-link-join`. Requires a ws(s) `relay` URL and a non-empty
/// `code`; returns `None` otherwise so the frontend never sees a half-formed
/// payload.
fn parse_join_deep_link(url: &Url) -> Option<serde_json::Value> {
    let mut code: Option<String> = None;
    let mut policy_receipt: Option<String> = None;
    for (k, v) in url.query_pairs() {
        let v = v.into_owned();
        if v.is_empty() {
            continue;
        }
        match k.as_ref() {
            "code" => code = Some(v),
            "policy_receipt" => policy_receipt = Some(v),
            _ => {}
        }
    }
    let code = code?;
    let relay_url = parse_websocket_relay_param(url)?;
    Some(serde_json::json!({
        "relayUrl": relay_url,
        "code": code,
        "policyReceipt": policy_receipt,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AddCommunityDeepLinkPayload {
    relay_url: String,
    name: Option<String>,
}

fn parse_websocket_relay_param(url: &Url) -> Option<String> {
    let relay_url = url
        .query_pairs()
        .find(|(key, _)| key == "relay")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())?;
    let parsed = Url::parse(&relay_url).ok()?;
    if !matches!(parsed.scheme(), "ws" | "wss") || parsed.host_str().is_none() {
        return None;
    }
    Some(relay_url)
}

fn parse_add_community_deep_link(url: &Url) -> Option<AddCommunityDeepLinkPayload> {
    Some(AddCommunityDeepLinkPayload {
        relay_url: parse_websocket_relay_param(url)?,
        name: optional_non_empty_param(url, "name"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct NostrBindDeepLinkPayload {
    challenge_id: String,
    nonce: String,
    verification_code: String,
    audience: String,
    action: String,
    protocol: String,
    version: String,
    origin: String,
    expires_at: String,
    return_mode: String,
    callback_url: Option<String>,
}

fn non_empty_param(url: &Url, name: &str) -> Result<String, String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing {name}"))
}

fn optional_non_empty_param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty())
}

fn validate_nostr_bind_callback_url(callback_url: &str, origin: &str) -> Result<(), String> {
    let callback =
        Url::parse(callback_url).map_err(|error| format!("invalid callback_url: {error}"))?;
    let origin = Url::parse(origin).map_err(|error| format!("invalid origin: {error}"))?;
    if callback.scheme() != "https" {
        return Err("callback_url must use https".into());
    }
    if callback.host_str().is_none() {
        return Err("callback_url missing host".into());
    }
    if !callback.username().is_empty() || callback.password().is_some() {
        return Err("callback_url must not include credentials".into());
    }
    if callback.scheme() != origin.scheme()
        || callback.host_str() != origin.host_str()
        || callback.port_or_known_default() != origin.port_or_known_default()
    {
        return Err("callback_url must match origin".into());
    }
    Ok(())
}

fn parse_nostr_bind_deep_link(url: &Url) -> Result<NostrBindDeepLinkPayload, String> {
    let challenge_id = non_empty_param(url, "challenge_id")?;
    let nonce = non_empty_param(url, "nonce")?;
    let verification_code = non_empty_param(url, "verification_code")?;
    let audience = non_empty_param(url, "audience")?;
    let action = non_empty_param(url, "action")?;
    let protocol = non_empty_param(url, "protocol")?;
    let version = non_empty_param(url, "version")?;
    let origin = non_empty_param(url, "origin")?;
    let expires_at = non_empty_param(url, "expires_at")?;
    let return_mode = non_empty_param(url, "return")?;
    let callback_url = optional_non_empty_param(url, "callback_url");

    nostr_bind::validate_challenge_id(&challenge_id)?;
    nostr_bind::validate_nonce(&nonce)?;
    nostr_bind::validate_verification_code(&verification_code)?;
    nostr_bind::validate_protocol_fields(&audience, &action, &protocol, &version)?;
    nostr_bind::validate_origin(&origin)?;
    // Expired links still reach the consent surface so the user gets an explicit
    // failure instead of a silent stderr-only rejection from a launched app.
    nostr_bind::validate_expires_at_format(&expires_at)?;
    match return_mode.as_str() {
        nostr_bind::RETURN_MODE_CLIPBOARD => {}
        nostr_bind::RETURN_MODE_BROWSER_FRAGMENT_V1 if callback_url.is_some() => {}
        nostr_bind::RETURN_MODE_BROWSER_FRAGMENT_V1 => {
            return Err("browser_fragment_v1 requires callback_url".into());
        }
        _ => return Err("unsupported return mode".into()),
    }
    if let Some(callback_url) = callback_url.as_deref() {
        validate_nostr_bind_callback_url(callback_url, &origin)?;
    }

    Ok(NostrBindDeepLinkPayload {
        challenge_id,
        nonce,
        verification_code,
        audience,
        action,
        protocol,
        version,
        origin,
        expires_at,
        return_mode,
        callback_url,
    })
}

/// Handle an incoming `buzz://` deep link URL.
///
/// Currently supports:
/// - `buzz://connect?relay=<ws(s)://...>` — emits `deep-link-connect` to the frontend
pub(crate) fn handle_deep_link_url(app: &tauri::AppHandle, url_str: &str) {
    let url = match Url::parse(url_str) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("buzz-desktop: invalid deep link URL {url_str:?}: {e}");
            return;
        }
    };

    if url.scheme() != "buzz" {
        eprintln!("buzz-desktop: ignoring unsupported deep link scheme: {url_str}");
        return;
    }

    match url.host_str() {
        Some("import-agent-snapshot") => {
            let Some(handoff_id) = parse_agent_snapshot_handoff_id(&url) else {
                eprintln!("buzz-desktop: snapshot handoff deep link has an invalid id: {url_str}");
                return;
            };
            if let Err(error) = queue_agent_snapshot_handoff(app, handoff_id) {
                eprintln!("buzz-desktop: rejecting agent snapshot handoff: {error}");
                return;
            }
            activate_main_window(app);
            let _ = app.emit(AGENT_SNAPSHOT_HANDOFF_EVENT, ());
        }
        Some("connect") => {
            let Some(relay_url) = parse_websocket_relay_param(&url) else {
                eprintln!("buzz-desktop: connect deep link missing/invalid relay: {url_str}");
                return;
            };
            activate_main_window(app);
            queue_community_deep_link(app, "connect", relay_url.clone(), None, None, None);
            let _ = app.emit("deep-link-connect", relay_url);
        }
        Some("join") => {
            // `buzz://join?relay=<ws(s)://...>&code=<invite code>` — fired by
            // the relay's /invite/<code> landing page. The frontend claims the
            // invite against the relay's HTTP API, then adds the workspace.
            let Some(payload) = parse_join_deep_link(&url) else {
                eprintln!("buzz-desktop: join deep link missing/invalid relay or code: {url_str}");
                return;
            };
            activate_main_window(app);
            let relay_url = payload["relayUrl"].as_str().unwrap_or_default().to_owned();
            let code = payload["code"].as_str().map(str::to_owned);
            let policy_receipt = payload["policyReceipt"].as_str().map(str::to_owned);
            queue_community_deep_link(app, "join", relay_url, code, policy_receipt, None);
            let _ = app.emit("deep-link-join", payload);
        }
        Some("add-community") => {
            let Some(payload) = parse_add_community_deep_link(&url) else {
                eprintln!("buzz-desktop: add-community deep link missing/invalid relay: {url_str}");
                return;
            };
            activate_main_window(app);
            queue_community_deep_link(
                app,
                "add-community",
                payload.relay_url.clone(),
                None,
                None,
                payload.name.clone(),
            );
            let _ = app.emit("deep-link-add-community", payload);
        }
        Some("message") => {
            // `buzz://message?channel=<uuid>&id=<eventId>[&thread=<rootId>]`
            //
            // Validation policy mirrors the `connect` arm: parse what we
            // need, refuse to emit anything if a required param is missing
            // so the frontend never sees a half-formed payload. The
            // frontend listener mirrors `parseMessageLink` in TS — we keep
            // structure on this side (serde JSON) and let the TS code own
            // any further normalisation.
            let Some(payload) = parse_message_deep_link(&url) else {
                eprintln!("buzz-desktop: message deep link missing channel or id: {url_str}");
                return;
            };
            activate_main_window(app);
            let _ = app.emit("deep-link-message", payload);
        }
        Some("nostr-bind") => match parse_nostr_bind_deep_link(&url) {
            Ok(payload) => {
                activate_main_window(app);
                let _ = app.emit("deep-link-nostr-bind", payload);
            }
            Err(error) => {
                eprintln!("buzz-desktop: rejecting nostr-bind deep link: {error}: {url_str}");
            }
        },
        Some(action) => {
            eprintln!("buzz-desktop: unknown deep link action: {action}");
        }
        None => {
            eprintln!("buzz-desktop: deep link missing action: {url_str}");
        }
    }
}

#[cfg(test)]
#[path = "deep_link/agent_snapshot_handoff.rs"]
mod agent_snapshot_handoff_tests;

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{
        parse_add_community_deep_link, parse_join_deep_link, parse_message_deep_link,
        parse_nostr_bind_deep_link, PendingCommunityDeepLink, PendingCommunityDeepLinks,
    };

    fn pending(id: &str, relay_url: &str, code: Option<&str>) -> PendingCommunityDeepLink {
        PendingCommunityDeepLink {
            id: id.to_owned(),
            kind: if code.is_some() { "join" } else { "connect" }.to_owned(),
            relay_url: relay_url.to_owned(),
            code: code.map(str::to_owned),
            policy_receipt: None,
            name: None,
        }
    }

    #[test]
    fn pending_join_serializes_policy_receipt_for_cold_launch_recovery() {
        let mut link = pending("join", "wss://relay.example", Some("invite"));
        link.policy_receipt = Some("relay-signed-receipt".to_owned());

        let payload = serde_json::to_value(link).unwrap();
        assert_eq!(payload["policyReceipt"], "relay-signed-receipt");
    }

    #[test]
    fn pending_community_links_are_fifo_and_acknowledged_in_order() {
        let queue = PendingCommunityDeepLinks::default();
        queue.enqueue(pending("first", "wss://one.example", Some("one")));
        queue.enqueue(pending("second", "wss://two.example", Some("two")));
        assert_eq!(queue.first().unwrap().id, "first");
        assert!(!queue.acknowledge("second"));
        assert!(queue.acknowledge("first"));
        assert_eq!(queue.first().unwrap().id, "second");
    }

    #[test]
    fn pending_community_links_dedupe_exact_intents() {
        let queue = PendingCommunityDeepLinks::default();
        queue.enqueue(pending("first", "wss://one.example", Some("one")));
        queue.enqueue(pending("duplicate", "wss://one.example", Some("one")));
        assert!(queue.acknowledge("first"));
        assert!(queue.first().is_none());
    }

    fn valid_nostr_bind_url() -> Url {
        Url::parse(
            "buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard",
        )
        .unwrap()
    }

    #[test]
    fn parse_add_community_deep_link_extracts_relay_and_name() {
        let url = Url::parse(
            "buzz://add-community?relay=wss%3A%2F%2Facme.communities.buzz.xyz&name=Acme%20Team&ignored=value",
        )
        .unwrap();
        let payload = parse_add_community_deep_link(&url).unwrap();
        assert_eq!(payload.relay_url, "wss://acme.communities.buzz.xyz");
        assert_eq!(payload.name.as_deref(), Some("Acme Team"));
    }

    #[test]
    fn parse_add_community_deep_link_accepts_an_omitted_or_empty_name() {
        for raw in [
            "buzz://add-community?relay=wss%3A%2F%2Facme.example",
            "buzz://add-community?relay=wss%3A%2F%2Facme.example&name=",
        ] {
            assert!(parse_add_community_deep_link(&Url::parse(raw).unwrap())
                .unwrap()
                .name
                .is_none());
        }
    }

    #[test]
    fn parse_add_community_deep_link_rejects_invalid_relays() {
        for raw in [
            "buzz://add-community",
            "buzz://add-community?relay=",
            "buzz://add-community?relay=not-a-url",
            "buzz://add-community?relay=https%3A%2F%2Facme.example",
            "buzz://add-community?relay=wss%3A%2F%2F",
        ] {
            assert!(parse_add_community_deep_link(&Url::parse(raw).unwrap()).is_none());
        }
    }

    #[test]
    fn parse_message_deep_link_extracts_required_params() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["channelId"], "abc");
        assert_eq!(payload["messageId"], "xyz");
        assert!(payload["threadRootId"].is_null());
    }

    #[test]
    fn parse_message_deep_link_accepts_buzz_scheme() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["channelId"], "abc");
        assert_eq!(payload["messageId"], "xyz");
    }

    #[test]
    fn parse_message_deep_link_includes_thread_root() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz&thread=root1").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["threadRootId"], "root1");
    }

    #[test]
    fn parse_message_deep_link_rejects_missing_id() {
        let url = Url::parse("buzz://message?channel=abc").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_rejects_empty_channel() {
        // Regression: `channel=&id=foo` previously produced channelId: "".
        let url = Url::parse("buzz://message?channel=&id=foo").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_rejects_empty_id() {
        let url = Url::parse("buzz://message?channel=abc&id=").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_treats_empty_thread_as_absent() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz&thread=").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert!(payload["threadRootId"].is_null());
    }

    #[test]
    fn parse_join_deep_link_extracts_relay_and_code() {
        let url = Url::parse("buzz://join?relay=wss%3A%2F%2Frelay.example&code=abc.def").unwrap();
        let payload = parse_join_deep_link(&url).expect("required params present");
        assert_eq!(payload["relayUrl"], "wss://relay.example");
        assert_eq!(payload["code"], "abc.def");
        assert!(payload["policyReceipt"].is_null());
    }

    #[test]
    fn parse_join_deep_link_extracts_policy_receipt() {
        let url = Url::parse(
            "buzz://join?relay=wss%3A%2F%2Frelay.example&code=abc.def&policy_receipt=receipt.value",
        )
        .unwrap();
        let payload = parse_join_deep_link(&url).expect("required params present");
        assert_eq!(payload["policyReceipt"], "receipt.value");
    }

    #[test]
    fn parse_join_deep_link_rejects_missing_code() {
        let url = Url::parse("buzz://join?relay=wss%3A%2F%2Frelay.example").unwrap();
        assert!(parse_join_deep_link(&url).is_none());
    }

    #[test]
    fn parse_join_deep_link_rejects_empty_code() {
        let url = Url::parse("buzz://join?relay=wss%3A%2F%2Frelay.example&code=").unwrap();
        assert!(parse_join_deep_link(&url).is_none());
    }

    #[test]
    fn parse_join_deep_link_rejects_missing_relay() {
        let url = Url::parse("buzz://join?code=abc.def").unwrap();
        assert!(parse_join_deep_link(&url).is_none());
    }

    #[test]
    fn parse_join_deep_link_rejects_non_websocket_relay() {
        let url = Url::parse("buzz://join?relay=https%3A%2F%2Frelay.example&code=abc.def").unwrap();
        assert!(parse_join_deep_link(&url).is_none());
    }

    #[test]
    fn parse_nostr_bind_deep_link_accepts_valid_url() {
        let payload = parse_nostr_bind_deep_link(&valid_nostr_bind_url()).unwrap();
        assert_eq!(payload.challenge_id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(payload.nonce, "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567");
        assert_eq!(payload.verification_code, "123456");
        assert_eq!(payload.audience, "buzz:nostr-identity");
        assert_eq!(payload.action, "bind_nostr_identity");
        assert_eq!(payload.protocol, "buzz-nostr-identity");
        assert_eq!(payload.version, "1");
        assert_eq!(payload.origin, "https://example.com");
        assert_eq!(payload.expires_at, "2999-01-01T00:00:00Z");
        assert_eq!(payload.return_mode, "clipboard");
        assert_eq!(payload.callback_url, None);
    }

    #[test]
    fn parse_nostr_bind_deep_link_accepts_same_origin_callback_url() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard&callback_url=https%3A%2F%2Fexample.com%2Fbuzz%3FmockSession%3D1").unwrap();
        let payload = parse_nostr_bind_deep_link(&url).unwrap();
        assert_eq!(
            payload.callback_url.as_deref(),
            Some("https://example.com/buzz?mockSession=1")
        );
    }

    #[test]
    fn parse_nostr_bind_deep_link_accepts_browser_fragment_return() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=browser_fragment_v1&callback_url=https%3A%2F%2Fexample.com%2Fbuzz").unwrap();
        let payload = parse_nostr_bind_deep_link(&url).unwrap();

        assert_eq!(payload.return_mode, "browser_fragment_v1");
        assert_eq!(
            payload.callback_url.as_deref(),
            Some("https://example.com/buzz")
        );
    }

    #[test]
    fn parse_nostr_bind_deep_link_requires_callback_for_browser_fragment_return() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=browser_fragment_v1").unwrap();

        assert_eq!(
            parse_nostr_bind_deep_link(&url).unwrap_err(),
            "browser_fragment_v1 requires callback_url"
        );
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_cross_origin_callback_url() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard&callback_url=https%3A%2F%2Fevil.example%2Fbuzz").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_http_callback_url() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard&callback_url=http%3A%2F%2Fexample.com%2Fbuzz").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_missing_challenge_id() {
        let url = Url::parse("buzz://nostr-bind?nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_empty_nonce() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_missing_verification_code() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_short_verification_code() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=12345&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_long_verification_code() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=1234567&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_non_digit_verification_code() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=12345a&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_wrong_action() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=wrong&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_wrong_audience() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=other&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_non_https_origin() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=http%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_origin_with_path() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com%2Fbind&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_origin_with_credentials() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fuser%40example.com&expires_at=2999-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_rejects_unsupported_return_mode() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2999-01-01T00%3A00%3A00Z&return=callback").unwrap();
        assert!(parse_nostr_bind_deep_link(&url).is_err());
    }

    #[test]
    fn parse_nostr_bind_deep_link_accepts_expired_link_for_user_facing_error() {
        let url = Url::parse("buzz://nostr-bind?challenge_id=550e8400-e29b-41d4-a716-446655440000&nonce=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567&verification_code=123456&audience=buzz%3Anostr-identity&action=bind_nostr_identity&protocol=buzz-nostr-identity&version=1&origin=https%3A%2F%2Fexample.com&expires_at=2000-01-01T00%3A00%3A00Z&return=clipboard").unwrap();
        let payload = parse_nostr_bind_deep_link(&url).unwrap();
        assert_eq!(payload.expires_at, "2000-01-01T00:00:00Z");
    }
}
