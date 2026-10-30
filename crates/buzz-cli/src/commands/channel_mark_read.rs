//! `buzz channels mark-read` — advance the NIP-RS channel markers of the signed-in
//! identity to the newest top-level message.
//!
//! The CLI owns one frontier-only slot. It never writes `ov_*` entries, never
//! lowers a value, and never touches a slot another client owns. Overrides
//! (manual mark-unread) are read-only here: a channel whose override stays live
//! after the write is reported as not cleared.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use nostr::nips::nip44;
use nostr::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::channel_unread::{message_kinds, resolve_channels, thread_root, Channel, BOUNDS};
use super::read_state::{self, ReadState, MAX_CONTEXTS};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::validate_uuid;

const MAX_PLAINTEXT_BYTES: usize = 32_768;
const TOP_LEVEL_SCAN: u32 = 200;
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(50);
const STORE_VERSION: u32 = 1;
const CLIENT_ID_PREFIX: &str = "buzz-cli:";

/// The CLI's own slot, as saved locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub client_id: String,
    pub slot_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Store {
    version: u32,
    identities: BTreeMap<String, Identity>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            version: STORE_VERSION,
            identities: BTreeMap::new(),
        }
    }
}

/// Why a channel was not cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotCleared {
    /// A manual mark-unread override is still live after the write.
    Override,
    /// The newest top-level message is not among the newest events scanned.
    Window,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ChannelResult {
    pub channel: String,
    pub cleared: bool,
    pub advanced: bool,
    pub marker: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<NotCleared>,
}

#[derive(Debug)]
pub(crate) struct Outcome {
    /// The normalized relay write response, or `None` when nothing was published.
    pub write: Option<Value>,
    pub channels: Vec<ChannelResult>,
}

/// Inputs that differ between production and tests.
pub(crate) struct Options {
    pub store_path: PathBuf,
    pub now: u64,
    pub lock_wait: Duration,
}

/// `<config dir>/buzz/read-state-cli.json`.
pub(crate) fn default_store_path() -> Result<PathBuf, CliError> {
    let dir = dirs::config_dir()
        .ok_or_else(|| CliError::Other("could not resolve the platform config directory".into()))?;
    Ok(dir.join("buzz").join("read-state-cli.json"))
}

fn io_error(path: &Path, e: std::io::Error) -> CliError {
    CliError::Other(format!("{}: {e}", path.display()))
}

fn random_hex() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

fn store_key(client: &BuzzClient) -> Result<String, CliError> {
    let url = url::Url::parse(client.relay_url())
        .map_err(|e| CliError::Other(format!("invalid relay URL: {e}")))?;
    Ok(format!(
        "{}@{}",
        client.keys().public_key().to_hex(),
        url.origin().ascii_serialization()
    ))
}

async fn lock_store(path: &Path, wait: Duration) -> Result<File, CliError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    }
    let lock_path = path.with_extension("lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| io_error(&lock_path, e))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if Instant::now() >= deadline {
                    return Err(CliError::Other(format!(
                        "another buzz process holds {}; retry when it finishes",
                        lock_path.display()
                    )));
                }
                tokio::time::sleep(LOCK_POLL).await;
            }
            Err(e) => return Err(io_error(&lock_path, e)),
        }
    }
}

fn load_store(path: &Path) -> Result<Store, CliError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(e) => return Err(io_error(path, e)),
    };
    let store: Store = serde_json::from_str(&raw).map_err(|e| {
        CliError::Other(format!(
            "{} is not a valid read-state store ({e}); fix or delete it",
            path.display()
        ))
    })?;
    if store.version != STORE_VERSION {
        return Err(CliError::Other(format!(
            "{} has unsupported version {}",
            path.display(),
            store.version
        )));
    }
    Ok(store)
}

fn save_store(path: &Path, store: &Store) -> Result<(), CliError> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|e| CliError::Other(format!("failed to encode {}: {e}", path.display())))?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| io_error(&tmp, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| io_error(&tmp, e))?;
    }
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_error(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_error(path, e))
}

fn fresh_identity() -> Identity {
    Identity {
        client_id: format!("{CLIENT_ID_PREFIX}{}", &random_hex()[..16]),
        slot_id: random_hex(),
    }
}

/// Serialize a slot blob, refusing one over the byte budget or entry cap.
/// Channel keys are never dropped to make it fit.
fn encode_blob(
    client_id: &str,
    contexts: &BTreeMap<String, u64>,
    max_bytes: usize,
) -> Result<String, CliError> {
    if contexts.len() > MAX_CONTEXTS {
        return Err(CliError::Other(format!(
            "read state would hold {} entries, over the {MAX_CONTEXTS}-entry limit; nothing published",
            contexts.len()
        )));
    }
    let plaintext = json!({"v": 1, "client_id": client_id, "contexts": contexts}).to_string();
    if plaintext.len() > max_bytes {
        return Err(CliError::Other(format!(
            "read state would be {} bytes, over the {max_bytes}-byte slot budget; nothing published",
            plaintext.len()
        )));
    }
    Ok(plaintext)
}

/// The newest top-level message time of a channel, from its newest
/// `TOP_LEVEL_SCAN` events. `Ok(None)` means the scan was exhaustive and found
/// none; `Err(Window)` means replies filled the scan.
async fn newest_top_level(
    client: &BuzzClient,
    channel: &Channel,
) -> Result<Result<Option<u64>, NotCleared>, CliError> {
    let filter = json!({
        "kinds": message_kinds(channel.is_dm),
        "#h": [channel.id],
    });
    let events = client.query_paginated(filter, TOP_LEVEL_SCAN).await?;
    let newest = events
        .iter()
        .filter(|e| thread_root(e).is_none())
        .filter_map(|e| e.get("created_at").and_then(Value::as_u64))
        .max();
    Ok(match newest {
        Some(at) => Ok(Some(at)),
        None if events.len() >= TOP_LEVEL_SCAN as usize => Err(NotCleared::Window),
        None => Ok(None),
    })
}

/// The slot this write replaces. The relay holding our slot id with another
/// `client_id` rotates `identity` to a new slot id and replaces nothing.
fn own_slot<'a>(
    state: &'a ReadState,
    identity: &mut Identity,
) -> Result<Option<&'a read_state::Slot>, CliError> {
    if state.is_skipped(&identity.slot_id) {
        return Err(CliError::Other(format!(
            "your CLI read-state slot {} was written by a newer schema; refusing to overwrite it",
            identity.slot_id
        )));
    }
    match state.slot(&identity.slot_id) {
        Some(slot) if slot.client_id != identity.client_id => {
            identity.slot_id = random_hex();
            Ok(None)
        }
        Some(slot) if slot.contexts.keys().any(|k| k.starts_with("ov_")) => {
            Err(CliError::Other(format!(
                "your CLI read-state slot {} holds override entries; the CLI never writes those, so it will not rewrite the slot",
                identity.slot_id
            )))
        }
        slot => Ok(slot),
    }
}

/// Advance the marker of each requested channel to its newest top-level message.
pub(crate) async fn mark_read(
    client: &BuzzClient,
    requested: &[String],
    options: &Options,
) -> Result<Outcome, CliError> {
    let mut requested: Vec<String> = requested
        .iter()
        .map(|c| validate_uuid(c).map(|()| c.to_lowercase()))
        .collect::<Result<_, _>>()?;
    requested.sort();
    requested.dedup();
    if requested.is_empty() {
        return Err(CliError::Usage("--channel is required".into()));
    }
    if requested.len() > BOUNDS.max_channels {
        return Err(CliError::Usage(format!(
            "{} channels given but at most {} can be marked per run",
            requested.len(),
            BOUNDS.max_channels
        )));
    }

    let _lock = lock_store(&options.store_path, options.lock_wait).await?;
    let mut store = load_store(&options.store_path)?;
    let key = store_key(client)?;
    let mut identity = store
        .identities
        .get(&key)
        .cloned()
        .unwrap_or_else(fresh_identity);

    let state = read_state::fetch(client).await?;
    let own = own_slot(&state, &mut identity)?;
    let mut contexts: BTreeMap<String, u64> = own
        .map(|slot| slot.contexts.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default();
    let previous_at = own.map(|slot| slot.created_at);

    let (channels, _) = resolve_channels(client, &requested, BOUNDS.max_channels).await?;
    let mut results = Vec::with_capacity(channels.len());
    let mut advanced_any = false;
    for channel in &channels {
        let frontier = state.frontier(&channel.id);
        let target = match newest_top_level(client, channel).await? {
            Ok(target) => target.map(|at| at.min(options.now)),
            Err(reason) => {
                results.push(ChannelResult {
                    channel: channel.id.clone(),
                    cleared: false,
                    advanced: false,
                    marker: frontier,
                    reason: Some(reason),
                });
                continue;
            }
        };
        let advance = target.filter(|t| frontier.is_none_or(|f| *t > f));
        if let Some(target) = advance {
            contexts.insert(channel.id.clone(), target);
            advanced_any = true;
        }
        let marker = advance.or(frontier);
        let live = state
            .register(&channel.id)
            .is_some_and(|r| r.is_active(marker.unwrap_or(0)));
        results.push(ChannelResult {
            channel: channel.id.clone(),
            cleared: !live,
            advanced: advance.is_some(),
            marker,
            reason: live.then_some(NotCleared::Override),
        });
    }

    if !advanced_any {
        return Ok(Outcome {
            write: None,
            channels: results,
        });
    }

    let plaintext = encode_blob(&identity.client_id, &contexts, MAX_PLAINTEXT_BYTES)?;
    let me = client.keys().public_key();
    let ciphertext = nip44::encrypt(
        client.keys().secret_key(),
        &me,
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|e| CliError::Other(format!("failed to encrypt read state: {e}")))?;
    let created_at = previous_at.map_or(options.now, |at| options.now.max(at + 1));
    let builder = buzz_sdk::build_read_state(&identity.slot_id, &ciphertext)
        .map_err(|e| CliError::Other(format!("build_read_state failed: {e}")))?
        .custom_created_at(Timestamp::from(created_at));
    let event = client.sign_event(builder)?;

    if store.identities.get(&key) != Some(&identity) {
        store.identities.insert(key, identity);
        save_store(&options.store_path, &store)?;
    }
    let raw = client.submit_event(event).await?;
    let write = super::parse_write_response(
        &raw,
        "the relay kept a newer version of the CLI read-state slot; rerun",
    )?;
    let write = serde_json::from_str(&write)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?;
    Ok(Outcome {
        write: Some(write),
        channels: results,
    })
}

/// The JSON written to stdout: the write envelope plus one result per channel.
fn render(outcome: &Outcome) -> Value {
    let channels = json!(&outcome.channels);
    match &outcome.write {
        Some(write) => {
            let mut envelope = write.clone();
            envelope["published"] = json!(true);
            envelope["channels"] = channels;
            envelope
        }
        None => json!({
            "event_id": null,
            "accepted": false,
            "published": false,
            "message": "nothing to publish: no marker advanced",
            "channels": channels,
        }),
    }
}

pub async fn cmd_mark_read(client: &BuzzClient, requested: &[String]) -> Result<(), CliError> {
    let options = Options {
        store_path: default_store_path()?,
        now: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| CliError::Other(format!("system clock is before 1970: {e}")))?
            .as_secs(),
        lock_wait: LOCK_WAIT,
    };
    let outcome = mark_read(client, requested, &options).await?;
    println!("{}", render(&outcome));
    exit_status(&outcome)
}

fn exit_status(outcome: &Outcome) -> Result<(), CliError> {
    let failed = outcome.channels.iter().filter(|c| !c.cleared).count();
    if failed > 0 {
        return Err(CliError::Other(format!(
            "{failed} channel(s) not marked read; see `reason` in the output"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::read_state::tests::slot_event;
    use crate::commands::test_relay::{fake_relay, Shared};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const CH1: &str = "11111111-1111-4111-8111-111111111111";
    const CH2: &str = "22222222-2222-4222-8222-222222222222";
    const NOW: u64 = 10_000;

    struct World {
        client: BuzzClient,
        relay: Shared,
        dir: tempfile::TempDir,
        counter: u64,
    }

    async fn world() -> World {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let world = World {
            client: BuzzClient::new(url, Keys::generate(), None, None).unwrap(),
            relay,
            dir: tempfile::tempdir().unwrap(),
            counter: 0,
        };
        for id in [CH1, CH2] {
            let tags = vec![Tag::identifier(id), Tag::parse(["name", id]).unwrap()];
            world.push(world.signed(&Keys::generate(), 39000, tags, 1));
        }
        world
    }

    impl World {
        fn options(&self, now: u64) -> Options {
            Options {
                store_path: self.dir.path().join("cfg").join("read-state-cli.json"),
                now,
                lock_wait: Duration::from_millis(100),
            }
        }

        fn push(&self, event: Value) {
            self.relay.lock().unwrap().events.push(event);
        }

        fn signed(&self, keys: &Keys, kind: u16, tags: Vec<Tag>, at: u64) -> Value {
            let event = EventBuilder::new(Kind::Custom(kind), "x")
                .tags(tags)
                .custom_created_at(Timestamp::from(at))
                .sign_with_keys(keys)
                .unwrap();
            serde_json::to_value(event).unwrap()
        }

        fn say(&mut self, channel: &str, at: u64, extra: &[Vec<&str>]) {
            self.counter += 1;
            let mut tags = vec![Tag::parse(["h", channel]).unwrap()];
            tags.extend(extra.iter().map(|t| Tag::parse(t.clone()).unwrap()));
            tags.push(Tag::parse(["nonce", &self.counter.to_string()]).unwrap());
            self.push(self.signed(&Keys::generate(), 9, tags, at));
        }

        fn reply(&mut self, channel: &str, at: u64) {
            let root = "a".repeat(64);
            self.say(
                channel,
                at,
                &[vec!["e", &root, "", "root"], vec!["e", &root, "", "reply"]],
            );
        }

        fn other_slot(&self, slot: &str, contexts: Value, at: u64) {
            let blob = json!({"v": 1, "client_id": "desktop", "contexts": contexts});
            self.push(slot_event(self.client.keys(), slot, &blob.to_string(), at));
        }

        fn own_slot_on_relay(
            &self,
            identity: &Identity,
            contexts: Value,
            client_id: &str,
            at: u64,
        ) {
            let blob = json!({"v": 1, "client_id": client_id, "contexts": contexts});
            self.push(slot_event(
                self.client.keys(),
                &identity.slot_id,
                &blob.to_string(),
                at,
            ));
        }

        async fn mark(&self, channels: &[&str], now: u64) -> Result<Outcome, CliError> {
            let channels: Vec<String> = channels.iter().map(|c| c.to_string()).collect();
            mark_read(&self.client, &channels, &self.options(now)).await
        }

        fn published(&self) -> Vec<Value> {
            self.relay.lock().unwrap().published.clone()
        }

        fn saved(&self) -> Identity {
            let store = load_store(&self.options(0).store_path).unwrap();
            store.identities.values().next().unwrap().clone()
        }

        fn blob_of(&self, event: &Value) -> Value {
            let me = self.client.keys().public_key();
            let plaintext = nip44::decrypt(
                self.client.keys().secret_key(),
                &me,
                event["content"].as_str().unwrap(),
            )
            .unwrap();
            serde_json::from_str(&plaintext).unwrap()
        }
    }

    fn result<'a>(outcome: &'a Outcome, id: &str) -> &'a ChannelResult {
        outcome.channels.iter().find(|c| c.channel == id).unwrap()
    }

    fn d_tag_of(event: &Value) -> String {
        event["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t[0] == "d")
            .unwrap()[1]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn marks_top_level_messages_read_with_one_stable_frontier_only_slot() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        w.say(CH2, 150, &[]);
        w.reply(CH2, 400);
        w.other_slot("desktop-a", json!({CH1: 120}), 5);

        let first = w.mark(&[CH2, CH1], NOW).await.unwrap();

        let published = w.published();
        assert_eq!(published.len(), 1, "one action, one publish");
        let blob = w.blob_of(&published[0]);
        assert_eq!(blob["v"], 1);
        assert!(blob["client_id"].as_str().unwrap().starts_with("buzz-cli:"));
        assert_eq!(
            blob["contexts"],
            json!({CH2: 150}),
            "the reply at 400 is not counted; CH1 is already covered by another slot's 120"
        );
        let write = first.write.as_ref().unwrap();
        assert_eq!(write["accepted"], true);
        assert_eq!(write["event_id"], published[0]["id"]);
        assert_eq!(
            result(&first, CH2),
            &ChannelResult {
                channel: CH2.into(),
                cleared: true,
                advanced: true,
                marker: Some(150),
                reason: None
            }
        );
        assert_eq!(result(&first, CH1).marker, Some(120));
        assert!(!result(&first, CH1).advanced);

        let identity = w.saved();
        assert_eq!(
            d_tag_of(&published[0]),
            format!("read-state:{}", identity.slot_id)
        );
        assert_eq!(identity.slot_id.len(), 32);
        assert!(blob["client_id"] == identity.client_id.as_str());

        let again = w.mark(&[CH1, CH2], NOW).await.unwrap();
        assert!(again.write.is_none(), "nothing advanced: no publish");
        assert_eq!(w.published().len(), 1);

        w.say(CH1, 200, &[]);
        w.mark(&[CH1, CH2], NOW + 5).await.unwrap();
        let published = w.published();
        assert_eq!(published.len(), 2);
        assert_eq!(
            d_tag_of(&published[1]),
            d_tag_of(&published[0]),
            "slot id is stable"
        );
        assert_eq!(w.saved(), identity);
        assert_eq!(
            w.blob_of(&published[1])["contexts"],
            json!({CH1: 200, CH2: 150}),
            "earlier entries carry forward"
        );

        let state = read_state::merge(
            w.client.keys(),
            w.relay
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|e| e["kind"] == 30078)
                .cloned()
                .collect(),
        )
        .unwrap();
        assert_eq!(state.frontier(CH1), Some(200));
        assert_eq!(state.frontier(CH2), Some(150));
    }

    #[tokio::test]
    async fn never_lowers_a_value_and_caps_the_target_at_now() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        w.say(CH2, 50_000, &[]);
        let identity = fresh_identity();
        let path = w.options(0).store_path;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut store = Store::default();
        store
            .identities
            .insert(store_key(&w.client).unwrap(), identity.clone());
        save_store(&path, &store).unwrap();
        w.own_slot_on_relay(
            &identity,
            json!({CH1: 500, "keep": 7}),
            &identity.client_id,
            9_000,
        );

        let outcome = w.mark(&[CH1, CH2], NOW).await.unwrap();

        assert!(!result(&outcome, CH1).advanced, "100 < 500");
        assert_eq!(result(&outcome, CH1).marker, Some(500));
        assert_eq!(
            result(&outcome, CH2).marker,
            Some(NOW),
            "future message capped at now"
        );
        let published = w.published();
        assert_eq!(
            w.blob_of(&published[0])["contexts"],
            json!({CH1: 500, CH2: NOW, "keep": 7})
        );
        assert_eq!(
            published[0]["created_at"], 10_000,
            "now (10000) beats the previous event time (9000) plus one"
        );
    }

    #[tokio::test]
    async fn a_new_version_never_ties_the_one_it_replaces() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        let identity = fresh_identity();
        let path = w.options(0).store_path;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut store = Store::default();
        store
            .identities
            .insert(store_key(&w.client).unwrap(), identity.clone());
        save_store(&path, &store).unwrap();
        w.own_slot_on_relay(&identity, json!({}), &identity.client_id, NOW);

        w.mark(&[CH1], NOW).await.unwrap();

        assert_eq!(w.published()[0]["created_at"], NOW + 1);
    }

    #[tokio::test]
    async fn never_writes_overrides_and_reports_a_live_one_as_not_cleared() {
        let mut w = world().await;
        w.say(CH1, 150, &[]);
        w.say(CH2, 150, &[]);
        let ch3 = "33333333-3333-4333-8333-333333333333";
        let ch4 = "44444444-4444-4444-8444-444444444444";
        for id in [ch3, ch4] {
            let tags = vec![Tag::identifier(id), Tag::parse(["name", id]).unwrap()];
            w.push(w.signed(&Keys::generate(), 39000, tags, 1));
            w.say(id, 150, &[]);
        }
        w.other_slot(
            "desktop-a",
            json!({
                format!("ov_s:{CH1}"): 2, format!("ov_c:{CH1}"): 1, format!("ov_b:{CH1}"): 1_000,
                format!("ov_s:{CH2}"): 2, format!("ov_c:{CH2}"): 1, format!("ov_b:{CH2}"): 100,
                format!("ov_s:{ch3}"): 2,
                format!("ov_c:{ch4}"): 5,
            }),
            5,
        );

        let outcome = w.mark(&[CH1, CH2, ch3, ch4], NOW).await.unwrap();

        assert_eq!(
            result(&outcome, CH1),
            &ChannelResult {
                channel: CH1.into(),
                cleared: false,
                advanced: true,
                marker: Some(150),
                reason: Some(NotCleared::Override)
            },
            "frontier 150 <= baseline 1000 and S 2 > C 1"
        );
        assert!(
            result(&outcome, CH2).cleared,
            "frontier 150 passed baseline 100"
        );
        assert!(
            result(&outcome, ch3).cleared,
            "a partial group is rejected whole"
        );
        assert!(
            result(&outcome, ch4).cleared,
            "a tombstone floor is inactive"
        );
        let rendered = render(&outcome);
        assert_eq!(rendered["channels"][0]["reason"], "override");
        assert_eq!(rendered["channels"][0]["cleared"], false);
        let status = exit_status(&outcome).unwrap_err();
        assert_eq!(crate::error::exit_code(&status), 4);
        assert!(status.to_string().contains("1 channel(s)"), "{status}");

        let blob = w.blob_of(&w.published()[0]);
        let keys: Vec<&String> = blob["contexts"].as_object().unwrap().keys().collect();
        assert!(keys.iter().all(|k| !k.starts_with("ov_")), "{keys:?}");
    }

    #[tokio::test]
    async fn a_duplicate_coordinate_in_the_snapshot_blocks_the_write() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        w.other_slot("desktop-a", json!({CH1: 1}), 5);
        w.other_slot("desktop-a", json!({CH1: 2}), 6);

        let err = w.mark(&[CH1], NOW).await.unwrap_err();

        assert!(
            err.to_string().contains("duplicate or conflicting"),
            "{err}"
        );
        assert!(w.published().is_empty());
    }

    #[tokio::test]
    async fn refuses_to_rewrite_a_slot_it_cannot_safely_carry() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        let identity = fresh_identity();
        let path = w.options(0).store_path;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut store = Store::default();
        store
            .identities
            .insert(store_key(&w.client).unwrap(), identity.clone());
        save_store(&path, &store).unwrap();

        let newer = json!({"v": 2, "client_id": identity.client_id, "contexts": {}});
        w.push(slot_event(
            w.client.keys(),
            &identity.slot_id,
            &newer.to_string(),
            5,
        ));
        let err = w.mark(&[CH1], NOW).await.unwrap_err();
        assert!(err.to_string().contains("newer schema"), "{err}");
        w.relay
            .lock()
            .unwrap()
            .events
            .retain(|e| e["kind"] != 30078);

        w.own_slot_on_relay(
            &identity,
            json!({format!("ov_c:{CH2}"): 3}),
            &identity.client_id,
            5,
        );
        let err = w.mark(&[CH1], NOW).await.unwrap_err();
        assert!(err.to_string().contains("override entries"), "{err}");
        assert!(w.published().is_empty());
    }

    #[tokio::test]
    async fn a_slot_held_with_another_client_id_is_never_published_to() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        let identity = fresh_identity();
        let path = w.options(0).store_path;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut store = Store::default();
        store
            .identities
            .insert(store_key(&w.client).unwrap(), identity.clone());
        save_store(&path, &store).unwrap();
        w.own_slot_on_relay(&identity, json!({"x": 9}), "someone-else", 5);

        w.mark(&[CH1], NOW).await.unwrap();

        let published = w.published();
        assert_ne!(
            d_tag_of(&published[0]),
            format!("read-state:{}", identity.slot_id)
        );
        let saved = w.saved();
        assert_ne!(saved.slot_id, identity.slot_id);
        assert_eq!(saved.client_id, identity.client_id);
        assert_eq!(
            d_tag_of(&published[0]),
            format!("read-state:{}", saved.slot_id)
        );
        assert_eq!(w.blob_of(&published[0])["contexts"], json!({CH1: 100}));
    }

    #[tokio::test]
    async fn an_oversized_slot_publishes_nothing_and_a_replies_only_scan_is_not_cleared() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        let identity = fresh_identity();
        let path = w.options(0).store_path;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut store = Store::default();
        store
            .identities
            .insert(store_key(&w.client).unwrap(), identity.clone());
        save_store(&path, &store).unwrap();
        let big: serde_json::Map<String, Value> = (0..800)
            .map(|i| (format!("{i:08}-1111-4111-8111-111111111111"), json!(1)))
            .collect();
        w.own_slot_on_relay(&identity, Value::Object(big), &identity.client_id, 5);

        let err = w.mark(&[CH1], NOW).await.unwrap_err();
        assert!(err.to_string().contains("slot budget"), "{err}");
        assert!(w.published().is_empty());

        w.relay
            .lock()
            .unwrap()
            .events
            .retain(|e| e["kind"] != 30078);
        for i in 0..TOP_LEVEL_SCAN as u64 {
            w.reply(CH2, 200 + i);
        }
        let outcome = w.mark(&[CH2], NOW).await.unwrap();
        assert_eq!(
            result(&outcome, CH2),
            &ChannelResult {
                channel: CH2.into(),
                cleared: false,
                advanced: false,
                marker: None,
                reason: Some(NotCleared::Window)
            }
        );
        assert!(w.published().is_empty());
    }

    #[test]
    fn the_blob_encoder_enforces_both_limits() {
        let one = BTreeMap::from([("c".to_string(), 1)]);
        assert!(encode_blob("id", &one, MAX_PLAINTEXT_BYTES).is_ok());
        let err = encode_blob("id", &one, 10).unwrap_err();
        assert!(err.to_string().contains("slot budget"), "{err}");
        let many: BTreeMap<String, u64> = (0..=MAX_CONTEXTS).map(|i| (i.to_string(), 1)).collect();
        let err = encode_blob("id", &many, usize::MAX).unwrap_err();
        assert!(err.to_string().contains("entry limit"), "{err}");
    }

    #[tokio::test]
    async fn the_local_store_is_private_atomic_and_locked() {
        let mut w = world().await;
        w.say(CH1, 100, &[]);
        w.mark(&[CH1], NOW).await.unwrap();
        let options = w.options(NOW);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&options.store_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!options.store_path.with_extension("json.tmp").exists());
        let key = store_key(&w.client).unwrap();
        assert!(key.starts_with(&w.client.keys().public_key().to_hex()));
        assert!(key.contains("@http://127.0.0.1:"));

        let held = lock_store(&options.store_path, Duration::from_millis(10))
            .await
            .unwrap();
        w.say(CH1, 300, &[]);
        let err = w.mark(&[CH1], NOW).await.unwrap_err();
        assert!(err.to_string().contains("holds"), "{err}");
        assert_eq!(w.published().len(), 1);
        drop(held);
        w.mark(&[CH1], NOW).await.unwrap();
        assert_eq!(w.published().len(), 2);

        std::fs::write(&options.store_path, "{").unwrap();
        let err = w.mark(&[CH1], NOW).await.unwrap_err();
        assert!(
            err.to_string().contains("not a valid read-state store"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn input_errors_come_before_any_network_or_file_work() {
        let w = world().await;
        let err = w.mark(&[], NOW).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        let err = w.mark(&["not-a-uuid"], NOW).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        let missing = "99999999-9999-4999-8999-999999999999";
        let err = w.mark(&[missing], NOW).await.unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)), "{err}");
        assert!(w.published().is_empty());
    }
}
