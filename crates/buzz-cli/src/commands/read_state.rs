//! Read-only view of the user's own NIP-RS read state (kind:30078,
//! `d=read-state:<slot>`, NIP-44 encrypted to self).
//!
//! Frontiers are decoded and max-merged the way desktop's
//! `readStateFormat.ts` / `readStateSnapshot.ts` do. Unlike desktop, which
//! drops a slot it cannot decode and carries on, a slot this CLI cannot read
//! is an error: an unread count built on a partial merge would overstate
//! unread, never understate it, and the user could not tell.

use std::collections::HashMap;

use buzz_core::kind::KIND_READ_STATE;
use nostr::nips::nip44;
use nostr::Event;
use serde_json::{json, Value};

use crate::client::BuzzClient;
use crate::error::CliError;

const D_TAG_PREFIX: &str = "read-state:";
const T_TAG: &str = "read-state";
const MAX_CONTEXTS: usize = 10_000;
const MAX_CONTEXT_KEY_BYTES: usize = 256;
const MAX_SLOT_ID_LEN: usize = 64;
const MAX_CLIENT_ID_UTF16_UNITS: usize = 64;
/// Relay-side ceiling on one full-state snapshot (NIP-RS, atomic snapshot).
const MAX_SLOTS: u32 = 4096;

/// A slot that was valid but written by a newer schema, so left out of the merge.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SkippedSlot {
    pub d_tag: String,
    pub version: i64,
}

/// Max-merged frontiers of every readable slot.
#[derive(Debug, Default)]
pub(crate) struct ReadState {
    frontiers: HashMap<String, u64>,
    pub skipped: Vec<SkippedSlot>,
}

impl ReadState {
    /// The merged read marker of a context, in unix seconds.
    pub(crate) fn frontier(&self, context: &str) -> Option<u64> {
        self.frontiers.get(context).copied()
    }
}

/// Load and merge every read-state slot of the signed-in identity.
pub(crate) async fn fetch(client: &BuzzClient) -> Result<ReadState, CliError> {
    let filter = json!({
        "kinds": [KIND_READ_STATE],
        "authors": [client.keys().public_key().to_hex()],
        "#t": [T_TAG],
        "consistency": "strong",
    });
    let events = client.query_all_bounded(filter, MAX_SLOTS).await?;
    merge(client.keys(), events)
}

fn unreadable(event: &Event, detail: impl std::fmt::Display) -> CliError {
    CliError::Other(format!(
        "your read-state slot {} is unreadable ({detail}); refusing to report unread counts from a partial read state",
        event.id.to_hex()
    ))
}

fn single_d_tag(event: &Event) -> Option<&str> {
    let mut d = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(String::as_str) == Some("d"));
    let tag = d.next()?;
    if d.next().is_some() {
        return None;
    }
    tag.as_slice().get(1).map(String::as_str)
}

fn is_slot_coordinate(event: &Event) -> Option<&str> {
    let d = single_d_tag(event)?;
    let slot = d.strip_prefix(D_TAG_PREFIX)?;
    if slot.is_empty() || slot.len() > MAX_SLOT_ID_LEN || !slot.is_ascii() {
        return None;
    }
    let t_count = event
        .tags
        .iter()
        .filter(|t| matches!(t.as_slice(), [k, v, ..] if k == "t" && v == T_TAG))
        .count();
    (t_count == 1).then_some(d)
}

/// JS `Number.isInteger` within the uint32 range.
fn uint32(value: &Value) -> Option<u64> {
    let n = value.as_u64().or_else(|| {
        let f = value.as_f64()?;
        (f >= 0.0 && f.fract() == 0.0 && f <= u32::MAX as f64).then_some(f as u64)
    })?;
    (n <= u64::from(u32::MAX)).then_some(n)
}

/// Decode one blob. `Ok(Err(version))` is a valid blob of an unknown schema.
fn decode(event: &Event, plaintext: &str) -> Result<Result<HashMap<String, u64>, i64>, CliError> {
    let blob: Value = serde_json::from_str(plaintext).map_err(|e| unreadable(event, e))?;
    let Some(blob) = blob.as_object() else {
        return Err(unreadable(event, "not a JSON object"));
    };
    let version = blob
        .get("v")
        .and_then(Value::as_f64)
        .filter(|v| v.fract() == 0.0);
    match version {
        Some(1.0) => {}
        Some(other) => return Ok(Err(other as i64)),
        None => return Err(unreadable(event, "missing or non-integer schema version")),
    }
    let client_id_ok = blob
        .get("client_id")
        .and_then(Value::as_str)
        .is_some_and(|id| (1..=MAX_CLIENT_ID_UTF16_UNITS).contains(&id.encode_utf16().count()));
    if !client_id_ok {
        return Err(unreadable(event, "invalid client_id"));
    }
    let Some(contexts) = blob.get("contexts").and_then(Value::as_object) else {
        return Err(unreadable(event, "contexts is not an object"));
    };
    if contexts.len() > MAX_CONTEXTS {
        return Err(unreadable(event, "too many contexts"));
    }
    Ok(Ok(contexts
        .iter()
        .filter(|(key, _)| key.len() <= MAX_CONTEXT_KEY_BYTES)
        .filter_map(|(key, value)| Some((key.clone(), uint32(value)?)))
        .collect()))
}

/// Verify, decrypt and max-merge raw relay events. Events that are not read-state
/// coordinates are ignored; a coordinate that cannot be read is an error.
pub(crate) fn merge(keys: &nostr::Keys, raw: Vec<Value>) -> Result<ReadState, CliError> {
    let me = keys.public_key();
    let mut heads: HashMap<String, Event> = HashMap::new();
    for value in raw {
        let event: Event = serde_json::from_value(value)
            .map_err(|e| CliError::Other(format!("relay returned a malformed event: {e}")))?;
        if event.pubkey != me || event.kind.as_u16() as u32 != KIND_READ_STATE {
            return Err(CliError::Other(format!(
                "relay returned event {} that is not your read state",
                event.id.to_hex()
            )));
        }
        event.verify().map_err(|_| {
            CliError::Other(format!(
                "relay returned event {} with an invalid signature",
                event.id.to_hex()
            ))
        })?;
        let Some(d) = is_slot_coordinate(&event) else {
            continue;
        };
        let d = d.to_string();
        let newer = heads
            .get(&d)
            .is_none_or(|head| (event.created_at, head.id) > (head.created_at, event.id));
        if newer {
            heads.insert(d, event);
        }
    }

    let mut state = ReadState::default();
    let mut ordered: Vec<_> = heads.into_iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    for (d_tag, event) in ordered {
        let plaintext = nip44::decrypt(keys.secret_key(), &me, &event.content)
            .map_err(|e| unreadable(&event, e))?;
        match decode(&event, &plaintext)? {
            Ok(contexts) => {
                for (key, ts) in contexts {
                    let slot = state.frontiers.entry(key).or_insert(0);
                    *slot = (*slot).max(ts);
                }
            }
            Err(version) => state.skipped.push(SkippedSlot { d_tag, version }),
        }
    }
    Ok(state)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    pub(crate) fn slot_event(keys: &Keys, slot: &str, plaintext: &str, at: u64) -> Value {
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), content)
            .tags([
                Tag::identifier(format!("{D_TAG_PREFIX}{slot}")),
                Tag::hashtag(T_TAG),
            ])
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(keys)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    fn blob(contexts: Value) -> String {
        json!({"v": 1, "client_id": "dev", "contexts": contexts}).to_string()
    }

    #[test]
    fn max_merges_every_slot_and_ignores_override_and_foreign_entries() {
        let keys = Keys::generate();
        let state = merge(
            &keys,
            vec![
                slot_event(&keys, "a", &blob(json!({"c1": 10, "c2": 50, "ov_s:c1": 3, "esc:ov_c:x": 9})), 1),
                slot_event(&keys, "b", &blob(json!({"c1": 20, "c2": 5, "c3": 1.0e3, "neg": -1, "k".repeat(257): 9, "big": 4294967296u64, "frac": 1.5, "str": "7"})), 2),
            ],
        )
        .unwrap();
        assert_eq!(state.frontier("c1"), Some(20));
        assert_eq!(state.frontier("c2"), Some(50));
        assert_eq!(state.frontier("c3"), Some(1000));
        assert_eq!(state.frontier(&"k".repeat(257)), None);
        for dropped in ["neg", "big", "frac", "str", "missing"] {
            assert_eq!(state.frontier(dropped), None, "{dropped}");
        }
        assert!(state.skipped.is_empty());
    }

    #[test]
    fn newest_version_of_a_coordinate_wins() {
        let keys = Keys::generate();
        let old = slot_event(&keys, "a", &blob(json!({"c1": 99})), 1);
        let new = slot_event(&keys, "a", &blob(json!({"c1": 5})), 2);
        let state = merge(&keys, vec![new, old]).unwrap();
        assert_eq!(state.frontier("c1"), Some(5));
    }

    #[test]
    fn an_unreadable_slot_is_an_error_not_an_empty_state() {
        let keys = Keys::generate();
        let good = slot_event(&keys, "a", &blob(json!({"c1": 1})), 1);
        for bad in [
            "not json",
            "[]",
            r#"{"client_id":"d","contexts":{}}"#,
            r#"{"v":1,"client_id":"","contexts":{}}"#,
            r#"{"v":1,"client_id":"d","contexts":[]}"#,
            r#"{"v":"1","client_id":"d","contexts":{}}"#,
        ] {
            let err =
                merge(&keys, vec![good.clone(), slot_event(&keys, "b", bad, 2)]).expect_err(bad);
            assert!(err.to_string().contains("unreadable"), "{bad}: {err}");
        }

        let other = Keys::generate();
        let foreign = slot_event(&other, "b", &blob(json!({})), 2);
        let err = merge(&keys, vec![foreign]).unwrap_err();
        assert!(err.to_string().contains("not your read state"), "{err}");

        let mut tampered = slot_event(&keys, "b", &blob(json!({"c1": 1})), 2);
        tampered["created_at"] = json!(3);
        assert!(merge(&keys, vec![tampered]).is_err());

        let mut undecryptable = slot_event(&keys, "b", &blob(json!({})), 2);
        let wrong = slot_event(&other, "b", &blob(json!({})), 2);
        undecryptable["content"] = wrong["content"].clone();
        assert!(merge(&keys, vec![undecryptable]).is_err());
    }

    #[test]
    fn a_newer_schema_is_reported_not_merged() {
        let keys = Keys::generate();
        let future = json!({"v": 2, "client_id": "d", "contexts": {"c1": 99}}).to_string();
        let state = merge(&keys, vec![slot_event(&keys, "a", &future, 1)]).unwrap();
        assert_eq!(state.frontier("c1"), None);
        assert_eq!(
            state.skipped,
            vec![SkippedSlot {
                d_tag: "read-state:a".into(),
                version: 2
            }]
        );
    }

    #[test]
    fn non_coordinates_are_ignored() {
        let keys = Keys::generate();
        let unrelated = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), "x")
            .tags([Tag::identifier("channel-sections"), Tag::hashtag(T_TAG)])
            .sign_with_keys(&keys)
            .unwrap();
        let no_t_event = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), "x")
            .tags([Tag::identifier("read-state:a")])
            .sign_with_keys(&keys)
            .unwrap();
        let state = merge(
            &keys,
            vec![
                serde_json::to_value(unrelated).unwrap(),
                serde_json::to_value(no_t_event).unwrap(),
            ],
        )
        .unwrap();
        assert!(state.frontiers.is_empty());
    }
}
