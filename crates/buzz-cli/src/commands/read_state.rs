//! Read-only view of the user's own NIP-RS read state (kind:30078,
//! `d=read-state:<slot>`, NIP-44 encrypted to self).
//!
//! Frontiers are decoded and max-merged the way desktop's
//! `readStateFormat.ts` / `readStateSnapshot.ts` do. Unlike desktop, which
//! drops a slot it cannot decode and carries on, a slot this CLI cannot read
//! is an error: an unread count built on a partial merge would overstate
//! unread, never understate it, and the user could not tell.

use std::collections::{HashMap, HashSet};

use buzz_core::kind::KIND_READ_STATE;
use nostr::nips::nip44;
use nostr::Event;
use serde_json::{json, Value};

use crate::client::BuzzClient;
use crate::error::CliError;

const D_TAG_PREFIX: &str = "read-state:";
const T_TAG: &str = "read-state";
pub(super) const MAX_CONTEXTS: usize = 10_000;
const MAX_CONTEXT_KEY_BYTES: usize = 256;
const MAX_SLOT_ID_LEN: usize = 64;
const MAX_CLIENT_ID_UTF16_UNITS: usize = 64;
const SNAPSHOT_VERSION: u64 = 1;

/// A slot that was valid but written by a newer schema, so left out of the merge.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SkippedSlot {
    pub d_tag: String,
    pub version: i64,
}

/// One decoded slot at its coordinate.
#[derive(Debug)]
pub(crate) struct Slot {
    pub created_at: u64,
    pub client_id: String,
    pub contexts: HashMap<String, u64>,
}

/// Merged manual-unread override register of one context: set counter,
/// clear counter, baseline.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Register {
    pub set: u64,
    pub clear: u64,
    pub baseline: u64,
}

impl Register {
    /// NIP-RS liveness predicate: `S > 0 && F <= B && S > C`.
    pub(crate) fn is_active(&self, frontier: u64) -> bool {
        self.set > 0 && frontier <= self.baseline && self.set > self.clear
    }
}

/// Max-merged frontiers and override registers of every readable slot.
#[derive(Debug, Default)]
pub(crate) struct ReadState {
    frontiers: HashMap<String, u64>,
    overrides: HashMap<String, Register>,
    slots: HashMap<String, Slot>,
    pub skipped: Vec<SkippedSlot>,
}

impl ReadState {
    /// The merged read marker of a context, in unix seconds.
    pub(crate) fn frontier(&self, context: &str) -> Option<u64> {
        self.frontiers.get(context).copied()
    }

    /// The merged override register of a context, if any slot carries one.
    pub(crate) fn register(&self, context: &str) -> Option<Register> {
        self.overrides.get(context).copied()
    }

    /// The decoded slot at `read-state:<slot_id>`.
    pub(crate) fn slot(&self, slot_id: &str) -> Option<&Slot> {
        self.slots.get(slot_id)
    }

    /// Whether a slot at `read-state:<slot_id>` exists but was left out of the
    /// merge because its schema version is unknown.
    pub(crate) fn is_skipped(&self, slot_id: &str) -> bool {
        let d_tag = format!("{D_TAG_PREFIX}{slot_id}");
        self.skipped.iter().any(|s| s.d_tag == d_tag)
    }
}

/// Load and merge every read-state slot of the signed-in identity.
///
/// Uses the relay's atomic snapshot (NIP-RS, Buzz Atomic Snapshot Extension):
/// paginating by cursor cannot see a slot another client replaces mid-load, so
/// a relay without the extension is an error rather than a partial read.
pub(crate) async fn fetch(client: &BuzzClient) -> Result<ReadState, CliError> {
    let community_id = discover_snapshot(client).await?;
    let me = client.keys().public_key().to_hex();
    let filter = json!({
        "kinds": [KIND_READ_STATE],
        "authors": [me],
        "read_state_snapshot": SNAPSHOT_VERSION,
    });
    let raw = client.query(&filter).await?;
    let events = snapshot_events(&raw, &community_id, &me)?;
    merge(client.keys(), events)
}

fn cannot_prove_complete(detail: impl std::fmt::Display) -> CliError {
    CliError::Other(format!(
        "cannot prove your read state is complete: {detail}; refusing to continue"
    ))
}

/// The community id a relay advertises for its atomic read-state snapshot.
async fn discover_snapshot(client: &BuzzClient) -> Result<String, CliError> {
    let raw = client.get_public("/info").await?;
    let info: Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid NIP-11 response: {e}")))?;
    let descriptor = info
        .get("read_state_snapshot")
        .filter(|d| d.is_object())
        .ok_or_else(|| {
            cannot_prove_complete("this relay does not offer the atomic read_state_snapshot")
        })?;
    if descriptor.get("version").and_then(Value::as_u64) != Some(SNAPSHOT_VERSION) {
        return Err(cannot_prove_complete(
            "this relay offers an unsupported read_state_snapshot version",
        ));
    }
    descriptor
        .get("community_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| cannot_prove_complete("the relay advertised no community_id"))
}

/// Validate the snapshot envelope and return its events. Anything but a
/// complete, version-1 envelope for this identity and community is rejected,
/// including the plain event array an older relay returns when it ignores the
/// extension.
fn snapshot_events(raw: &str, community_id: &str, me: &str) -> Result<Vec<Value>, CliError> {
    let mut envelope: Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    let field = |name: &str| envelope.get(name);
    if field("read_state_snapshot").and_then(Value::as_u64) != Some(SNAPSHOT_VERSION)
        || field("complete") != Some(&Value::Bool(true))
    {
        return Err(cannot_prove_complete(
            "the relay did not return a complete read_state_snapshot",
        ));
    }
    if field("community_id").and_then(Value::as_str) != Some(community_id)
        || field("pubkey").and_then(Value::as_str) != Some(me)
    {
        return Err(cannot_prove_complete(
            "the snapshot belongs to a different community or identity",
        ));
    }
    match envelope.get_mut("events").map(Value::take) {
        Some(Value::Array(events)) => Ok(events),
        _ => Err(cannot_prove_complete("the snapshot has no events array")),
    }
}

fn unreadable(event: &Event, detail: impl std::fmt::Display) -> CliError {
    CliError::Other(format!(
        "your read-state slot {} is unreadable ({detail}); refusing to continue from a partial read state",
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

struct Decoded {
    client_id: String,
    contexts: HashMap<String, u64>,
    overrides: HashMap<String, Register>,
}

/// Collect override groups from the raw entries, before the generic per-entry
/// discard: a group is a live triple or a lone `ov_c` floor with valid uint32
/// values, and anything else is rejected whole.
fn override_groups(contexts: &serde_json::Map<String, Value>) -> HashMap<String, Register> {
    let mut groups: HashMap<&str, [Option<&Value>; 3]> = HashMap::new();
    for (key, value) in contexts {
        for (slot, prefix) in ["ov_s:", "ov_c:", "ov_b:"].into_iter().enumerate() {
            if let Some(ctx) = key.strip_prefix(prefix) {
                groups.entry(ctx).or_default()[slot] = Some(value);
            }
        }
    }
    groups
        .into_iter()
        .filter_map(|(ctx, [set, clear, baseline])| {
            let register = match (set, clear, baseline) {
                (Some(s), Some(c), Some(b)) => Register {
                    set: uint32(s)?,
                    clear: uint32(c)?,
                    baseline: uint32(b)?,
                },
                (None, Some(c), None) => Register {
                    clear: uint32(c)?,
                    ..Register::default()
                },
                _ => return None,
            };
            Some((ctx.to_string(), register))
        })
        .collect()
}

/// Decode one blob. `Ok(Err(version))` is a valid blob of an unknown schema.
fn decode(event: &Event, plaintext: &str) -> Result<Result<Decoded, i64>, CliError> {
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
    let client_id = blob["client_id"].as_str().unwrap_or_default().to_string();
    Ok(Ok(Decoded {
        client_id,
        overrides: override_groups(contexts),
        contexts: contexts
            .iter()
            .filter(|(key, _)| key.len() <= MAX_CONTEXT_KEY_BYTES)
            .filter_map(|(key, value)| Some((key.clone(), uint32(value)?)))
            .collect(),
    }))
}

/// Verify, decrypt and max-merge raw relay events. Events that are not read-state
/// coordinates are ignored; a coordinate that cannot be read is an error, and so
/// is one coordinate held by more than one event.
pub(crate) fn merge(keys: &nostr::Keys, raw: Vec<Value>) -> Result<ReadState, CliError> {
    let me = keys.public_key();
    let mut heads: HashMap<String, Event> = HashMap::new();
    let mut coordinates: HashSet<String> = HashSet::new();
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
        if let Some(d) = single_d_tag(&event) {
            if !coordinates.insert(d.to_string()) {
                return Err(CliError::Other(format!(
                    "relay returned a duplicate or conflicting coordinate {d:?} in the read-state snapshot; refusing to pick one"
                )));
            }
        }
        let Some(d) = is_slot_coordinate(&event) else {
            continue;
        };
        heads.insert(d.to_string(), event);
    }

    let mut state = ReadState::default();
    let mut ordered: Vec<_> = heads.into_iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    for (d_tag, event) in ordered {
        let plaintext = nip44::decrypt(keys.secret_key(), &me, &event.content)
            .map_err(|e| unreadable(&event, e))?;
        match decode(&event, &plaintext)? {
            Ok(decoded) => {
                for (key, ts) in &decoded.contexts {
                    let slot = state.frontiers.entry(key.clone()).or_insert(0);
                    *slot = (*slot).max(*ts);
                }
                for (ctx, register) in decoded.overrides {
                    let merged = state.overrides.entry(ctx).or_default();
                    merged.set = merged.set.max(register.set);
                    merged.clear = merged.clear.max(register.clear);
                    merged.baseline = merged.baseline.max(register.baseline);
                }
                let slot_id = d_tag.strip_prefix(D_TAG_PREFIX).unwrap_or(&d_tag);
                state.slots.insert(
                    slot_id.to_string(),
                    Slot {
                        created_at: event.created_at.as_secs(),
                        client_id: decoded.client_id,
                        contexts: decoded.contexts,
                    },
                );
            }
            Err(version) => state.skipped.push(SkippedSlot { d_tag, version }),
        }
    }
    Ok(state)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::get, routing::post, Json, Router};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    pub(crate) const COMMUNITY: &str = "5f0c1b9e-2d0b-4f4e-9a7c-0d6f3f3f3f3f";

    pub(crate) fn snapshot_envelope(pubkey: &str, events: Vec<Value>) -> Value {
        json!({
            "read_state_snapshot": 1,
            "complete": true,
            "community_id": COMMUNITY,
            "pubkey": pubkey,
            "snapshot_id": "0".repeat(64),
            "events": events,
        })
    }

    pub(crate) fn snapshot_descriptor() -> Value {
        json!({"version": 1, "community_id": COMMUNITY, "max_events": 4096})
    }

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
    fn a_duplicate_or_conflicting_coordinate_is_rejected_not_picked() {
        let keys = Keys::generate();
        let old = slot_event(&keys, "a", &blob(json!({"c1": 99})), 1);
        let new = slot_event(&keys, "a", &blob(json!({"c1": 5})), 2);
        let untagged = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), "x")
            .tags([Tag::identifier("read-state:a")])
            .sign_with_keys(&keys)
            .unwrap();
        let untagged = serde_json::to_value(untagged).unwrap();
        for (name, events) in [
            ("two versions", vec![new.clone(), old.clone()]),
            ("same event twice", vec![old.clone(), old.clone()]),
            ("tagged and untagged", vec![old.clone(), untagged]),
        ] {
            let err = merge(&keys, events).expect_err(name);
            let msg = err.to_string();
            assert!(msg.contains("duplicate or conflicting"), "{name}: {msg}");
            assert!(msg.contains("read-state:a"), "{name}: {msg}");
        }
        let other = slot_event(&keys, "b", &blob(json!({"c1": 5})), 2);
        assert!(merge(&keys, vec![old, other]).is_ok());
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

    type Bodies = Arc<Mutex<Vec<Value>>>;

    #[derive(Clone)]
    struct Canned {
        info: Value,
        query: String,
        bodies: Bodies,
    }

    async fn relay(keys: &Keys, info: Value, query: Value) -> (BuzzClient, Bodies) {
        let canned = Canned {
            info,
            query: query.to_string(),
            bodies: Bodies::default(),
        };
        let bodies = canned.bodies.clone();
        let app = Router::new()
            .route(
                "/info",
                get(|State(c): State<Canned>| async move { Json(c.info) }),
            )
            .route(
                "/query",
                post(|State(c): State<Canned>, body: Bytes| async move {
                    c.bodies
                        .lock()
                        .unwrap()
                        .push(serde_json::from_slice(&body).unwrap());
                    c.query
                }),
            )
            .with_state(canned);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            BuzzClient::new(url, keys.clone(), None, None).unwrap(),
            bodies,
        )
    }

    fn advertised() -> Value {
        json!({"read_state_snapshot": snapshot_descriptor()})
    }

    #[tokio::test]
    async fn fetch_loads_one_atomic_snapshot() {
        let keys = Keys::generate();
        let slots = vec![
            slot_event(&keys, "a", &blob(json!({"c1": 10})), 1),
            slot_event(&keys, "b", &blob(json!({"c1": 20})), 2),
        ];
        let (client, bodies) = relay(
            &keys,
            advertised(),
            snapshot_envelope(&keys.public_key().to_hex(), slots),
        )
        .await;

        let state = fetch(&client).await.unwrap();

        assert_eq!(state.frontier("c1"), Some(20));
        assert_eq!(
            *bodies.lock().unwrap(),
            vec![json!([{
                "kinds": [KIND_READ_STATE],
                "authors": [keys.public_key().to_hex()],
                "read_state_snapshot": 1,
            }])],
            "one exact snapshot request: no tag, limit or cursor that could page"
        );
    }

    #[tokio::test]
    async fn fetch_refuses_a_load_it_cannot_prove_complete() {
        let keys = Keys::generate();
        let slot = slot_event(&keys, "a", &blob(json!({"c1": 10})), 1);
        let good = snapshot_envelope(&keys.public_key().to_hex(), vec![slot.clone()]);
        let with = |field: &str, value: Value| {
            let mut envelope = good.clone();
            envelope[field] = value;
            envelope
        };
        let cases: Vec<(&str, Value, Value)> = vec![
            ("no descriptor", json!({}), good.clone()),
            (
                "descriptor version",
                json!({"read_state_snapshot": {"version": 2, "community_id": COMMUNITY}}),
                good.clone(),
            ),
            (
                "descriptor community",
                json!({"read_state_snapshot": {"version": 1}}),
                good.clone(),
            ),
            ("relay ignored the extension", advertised(), json!([slot])),
            ("incomplete", advertised(), with("complete", json!(false))),
            ("unmarked", advertised(), with("complete", Value::Null)),
            (
                "version",
                advertised(),
                with("read_state_snapshot", json!(2)),
            ),
            (
                "community",
                advertised(),
                with("community_id", json!("other")),
            ),
            (
                "identity",
                advertised(),
                with("pubkey", json!("f".repeat(64))),
            ),
            ("events", advertised(), with("events", Value::Null)),
        ];
        for (name, info, response) in cases {
            let (client, _) = relay(&keys, info, response).await;
            let err = fetch(&client).await.expect_err(name);
            assert!(err.to_string().contains("cannot prove"), "{name}: {err}");
        }
    }
}
