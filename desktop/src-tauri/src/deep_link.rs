use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use url::Url;

use crate::nostr_bind;

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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingAgentSnapshotImport {
    id: String,
    file_bytes: Vec<u8>,
    file_name: String,
}

#[derive(Default)]
pub(crate) struct PendingAgentSnapshotImports(Mutex<VecDeque<PendingAgentSnapshotImport>>);

const AGENT_IMPORT_DEEP_LINK_VERSION: &str = "1";

impl PendingAgentSnapshotImports {
    fn enqueue(&self, pending: PendingAgentSnapshotImport) -> Result<(), String> {
        let mut queue = self
            .0
            .lock()
            .map_err(|error| format!("pending agent-import queue poisoned: {error}"))?;
        if queue.iter().any(|item| {
            item.file_name == pending.file_name && item.file_bytes == pending.file_bytes
        }) {
            return Ok(());
        }
        if !queue.is_empty() {
            return Err("another agent snapshot import is already pending".into());
        }
        queue.push_back(pending);
        Ok(())
    }

    fn first(&self) -> Result<Option<PendingAgentSnapshotImport>, String> {
        self.0
            .lock()
            .map_err(|error| format!("pending agent-import queue poisoned: {error}"))
            .map(|queue| queue.front().cloned())
    }

    fn acknowledge(&self, id: &str) -> Result<bool, String> {
        let mut queue = self
            .0
            .lock()
            .map_err(|error| format!("pending agent-import queue poisoned: {error}"))?;
        if queue.front().is_some_and(|item| item.id == id) {
            queue.pop_front();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

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

#[tauri::command]
pub(crate) fn take_pending_agent_snapshot_import(
    pending: State<'_, PendingAgentSnapshotImports>,
) -> Result<Option<PendingAgentSnapshotImport>, String> {
    pending.first()
}

#[tauri::command]
pub(crate) fn acknowledge_pending_agent_snapshot_import(
    id: String,
    pending: State<'_, PendingAgentSnapshotImports>,
) -> Result<bool, String> {
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

fn parse_agent_import_deep_link(url: &Url) -> Result<PathBuf, String> {
    let version = non_empty_param(url, "v")?;
    if version != AGENT_IMPORT_DEEP_LINK_VERSION {
        return Err(format!("unsupported agent-import version: {version}"));
    }
    let file_url = non_empty_param(url, "file")?;
    let parsed = Url::parse(&file_url).map_err(|error| format!("invalid file URL: {error}"))?;
    if parsed.scheme() != "file" {
        return Err("agent-import file must use the file scheme".into());
    }
    let path = parsed
        .to_file_path()
        .map_err(|()| "agent-import file URL is not a local path".to_string())?;
    let lower_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "agent-import filename is not valid UTF-8".to_string())?
        .to_ascii_lowercase();
    if !lower_name.ends_with(".agent.json") && !lower_name.ends_with(".agent.png") {
        return Err("agent-import file must end with .agent.json or .agent.png".into());
    }
    Ok(path)
}

fn load_agent_snapshot_import(path: &Path) -> Result<PendingAgentSnapshotImport, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot read agent snapshot: {error}"))?;
    let metadata = canonical
        .metadata()
        .map_err(|error| format!("cannot inspect agent snapshot: {error}"))?;
    if !metadata.is_file() {
        return Err("agent snapshot path must point to a regular file".into());
    }
    let file_name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "agent snapshot filename is not valid UTF-8".to_string())?
        .to_string();
    let is_png = file_name.to_ascii_lowercase().ends_with(".agent.png");
    let cap = if is_png {
        crate::commands::MAX_SNAPSHOT_PNG_BYTES
    } else {
        crate::commands::MAX_SNAPSHOT_JSON_BYTES
    };
    if metadata.len() > cap as u64 {
        return Err(format!(
            "agent snapshot is too large (maximum {} MiB)",
            cap / (1024 * 1024)
        ));
    }
    let file = std::fs::File::open(&canonical)
        .map_err(|error| format!("cannot read agent snapshot: {error}"))?;
    let mut file_bytes = Vec::with_capacity((metadata.len() as usize).min(cap));
    file.take((cap + 1) as u64)
        .read_to_end(&mut file_bytes)
        .map_err(|error| format!("cannot read agent snapshot: {error}"))?;
    if file_bytes.len() > cap {
        return Err(format!(
            "agent snapshot is too large (maximum {} MiB)",
            cap / (1024 * 1024)
        ));
    }
    crate::commands::decode_snapshot_from_bytes(&file_bytes)
        .map_err(|error| format!("invalid agent snapshot: {error}"))?;

    Ok(PendingAgentSnapshotImport {
        id: uuid::Uuid::new_v4().to_string(),
        file_bytes,
        file_name,
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
/// - `buzz://agent-import?v=1&file=<file://...>` — opens the existing agent import preview
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
        Some("agent-import") => {
            let pending = parse_agent_import_deep_link(&url)
                .and_then(|path| load_agent_snapshot_import(&path));
            match pending {
                Ok(pending) => {
                    let queue = app.state::<PendingAgentSnapshotImports>();
                    if let Err(error) = queue.enqueue(pending) {
                        eprintln!("buzz-desktop: could not queue agent import: {error}");
                        return;
                    }
                    activate_main_window(app);
                    let _ = app.emit("deep-link-agent-import", ());
                }
                Err(error) => {
                    eprintln!("buzz-desktop: rejecting agent-import deep link: {error}: {url_str}");
                }
            }
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
mod tests {
    use url::Url;

    use super::{
        parse_add_community_deep_link, parse_agent_import_deep_link, parse_join_deep_link,
        parse_message_deep_link, parse_nostr_bind_deep_link, PendingAgentSnapshotImport,
        PendingAgentSnapshotImports, PendingCommunityDeepLink, PendingCommunityDeepLinks,
        AGENT_IMPORT_DEEP_LINK_VERSION,
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

    fn pending_agent_import(id: &str, name: &str) -> PendingAgentSnapshotImport {
        PendingAgentSnapshotImport {
            id: id.to_owned(),
            file_bytes: vec![1, 2, 3],
            file_name: name.to_owned(),
        }
    }

    #[test]
    fn pending_agent_import_queue_deduplicates_and_rejects_overlap() {
        let queue = PendingAgentSnapshotImports::default();
        queue
            .enqueue(pending_agent_import("first", "one.agent.json"))
            .unwrap();
        queue
            .enqueue(pending_agent_import("duplicate", "one.agent.json"))
            .expect("identical snapshot should be idempotent");
        assert!(
            queue
                .enqueue(pending_agent_import("second", "two.agent.json"))
                .is_err(),
            "a second preview must not replace the pending import"
        );
        assert_eq!(queue.first().unwrap().unwrap().id, "first");
        assert!(!queue.acknowledge("second").unwrap());
        assert!(queue.acknowledge("first").unwrap());
        assert!(queue.first().unwrap().is_none());
    }

    #[test]
    fn parse_agent_import_deep_link_accepts_local_agent_snapshot() {
        let directory = tempfile::tempdir().expect("temp directory");
        let snapshot_path = directory.path().join("test agent.agent.json");
        let file_url = Url::from_file_path(&snapshot_path).unwrap();
        let mut url = Url::parse("buzz://agent-import").unwrap();
        url.query_pairs_mut()
            .append_pair("v", AGENT_IMPORT_DEEP_LINK_VERSION)
            .append_pair("file", file_url.as_str());
        let parsed = parse_agent_import_deep_link(&url).unwrap();
        assert_eq!(parsed, snapshot_path);
    }

    #[test]
    fn parse_agent_import_deep_link_rejects_remote_wrong_extension_or_version() {
        for (version, file) in [
            ("1", "https://example.com/test.agent.json"),
            ("1", "file:///tmp/test.json"),
            ("2", "file:///tmp/test.agent.json"),
        ] {
            let mut url = Url::parse("buzz://agent-import").unwrap();
            url.query_pairs_mut()
                .append_pair("v", version)
                .append_pair("file", file);
            assert!(parse_agent_import_deep_link(&url).is_err());
        }

        let mut missing_version = Url::parse("buzz://agent-import").unwrap();
        missing_version
            .query_pairs_mut()
            .append_pair("file", "file:///tmp/test.agent.json");
        assert!(parse_agent_import_deep_link(&missing_version).is_err());
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
