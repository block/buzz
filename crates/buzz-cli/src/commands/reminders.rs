//! `buzz reminders` — your own NIP-ER reminders (kind:30300, NIP-44 encrypted
//! to yourself), the same records the desktop and mobile reminder panels edit.
//!
//! Every write is one read of the current head followed by one publish. A head
//! this CLI cannot decode is never overwritten.

use std::collections::HashMap;
use std::time::SystemTime;

use buzz_core::kind::KIND_EVENT_REMINDER;
use chrono::DateTime;
use nostr::nips::nip44;
use nostr::{Event, EventBuilder, EventId, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::parse_write_response;
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::validate_hex64;
use crate::{ReminderStatusFilter, RemindersCmd};

const ALT: &str = "Encrypted reminder";
/// Largest `not_before` the relay accepts (`Number.MAX_SAFE_INTEGER`).
const MAX_NOT_BEFORE: u64 = 9_007_199_254_740_991;
/// The relay's horizon when it advertises none.
const DEFAULT_MAX_NOT_BEFORE_DELTA: u64 = 31_536_000;
const NIP_ER_EXTENSION: &str = "nip-er";
/// Desktop clips target previews to this many UTF-16 code units.
const PREVIEW_UNITS: usize = 100;
const MAX_LIST_EVENTS: u32 = 5_000;
const MAX_D_LEN: usize = 256;
const MAX_CIPHERTEXT_BYTES: usize = 65_535;
const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    event_id: String,
    channel_id: String,
    preview: String,
    author_pubkey: String,
}

/// Decrypted reminder plaintext, field order as desktop's `JSON.stringify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<Target>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    status: &'static str,
}

#[derive(Debug, Clone)]
struct Reminder {
    d: String,
    not_before: Option<u64>,
    content: Content,
    created_at: u64,
    event_id: String,
}

impl Reminder {
    fn to_json(&self) -> Value {
        json!({
            "reminder": self.d,
            "status": self.content.status,
            "not_before": self.not_before,
            "note": self.content.note,
            "target": self.content.target,
            "created_at": self.created_at,
            "event_id": self.event_id,
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_status(value: &Value) -> Option<&'static str> {
    match value.as_str()? {
        "pending" => Some("pending"),
        "done" => Some("done"),
        "cancelled" => Some("cancelled"),
        _ => None,
    }
}

fn parse_target(value: &Value) -> Option<Target> {
    let object = value.as_object()?;
    let field = |name: &str| Some(object.get(name)?.as_str()?.to_string());
    Some(Target {
        event_id: field("eventId")?,
        channel_id: field("channelId")?,
        preview: field("preview")?,
        author_pubkey: field("authorPubkey")?,
    })
}

/// Desktop's `parseReminderContent`: anything off-shape is `None`.
fn parse_content(plaintext: &str) -> Option<Content> {
    let parsed: Value = serde_json::from_str(plaintext).ok()?;
    let object = parsed.as_object()?;
    let status = parse_status(object.get("status")?)?;
    let note = match object.get("note") {
        None => None,
        Some(value) => Some(value.as_str()?.to_string()),
    };
    let target = match object.get("target") {
        None => None,
        Some(value) => Some(parse_target(value)?),
    };
    if target.is_none() && note.as_deref().is_none_or(str::is_empty) {
        return None;
    }
    Some(Content {
        target,
        note,
        status,
    })
}

/// Desktop's `parseNotBefore`, which mirrors the relay's `validate_not_before`.
fn parse_not_before(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if raw.len() > 1 && raw.starts_with('0') {
        return None;
    }
    raw.parse().ok().filter(|value| *value <= MAX_NOT_BEFORE)
}

fn d_tag(event: &Event) -> Option<&str> {
    let mut values = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().map(String::as_str) == Some("d"))
        .map(|tag| tag.get(1).map(String::as_str).unwrap_or(""));
    match (values.next(), values.next()) {
        (Some(d), None) if !d.is_empty() => Some(d),
        _ => None,
    }
}

fn decode(event: &Event, client: &BuzzClient) -> Result<Reminder, String> {
    let d = d_tag(event).ok_or("it has no single non-empty d tag")?;
    let keys = client.keys();
    let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
        .map_err(|e| format!("it cannot be decrypted: {e}"))?;
    let content =
        parse_content(&plaintext).ok_or("its content is not a desktop-format reminder")?;
    let not_before = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().map(String::as_str) == Some("not_before"))
        .and_then(|tag| tag.get(1))
        .and_then(|raw| parse_not_before(raw));
    Ok(Reminder {
        d: d.to_string(),
        not_before,
        content,
        created_at: event.created_at.as_secs(),
        event_id: event.id.to_hex(),
    })
}

/// NIP-01 replacement order: newest `created_at`, then the lower id.
fn dominates(a: &Event, b: &Event) -> bool {
    a.created_at > b.created_at || (a.created_at == b.created_at && a.id.to_hex() < b.id.to_hex())
}

/// Events from the relay that are this identity's signed reminders.
fn own_valid_events(raw: Vec<Value>, client: &BuzzClient) -> (Vec<Event>, usize) {
    let me = client.keys().public_key();
    let mut invalid = 0;
    let events = raw
        .into_iter()
        .filter_map(|value| {
            let event = serde_json::from_value::<Event>(value).ok().filter(|event| {
                event.kind == Kind::Custom(KIND_EVENT_REMINDER as u16)
                    && event.pubkey == me
                    && event.verify().is_ok()
            });
            if event.is_none() {
                invalid += 1;
            }
            event
        })
        .collect();
    (events, invalid)
}

async fn fetch_head(client: &BuzzClient, d: &str) -> Result<(Event, Reminder), CliError> {
    if d.is_empty() || d.len() > MAX_D_LEN {
        return Err(CliError::Usage(format!(
            "--reminder must be 1-{MAX_D_LEN} characters"
        )));
    }
    let filter = json!({
        "kinds": [KIND_EVENT_REMINDER],
        "authors": [client.keys().public_key().to_hex()],
        "#d": [d],
        "limit": 1,
        "consistency": "strong",
    });
    let raw = client.query(&filter).await?;
    let raw: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse reminder query: {e}")))?;
    let returned = raw.len();
    let (events, _) = own_valid_events(raw, client);
    let head = events
        .into_iter()
        .filter(|event| {
            event
                .tags
                .iter()
                .any(|tag| matches!(tag.as_slice(), [name, value, ..] if name == "d" && value == d))
        })
        .reduce(|best, event| {
            if dominates(&event, &best) {
                event
            } else {
                best
            }
        });
    let Some(head) = head else {
        if returned > 0 {
            return Err(CliError::Other(
                "the relay returned a reminder event that is not yours or fails verification; refusing to overwrite it".into(),
            ));
        }
        return Err(CliError::NotFound(format!("no reminder with id {d}")));
    };
    let reminder = decode(&head, client).map_err(|detail| {
        CliError::Other(format!(
            "reminder {d} is unreadable ({detail}); refusing to overwrite it"
        ))
    })?;
    Ok((head, reminder))
}

fn encrypt(client: &BuzzClient, content: &Content) -> Result<String, CliError> {
    let plaintext = serde_json::to_string(content)
        .map_err(|e| CliError::Other(format!("failed to encode reminder: {e}")))?;
    let too_large = || CliError::Usage("reminder would exceed the relay's event size limit".into());
    // NIP-44 rejects longer plaintexts outright.
    if plaintext.len() > u16::MAX as usize {
        return Err(too_large());
    }
    let keys = client.keys();
    let ciphertext = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|e| CliError::Other(format!("failed to encrypt reminder: {e}")))?;
    if ciphertext.len() > MAX_CIPHERTEXT_BYTES {
        return Err(too_large());
    }
    Ok(ciphertext)
}

async fn publish(
    client: &BuzzClient,
    d: &str,
    content: &Content,
    schedule: Schedule,
    created_at: u64,
) -> Result<Value, CliError> {
    let mut tags = vec![Tag::identifier(d)];
    match schedule {
        Schedule::At(not_before) => {
            tags.push(parse_tag(["not_before", &not_before.to_string()])?);
            tags.push(parse_tag(["alt", ALT])?);
        }
        Schedule::ExpiresAt(expiration) => {
            tags.push(parse_tag(["alt", ALT])?);
            tags.push(Tag::expiration(Timestamp::from(expiration)));
        }
    }
    let builder = EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER as u16),
        encrypt(client, content)?,
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(created_at));
    let raw = client.submit_event(client.sign_event(builder)?).await?;
    let normalized = parse_write_response(
        &raw,
        "a newer version of this reminder exists; run `buzz reminders list` and retry",
    )?;
    let mut response: Value = serde_json::from_str(&normalized)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?;
    response["reminder"] = json!(d);
    Ok(response)
}

fn parse_tag<const N: usize>(parts: [&str; N]) -> Result<Tag, CliError> {
    Tag::parse(parts).map_err(|e| CliError::Other(format!("invalid reminder tag: {e}")))
}

enum Schedule {
    At(u64),
    ExpiresAt(u64),
}

fn jittered_expiration(now: u64) -> u64 {
    now + (30 + rand::random_range(0..60u64)) * SECONDS_PER_DAY
}

/// Accepts a unix timestamp, an RFC 3339 / ISO-8601 instant with an explicit
/// offset, or a relative duration (`30m`, `2h`, `1d`, `1w`).
fn parse_time(input: &str, now: u64) -> Result<u64, CliError> {
    let input = input.trim();
    let invalid = || {
        CliError::Usage(format!(
            "invalid time {input:?}: use a unix timestamp, an ISO-8601 instant with an offset (2030-01-02T09:00:00Z), or a duration such as 30m, 2h, 1d"
        ))
    };
    if input.is_empty() {
        return Err(invalid());
    }
    if input.bytes().all(|b| b.is_ascii_digit()) {
        return input.parse().map_err(|_| invalid());
    }
    if let Some((digits, unit)) = input.split_at_checked(input.len() - 1) {
        let seconds = match unit {
            "m" | "M" => Some(60),
            "h" | "H" => Some(3_600),
            "d" | "D" => Some(SECONDS_PER_DAY),
            "w" | "W" => Some(7 * SECONDS_PER_DAY),
            _ => None,
        };
        if let Some(seconds) = seconds {
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                return digits
                    .parse::<u64>()
                    .ok()
                    .and_then(|count| count.checked_mul(seconds))
                    .and_then(|delta| now.checked_add(delta))
                    .ok_or_else(invalid);
            }
        }
    }
    let instant = DateTime::parse_from_rfc3339(input).map_err(|_| invalid())?;
    u64::try_from(instant.timestamp()).map_err(|_| invalid())
}

/// Parses `--at` and rejects anything the relay or the user would find wrong:
/// a past time, or one beyond the relay's horizon.
fn due_time(input: &str, now: u64, max_delta: u64) -> Result<u64, CliError> {
    let at = parse_time(input, now)?;
    if at <= now {
        return Err(CliError::Usage(format!(
            "--at {input:?} is not in the future"
        )));
    }
    if at > MAX_NOT_BEFORE || at > now.saturating_add(max_delta) {
        return Err(CliError::Usage(format!(
            "--at {input:?} is more than {max_delta} seconds from now, which the relay rejects"
        )));
    }
    Ok(at)
}

/// The relay's advertised `not_before` horizon. Refuses relays that do not
/// advertise NIP-ER: they would store the event without validating it.
async fn relay_horizon(client: &BuzzClient) -> Result<u64, CliError> {
    let raw = client.get_public("/info").await?;
    let info: Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid NIP-11 response: {e}")))?;
    let supported = info
        .get("supported_extensions")
        .and_then(Value::as_array)
        .is_some_and(|ext| ext.iter().any(|e| e.as_str() == Some(NIP_ER_EXTENSION)));
    if !supported {
        return Err(CliError::Other(format!(
            "this relay does not support reminders (missing \"{NIP_ER_EXTENSION}\" in supported_extensions)"
        )));
    }
    Ok(info
        .pointer("/limitation/max_not_before_delta")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_NOT_BEFORE_DELTA))
}

/// Desktop clips with `String.slice(0, 100)`, which counts UTF-16 code units;
/// this never splits a surrogate pair.
fn clip_preview(text: &str) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= PREVIEW_UNITS
        })
        .collect()
}

async fn fetch_target(client: &BuzzClient, event_id: &str) -> Result<Target, CliError> {
    validate_hex64(event_id)?;
    let id =
        EventId::parse(event_id).map_err(|e| CliError::Usage(format!("invalid event ID: {e}")))?;
    let filter = json!({ "ids": [id.to_hex()], "limit": 1 });
    let raw = client.query(&filter).await?;
    let events: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    let event = events
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound(format!("event {event_id} not found")))?;
    let event: Event = serde_json::from_value(event)
        .map_err(|e| CliError::Other(format!("relay returned a malformed event: {e}")))?;
    if event.id != id || event.verify().is_err() {
        return Err(CliError::Other(format!(
            "the relay returned an event that is not {event_id} or fails verification"
        )));
    }
    let channel_id = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().map(String::as_str) == Some("h"))
        .and_then(|tag| tag.get(1).cloned())
        .ok_or_else(|| {
            CliError::Usage(format!(
                "event {event_id} has no channel (h tag); only channel messages can be reminder targets"
            ))
        })?;
    Ok(Target {
        event_id: id.to_hex(),
        channel_id,
        preview: clip_preview(&event.content),
        author_pubkey: event.pubkey.to_hex(),
    })
}

async fn list(
    client: &BuzzClient,
    filter: ReminderStatusFilter,
    max_events: u32,
) -> Result<Value, CliError> {
    let query = json!({
        "kinds": [KIND_EVENT_REMINDER],
        "authors": [client.keys().public_key().to_hex()],
        "consistency": "strong",
    });
    let raw = client.query_all_bounded(query, max_events).await?;
    let (events, mut skipped) = own_valid_events(raw, client);

    let mut heads: HashMap<String, Event> = HashMap::new();
    for event in events {
        let Some(d) = d_tag(&event).map(str::to_string) else {
            skipped += 1;
            continue;
        };
        match heads.get(&d) {
            Some(current) if !dominates(&event, current) => {}
            _ => {
                heads.insert(d, event);
            }
        }
    }

    let mut reminders = Vec::new();
    for event in heads.values() {
        match decode(event, client) {
            Ok(reminder) => reminders.push(reminder),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} reminder event(s) this CLI cannot read");
    }

    reminders.retain(|reminder| match filter {
        ReminderStatusFilter::All => true,
        ReminderStatusFilter::Pending => reminder.content.status == "pending",
        ReminderStatusFilter::Done => reminder.content.status == "done",
        ReminderStatusFilter::Cancelled => reminder.content.status == "cancelled",
    });
    reminders.sort_by(|a, b| {
        a.not_before
            .unwrap_or(u64::MAX)
            .cmp(&b.not_before.unwrap_or(u64::MAX))
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.d.cmp(&b.d))
    });
    Ok(Value::Array(
        reminders.iter().map(Reminder::to_json).collect(),
    ))
}

async fn add(
    client: &BuzzClient,
    event: Option<&str>,
    at: &str,
    note: Option<&str>,
    now: u64,
) -> Result<Value, CliError> {
    if note.is_some_and(|note| note.trim().is_empty()) {
        return Err(CliError::Usage("--note must not be empty".into()));
    }
    if event.is_none() && note.is_none() {
        return Err(CliError::Usage("pass --event, --note, or both".into()));
    }
    parse_time(at, now)?;
    let max_delta = relay_horizon(client).await?;
    let not_before = due_time(at, now, max_delta)?;
    let target = match event {
        Some(event) => Some(fetch_target(client, event).await?),
        None => None,
    };
    let content = Content {
        target,
        note: note.map(str::to_string),
        status: "pending",
    };
    let d = format!("{:032x}", rand::random::<u128>());
    publish(client, &d, &content, Schedule::At(not_before), now).await
}

enum Transition {
    Done,
    Cancel,
    Snooze(String),
}

async fn transition(
    client: &BuzzClient,
    d: &str,
    change: Transition,
    now: u64,
) -> Result<Value, CliError> {
    if let Transition::Snooze(at) = &change {
        parse_time(at, now)?;
    }
    let (_, head) = fetch_head(client, d).await?;
    if head.content.status != "pending" {
        return Err(CliError::Usage(format!(
            "reminder {d} is already {}",
            head.content.status
        )));
    }
    let (status, schedule) = match change {
        Transition::Done => ("done", Schedule::ExpiresAt(jittered_expiration(now))),
        Transition::Cancel => ("cancelled", Schedule::ExpiresAt(jittered_expiration(now))),
        Transition::Snooze(at) => {
            let max_delta = relay_horizon(client).await?;
            ("pending", Schedule::At(due_time(&at, now, max_delta)?))
        }
    };
    let content = Content {
        status,
        ..head.content
    };
    let created_at = now.max(head.created_at + 1);
    publish(client, d, &content, schedule, created_at).await
}

async fn run(cmd: RemindersCmd, client: &BuzzClient, now: u64) -> Result<Value, CliError> {
    match cmd {
        RemindersCmd::List { status } => list(client, status, MAX_LIST_EVENTS).await,
        RemindersCmd::Add { event, at, note } => {
            add(client, event.as_deref(), &at, note.as_deref(), now).await
        }
        RemindersCmd::Done { reminder } => {
            transition(client, &reminder, Transition::Done, now).await
        }
        RemindersCmd::Snooze { reminder, at } => {
            transition(client, &reminder, Transition::Snooze(at), now).await
        }
        RemindersCmd::Cancel { reminder } => {
            transition(client, &reminder, Transition::Cancel, now).await
        }
    }
}

/// Runs a `reminders` subcommand and prints its JSON result.
pub async fn dispatch(cmd: RemindersCmd, client: &BuzzClient) -> Result<(), CliError> {
    let output = run(cmd, client, unix_now()).await?;
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::get, routing::post, Json, Router};
    use nostr::Keys;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const NOW: u64 = 1_800_000_000;

    struct Relay {
        events: Vec<Value>,
        /// Extra raw events returned by every reminder query, beyond the store.
        decoys: Vec<Value>,
        info: Value,
        queries: Vec<Value>,
        /// Mirrors the relay: a dominated replacement is acknowledged as a duplicate.
        reject_as_duplicate: bool,
    }
    type Shared = Arc<Mutex<Relay>>;

    fn event_from(value: &Value) -> Event {
        serde_json::from_value(value.clone()).unwrap()
    }

    /// Fake relay: `/query` answers id lookups and reminder reads (checking the
    /// filter shape the commands send); `/events` stores a publish with NIP-33
    /// replacement; `/info` serves NIP-11.
    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/info",
                get(|State(relay): State<Shared>| async move {
                    Json(relay.lock().unwrap().info.clone())
                }),
            )
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    let mut relay = relay.lock().unwrap();
                    relay.queries.push(filter.clone());
                    if let Some(ids) = filter.get("ids") {
                        let found: Vec<_> = relay
                            .events
                            .iter()
                            .filter(|e| ids.as_array().unwrap().contains(&e["id"]))
                            .cloned()
                            .collect();
                        return Json(json!(found));
                    }
                    assert_eq!(filter["kinds"], json!([KIND_EVENT_REMINDER]));
                    assert_eq!(filter["consistency"], json!("strong"));
                    let author = &filter["authors"][0];
                    let wanted_d = filter.get("#d").map(|d| d[0].clone());
                    let mut found: Vec<Value> = relay
                        .events
                        .iter()
                        .filter(|e| e["kind"] == json!(KIND_EVENT_REMINDER))
                        .filter(|e| &e["pubkey"] == author)
                        .filter(|e| match &wanted_d {
                            Some(d) => e["tags"].as_array().unwrap().iter().any(|t| t[0] == "d" && &t[1] == d),
                            None => true,
                        })
                        .cloned()
                        .collect();
                    found.extend(relay.decoys.iter().cloned());
                    if wanted_d.is_none() {
                        found.truncate(filter["limit"].as_u64().unwrap() as usize);
                    }
                    Json(json!(found))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    let incoming = event_from(&event);
                    let d = d_tag(&incoming).map(str::to_string);
                    let existing = relay
                        .events
                        .iter()
                        .position(|e| {
                            let e = event_from(e);
                            e.pubkey == incoming.pubkey && e.kind == incoming.kind && d_tag(&e).map(str::to_string) == d
                        });
                    let dominated = relay.reject_as_duplicate
                        || existing.is_some_and(|i| !dominates(&incoming, &event_from(&relay.events[i])));
                    if dominated {
                        return Json(json!({"event_id": event["id"], "accepted": true, "message": "duplicate: newer head"}));
                    }
                    match existing {
                        Some(i) => relay.events[i] = event.clone(),
                        None => relay.events.push(event.clone()),
                    }
                    Json(json!({"event_id": event["id"], "accepted": true, "message": ""}))
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup() -> (BuzzClient, Shared) {
        let relay = Arc::new(Mutex::new(Relay {
            events: vec![],
            decoys: vec![],
            info: json!({
                "supported_extensions": ["nip-er"],
                "limitation": {"max_not_before_delta": 31_536_000},
            }),
            queries: vec![],
            reject_as_duplicate: false,
        }));
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    fn reminders(relay: &Shared) -> Vec<Event> {
        relay
            .lock()
            .unwrap()
            .events
            .iter()
            .map(event_from)
            .filter(|e| e.kind == Kind::Custom(KIND_EVENT_REMINDER as u16))
            .collect()
    }

    fn plaintext(client: &BuzzClient, event: &Event) -> Value {
        let keys = client.keys();
        let text = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn tag_values(event: &Event, name: &str) -> Vec<String> {
        event
            .tags
            .iter()
            .map(|t| t.as_slice())
            .filter(|t| t[0] == name)
            .map(|t| t[1].clone())
            .collect()
    }

    fn build_reminder(
        client: &BuzzClient,
        d: &str,
        body: &str,
        extra_tags: Vec<Tag>,
        created_at: u64,
    ) -> Event {
        let keys = client.keys();
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            body,
            nip44::Version::V2,
        )
        .unwrap();
        EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), content)
            .tags([vec![Tag::identifier(d)], extra_tags].concat())
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn seed_reminder(
        client: &BuzzClient,
        relay: &Shared,
        d: &str,
        body: &str,
        extra_tags: Vec<Tag>,
        created_at: u64,
    ) -> Event {
        let event = build_reminder(client, d, body, extra_tags, created_at);
        relay
            .lock()
            .unwrap()
            .events
            .push(serde_json::to_value(&event).unwrap());
        event
    }

    fn pending_body(note: &str) -> String {
        json!({"note": note, "status": "pending"}).to_string()
    }

    fn not_before_tag(at: u64) -> Tag {
        Tag::parse(["not_before", &at.to_string()]).unwrap()
    }

    fn seed_message(relay: &Shared, content: &str, with_channel: bool) -> Event {
        let author = Keys::generate();
        let tags: Vec<Tag> = if with_channel {
            vec![Tag::parse(["h", "11111111-1111-4111-8111-111111111111"]).unwrap()]
        } else {
            vec![]
        };
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags(tags)
            .sign_with_keys(&author)
            .unwrap();
        relay
            .lock()
            .unwrap()
            .events
            .push(serde_json::to_value(&event).unwrap());
        event
    }

    fn add_cmd(event: Option<&str>, at: &str, note: Option<&str>) -> RemindersCmd {
        RemindersCmd::Add {
            event: event.map(str::to_string),
            at: at.to_string(),
            note: note.map(str::to_string),
        }
    }

    fn id_of(response: &Value) -> String {
        response["reminder"].as_str().unwrap().to_string()
    }

    fn exit_code(error: &CliError) -> i32 {
        crate::error::exit_code(error)
    }

    async fn list_all(client: &BuzzClient) -> Vec<Value> {
        let value = run(
            RemindersCmd::List {
                status: ReminderStatusFilter::All,
            },
            client,
            NOW,
        )
        .await
        .unwrap();
        value.as_array().unwrap().clone()
    }

    #[tokio::test]
    async fn add_publishes_one_desktop_format_event_with_a_clipped_target_preview() {
        let (client, relay) = setup().await;
        let long = format!("{}😀tail", "a".repeat(99));
        let message = seed_message(&relay, &long, true);

        let response = run(
            add_cmd(Some(&message.id.to_hex()), "2h", Some("follow up")),
            &client,
            NOW,
        )
        .await
        .unwrap();

        let events = reminders(&relay);
        assert_eq!(events.len(), 1, "one user action is one publish");
        let event = &events[0];
        assert_eq!(event.pubkey, client.keys().public_key());
        let d = tag_values(event, "d");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].len(), 32);
        assert!(d[0].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_eq!(id_of(&response), d[0]);
        assert_eq!(
            tag_values(event, "not_before"),
            vec![(NOW + 7_200).to_string()]
        );
        assert!(tag_values(event, "expiration").is_empty());
        assert_eq!(event.created_at.as_secs(), NOW);
        assert_eq!(
            plaintext(&client, event),
            json!({
                "target": {
                    "eventId": message.id.to_hex(),
                    "channelId": "11111111-1111-4111-8111-111111111111",
                    "preview": "a".repeat(99),
                    "authorPubkey": message.pubkey.to_hex(),
                },
                "note": "follow up",
                "status": "pending",
            })
        );
        let text = nip44::decrypt(
            client.keys().secret_key(),
            &client.keys().public_key(),
            &event.content,
        )
        .unwrap();
        assert!(
            text.starts_with(r#"{"target":{"eventId":"#)
                && text.ends_with(r#""status":"pending"}"#),
            "key order matches desktop's JSON.stringify: {text}"
        );
    }

    #[tokio::test]
    async fn add_note_only_omits_the_target() {
        let (client, relay) = setup().await;
        run(add_cmd(None, "1d", Some("renew passport")), &client, NOW)
            .await
            .unwrap();
        let events = reminders(&relay);
        assert_eq!(
            plaintext(&client, &events[0]),
            json!({"note": "renew passport", "status": "pending"})
        );
        assert_eq!(
            tag_values(&events[0], "not_before"),
            vec![(NOW + 86_400).to_string()]
        );
    }

    #[tokio::test]
    async fn add_refuses_bad_input_before_publishing() {
        let (client, relay) = setup().await;
        let no_channel = seed_message(&relay, "hi", false);
        let missing = EventId::all_zeros().to_hex();
        let horizon = (NOW + 31_536_001).to_string();
        let cases: Vec<(RemindersCmd, i32)> = vec![
            (add_cmd(None, "1h", None), 1),
            (add_cmd(None, "1h", Some("  ")), 1),
            (add_cmd(None, "-5m", Some("x")), 1),
            (add_cmd(None, &(NOW - 1).to_string(), Some("x")), 1),
            (add_cmd(None, &NOW.to_string(), Some("x")), 1),
            (add_cmd(None, "2000-01-01T00:00:00Z", Some("x")), 1),
            (add_cmd(None, &horizon, Some("x")), 1),
            (add_cmd(None, "9007199254740992", Some("x")), 1),
            (add_cmd(None, "tomorrow", Some("x")), 1),
            (add_cmd(Some("zz"), "1h", None), 1),
            (add_cmd(Some(&missing), "1h", None), 1),
            (add_cmd(Some(&no_channel.id.to_hex()), "1h", None), 1),
            (add_cmd(None, "1h", Some(&"n".repeat(70_000))), 1),
        ];
        for (cmd, code) in cases {
            let label = format!("{:?}", matches!(cmd, RemindersCmd::Add { .. }));
            let error = run(cmd, &client, NOW).await.unwrap_err();
            assert_eq!(exit_code(&error), code, "{label}: {error}");
        }
        assert!(reminders(&relay).is_empty(), "nothing may be published");
    }

    #[tokio::test]
    async fn add_refuses_a_relay_that_cannot_validate_reminders() {
        let (client, relay) = setup().await;
        relay.lock().unwrap().info = json!({"supported_extensions": []});
        let error = run(add_cmd(None, "1h", Some("x")), &client, NOW)
            .await
            .unwrap_err();
        assert_eq!(exit_code(&error), 4, "{error}");
        assert!(reminders(&relay).is_empty());

        relay.lock().unwrap().info = json!({
            "supported_extensions": ["nip-er"],
            "limitation": {"max_not_before_delta": 3_600},
        });
        let error = run(add_cmd(None, "2h", Some("x")), &client, NOW)
            .await
            .unwrap_err();
        assert_eq!(
            exit_code(&error),
            1,
            "advertised horizon is enforced: {error}"
        );
        run(add_cmd(None, "30m", Some("x")), &client, NOW)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn add_rejects_a_target_whose_signature_fails() {
        let (client, relay) = setup().await;
        let message = seed_message(&relay, "hello", true);
        {
            let mut relay = relay.lock().unwrap();
            relay.events[0]["content"] = json!("tampered");
        }
        let error = run(
            add_cmd(Some(&message.id.to_hex()), "1h", None),
            &client,
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(exit_code(&error), 4, "{error}");
        assert!(reminders(&relay).is_empty());
    }

    #[tokio::test]
    async fn lifecycle_snooze_done_and_cancel_replace_the_same_address() {
        let (client, relay) = setup().await;
        let first = id_of(
            &run(add_cmd(None, "1h", Some("a")), &client, NOW)
                .await
                .unwrap(),
        );
        let second = id_of(
            &run(add_cmd(None, "3h", Some("b")), &client, NOW)
                .await
                .unwrap(),
        );

        let pending = run(
            RemindersCmd::List {
                status: ReminderStatusFilter::Pending,
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        let ids: Vec<_> = pending
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["reminder"].clone())
            .collect();
        assert_eq!(
            ids,
            vec![json!(first), json!(second)],
            "pending sorts by due time"
        );
        assert_eq!(pending[0]["not_before"], json!(NOW + 3_600));
        assert_eq!(pending[0]["note"], json!("a"));
        assert_eq!(pending[0]["target"], Value::Null);

        let later = NOW + 5 * 3_600;
        let response = run(
            RemindersCmd::Snooze {
                reminder: first.clone(),
                at: later.to_string(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(response["accepted"], json!(true));
        assert_eq!(id_of(&response), first);
        assert_eq!(
            reminders(&relay).len(),
            2,
            "replacement, not a second address"
        );
        let snoozed = list_all(&client).await;
        let snoozed = snoozed
            .iter()
            .find(|r| r["reminder"] == json!(first))
            .unwrap();
        assert_eq!(snoozed["status"], json!("pending"));
        assert_eq!(snoozed["not_before"], json!(later));
        assert_eq!(
            snoozed["created_at"],
            json!(NOW + 1),
            "created_at is head + 1 within the same second"
        );

        let done = run(
            RemindersCmd::Done {
                reminder: first.clone(),
            },
            &client,
            NOW + 10,
        )
        .await
        .unwrap();
        assert_eq!(id_of(&done), first);
        let head = reminders(&relay)
            .into_iter()
            .find(|e| tag_values(e, "d") == vec![first.clone()])
            .unwrap();
        assert!(tag_values(&head, "not_before").is_empty());
        let expiry: u64 = tag_values(&head, "expiration")[0].parse().unwrap();
        let days = (expiry - (NOW + 10)) / 86_400;
        assert!(
            (30..90).contains(&days),
            "expiration is 30-90 days out: {days}"
        );
        assert_eq!(plaintext(&client, &head)["status"], json!("done"));
        assert_eq!(plaintext(&client, &head)["note"], json!("a"));

        run(
            RemindersCmd::Cancel {
                reminder: second.clone(),
            },
            &client,
            NOW + 20,
        )
        .await
        .unwrap();
        assert!(
            relay
                .lock()
                .unwrap()
                .events
                .iter()
                .all(|e| e["kind"] != json!(5)),
            "cancel publishes a replacement and never a deletion"
        );
        let cancelled = run(
            RemindersCmd::List {
                status: ReminderStatusFilter::Cancelled,
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(cancelled.as_array().unwrap().len(), 1);
        assert_eq!(cancelled[0]["reminder"], json!(second));
        assert_eq!(cancelled[0]["not_before"], Value::Null);
        let done_list = run(
            RemindersCmd::List {
                status: ReminderStatusFilter::Done,
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(done_list.as_array().unwrap().len(), 1);
        let still_pending = run(
            RemindersCmd::List {
                status: ReminderStatusFilter::Pending,
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        assert!(still_pending.as_array().unwrap().is_empty());

        let before = reminders(&relay).len();
        for cmd in [
            RemindersCmd::Done {
                reminder: first.clone(),
            },
            RemindersCmd::Cancel {
                reminder: first.clone(),
            },
            RemindersCmd::Snooze {
                reminder: first.clone(),
                at: "1h".into(),
            },
        ] {
            let error = run(cmd, &client, NOW + 30).await.unwrap_err();
            assert_eq!(exit_code(&error), 1, "{error}");
        }
        let unknown = run(
            RemindersCmd::Done {
                reminder: "nope".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(exit_code(&unknown), 1);
        assert!(matches!(unknown, CliError::NotFound(_)));
        assert_eq!(reminders(&relay).len(), before);
    }

    #[tokio::test]
    async fn snooze_rejects_past_times_without_publishing() {
        let (client, relay) = setup().await;
        let d = id_of(
            &run(add_cmd(None, "1h", Some("a")), &client, NOW)
                .await
                .unwrap(),
        );
        let head = reminders(&relay)[0].id;
        for at in [&(NOW - 5).to_string(), "2000-01-01T00:00:00Z", "later"] {
            let error = run(
                RemindersCmd::Snooze {
                    reminder: d.clone(),
                    at: at.to_string(),
                },
                &client,
                NOW,
            )
            .await
            .unwrap_err();
            assert_eq!(exit_code(&error), 1, "{error}");
        }
        assert_eq!(reminders(&relay)[0].id, head);
    }

    #[tokio::test]
    async fn a_dominated_write_exits_with_conflict() {
        let (client, relay) = setup().await;
        let d = id_of(
            &run(add_cmd(None, "1h", Some("a")), &client, NOW)
                .await
                .unwrap(),
        );
        relay.lock().unwrap().reject_as_duplicate = true;
        let error = run(RemindersCmd::Done { reminder: d }, &client, NOW)
            .await
            .unwrap_err();
        assert_eq!(exit_code(&error), 5, "{error}");
    }

    #[tokio::test]
    async fn created_at_is_the_later_of_now_and_head_plus_one() {
        let (client, relay) = setup().await;
        seed_reminder(
            &client,
            &relay,
            "future",
            &pending_body("x"),
            vec![not_before_tag(NOW + 500)],
            NOW + 1_000,
        );
        seed_reminder(
            &client,
            &relay,
            "past",
            &pending_body("y"),
            vec![not_before_tag(NOW + 500)],
            NOW - 1_000,
        );
        run(
            RemindersCmd::Cancel {
                reminder: "future".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        run(
            RemindersCmd::Cancel {
                reminder: "past".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        let at = |d: &str| {
            reminders(&relay)
                .into_iter()
                .find(|e| tag_values(e, "d") == vec![d.to_string()])
                .unwrap()
                .created_at
                .as_secs()
        };
        assert_eq!(at("future"), NOW + 1_001);
        assert_eq!(at("past"), NOW);
    }

    #[tokio::test]
    async fn head_is_the_newest_event_then_the_lowest_id() {
        let (client, relay) = setup().await;
        let older = build_reminder(
            &client,
            "d1",
            &json!({"note": "x", "status": "done"}).to_string(),
            vec![],
            NOW - 100,
        );
        let newer = seed_reminder(
            &client,
            &relay,
            "d1",
            &pending_body("x"),
            vec![not_before_tag(NOW + 60)],
            NOW - 50,
        );
        relay
            .lock()
            .unwrap()
            .decoys
            .push(serde_json::to_value(&older).unwrap());
        run(
            RemindersCmd::Done {
                reminder: "d1".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        let published = reminders(&relay).pop().unwrap();
        assert_ne!(published.id, newer.id);
        assert_eq!(plaintext(&client, &published)["status"], json!("done"));

        // Same created_at: the lower id is the head. The readable event is
        // made the lower one and the loser is unreadable, so only the right
        // tie-break lets the command succeed.
        let (client, relay) = setup().await;
        let (good, bad) = loop {
            let good = build_reminder(
                &client,
                "d2",
                &pending_body("a"),
                vec![not_before_tag(NOW + 60)],
                NOW - 10,
            );
            let bad = build_reminder(&client, "d2", "garbage", vec![], NOW - 10);
            if good.id.to_hex() < bad.id.to_hex() {
                break (good, bad);
            }
        };
        {
            let mut relay = relay.lock().unwrap();
            relay.events.push(serde_json::to_value(&bad).unwrap());
            relay.decoys.push(serde_json::to_value(&good).unwrap());
        }
        run(
            RemindersCmd::Done {
                reminder: "d2".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap();
        let published = reminders(&relay)
            .into_iter()
            .find(|e| e.id != bad.id)
            .unwrap();
        assert_eq!(plaintext(&client, &published)["status"], json!("done"));
    }

    #[tokio::test]
    async fn an_undecodable_head_is_never_overwritten() {
        let (client, relay) = setup().await;
        let desktop_target = json!({
            "target": {"id": "ab".repeat(32), "a": "30023:ab:x"},
            "status": "pending",
        })
        .to_string();
        let bad_bodies = [
            "not json".to_string(),
            desktop_target,
            json!({"status": "pending"}).to_string(),
            json!({"note": null, "status": "pending"}).to_string(),
            json!({"note": "x", "status": "snoozed"}).to_string(),
        ];
        for (i, body) in bad_bodies.iter().enumerate() {
            let d = format!("bad{i}");
            let head = seed_reminder(
                &client,
                &relay,
                &d,
                body,
                vec![not_before_tag(NOW + 60)],
                NOW - 10,
            );
            for cmd in [
                RemindersCmd::Done {
                    reminder: d.clone(),
                },
                RemindersCmd::Cancel {
                    reminder: d.clone(),
                },
                RemindersCmd::Snooze {
                    reminder: d.clone(),
                    at: "1h".into(),
                },
            ] {
                let error = run(cmd, &client, NOW).await.unwrap_err();
                assert_eq!(exit_code(&error), 4, "{body}: {error}");
            }
            let stored: Vec<_> = reminders(&relay)
                .into_iter()
                .filter(|e| tag_values(e, "d") == vec![d.clone()])
                .collect();
            assert_eq!(stored.len(), 1);
            assert_eq!(stored[0].id, head.id);
        }

        let foreign = Keys::generate();
        let forged = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "x")
            .tags([Tag::identifier("alien")])
            .sign_with_keys(&foreign)
            .unwrap();
        relay
            .lock()
            .unwrap()
            .decoys
            .push(serde_json::to_value(&forged).unwrap());
        let error = run(
            RemindersCmd::Done {
                reminder: "alien".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(exit_code(&error), 4, "{error}");
        assert!(error.to_string().contains("not yours"), "{error}");

        let doubled = build_reminder(
            &client,
            "twice",
            &pending_body("x"),
            vec![Tag::identifier("other"), not_before_tag(NOW + 60)],
            NOW - 10,
        );
        relay
            .lock()
            .unwrap()
            .events
            .push(serde_json::to_value(&doubled).unwrap());
        let error = run(
            RemindersCmd::Done {
                reminder: "twice".into(),
            },
            &client,
            NOW,
        )
        .await
        .unwrap_err();
        assert_eq!(exit_code(&error), 4, "two d tags are unreadable: {error}");
        assert!(error.to_string().contains("unreadable"), "{error}");
    }

    #[tokio::test]
    async fn list_is_bounded_and_reports_truncation() {
        let (client, relay) = setup().await;
        for i in 0..3 {
            seed_reminder(
                &client,
                &relay,
                &format!("r{i}"),
                &pending_body("x"),
                vec![not_before_tag(NOW + 60)],
                NOW - 10,
            );
        }
        let all = list(&client, ReminderStatusFilter::All, 3).await.unwrap();
        assert_eq!(all.as_array().unwrap().len(), 3);
        let error = list(&client, ReminderStatusFilter::All, 2)
            .await
            .unwrap_err();
        assert_eq!(exit_code(&error), 4, "{error}");
    }

    #[tokio::test]
    async fn list_reports_readable_reminders_and_skips_the_rest() {
        let (client, relay) = setup().await;
        seed_reminder(
            &client,
            &relay,
            "ok",
            &pending_body("x"),
            vec![not_before_tag(NOW + 60)],
            NOW - 10,
        );
        seed_reminder(&client, &relay, "junk", "garbage", vec![], NOW - 10);
        seed_reminder(
            &client,
            &relay,
            "lax",
            &pending_body("y"),
            vec![Tag::parse(["not_before", "007"]).unwrap()],
            NOW - 10,
        );
        let all = list_all(&client).await;
        let ids: Vec<_> = all
            .iter()
            .map(|r| r["reminder"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec!["ok", "lax"],
            "an invalid not_before reads as absent, as desktop does"
        );
        assert_eq!(all[1]["not_before"], Value::Null);
    }

    #[test]
    fn terminal_expiration_spans_thirty_to_ninety_days() {
        let days: Vec<u64> = (0..2_000)
            .map(|_| (jittered_expiration(NOW) - NOW) / SECONDS_PER_DAY)
            .collect();
        assert_eq!(days.iter().min(), Some(&30));
        assert_eq!(days.iter().max(), Some(&89));
    }

    #[test]
    fn time_input_forms() {
        let ok = |input: &str| parse_time(input, NOW).unwrap();
        assert_eq!(ok("1900000000"), 1_900_000_000);
        assert_eq!(ok("30m"), NOW + 1_800);
        assert_eq!(ok("2h"), NOW + 7_200);
        assert_eq!(ok("1d"), NOW + 86_400);
        assert_eq!(ok("1w"), NOW + 604_800);
        assert_eq!(ok(" 2H "), NOW + 7_200);
        assert_eq!(ok("2030-01-02T09:00:00Z"), 1_893_574_800);
        assert_eq!(ok("2030-01-02T10:00:00+01:00"), 1_893_574_800);
        for bad in [
            "",
            "m",
            "h",
            "1.5h",
            "-1h",
            "+5",
            "10x",
            "2030-01-02",
            "2030-01-02T09:00:00",
            "99999999999999999999",
            "18446744073709551615d",
            "tomorrow",
        ] {
            assert!(parse_time(bad, NOW).is_err(), "{bad:?} must be rejected");
        }
    }

    /// The relay's `validate_not_before` (buzz-relay `handlers/ingest.rs`), copied
    /// so that every value the CLI can publish is shown to pass it.
    fn relay_accepts(tag_value: &str) -> bool {
        const MAX: u64 = 9_007_199_254_740_991;
        if tag_value.is_empty() || !tag_value.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if tag_value.len() > 1 && tag_value.starts_with('0') {
            return false;
        }
        tag_value.parse::<u64>().is_ok_and(|v| v <= MAX)
    }

    #[test]
    fn every_publishable_not_before_passes_the_relay_parser() {
        for raw in [
            "0",
            "1",
            "007",
            "+1",
            "-1",
            " 1",
            "1.0",
            "1e3",
            "",
            "٣",
            "9007199254740991",
            "9007199254740992",
            "18446744073709551616",
        ] {
            assert_eq!(
                parse_not_before(raw).is_some(),
                relay_accepts(raw),
                "{raw:?}"
            );
        }
        for (now, input) in [(NOW, "1h"), (NOW, "1w"), (0, "1"), (NOW, "1900000000")] {
            if let Ok(at) = due_time(input, now, u64::MAX / 2) {
                assert!(relay_accepts(&at.to_string()), "{at}");
            }
        }
        assert!(due_time(&MAX_NOT_BEFORE.to_string(), 0, u64::MAX).is_ok());
        assert!(due_time(&(MAX_NOT_BEFORE + 1).to_string(), 0, u64::MAX).is_err());
    }

    #[test]
    fn preview_clips_like_desktop_without_splitting_pairs() {
        assert_eq!(clip_preview("short"), "short");
        assert_eq!(clip_preview(&"é".repeat(150)), "é".repeat(100));
        let boundary = format!("{}😀", "a".repeat(98));
        assert_eq!(
            clip_preview(&boundary),
            boundary,
            "98 + 2 units fit exactly"
        );
        let over = format!("{}😀", "a".repeat(99));
        assert_eq!(clip_preview(&over), "a".repeat(99));
    }
    const INTEROP_ENV: &str = "BUZZ_REMINDER_INTEROP_DIR";

    /// Writes CLI-produced reminders for the desktop parser check
    /// (`desktop` TypeScript run under node). Skipped unless `INTEROP_ENV` is set.
    #[tokio::test]
    #[ignore = "writes fixtures for the node interop check"]
    async fn write_interop_fixtures() {
        let dir = std::env::var(INTEROP_ENV).expect("set BUZZ_REMINDER_INTEROP_DIR");
        let (client, relay) = setup().await;
        let message = seed_message(&relay, "ship the release notes 🚀", true);
        let with_target = id_of(
            &run(
                add_cmd(Some(&message.id.to_hex()), "2h", Some("check CI")),
                &client,
                NOW,
            )
            .await
            .unwrap(),
        );
        let note_only = id_of(
            &run(add_cmd(None, "1d", Some("renew")), &client, NOW)
                .await
                .unwrap(),
        );
        let snoozed = id_of(
            &run(add_cmd(None, "1h", Some("snooze me")), &client, NOW)
                .await
                .unwrap(),
        );
        let done = id_of(
            &run(add_cmd(None, "1h", Some("finish")), &client, NOW)
                .await
                .unwrap(),
        );
        let cancelled = id_of(
            &run(add_cmd(None, "1h", Some("drop")), &client, NOW)
                .await
                .unwrap(),
        );
        let later = NOW + 86_400 * 3;
        let snooze = RemindersCmd::Snooze {
            reminder: snoozed.clone(),
            at: later.to_string(),
        };
        run(snooze, &client, NOW + 5).await.unwrap();
        run(
            RemindersCmd::Done {
                reminder: done.clone(),
            },
            &client,
            NOW + 5,
        )
        .await
        .unwrap();
        run(
            RemindersCmd::Cancel {
                reminder: cancelled.clone(),
            },
            &client,
            NOW + 5,
        )
        .await
        .unwrap();
        let events: Vec<Value> = reminders(&relay)
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        let doc = json!({
            "secret_key": client.keys().secret_key().to_secret_hex(),
            "pubkey": client.keys().public_key().to_hex(),
            "message": {"id": message.id.to_hex(), "pubkey": message.pubkey.to_hex()},
            "ids": {
                "with_target": with_target, "note_only": note_only, "snoozed": snoozed,
                "done": done, "cancelled": cancelled,
            },
            "snoozed_at": later,
            "events": events,
        });
        std::fs::write(format!("{dir}/cli.json"), doc.to_string()).unwrap();
    }

    /// Reads reminders written by desktop's own `createReminder`,
    /// `snoozeReminder`, `completeReminder` and `cancelReminder`, and acts on them.
    #[tokio::test]
    #[ignore = "reads fixtures produced by the node interop check"]
    async fn read_desktop_fixtures() {
        let dir = std::env::var(INTEROP_ENV).expect("set BUZZ_REMINDER_INTEROP_DIR");
        let doc: Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{dir}/desktop.json")).unwrap())
                .unwrap();
        let keys = Keys::parse(doc["secret_key"].as_str().unwrap()).unwrap();
        let (client, relay) = setup().await;
        let client = BuzzClient::new(client.relay_url().to_string(), keys, None, None).unwrap();
        for event in doc["events"].as_array().unwrap() {
            relay.lock().unwrap().events.push(event.clone());
        }
        let all = list_all(&client).await;
        let by_id = |id: &str| {
            all.iter()
                .find(|r| r["reminder"] == json!(id))
                .unwrap()
                .clone()
        };
        let ids = &doc["ids"];
        let pending = by_id(ids["pending"].as_str().unwrap());
        assert_eq!(pending["status"], json!("pending"));
        assert_eq!(pending["not_before"], doc["expect"]["pending_not_before"]);
        assert_eq!(pending["note"], json!("desktop note"));
        assert_eq!(pending["target"], doc["expect"]["target"]);
        assert_eq!(
            by_id(ids["done"].as_str().unwrap())["status"],
            json!("done")
        );
        assert_eq!(
            by_id(ids["cancelled"].as_str().unwrap())["status"],
            json!("cancelled")
        );
        let snoozed = by_id(ids["snoozed"].as_str().unwrap());
        assert_eq!(snoozed["status"], json!("pending"));
        assert_eq!(snoozed["not_before"], doc["expect"]["snoozed_not_before"]);
        assert_eq!(all.len(), 4, "every desktop-written reminder is readable");

        let now = doc["expect"]["now"].as_u64().unwrap();
        let response = run(
            RemindersCmd::Done {
                reminder: ids["pending"].as_str().unwrap().to_string(),
            },
            &client,
            now,
        )
        .await
        .unwrap();
        assert_eq!(response["accepted"], json!(true));
        let head = reminders(&relay)
            .into_iter()
            .find(|e| tag_values(e, "d") == vec![ids["pending"].as_str().unwrap().to_string()])
            .unwrap();
        assert_eq!(plaintext(&client, &head)["target"], doc["expect"]["target"]);
        let events: Vec<Value> = reminders(&relay)
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        std::fs::write(
            format!("{dir}/cli_after.json"),
            json!({"events": events}).to_string(),
        )
        .unwrap();
    }
}
