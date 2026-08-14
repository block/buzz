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
    let (mut owner, mut dtag, mut id, mut tab) = (None, None, None, None);
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "owner" => &mut owner,
            "d" => &mut dtag,
            "id" if needs_event_id => &mut id,
            "tab" if allows_tab => &mut tab,
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
    if let Some(tab) = tab {
        if !ENTITY_LINK_TABS.contains(&tab.as_str()) {
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
        Some("repo" | "project" | "pr" | "issue") => {
            // `buzz://repo|project?owner=<pubkey>&d=<dtag>` and
            // `buzz://pr|issue?id=<eventId>&owner=<pubkey>&d=<dtag>` — the
            // share links copied from the Projects UI. The frontend owns
            // routing (`useEntityDeepLinks`), so the validated URL is
            // forwarded unchanged.
            if parse_entity_deep_link(&url).is_none() {
                eprintln!("buzz-desktop: malformed entity deep link: {url_str}");
                return;
            }
            activate_main_window(app);
            let pending = queue_entity_deep_link(app, url_str.to_owned());
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
mod tests {
    use url::Url;

    use super::{
        parse_add_community_deep_link, parse_entity_deep_link, parse_join_deep_link,
        parse_message_deep_link, parse_nostr_bind_deep_link, PendingCommunityDeepLink,
        PendingCommunityDeepLinks, PendingEntityDeepLinks, ENTITY_LINK_TABS,
    };

    fn entity_link_golden() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../test-fixtures/entity-links.json"))
            .expect("valid entity-links golden fixture")
    }

    #[test]
    fn parse_entity_deep_link_accepts_every_share_link_shape() {
        let golden = entity_link_golden();
        let owner = golden["owner"].as_str().unwrap();
        let dtag = golden["dtag"].as_str().unwrap();
        for raw in golden["links"]
            .as_object()
            .unwrap()
            .values()
            .map(|value| value.as_str().unwrap().to_owned())
            .chain(golden["tabs"].as_array().unwrap().iter().map(|tab| {
                format!(
                    "buzz://repo?owner={owner}&d={dtag}&tab={}",
                    tab.as_str().unwrap()
                )
            }))
        {
            assert!(
                parse_entity_deep_link(&Url::parse(&raw).unwrap()).is_some(),
                "{raw}"
            );
        }
        let expected_tabs = golden["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ENTITY_LINK_TABS.as_slice(), expected_tabs);
    }

    #[test]
    fn parse_entity_deep_link_rejects_malformed_and_non_canonical_links() {
        let golden = entity_link_golden();
        let owner = golden["owner"].as_str().unwrap();
        let event_id = golden["eventId"].as_str().unwrap();
        for raw in [
            // Missing or malformed identifiers.
            format!("buzz://repo?owner={owner}"),
            "buzz://repo?owner=nope&d=buzz-world".to_owned(),
            format!("buzz://repo?owner={owner}&d=.hidden"),
            format!("buzz://repo?owner={owner}&d=has%20space"),
            format!("buzz://pr?owner={owner}&d=buzz-world"),
            format!("buzz://pr?id=short&owner={owner}&d=buzz-world"),
            // Coordinate links take no event id.
            format!("buzz://repo?id={event_id}&owner={owner}&d=buzz-world"),
            // Non-canonical: unknown param, duplicate param, path, fragment.
            format!("buzz://repo?owner={owner}&d=buzz-world&relay=wss%3A%2F%2Fx.example"),
            format!("buzz://repo?owner={owner}&owner={owner}&d=buzz-world"),
            // Unknown tab value, duplicate tab, and tab on an event link.
            format!("buzz://repo?owner={owner}&d=buzz-world&tab=overview"),
            format!("buzz://repo?owner={owner}&d=buzz-world&tab=prs&tab=prs"),
            format!("buzz://pr?id={event_id}&owner={owner}&d=buzz-world&tab=prs"),
            format!("buzz://repo/extra?owner={owner}&d=buzz-world"),
            format!("buzz://repo?owner={owner}&d=buzz-world#top"),
            // Not an entity host.
            format!("buzz://message?owner={owner}&d=buzz-world"),
        ] {
            assert!(
                parse_entity_deep_link(&Url::parse(&raw).unwrap()).is_none(),
                "{raw}"
            );
        }
    }

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

    #[test]
    fn pending_entity_links_survive_until_acknowledged_in_order() {
        let queue = PendingEntityDeepLinks::default();
        let first = queue.enqueue("buzz://project?owner=aa&d=first".to_owned());
        let second = queue.enqueue("buzz://project?owner=aa&d=second".to_owned());

        assert_eq!(queue.first(), Some(first.clone()));
        assert!(!queue.acknowledge(&second.id));
        assert!(queue.acknowledge(&first.id));
        assert_eq!(queue.first(), Some(second));
    }

    #[test]
    fn pending_entity_links_dedupe_launch_and_open_callbacks() {
        let queue = PendingEntityDeepLinks::default();
        let href = "buzz://project?owner=aa&d=buzz".to_owned();
        let first = queue.enqueue(href.clone());
        let duplicate = queue.enqueue(href);

        assert_eq!(duplicate.id, first.id);
        assert!(queue.acknowledge(&first.id));
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
