use std::{collections::VecDeque, sync::Mutex};

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
pub(crate) struct PendingNavigationDeepLink {
    id: String,
    kind: String,
    channel_id: String,
    message_id: Option<String>,
    thread_root_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct PendingNavigationDeepLinks(Mutex<VecDeque<PendingNavigationDeepLink>>);

impl PendingNavigationDeepLinks {
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<PendingNavigationDeepLink>> {
        self.0.lock().unwrap_or_else(|poisoned| {
            eprintln!("buzz-desktop: recovering poisoned pending navigation deep-link queue");
            poisoned.into_inner()
        })
    }

    fn enqueue(&self, pending: PendingNavigationDeepLink) {
        let mut queue = self.lock();
        if queue.iter().any(|item| {
            item.kind == pending.kind
                && item.channel_id == pending.channel_id
                && item.message_id == pending.message_id
                && item.thread_root_id == pending.thread_root_id
        }) {
            return;
        }
        queue.push_back(pending);
    }

    fn clear(&self) {
        self.lock().clear();
    }

    fn first(&self) -> Option<PendingNavigationDeepLink> {
        self.lock().front().cloned()
    }

    fn acknowledge(&self, id: &str) -> bool {
        let mut queue = self.lock();
        if queue.front().is_some_and(|item| item.id == id) {
            queue.pop_front();
            true
        } else {
            false
        }
    }
}

#[tauri::command]
pub(crate) fn clear_pending_navigation_deep_links(pending: State<'_, PendingNavigationDeepLinks>) {
    pending.clear();
}

#[tauri::command]
pub(crate) fn take_pending_navigation_deep_link(
    pending: State<'_, PendingNavigationDeepLinks>,
) -> Option<PendingNavigationDeepLink> {
    pending.first()
}

#[tauri::command]
pub(crate) fn acknowledge_pending_navigation_deep_link(
    id: String,
    pending: State<'_, PendingNavigationDeepLinks>,
) -> bool {
    pending.acknowledge(&id)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingEntityDeepLink {
    id: String,
    href: String,
}

#[derive(Default)]
pub(crate) struct PendingEntityDeepLinks(Mutex<VecDeque<PendingEntityDeepLink>>);

impl PendingEntityDeepLinks {
    fn enqueue(&self, href: String) -> PendingEntityDeepLink {
        let mut queue = self.0.lock().expect("pending deep-link queue poisoned");
        if let Some(existing) = queue.iter().find(|item| item.href == href) {
            return existing.clone();
        }
        let pending = PendingEntityDeepLink {
            id: uuid::Uuid::new_v4().to_string(),
            href,
        };
        queue.push_back(pending.clone());
        pending
    }

    fn first(&self) -> Option<PendingEntityDeepLink> {
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
pub(crate) fn take_pending_entity_deep_link(
    pending: State<'_, PendingEntityDeepLinks>,
) -> Option<PendingEntityDeepLink> {
    pending.first()
}

#[tauri::command]
pub(crate) fn acknowledge_pending_entity_deep_link(
    id: String,
    pending: State<'_, PendingEntityDeepLinks>,
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

fn queue_navigation_deep_link(app: &tauri::AppHandle, kind: &str, payload: &serde_json::Value) {
    let Some(channel_id) = payload["channelId"].as_str() else {
        return;
    };
    app.state::<PendingNavigationDeepLinks>()
        .enqueue(PendingNavigationDeepLink {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.to_owned(),
            channel_id: channel_id.to_owned(),
            message_id: payload["messageId"].as_str().map(str::to_owned),
            thread_root_id: payload["threadRootId"].as_str().map(str::to_owned),
        });
}

fn queue_entity_deep_link(app: &tauri::AppHandle, href: String) -> PendingEntityDeepLink {
    app.state::<PendingEntityDeepLinks>().enqueue(href)
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

fn parse_channel_deep_link(url: &Url) -> Option<serde_json::Value> {
    if url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    let channel_id = segments.next()?;
    let message_id = segments.next();
    if segments.next().is_some() {
        return None;
    }
    let channel_id = uuid::Uuid::parse_str(channel_id).ok()?.to_string();
    if message_id.is_some_and(|value| {
        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return None;
    }
    Some(match message_id {
        Some(message_id) => serde_json::json!({
            "channelId": channel_id,
            "messageId": message_id.to_ascii_lowercase(),
        }),
        None => serde_json::json!({ "channelId": channel_id }),
    })
}

#[cfg(desktop)]
pub(crate) fn install_deep_link_handlers(app: &mut tauri::App) {
    use tauri_plugin_deep_link::DeepLinkExt;

    let dl_handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            handle_deep_link_url(&dl_handle, url.as_str());
        }
    });

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    match app.deep_link().get_current() {
        Ok(Some(urls)) => {
            for url in urls {
                handle_deep_link_url(app.handle(), url.as_str());
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!("buzz-desktop: failed to read launch deep link: {error}"),
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

/// Hosts of the `buzz://` git-entity links built by
/// `desktop/src/shared/lib/entityLink.ts` and `crates/buzz-cli/src/links.rs`.
const ENTITY_LINK_HOSTS: [&str; 4] = ["repo", "project", "pr", "issue"];

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Mirrors `isValidDtag` in `entityLink.ts` — the link format addresses a
/// narrower d-tag charset than Nostr allows.
fn is_linkable_dtag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !value.starts_with('.')
        && !value.contains("..")
}

/// Validate a `buzz://repo|project|pr|issue?…` link and return it verbatim
/// for the frontend, which re-parses it with `parseEntityLink` before
/// navigating. Validating here too keeps a malformed link from raising and
/// focusing the window for a navigation that would then be declined.
///
/// Workspace tabs addressable by `buzz://repo|project` links — mirrors
/// `ENTITY_LINK_TABS` in `entityLink.ts`.
const ENTITY_LINK_TABS: [&str; 6] = [
    "files",
    "commits",
    "issues",
    "prs",
    "contributors",
    "channels",
];

/// Validate the build-specific transport URL, then hand the frontend its
/// canonical entity-link representation. Never broaden frontend scheme trust.
fn canonical_entity_deep_link(url: &Url, build_scheme: &str) -> Option<String> {
    if url.scheme() != build_scheme {
        return None;
    }
    parse_entity_deep_link(url)?;
    let mut canonical = url.clone();
    canonical.set_scheme("buzz").ok()?;
    Some(canonical.into())
}

/// The canonical-form rules match `parseEntityLink`: no path segments, no
/// fragment, and no parameters beyond `owner`/`d` (plus `id` for event
/// links and the optional `tab` for coordinate links), so a future
/// extension of the format is declined by old builds rather than silently
/// misread.
fn parse_entity_deep_link(url: &Url) -> Option<()> {
    let host = url.host_str()?;
    if !ENTITY_LINK_HOSTS.contains(&host) {
        return None;
    }
    if !matches!(url.path(), "" | "/") || url.fragment().is_some() {
        return None;
    }

    let needs_event_id = host == "pr" || host == "issue";
    let allows_tab = host == "repo" || host == "project";
    let (mut owner, mut dtag, mut id, mut tab, mut commit) = (None, None, None, None, None);
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "owner" => &mut owner,
            "d" => &mut dtag,
            "id" if needs_event_id => &mut id,
            "tab" if allows_tab => &mut tab,
            "commit" if host == "repo" => &mut commit,
            _ => return None,
        };
        if slot.is_some() {
            return None;
        }
        *slot = Some(value.into_owned());
    }

    if !owner.is_some_and(|owner| is_hex64(&owner)) {
        return None;
    }
    if !dtag.is_some_and(|dtag| is_linkable_dtag(&dtag)) {
        return None;
    }
    if needs_event_id && !id.is_some_and(|id| is_hex64(&id)) {
        return None;
    }
    if let Some(tab) = tab.as_deref() {
        if !ENTITY_LINK_TABS.contains(&tab) {
            return None;
        }
    }
    if let Some(commit) = commit {
        if tab.as_deref() != Some("commits") || !is_git_object_id(&commit) {
            return None;
        }
    }
    Some(())
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
/// - `buzz://repo|project|pr|issue?…` — emits `deep-link-entity` to the frontend
pub(crate) fn handle_deep_link_url(app: &tauri::AppHandle, url_str: &str) {
    let url = match Url::parse(url_str) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("buzz-desktop: invalid deep link URL {url_str:?}: {e}");
            return;
        }
    };

    if url.scheme() != crate::build_identity::deep_link_scheme() {
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
        Some("channel") => {
            let Some(payload) = parse_channel_deep_link(&url) else {
                eprintln!("buzz-desktop: channel deep link missing/invalid channel: {url_str}");
                return;
            };
            activate_main_window(app);
            if payload["messageId"].is_string() {
                queue_navigation_deep_link(app, "message", &payload);
                let _ = app.emit("deep-link-message", payload);
            } else {
                queue_navigation_deep_link(app, "channel", &payload);
                let _ = app.emit("deep-link-channel", payload);
            }
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
            queue_navigation_deep_link(app, "message", &payload);
            let _ = app.emit("deep-link-message", payload);
        }
        Some("repo" | "project" | "pr" | "issue") => {
            // OS routing uses this build's scheme; frontend navigation consumes
            // canonical buzz:// entity links rather than transport identity.
            let Some(href) = canonical_entity_deep_link(
                &url,
                crate::build_identity::deep_link_scheme().as_ref(),
            ) else {
                eprintln!("buzz-desktop: malformed entity deep link: {url_str}");
                return;
            };
            activate_main_window(app);
            let pending = queue_entity_deep_link(app, href);
            let _ = app.emit("deep-link-entity", pending);
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
#[path = "deep_link_tests.rs"]
mod tests;
