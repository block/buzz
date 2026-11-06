//! Read and write path shared by every synced sidebar document (kind:30078,
//! NIP-44 encrypted to yourself): one verified head read, one signed publish.

use std::time::SystemTime;

use buzz_core::kind::KIND_READ_STATE;
use nostr::nips::nip44;
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde_json::{json, Value};

use super::parse_write_response;
use crate::client::BuzzClient;
use crate::error::CliError;

/// Relay event size ceiling shared with desktop's sidebar blobs.
const MAX_CIPHERTEXT_BYTES: usize = 65_535;

/// One synced document: its `d` tag and the name used in messages.
#[derive(Clone, Copy)]
pub(crate) struct Lane {
    pub d_tag: &'static str,
    pub label: &'static str,
}

/// The decoded head. `doc` is `None` when the relay holds no event yet.
pub(crate) struct Head<T> {
    pub created_at: u64,
    pub doc: Option<T>,
}

pub(crate) fn now() -> std::time::Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}

/// Channel ids in the lowercase hyphenated form desktop keys entries by.
pub(crate) fn channel_ids(channels: &[String]) -> Result<Vec<String>, CliError> {
    channels
        .iter()
        .map(|channel| {
            uuid::Uuid::parse_str(channel.trim())
                .map(|id| id.hyphenated().to_string())
                .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel}")))
        })
        .collect()
}

/// The relay's current event for `lane` and this identity, decrypted and
/// passed to `decode` with the event's `created_at`. An absent head is
/// `doc: None`; an unreadable one is an error, never an empty document. The
/// read is pinned to the writer (`consistency: strong`): a write derives a
/// full snapshot from it, and a lagging replica would drop the caller's own
/// just-accepted edits.
pub(crate) async fn fetch_head<T>(
    client: &BuzzClient,
    lane: Lane,
    decode: impl FnOnce(&Value, u64) -> Option<T>,
) -> Result<Head<T>, CliError> {
    let me = client.keys().public_key();
    let filter = json!({
        "kinds": [KIND_READ_STATE],
        "authors": [me.to_hex()],
        "#d": [lane.d_tag],
        "limit": 1,
        "consistency": "strong",
    });
    let raw = client.query(&filter).await?;
    let events: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse {} query: {e}", lane.label)))?;
    let returned = events.len();
    let head = events
        .into_iter()
        .filter_map(|event| serde_json::from_value::<nostr::Event>(event).ok())
        .filter(|event| {
            event.pubkey == me
                && event.tags.identifier() == Some(lane.d_tag)
                && event.verify().is_ok()
        })
        .min_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.to_hex().cmp(&b.id.to_hex()))
        });
    let Some(event) = head else {
        if returned > 0 {
            return Err(CliError::Other(format!(
                "the relay returned a {} event that is not yours or fails verification; refusing to overwrite it",
                lane.label
            )));
        }
        return Ok(Head {
            created_at: 0,
            doc: None,
        });
    };

    let unreadable = |detail: String| {
        CliError::Other(format!(
            "your {} event {} is unreadable ({detail}); refusing to read or overwrite it",
            lane.label,
            event.id.to_hex()
        ))
    };
    let plaintext = nip44::decrypt(client.keys().secret_key(), &me, &event.content)
        .map_err(|e| unreadable(e.to_string()))?;
    let json: Value = serde_json::from_str(&plaintext).map_err(|e| unreadable(e.to_string()))?;
    let created_at = event.created_at.as_secs();
    let doc = decode(&json, created_at).ok_or_else(|| {
        unreadable("unsupported format; a newer client may have written it".into())
    })?;
    Ok(Head {
        created_at,
        doc: Some(doc),
    })
}

/// Encrypts `plaintext` to yourself and publishes it as the new head, one
/// second after `head_created_at` at the earliest. A relay `duplicate:` reply
/// is a conflict.
pub(crate) async fn publish(
    client: &BuzzClient,
    lane: Lane,
    plaintext: String,
    head_created_at: u64,
) -> Result<Value, CliError> {
    let keys = client.keys();
    let too_large = || {
        CliError::Usage(format!(
            "{} would exceed the relay's event size limit",
            lane.label
        ))
    };
    // NIP-44 rejects longer plaintexts outright.
    if plaintext.len() > u16::MAX as usize {
        return Err(too_large());
    }
    let content = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|e| CliError::Other(format!("failed to encrypt {}: {e}", lane.label)))?;
    if content.len() > MAX_CIPHERTEXT_BYTES {
        return Err(too_large());
    }
    let created_at = now().as_secs().max(head_created_at + 1);
    let builder = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), content)
        .tags([Tag::identifier(lane.d_tag), Tag::hashtag(lane.d_tag)])
        .custom_created_at(Timestamp::from(created_at));
    let raw = client.submit_event(client.sign_event(builder)?).await?;
    let normalized = parse_write_response(
        &raw,
        &format!("a newer {} update won; run the command again", lane.label),
    )?;
    serde_json::from_str(&normalized)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))
}

/// The response of a command that changed nothing and published nothing.
pub(crate) fn unchanged() -> Value {
    json!({"event_id": null, "accepted": true, "message": "unchanged"})
}

/// Shared fake relay and signing helpers for the sidebar command tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use nostr::Keys;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    pub struct Relay {
        pub events: Vec<Value>,
        pub duplicate: bool,
        /// Answer every query with all stored events, whoever authored them.
        pub leaky: bool,
    }
    pub type Shared = Arc<Mutex<Relay>>;

    /// Fake relay: `/query` checks the head filter and returns every stored
    /// event matching its author and requested d-tag (more than the relay's
    /// `limit`, so head selection is exercised); `/events` stores one.
    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    assert_eq!(filter["kinds"], json!([KIND_READ_STATE]));
                    assert_eq!(filter["limit"], json!(1));
                    assert_eq!(filter["consistency"], json!("strong"));
                    let author = &filter["authors"][0];
                    let d_tag = &filter["#d"][0];
                    let relay = relay.lock().unwrap();
                    let matching: Vec<_> = relay
                        .events
                        .iter()
                        .filter(|e| {
                            relay.leaky || (&e["pubkey"] == author && &e["tags"][0][1] == d_tag)
                        })
                        .cloned()
                        .collect();
                    Json(json!(matching))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    let message = if relay.duplicate {
                        "duplicate: newer head"
                    } else {
                        ""
                    };
                    if !relay.duplicate {
                        relay.events.push(event.clone());
                    }
                    Json(json!({"event_id": event["id"], "accepted": true, "message": message}))
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    pub async fn setup() -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    pub fn seed_at(
        client: &BuzzClient,
        relay: &Shared,
        lane: Lane,
        plaintext: &str,
        created_at: u64,
    ) -> Value {
        let keys = client.keys();
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), content)
            .tags([Tag::identifier(lane.d_tag)])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap();
        let event = serde_json::to_value(event).unwrap();
        relay.lock().unwrap().events.push(event.clone());
        event
    }

    /// The decrypted document of the newest published event.
    pub fn published(client: &BuzzClient, relay: &Shared) -> (nostr::Event, Value) {
        let raw = relay.lock().unwrap().events.last().cloned().unwrap();
        let event: nostr::Event = serde_json::from_value(raw).unwrap();
        let keys = client.keys();
        let plaintext =
            nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content).unwrap();
        (event, serde_json::from_str(&plaintext).unwrap())
    }

    pub fn published_count(relay: &Shared) -> usize {
        relay.lock().unwrap().events.len()
    }
}
