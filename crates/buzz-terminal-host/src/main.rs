#![deny(unsafe_code)]

mod channel;

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use buzz_core::observer::{
    decrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY,
};
use buzz_sdk::{build_message, nip_oa, ThreadRef};
use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use futures_util::StreamExt;
use nostr::{Event, EventId, Keys, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{self, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, LinesCodec};
use url::Url;
use uuid::Uuid;

const MAX_LINE: usize = 128 * 1024;
const MAX_SUBSCRIPTIONS: usize = 64;
const MAX_FILTERS: usize = 16;
const MAX_CACHE: usize = 256;

#[derive(Deserialize)]
struct Request {
    id: String,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct MessageRequest {
    request_key: String,
    channel_id: String,
    content: String,
    recipient_pubkeys: Vec<String>,
    root_event_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeRequest {
    subscription_id: String,
    filters: Vec<Value>,
}

enum Command {
    Request(Request),
}

#[derive(Clone)]
struct CachedSend {
    payload: Value,
    event: Event,
    state: SendState,
}

#[derive(Clone, Copy)]
enum SendState {
    Signed,
    Uncertain,
    Accepted,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let keys = load_keys()?;
    let relay_url = load_relay_url()?;
    let relay_pubkey = load_relay_pubkey(&relay_url).await?;
    let auth_tag = load_auth_tag()?;

    let (out_tx, out_rx) = mpsc::channel::<Value>(256);
    let writer = tokio::spawn(write_output(out_rx));
    emit(
        &out_tx,
        json!({
            "type": "ready", "protocolVersion": 1,
            "pubkey": keys.public_key().to_hex(), "relayPubkey": relay_pubkey,
            "relayUrl": relay_url
        }),
    )
    .await?;

    let (cmd_tx, cmd_rx) = mpsc::channel(128);
    let actor = tokio::spawn(run_relay(relay_url, keys, auth_tag, cmd_rx, out_tx.clone()));
    let stdin = io::stdin();
    let mut lines = FramedRead::new(stdin, LinesCodec::new_with_max_length(MAX_LINE));
    while let Some(line) = lines.next().await {
        match line {
            Ok(line) => match serde_json::from_str::<Request>(&line) {
                Ok(req) => {
                    if cmd_tx.send(Command::Request(req)).await.is_err() {
                        break;
                    }
                }
                Err(_) => {
                    emit(
                        &out_tx,
                        error("", "invalid_request", "invalid JSON request", None),
                    )
                    .await?
                }
            },
            Err(_) => {
                emit(
                    &out_tx,
                    error("", "line_too_long", "request exceeds 128 KiB", None),
                )
                .await?;
                break;
            }
        }
    }
    drop(cmd_tx);
    actor.abort();
    drop(out_tx);
    writer.await.context("output task failed")??;
    Ok(())
}

fn load_keys() -> Result<Keys> {
    let raw = std::env::var("BUZZ_PRIVATE_KEY").context("BUZZ_PRIVATE_KEY is required")?;
    Keys::parse(&raw).map_err(|_| anyhow!("BUZZ_PRIVATE_KEY is invalid"))
}

fn load_relay_url() -> Result<String> {
    let raw = std::env::var("BUZZ_RELAY_URL").context("BUZZ_RELAY_URL is required")?;
    let url = Url::parse(&raw).map_err(|_| anyhow!("BUZZ_RELAY_URL is invalid"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(anyhow!(
            "relay URL must not contain credentials, a query, or a fragment"
        ));
    }
    match url.scheme() {
        "wss" => Ok(raw),
        "ws" if is_loopback(&url) => Ok(raw),
        "ws" => Err(anyhow!("cleartext remote relay URLs are not allowed")),
        _ => Err(anyhow!("BUZZ_RELAY_URL must use wss (or ws on loopback)")),
    }
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

async fn load_relay_pubkey(relay_url: &str) -> Result<String> {
    if let Ok(value) = std::env::var("BUZZ_RELAY_PUBKEY") {
        return PublicKey::from_hex(&value)
            .map(|p| p.to_hex())
            .map_err(|_| anyhow!("BUZZ_RELAY_PUBKEY is invalid"));
    }
    let mut url = Url::parse(relay_url)?;
    url.set_scheme(if url.scheme() == "wss" {
        "https"
    } else {
        "http"
    })
    .map_err(|_| anyhow!("could not form NIP-11 URL"))?;
    let mut response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(url)
        .header("Accept", "application/nostr+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("failed to fetch NIP-11 relay info")?
        .error_for_status()
        .context("NIP-11 relay info request failed")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_LINE {
            return Err(anyhow!("NIP-11 relay info exceeds 128 KiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let info: Value = serde_json::from_slice(&bytes).context("invalid NIP-11 relay info")?;
    let key = info
        .get("self")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("NIP-11 relay info is missing self"))?;
    PublicKey::from_hex(key)
        .map(|p| p.to_hex())
        .map_err(|_| anyhow!("NIP-11 self is invalid"))
}

fn load_auth_tag() -> Result<Option<Tag>> {
    let Ok(raw) = std::env::var("BUZZ_AUTH_TAG") else {
        return Ok(None);
    };
    let parts: Vec<String> =
        serde_json::from_str(&raw).map_err(|_| anyhow!("BUZZ_AUTH_TAG is malformed"))?;
    Tag::parse(parts)
        .map(Some)
        .map_err(|_| anyhow!("BUZZ_AUTH_TAG is malformed"))
}

async fn write_output(mut rx: mpsc::Receiver<Value>) -> Result<()> {
    let mut stdout = io::BufWriter::new(io::stdout());
    while let Some(value) = rx.recv().await {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        stdout.write_all(&bytes).await?;
        stdout.flush().await?;
    }
    Ok(())
}

async fn emit(tx: &mpsc::Sender<Value>, value: Value) -> Result<()> {
    tx.send(value).await.map_err(|_| anyhow!("stdout closed"))
}

fn error(id: &str, code: &str, message: &str, event_id: Option<&str>) -> Value {
    let mut detail = json!({"code": code, "message": message});
    if let Some(event_id) = event_id {
        detail["eventId"] = json!(event_id);
    }
    json!({"id": id, "error": detail})
}

async fn run_relay(
    relay_url: String,
    keys: Keys,
    auth_tag: Option<Tag>,
    mut commands: mpsc::Receiver<Command>,
    out: mpsc::Sender<Value>,
) {
    let mut subscriptions: HashMap<String, Vec<Value>> = HashMap::new();
    let mut cache: HashMap<String, CachedSend> = HashMap::new();
    let mut connection = None;
    let mut failures = 0u8;
    let mut reconnect_at = tokio::time::Instant::now();
    loop {
        if connection.is_none() && failures < 6 && tokio::time::Instant::now() >= reconnect_at {
            let _ = emit(&out, json!({"type":"connection","status":"connecting"})).await;
            if let Ok(Ok(mut conn)) = tokio::time::timeout(
                Duration::from_secs(45),
                NostrWsConnection::connect_authenticated(&relay_url, &keys, auth_tag.as_ref()),
            )
            .await
            {
                let mut failed = false;
                for (id, filters) in &subscriptions {
                    let mut req = vec![json!("REQ"), json!(id)];
                    req.extend(filters.iter().cloned());
                    if conn.send_raw(&Value::Array(req)).await.is_err() {
                        failed = true;
                        break;
                    }
                }
                if !failed {
                    connection = Some(conn);
                    failures = 0;
                    let _ = emit(&out, json!({"type":"connection","status":"connected"})).await;
                }
            }
            if connection.is_none() {
                failures += 1;
                let status = if failures >= 6 {
                    "failed"
                } else {
                    "disconnected"
                };
                let _ = emit(&out, json!({"type":"connection","status":status,"message":"relay connection failed"})).await;
                reconnect_at =
                    tokio::time::Instant::now() + Duration::from_secs(1u64 << failures.min(5));
            }
        }

        let wait = if connection.is_none() && failures < 6 {
            reconnect_at.saturating_duration_since(tokio::time::Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        tokio::select! {
            command = commands.recv() => match command {
                None => break,
                Some(Command::Request(req)) => {
                    if req.method == "reconnect" {
                        failures = 0; reconnect_at = tokio::time::Instant::now(); connection = None;
                        let _ = emit(&out, json!({"id":req.id,"result":{}})).await;
                    } else {
                        handle_request(req, &keys, auth_tag.as_ref(), &mut connection, &mut subscriptions, &mut cache, &out).await;
                    }
                }
            },
            _ = tokio::time::sleep(wait), if connection.is_none() => {},
            received = receive(&mut connection), if connection.is_some() => {
                match received {
                    Ok(message) => handle_relay_message(message, &keys, &subscriptions, &out).await,
                    Err(WsClientError::Timeout) => {},
                    Err(_) => {
                        connection = None; failures = 1;
                        reconnect_at = tokio::time::Instant::now() + Duration::from_secs(2);
                        let _ = emit(&out, json!({"type":"connection","status":"disconnected","message":"relay connection lost"})).await;
                    }
                }
            }
        }
    }
}

async fn receive(
    connection: &mut Option<NostrWsConnection>,
) -> Result<RelayMessage, WsClientError> {
    match connection {
        Some(connection) => connection.next_event(Duration::from_secs(3600)).await,
        None => std::future::pending().await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_request(
    req: Request,
    keys: &Keys,
    auth_tag: Option<&Tag>,
    connection: &mut Option<NostrWsConnection>,
    subscriptions: &mut HashMap<String, Vec<Value>>,
    cache: &mut HashMap<String, CachedSend>,
    out: &mpsc::Sender<Value>,
) {
    let response = match req.method.as_str() {
        "subscribe" => subscribe(&req, connection, subscriptions).await,
        "unsubscribe" => unsubscribe(&req, connection, subscriptions).await,
        "sendMessage" | "createChannel" | "joinChannel" | "addMember" => {
            send_message(&req, keys, auth_tag, connection, cache, subscriptions, out).await
        }
        _ => Err(("method_not_found", "unknown method".to_string(), None)),
    };
    let value = match response {
        Ok(result) => json!({"id": req.id, "result": result}),
        Err((code, message, event_id)) => error(&req.id, code, &message, event_id.as_deref()),
    };
    let _ = emit(out, value).await;
}

type RpcResult = std::result::Result<Value, (&'static str, String, Option<String>)>;

async fn subscribe(
    req: &Request,
    connection: &mut Option<NostrWsConnection>,
    subscriptions: &mut HashMap<String, Vec<Value>>,
) -> RpcResult {
    let mut params: SubscribeRequest =
        serde_json::from_value(req.params.clone()).map_err(|_| {
            (
                "invalid_params",
                "invalid subscribe parameters".into(),
                None,
            )
        })?;
    if params.subscription_id.is_empty()
        || params.subscription_id.len() > 128
        || params.filters.is_empty()
        || params.filters.len() > MAX_FILTERS
    {
        return Err((
            "invalid_params",
            "subscription or filter count is invalid".into(),
            None,
        ));
    }
    if !subscriptions.contains_key(&params.subscription_id)
        && subscriptions.len() >= MAX_SUBSCRIPTIONS
    {
        return Err(("capacity", "subscription capacity reached".into(), None));
    }
    for filter in &mut params.filters {
        validate_filter(filter)?;
        if filter.get("limit").is_none() {
            filter["limit"] = json!(500);
        }
    }
    let conn =
        connection
            .as_mut()
            .ok_or(("disconnected", "relay is not connected".into(), None))?;
    let mut wire = vec![json!("REQ"), json!(params.subscription_id)];
    wire.extend(params.filters.iter().cloned());
    conn.send_raw(&Value::Array(wire))
        .await
        .map_err(|_| ("network", "failed to register subscription".into(), None))?;
    subscriptions.insert(params.subscription_id, params.filters);
    Ok(json!({}))
}

fn validate_filter(filter: &Value) -> RpcResult {
    let object =
        filter
            .as_object()
            .ok_or(("invalid_params", "filter must be an object".into(), None))?;
    let kinds = object.get("kinds").and_then(Value::as_array).ok_or((
        "invalid_params",
        "every filter requires explicit kinds".into(),
        None,
    ))?;
    if kinds.is_empty()
        || kinds.len() > 64
        || kinds.iter().any(|k| k.as_u64().is_none_or(|v| v > 65535))
    {
        return Err(("invalid_params", "filter kinds are invalid".into(), None));
    }
    if object
        .get("limit")
        .and_then(Value::as_u64)
        .is_some_and(|v| v > 1000)
        || ["authors", "ids", "#h", "#d", "#p", "#e"]
            .iter()
            .any(|name| {
                object
                    .get(*name)
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.len() > 256)
            })
    {
        return Err(("invalid_params", "filter bounds are too large".into(), None));
    }
    if serde_json::to_vec(filter).map_or(true, |v| v.len() > 16 * 1024) {
        return Err(("invalid_params", "filter is too large".into(), None));
    }
    Ok(json!({}))
}

async fn unsubscribe(
    req: &Request,
    connection: &mut Option<NostrWsConnection>,
    subscriptions: &mut HashMap<String, Vec<Value>>,
) -> RpcResult {
    let id = req
        .params
        .get("subscriptionId")
        .and_then(Value::as_str)
        .ok_or(("invalid_params", "subscriptionId is required".into(), None))?;
    subscriptions.remove(id);
    if let Some(conn) = connection {
        conn.send_raw(&json!(["CLOSE", id]))
            .await
            .map_err(|_| ("network", "failed to close subscription".into(), None))?;
    }
    Ok(json!({}))
}

#[allow(clippy::too_many_arguments)]
async fn send_message(
    req: &Request,
    keys: &Keys,
    auth_tag: Option<&Tag>,
    connection: &mut Option<NostrWsConnection>,
    cache: &mut HashMap<String, CachedSend>,
    subscriptions: &HashMap<String, Vec<Value>>,
    out: &mpsc::Sender<Value>,
) -> RpcResult {
    let request_key = req
        .params
        .get("requestKey")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty() && key.len() <= 128)
        .ok_or((
            "invalid_params",
            "requestKey is required (max 128 bytes)".into(),
            None,
        ))?;
    let payload = json!({"method": req.method, "params": req.params});
    let was_uncertain = cache
        .get(request_key)
        .is_some_and(|cached| matches!(cached.state, SendState::Uncertain));
    let event = if let Some(cached) = cache.get(request_key) {
        if cached.payload != payload {
            return Err((
                "request_key_conflict",
                "requestKey was used with a different payload".into(),
                Some(cached.event.id.to_hex()),
            ));
        }
        if matches!(cached.state, SendState::Accepted) {
            return Ok(json!({"event": cached.event, "accepted": true}));
        }
        cached.event.clone()
    } else {
        if cache.len() >= MAX_CACHE {
            return Err((
                "capacity",
                "session send limit reached (256); resolve pending deliveries before restarting"
                    .into(),
                None,
            ));
        }
        let mut builder = if req.method != "sendMessage" {
            channel::build(&req.method, req.params.clone())?
        } else {
            let message: MessageRequest = serde_json::from_value(req.params.clone())
                .map_err(|_| ("invalid_params", "invalid message parameters".into(), None))?;
            validate_message(&message)?;
            let channel = Uuid::parse_str(&message.channel_id)
                .map_err(|_| ("invalid_params", "channelId must be a UUID".into(), None))?;
            let thread = message
                .root_event_id
                .as_ref()
                .map(|id| {
                    EventId::from_hex(id).map(|event_id| ThreadRef {
                        root_event_id: event_id,
                        parent_event_id: event_id,
                    })
                })
                .transpose()
                .map_err(|_| ("invalid_params", "rootEventId is invalid".into(), None))?;
            let mentions: Vec<&str> = message
                .recipient_pubkeys
                .iter()
                .map(String::as_str)
                .collect();
            build_message(
                channel,
                &message.content,
                thread.as_ref(),
                &mentions,
                false,
                &[],
                &[],
            )
            .map_err(|_| ("invalid_params", "message could not be signed".into(), None))?
        };
        if let Some(auth_tag) = auth_tag {
            builder = builder.tags([auth_tag.clone()]);
        }
        let event = builder
            .sign_with_keys(keys)
            .map_err(|_| ("invalid_params", "message could not be signed".into(), None))?;
        cache.insert(
            request_key.to_owned(),
            CachedSend {
                payload: payload.clone(),
                event: event.clone(),
                state: SendState::Signed,
            },
        );
        event
    };
    let event_id = event.id.to_hex();
    let conn = connection.as_mut().ok_or((
        "uncertain",
        "relay disconnected; retry with the same requestKey".into(),
        Some(event_id.clone()),
    ))?;
    let published = tokio::time::timeout(Duration::from_secs(30), async {
        conn.send_raw(&json!(["EVENT", event])).await?;
        loop {
            match conn.next_event(Duration::from_secs(30)).await? {
                RelayMessage::Ok(ok) if ok.event_id == event_id => {
                    return Ok::<_, WsClientError>(ok)
                }
                message => handle_relay_message(message, keys, subscriptions, out).await,
            }
        }
    })
    .await;
    match published {
        Ok(Ok(ok)) if ok.accepted => {
            if let Some(entry) = cache.get_mut(request_key) {
                entry.state = SendState::Accepted;
            }
            Ok(json!({"event": event, "accepted": true}))
        }
        Ok(Ok(_)) if was_uncertain => Err((
            "uncertain",
            "retry was rejected; the original delivery remains unknown".into(),
            Some(event_id),
        )),
        Ok(Ok(ok)) => {
            cache.remove(request_key);
            Err((
                "rejected",
                if ok.message.is_empty() {
                    "relay rejected event".into()
                } else {
                    ok.message
                },
                Some(event_id),
            ))
        }
        _ => {
            *connection = None;
            if let Some(entry) = cache.get_mut(request_key) {
                entry.state = SendState::Uncertain;
            }
            Err((
                "uncertain",
                "publish outcome is unknown; retry with the same requestKey".into(),
                Some(event_id),
            ))
        }
    }
}

fn validate_message(payload: &MessageRequest) -> RpcResult {
    if payload.request_key.is_empty()
        || payload.request_key.len() > 128
        || payload.content.trim().is_empty()
        || payload.content.len() > 32 * 1024
    {
        return Err((
            "invalid_params",
            "requestKey/content length is invalid".into(),
            None,
        ));
    }
    if payload.recipient_pubkeys.is_empty() || payload.recipient_pubkeys.len() > 50 {
        return Err((
            "invalid_params",
            "recipientPubkeys must contain 1 to 50 keys".into(),
            None,
        ));
    }
    let mut exact = std::collections::HashSet::new();
    for key in &payload.recipient_pubkeys {
        let public_key = PublicKey::from_hex(key)
            .map_err(|_| ("invalid_params", "recipient pubkey is invalid".into(), None))?;
        if !exact.insert(public_key) {
            return Err((
                "invalid_params",
                "recipient pubkeys must be distinct".into(),
                None,
            ));
        }
    }
    Ok(json!({}))
}

async fn handle_relay_message(
    message: RelayMessage,
    keys: &Keys,
    subscriptions: &HashMap<String, Vec<Value>>,
    out: &mpsc::Sender<Value>,
) {
    let value = match message {
        RelayMessage::Event {
            subscription_id,
            event,
        } if subscriptions.contains_key(&subscription_id) => {
            if event.verify().is_err() {
                return;
            }
            if event.kind.as_u16() == 24200 {
                observer_notification(&event, keys)
            } else {
                let owner = if event.kind.as_u16() == 0 {
                    owner_pubkey(&event)
                } else {
                    None
                };
                let mut value =
                    json!({"type":"event","subscriptionId":subscription_id,"event":event});
                if let Some(owner) = owner {
                    value["ownerPubkey"] = json!(owner);
                }
                Some(value)
            }
        }
        RelayMessage::Eose { subscription_id } if subscriptions.contains_key(&subscription_id) => {
            Some(json!({"type":"eose","subscriptionId":subscription_id}))
        }
        RelayMessage::Closed {
            subscription_id,
            message,
        } if subscriptions.contains_key(&subscription_id) => {
            Some(json!({"type":"closed","subscriptionId":subscription_id,"message":message}))
        }
        RelayMessage::Notice { message } => Some(json!({"type":"notice","message":message})),
        _ => None,
    };
    if let Some(value) = value {
        let _ = emit(out, value).await;
    }
}

fn owner_pubkey(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some("auth") && parts.len() == 4)
            .then(|| serde_json::to_string(parts).ok())
            .flatten()
            .and_then(|raw| nip_oa::verify_auth_tag(&raw, &event.pubkey).ok())
            .map(|owner| owner.to_hex())
    })
}

fn observer_notification(event: &Event, keys: &Keys) -> Option<Value> {
    let tag_value = |name: &str| {
        event.tags.iter().find_map(|tag| {
            let p = tag.as_slice();
            (p.first().map(String::as_str) == Some(name))
                .then(|| p.get(1).cloned())
                .flatten()
        })
    };
    let agent = tag_value(OBSERVER_AGENT_TAG)?;
    if tag_value(OBSERVER_FRAME_TAG).as_deref() != Some(OBSERVER_FRAME_TELEMETRY)
        || agent != event.pubkey.to_hex()
        || !event.tags.iter().any(|t| {
            t.as_slice().first().map(String::as_str) == Some("p")
                && t.as_slice().get(1).map(String::as_str)
                    == Some(keys.public_key().to_hex().as_str())
        })
    {
        return None;
    }
    let envelope: Value = decrypt_observer_payload(keys, event).ok()?;
    Some(
        json!({"type":"observer","eventId":event.id.to_hex(),"agentPubkey":agent,"envelope":envelope}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn channel_retry_keeps_signed_identity_and_binds_method_and_payload() {
        let keys = Keys::generate();
        let mut req = Request {
            id: "rpc-1".into(),
            method: "createChannel".into(),
            params: json!({"requestKey":"new-channel","channelId":Uuid::new_v4()}),
        };
        let mut cache = HashMap::new();
        let mut connection = None;
        let subscriptions = HashMap::new();
        let (out, _rx) = mpsc::channel(1);
        let first = send_message(
            &req,
            &keys,
            None,
            &mut connection,
            &mut cache,
            &subscriptions,
            &out,
        )
        .await
        .expect_err("offline");
        assert_eq!(first.0, "uncertain");
        assert!(first.2.is_some());
        let original = cache.get("new-channel").expect("cached").event.clone();
        original.verify().expect("signature");
        req.id = "rpc-2".into();
        let retry = send_message(
            &req,
            &keys,
            None,
            &mut connection,
            &mut cache,
            &subscriptions,
            &out,
        )
        .await
        .expect_err("offline retry");
        assert_eq!(retry.2, first.2);
        assert_eq!(cache.get("new-channel").expect("cached").event, original);
        req.method = "joinChannel".into();
        let conflict = send_message(
            &req,
            &keys,
            None,
            &mut connection,
            &mut cache,
            &subscriptions,
            &out,
        )
        .await
        .expect_err("method conflict");
        assert_eq!(conflict.0, "request_key_conflict");
        req.method = "createChannel".into();
        req.params["channelId"] = json!(Uuid::new_v4());
        let conflict = send_message(
            &req,
            &keys,
            None,
            &mut connection,
            &mut cache,
            &subscriptions,
            &out,
        )
        .await
        .expect_err("payload conflict");
        assert_eq!(conflict.0, "request_key_conflict");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn observer_requires_verified_shape_and_decrypts() {
        use buzz_core::observer::encrypt_observer_payload;
        use buzz_sdk::build_agent_observer_frame;
        let owner = Keys::generate();
        let agent = Keys::generate();
        let envelope = json!({"seq":1,"timestamp":"now","kind":"turn","channelId":null,"sessionId":null,"turnId":null,"payload":{}});
        let encrypted =
            encrypt_observer_payload(&agent, &owner.public_key(), &envelope).expect("encrypt");
        let event = build_agent_observer_frame(
            &owner.public_key().to_hex(),
            &agent.public_key().to_hex(),
            OBSERVER_FRAME_TELEMETRY,
            &encrypted,
        )
        .expect("build")
        .sign_with_keys(&agent)
        .expect("sign");
        assert_eq!(
            observer_notification(&event, &owner).expect("notification")["envelope"],
            envelope
        );
        assert!(observer_notification(&event, &Keys::generate()).is_none());
    }
}
