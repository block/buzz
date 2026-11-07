//! Durable bridge identity mappings.
//!
//! Slack timestamps and Buzz event IDs are the cross-system idempotency keys.
//! State is written through an atomic same-directory rename so a process crash
//! cannot leave a partially written JSON document.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const STATE_VERSION: u32 = 1;
const MAX_ALIAS_HOPS: usize = 16;

/// Slack-side identity for one bridged message.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct SlackMessageRef {
    pub(crate) team_id: String,
    pub(crate) channel_id: String,
    pub(crate) ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) thread_ts: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedState {
    version: u32,
    #[serde(default)]
    slack_to_buzz: BTreeMap<String, String>,
    #[serde(default)]
    buzz_to_slack: BTreeMap<String, SlackMessageRef>,
    #[serde(default)]
    channel_aliases: BTreeMap<String, String>,
    #[serde(default)]
    paused_buzz_channels: BTreeSet<Uuid>,
    #[serde(default)]
    last_buzz_created_at: Option<u64>,
    #[serde(default)]
    slack_user_names: BTreeMap<String, String>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            slack_to_buzz: BTreeMap::new(),
            buzz_to_slack: BTreeMap::new(),
            channel_aliases: BTreeMap::new(),
            paused_buzz_channels: BTreeSet::new(),
            last_buzz_created_at: None,
            slack_user_names: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct StateStore {
    path: PathBuf,
    data: PersistedState,
}

impl StateStore {
    pub(crate) fn load(path: PathBuf) -> Result<Self> {
        let data = match std::fs::read(&path) {
            Ok(bytes) => {
                let parsed: PersistedState = serde_json::from_slice(&bytes)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                if parsed.version != STATE_VERSION {
                    bail!(
                        "unsupported bridge state version {} in {} (expected {STATE_VERSION})",
                        parsed.version,
                        path.display()
                    );
                }
                validate_aliases(&parsed.channel_aliases)?;
                parsed
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PersistedState::default(),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, data })
    }

    pub(crate) fn buzz_event_for_slack(
        &self,
        buzz_channel_id: Uuid,
        slack_ts: &str,
    ) -> Option<&str> {
        self.data
            .slack_to_buzz
            .get(&slack_key(buzz_channel_id, slack_ts))
            .map(String::as_str)
    }

    pub(crate) fn slack_message_for_buzz(&self, buzz_event_id: &str) -> Option<&SlackMessageRef> {
        self.data.buzz_to_slack.get(buzz_event_id)
    }

    pub(crate) fn record_message_pair(
        &mut self,
        buzz_channel_id: Uuid,
        buzz_event_id: &str,
        slack: SlackMessageRef,
    ) -> Result<()> {
        let event_id = buzz_event_id.to_owned();
        let slack_ts = slack.ts.clone();
        self.commit(move |next| {
            next.slack_to_buzz
                .insert(slack_key(buzz_channel_id, &slack_ts), event_id.clone());
            next.buzz_to_slack.insert(event_id, slack);
            Ok(())
        })
    }

    pub(crate) fn canonical_channel_id(&self, team_id: &str, channel_id: &str) -> String {
        canonical_channel_id(&self.data.channel_aliases, team_id, channel_id)
    }

    pub(crate) fn record_channel_id_change(
        &mut self,
        team_id: &str,
        old_channel_id: &str,
        new_channel_id: &str,
    ) -> Result<()> {
        let team_id = team_id.to_owned();
        let old_channel_id = old_channel_id.to_owned();
        let new_channel_id = new_channel_id.to_owned();
        self.commit(move |next| {
            let old_canonical =
                canonical_channel_id(&next.channel_aliases, &team_id, &old_channel_id);
            let new_canonical =
                canonical_channel_id(&next.channel_aliases, &team_id, &new_channel_id);
            if old_canonical == new_canonical {
                return Ok(());
            }
            if new_canonical == old_channel_id {
                bail!("Slack channel ID change would create an alias cycle");
            }
            next.channel_aliases
                .insert(channel_alias_key(&team_id, &old_canonical), new_canonical);
            validate_aliases(&next.channel_aliases)
        })
    }

    pub(crate) fn set_route_paused(&mut self, buzz_channel_id: Uuid, paused: bool) -> Result<()> {
        self.commit(move |next| {
            if paused {
                next.paused_buzz_channels.insert(buzz_channel_id);
            } else {
                next.paused_buzz_channels.remove(&buzz_channel_id);
            }
            Ok(())
        })
    }

    pub(crate) fn route_is_paused(&self, buzz_channel_id: Uuid) -> bool {
        self.data.paused_buzz_channels.contains(&buzz_channel_id)
    }

    /// Return a safe replay cursor.
    ///
    /// A brand-new bridge begins at `now` so enabling it cannot backfill a
    /// channel into Slack. Restarts replay a bounded window before the last
    /// successfully handled event; durable message mappings suppress
    /// duplicates inside that window.
    pub(crate) fn subscription_since(&mut self, now: u64, lookback_secs: u64) -> Result<u64> {
        if let Some(cursor) = self.data.last_buzz_created_at {
            return Ok(cursor.saturating_sub(lookback_secs));
        }
        self.commit(move |next| {
            next.last_buzz_created_at = Some(now);
            Ok(())
        })?;
        Ok(now)
    }

    pub(crate) fn record_buzz_cursor(&mut self, created_at: u64) -> Result<()> {
        if self
            .data
            .last_buzz_created_at
            .is_some_and(|current| current >= created_at)
        {
            return Ok(());
        }
        self.commit(move |next| {
            next.last_buzz_created_at = Some(created_at);
            Ok(())
        })
    }

    pub(crate) fn slack_user_name(&self, user_id: &str) -> Option<&str> {
        self.data.slack_user_names.get(user_id).map(String::as_str)
    }

    pub(crate) fn record_slack_user_name(&mut self, user_id: &str, name: &str) -> Result<()> {
        if self.data.slack_user_names.get(user_id).map(String::as_str) == Some(name) {
            return Ok(());
        }
        let user_id = user_id.to_owned();
        let name = name.to_owned();
        self.commit(move |next| {
            next.slack_user_names.insert(user_id, name);
            Ok(())
        })
    }

    fn commit(&mut self, update: impl FnOnce(&mut PersistedState) -> Result<()>) -> Result<()> {
        let mut next = self.data.clone();
        update(&mut next)?;
        write_atomic(&self.path, &next)?;
        self.data = next;
        Ok(())
    }
}

fn slack_key(buzz_channel_id: Uuid, slack_ts: &str) -> String {
    format!("{buzz_channel_id}:{slack_ts}")
}

fn channel_alias_key(team_id: &str, channel_id: &str) -> String {
    format!("{team_id}:{channel_id}")
}

fn canonical_channel_id(
    aliases: &BTreeMap<String, String>,
    team_id: &str,
    channel_id: &str,
) -> String {
    let mut current = channel_id.to_owned();
    let mut seen = HashSet::new();
    for _ in 0..MAX_ALIAS_HOPS {
        if !seen.insert(current.clone()) {
            break;
        }
        let Some(next) = aliases.get(&channel_alias_key(team_id, &current)) else {
            break;
        };
        current.clone_from(next);
    }
    current
}

fn validate_aliases(aliases: &BTreeMap<String, String>) -> Result<()> {
    for key in aliases.keys() {
        let Some((team_id, channel_id)) = key.split_once(':') else {
            bail!("invalid Slack channel alias key in bridge state");
        };
        let mut current = channel_id.to_owned();
        let mut seen = HashSet::new();
        for _ in 0..=MAX_ALIAS_HOPS {
            if !seen.insert(current.clone()) {
                bail!("Slack channel alias cycle in bridge state");
            }
            let Some(next) = aliases.get(&channel_alias_key(team_id, &current)) else {
                break;
            };
            current.clone_from(next);
        }
        if seen.len() > MAX_ALIAS_HOPS {
            bail!("Slack channel alias chain is too deep in bridge state");
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, data: &PersistedState) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("bridge state path must have a UTF-8 file name")?;
    let temp_path = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(data).context("failed to serialize bridge state")?;

    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp_path)
        .with_context(|| format!("failed to open {}", temp_path.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", temp_path.display()))?;
    std::fs::rename(&temp_path, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    #[cfg(unix)]
    {
        std::fs::File::open(parent)
            .with_context(|| format!("failed to open {} for sync", parent.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_pairs_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let channel = Uuid::new_v4();
        let slack = SlackMessageRef {
            team_id: "T12345678".into(),
            channel_id: "C12345678".into(),
            ts: "123.456".into(),
            thread_ts: None,
        };

        let mut state = StateStore::load(path.clone()).unwrap();
        state
            .record_message_pair(channel, &"a".repeat(64), slack.clone())
            .unwrap();

        let reloaded = StateStore::load(path).unwrap();
        assert_eq!(
            reloaded.buzz_event_for_slack(channel, "123.456"),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(
            reloaded.slack_message_for_buzz(&"a".repeat(64)),
            Some(&slack)
        );
    }

    #[test]
    fn channel_aliases_follow_multiple_id_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = StateStore::load(dir.path().join("state.json")).unwrap();
        state
            .record_channel_id_change("T12345678", "G12345678", "C12345678")
            .unwrap();
        state
            .record_channel_id_change("T12345678", "C12345678", "C87654321")
            .unwrap();
        assert_eq!(
            state.canonical_channel_id("T12345678", "G12345678"),
            "C87654321"
        );
    }

    #[test]
    fn pause_state_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let channel = Uuid::new_v4();
        let mut state = StateStore::load(path.clone()).unwrap();
        state.set_route_paused(channel, true).unwrap();
        assert!(StateStore::load(path).unwrap().route_is_paused(channel));
    }

    #[test]
    fn rejects_alias_cycles_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{
              "version": 1,
              "channel_aliases": {
                "T12345678:C12345678": "C87654321",
                "T12345678:C87654321": "C12345678"
              }
            }"#,
        )
        .unwrap();
        let error = StateStore::load(path).unwrap_err().to_string();
        assert!(error.contains("alias cycle"), "{error}");
    }

    #[test]
    fn first_subscription_starts_now_and_restart_replays_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = StateStore::load(path.clone()).unwrap();
        assert_eq!(state.subscription_since(10_000, 3_600).unwrap(), 10_000);
        state.record_buzz_cursor(12_000).unwrap();

        let mut reloaded = StateStore::load(path).unwrap();
        assert_eq!(reloaded.subscription_since(20_000, 3_600).unwrap(), 8_400);
    }

    #[test]
    fn slack_user_names_are_durable_for_deterministic_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = StateStore::load(path.clone()).unwrap();
        state
            .record_slack_user_name("U12345678", "External Partner")
            .unwrap();
        assert_eq!(
            StateStore::load(path).unwrap().slack_user_name("U12345678"),
            Some("External Partner")
        );
    }
}
