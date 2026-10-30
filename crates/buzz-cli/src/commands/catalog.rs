//! Shared head selection and field hygiene for the read-only persona and team
//! catalogs (kinds 30175, 30176, 30178), mirroring desktop's native catalog
//! fetch: verify every signature, keep the newest event per `(author, d)`
//! coordinate, and only then judge visibility and content.

use std::collections::HashSet;

use nostr::Event;
use serde_json::Value;

use crate::client::BuzzClient;
use crate::error::CliError;

/// Desktop walks at most 40 pages of 500; a larger catalog is an error there
/// and here, never a silently short list.
pub(crate) const MAX_CATALOG_EVENTS: u32 = 20_000;
const MAX_HTTP_AVATAR_LENGTH: usize = 2_048;

/// A coordinate head: the newest event for one `(author, d)` pair.
pub(crate) struct Head {
    pub event: Event,
    pub d_tag: String,
}

/// Fetch every event of `kind` (optionally for one author) and return the
/// coordinate heads, newest first.
///
/// Events that are unparsable, unsigned, of another kind, or by another author
/// never become heads; their count is reported on stderr.
pub(crate) async fn fetch_heads(
    client: &BuzzClient,
    kind: u32,
    author: Option<&str>,
) -> Result<Vec<Head>, CliError> {
    let mut filter = serde_json::json!({ "kinds": [kind] });
    if let Some(author) = author {
        filter["authors"] = serde_json::json!([author]);
    }
    let raw = client.query_all_bounded(filter, MAX_CATALOG_EVENTS).await?;
    Ok(select_heads(raw, kind, author))
}

pub(crate) fn select_heads(raw: Vec<Value>, kind: u32, author: Option<&str>) -> Vec<Head> {
    let total = raw.len();
    let mut events: Vec<Event> = raw
        .into_iter()
        .filter_map(|value| serde_json::from_value::<Event>(value).ok())
        .filter(|event| {
            u32::from(event.kind.as_u16()) == kind
                && author.is_none_or(|author| event.pubkey.to_hex() == author)
                && event.verify().is_ok()
        })
        .collect();
    if events.len() < total {
        eprintln!(
            "warning: ignored {} kind:{kind} events that failed parsing or signature checks",
            total - events.len()
        );
    }
    events.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut claimed = HashSet::new();
    let mut heads = Vec::new();
    for event in events {
        let Some(d_tag) = single_tag(&event, "d").filter(|d| !d.is_empty()) else {
            continue;
        };
        if claimed.insert((event.pubkey, d_tag.clone())) {
            heads.push(Head { event, d_tag });
        }
    }
    heads
}

/// A tag's value, only when the event carries exactly one tag of that name.
pub(crate) fn single_tag(event: &Event, name: &str) -> Option<String> {
    let mut values = event.tags.iter().filter_map(|tag| {
        let tag = tag.as_slice();
        (tag.len() >= 2 && tag[0] == name).then(|| tag[1].clone())
    });
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

/// A non-blank string field; anything else is absent.
pub(crate) fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

/// Only plain http(s) avatar URLs survive; inline `data:` images are dropped.
pub(crate) fn safe_avatar(value: Option<&Value>) -> Option<String> {
    let value = value.and_then(Value::as_str)?;
    let safe = value.len() <= MAX_HTTP_AVATAR_LENGTH
        && !value.chars().any(char::is_whitespace)
        && !value.contains(['(', ')'])
        && url::Url::parse(value)
            .ok()
            .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    safe.then(|| value.to_owned())
}

/// Report catalog entries skipped for malformed content, so a bad head is
/// never mistaken for an absent one.
pub(crate) fn warn_skipped(what: &str, skipped: &[String]) {
    if !skipped.is_empty() {
        eprintln!(
            "warning: skipped {} {what} with unreadable content: {}",
            skipped.len(),
            skipped.join(", ")
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Arc, Mutex};

    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    use crate::client::BuzzClient;

    #[derive(Default)]
    pub struct Relay {
        pub events: Vec<Value>,
        pub filters: Vec<Value>,
    }
    pub type Shared = Arc<Mutex<Relay>>;

    fn matches(event: &Value, filter: &Value) -> bool {
        let in_list = |key: &str, actual: &Value| {
            filter.get(key).is_none_or(|wanted| {
                wanted
                    .as_array()
                    .is_some_and(|wanted| wanted.contains(actual))
            })
        };
        let tag_has = |name: &str, wanted: &[Value]| {
            event["tags"].as_array().is_some_and(|tags| {
                tags.iter()
                    .any(|tag| tag[0] == json!(name) && wanted.contains(&tag[1]))
            })
        };
        in_list("kinds", &event["kind"])
            && in_list("authors", &event["pubkey"])
            && filter["#p"]
                .as_array()
                .is_none_or(|wanted| tag_has("p", wanted))
            && filter["since"]
                .as_u64()
                .is_none_or(|since| event["created_at"].as_u64() >= Some(since))
    }

    /// A fake `/query` that applies kinds/authors/#p/since, returns newest
    /// first, and honours the composite `(until, before_id)` cursor and `limit`
    /// the production pager sends. Every filter received is recorded.
    pub async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    let mut relay = relay.lock().unwrap();
                    relay.filters.push(filter.clone());
                    let mut hits: Vec<Value> = relay
                        .events
                        .iter()
                        .filter(|event| matches(event, filter))
                        .cloned()
                        .collect();
                    hits.sort_by(|a, b| {
                        b["created_at"]
                            .as_u64()
                            .cmp(&a["created_at"].as_u64())
                            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
                    });
                    if let (Some(until), Some(before)) =
                        (filter["until"].as_u64(), filter["before_id"].as_str())
                    {
                        hits.retain(|e| {
                            let at = e["created_at"].as_u64().unwrap();
                            at < until || (at == until && e["id"].as_str().unwrap() > before)
                        });
                    }
                    hits.truncate(filter["limit"].as_u64().unwrap_or(500) as usize);
                    Json(json!(hits))
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

    pub fn event(
        keys: &Keys,
        kind: u32,
        content: &str,
        tags: &[&[&str]],
        created_at: u64,
    ) -> Value {
        let tags: Vec<Tag> = tags
            .iter()
            .map(|t| Tag::parse(t.iter().copied()).unwrap())
            .collect();
        let event = EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    pub fn seed(relay: &Shared, event: Value) -> Value {
        relay.lock().unwrap().events.push(event.clone());
        event
    }

    /// The kinds carried by every filter the relay received.
    pub fn every_filter_names_kinds(relay: &Shared) -> bool {
        let relay = relay.lock().unwrap();
        !relay.filters.is_empty()
            && relay
                .filters
                .iter()
                .all(|f| f["kinds"].as_array().is_some_and(|k| !k.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::event;
    use super::*;
    use nostr::Keys;

    #[test]
    fn heads_ignore_events_of_another_kind_or_author() {
        let (mine, theirs) = (Keys::generate(), Keys::generate());
        let raw = vec![
            event(&mine, 30176, "{}", &[&["d", "a"]], 1),
            event(&theirs, 30176, "{}", &[&["d", "b"]], 1),
            event(&mine, 30175, "{}", &[&["d", "c"]], 1),
        ];
        let heads = select_heads(raw, 30176, Some(&mine.public_key().to_hex()));
        let ids: Vec<&str> = heads.iter().map(|head| head.d_tag.as_str()).collect();
        assert_eq!(ids, ["a"]);
    }
}
