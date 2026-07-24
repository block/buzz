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
    /// Claim-service base URL, only set for the `join-slack` kind.
    service: Option<String>,
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
                && item.service == pending.service
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
pub(crate) struct PendingImportClaimDeepLink {
    request_id: String,
    #[serde(flatten)]
    payload: ImportClaimDeepLinkPayload,
}

#[derive(Default)]
pub(crate) struct PendingImportClaimDeepLinks(Mutex<VecDeque<PendingImportClaimDeepLink>>);

impl PendingImportClaimDeepLinks {
    fn enqueue(&self, pending: PendingImportClaimDeepLink) {
        let mut queue = self
            .0
            .lock()
            .expect("pending import-claim deep-link queue poisoned");
        if queue.iter().any(|item| item.payload == pending.payload) {
            return;
        }
        queue.push_back(pending);
    }

    fn first(&self) -> Option<PendingImportClaimDeepLink> {
        self.0
            .lock()
            .expect("pending import-claim deep-link queue poisoned")
            .front()
            .cloned()
    }

    fn acknowledge(&self, request_id: &str) -> bool {
        let mut queue = self
            .0
            .lock()
            .expect("pending import-claim deep-link queue poisoned");
        if queue
            .front()
            .is_some_and(|item| item.request_id == request_id)
        {
            queue.pop_front();
            true
        } else {
            false
        }
    }
}

#[tauri::command]
pub(crate) fn take_pending_import_claim_deep_link(
    pending: State<'_, PendingImportClaimDeepLinks>,
) -> Option<PendingImportClaimDeepLink> {
    pending.first()
}

#[tauri::command]
pub(crate) fn acknowledge_pending_import_claim_deep_link(
    request_id: String,
    pending: State<'_, PendingImportClaimDeepLinks>,
) -> bool {
    pending.acknowledge(&request_id)
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

#[allow(clippy::too_many_arguments)]
fn queue_community_deep_link(
    app: &tauri::AppHandle,
    kind: &str,
    relay_url: String,
    code: Option<String>,
    policy_receipt: Option<String>,
    name: Option<String>,
    service: Option<String>,
) {
    app.state::<PendingCommunityDeepLinks>()
        .enqueue(PendingCommunityDeepLink {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.to_owned(),
            relay_url,
            code,
            policy_receipt,
            name,
            service,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportClaimDeepLinkPayload {
    /// `<source>:<foreign id>`, e.g. `slack:T0266FRGM:U060976D0QN`.
    subject: String,
    /// Email channel: the single-use magic-link token to redeem at `service`.
    token: Option<String>,
    /// Base URL of the operator claim-service. The email channel POSTs to
    /// `/email/complete`; the OIDC channel uses it to bind the callback to the
    /// pending `join-slack` transaction.
    service: Option<String>,
    /// OIDC channel marker (`"oidc"`). The service has NOT yet published the
    /// attestation — the app must first redeem `code` at `/oidc/finalize` with a
    /// signed self-claim (proof of key possession), then publish that self-claim.
    via: Option<String>,
    /// OIDC join channel: relay that received membership + attestation.
    relay_url: Option<String>,
    /// OIDC join channel: short-lived finalize code from the Slack callback.
    code: Option<String>,
}

/// A foreign-identity subject is `<source>:<id>` with both parts present and an
/// alphanumeric source (e.g. `slack:T0266FRGM:U060`).
fn validate_import_claim_subject(subject: &str) -> Result<(), String> {
    let (source, id) = subject
        .split_once(':')
        .ok_or_else(|| "subject must be <source>:<id>".to_string())?;
    if source.is_empty() || id.is_empty() {
        return Err("subject must be <source>:<id>".into());
    }
    if !source.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("subject source must be alphanumeric".into());
    }
    Ok(())
}

/// The claim-service URL is attacker-influenced (it rides in the link), so pin
/// it to a plain http(s) origin with no embedded credentials before the app
/// will POST to it.
fn validate_claim_service(service: &str) -> Result<(), String> {
    let url = Url::parse(service).map_err(|error| format!("invalid service url: {error}"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("service must use http or https".into());
    }
    if url.host_str().is_none() {
        return Err("service missing host".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("service must not include credentials".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("service must not include a query or fragment".into());
    }
    if url.path() != "/" {
        return Err("service must be an origin without a path".into());
    }
    if url.scheme() == "http" && !is_loopback_host(&url) {
        return Err("service must use https unless it is local development".into());
    }
    Ok(())
}

fn is_loopback_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// `buzz://import-claim?subject=slack:T0266FRGM:U060&token=…&service=https://…`
/// (email), or
/// `buzz://import-claim?subject=slack:T0266FRGM:U060&via=oidc&relay=wss://…&service=https://…`
/// (OIDC). Rejects a link that identifies neither complete channel so the
/// dialog never sees a half-formed one.
fn parse_import_claim_deep_link(url: &Url) -> Result<ImportClaimDeepLinkPayload, String> {
    let subject = non_empty_param(url, "subject")?;
    validate_import_claim_subject(&subject)?;
    let token = optional_non_empty_param(url, "token");
    let service = optional_non_empty_param(url, "service");
    let via = optional_non_empty_param(url, "via");
    let code = optional_non_empty_param(url, "code");
    let mut relay_url = None;

    match (token.as_deref(), service.as_deref(), via.as_deref()) {
        // Email channel: both halves present; the service must be well-formed.
        (Some(_), Some(service), None) => validate_claim_service(service)?,
        // OIDC channel: bind the callback to the target relay and the claim
        // service from the pending join-slack transaction, and require the
        // short-lived finalize code the app redeems with its signed self-claim.
        (None, Some(service), Some("oidc")) => {
            validate_claim_service(service)?;
            if code.is_none() {
                return Err("import-claim OIDC requires a finalize code".into());
            }
            relay_url = Some(
                parse_websocket_relay_param(url)
                    .ok_or_else(|| "import-claim OIDC requires a valid relay".to_string())?,
            );
        }
        _ => {
            return Err(
                "import-claim requires token+service (email) or via=oidc+code+relay+service".into(),
            )
        }
    }

    Ok(ImportClaimDeepLinkPayload {
        subject,
        token,
        service,
        via,
        relay_url,
        code,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinSlackDeepLinkPayload {
    relay_url: String,
    service: String,
}

/// `buzz://join-slack?relay=<ws(s)://...>&service=<https://...>` — the join
/// method for a Slack-migration community. The person signs in with Slack at
/// `service`, which registers them and attests their imported identity; the
/// relay is the community they join. Both params are required and validated so
/// onboarding never sees a half-formed link.
fn parse_join_slack_deep_link(url: &Url) -> Result<JoinSlackDeepLinkPayload, String> {
    let relay_url =
        parse_websocket_relay_param(url).ok_or_else(|| "missing or invalid relay".to_string())?;
    let service = non_empty_param(url, "service")?;
    validate_claim_service(&service)?;
    Ok(JoinSlackDeepLinkPayload { relay_url, service })
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
        Some("connect") => {
            let Some(relay_url) = parse_websocket_relay_param(&url) else {
                eprintln!("buzz-desktop: connect deep link missing/invalid relay: {url_str}");
                return;
            };
            activate_main_window(app);
            queue_community_deep_link(app, "connect", relay_url.clone(), None, None, None, None);
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
            queue_community_deep_link(app, "join", relay_url, code, policy_receipt, None, None);
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
                None,
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
        Some("import-claim") => match parse_import_claim_deep_link(&url) {
            Ok(payload) => {
                activate_main_window(app);
                // OAuth commonly returns while the app is already open, but a
                // relaunch must not lose the callback before React subscribes.
                let pending = PendingImportClaimDeepLink {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    payload,
                };
                app.state::<PendingImportClaimDeepLinks>()
                    .enqueue(pending.clone());
                let _ = app.emit("deep-link-import-claim", pending);
            }
            Err(error) => {
                eprintln!("buzz-desktop: rejecting import-claim deep link: {error}: {url_str}");
            }
        },
        Some("join-slack") => match parse_join_slack_deep_link(&url) {
            Ok(payload) => {
                activate_main_window(app);
                // Queue for cold-launch survival: a fresh install must create a
                // key before the Slack sign-in can begin.
                queue_community_deep_link(
                    app,
                    "join-slack",
                    payload.relay_url.clone(),
                    None,
                    None,
                    None,
                    Some(payload.service.clone()),
                );
                let _ = app.emit("deep-link-join-slack", payload);
            }
            Err(error) => {
                eprintln!("buzz-desktop: rejecting join-slack deep link: {error}: {url_str}");
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
mod tests;
