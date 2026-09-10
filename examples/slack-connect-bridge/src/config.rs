//! Bridge configuration and validation.

use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use buzz_sdk::nip_oa;
use nostr::{Keys, Tag};
use serde::Deserialize;
use uuid::Uuid;

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:3100";
const DEFAULT_REPLAY_LOOKBACK_SECS: u64 = 86_400;
const MAX_REPLAY_LOOKBACK_SECS: u64 = 30 * 86_400;

/// One explicit Slack channel to Buzz channel route.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChannelMapping {
    pub(crate) slack_team_id: String,
    pub(crate) slack_channel_id: String,
    pub(crate) buzz_channel_id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default = "default_listen_addr")]
    listen_addr: String,
    #[serde(default = "default_state_path")]
    state_path: PathBuf,
    #[serde(default)]
    allow_non_shared_channels: bool,
    #[serde(default = "default_replay_lookback_secs")]
    replay_lookback_secs: u64,
    channels: Vec<ChannelMapping>,
}

fn default_listen_addr() -> String {
    DEFAULT_LISTEN_ADDR.to_owned()
}

fn default_state_path() -> PathBuf {
    PathBuf::from("slack-connect-bridge-state.json")
}

const fn default_replay_lookback_secs() -> u64 {
    DEFAULT_REPLAY_LOOKBACK_SECS
}

/// Fully validated runtime configuration.
pub(crate) struct Config {
    pub(crate) relay_url: String,
    pub(crate) bridge_keys: Keys,
    pub(crate) owner_auth_tag: Option<Tag>,
    pub(crate) slack_signing_secret: String,
    pub(crate) slack_bot_token: String,
    pub(crate) listen_addr: SocketAddr,
    pub(crate) state_path: PathBuf,
    pub(crate) allow_non_shared_channels: bool,
    pub(crate) replay_lookback_secs: u64,
    pub(crate) channels: Vec<ChannelMapping>,
}

impl Config {
    pub(crate) fn from_env() -> Result<Self> {
        let config_path = PathBuf::from(required_env("BUZZ_SLACK_BRIDGE_CONFIG")?);
        let raw = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let file: FileConfig = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
        validate_file_config(&file)?;

        let relay_url =
            std::env::var("BUZZ_RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_owned());
        let bridge_keys = Keys::parse(&required_env("BUZZ_SLACK_BRIDGE_PRIVATE_KEY")?)
            .context("BUZZ_SLACK_BRIDGE_PRIVATE_KEY must be an nsec or hex private key")?;
        let owner_auth_tag = parse_owner_auth_tag(&bridge_keys)?;
        let listen_addr = file
            .listen_addr
            .parse()
            .with_context(|| format!("invalid listen_addr {:?}", file.listen_addr))?;
        let state_path = resolve_relative_path(&config_path, &file.state_path);

        Ok(Self {
            relay_url,
            bridge_keys,
            owner_auth_tag,
            slack_signing_secret: required_env("BUZZ_SLACK_SIGNING_SECRET")?,
            slack_bot_token: required_env("BUZZ_SLACK_BOT_TOKEN")?,
            listen_addr,
            state_path,
            allow_non_shared_channels: file.allow_non_shared_channels,
            replay_lookback_secs: file.replay_lookback_secs,
            channels: file.channels,
        })
    }
}

fn parse_owner_auth_tag(bridge_keys: &Keys) -> Result<Option<Tag>> {
    let Ok(raw) = std::env::var("BUZZ_AUTH_TAG") else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    nip_oa::verify_auth_tag(&raw, &bridge_keys.public_key())
        .context("BUZZ_AUTH_TAG is not valid for BUZZ_SLACK_BRIDGE_PRIVATE_KEY")?;
    Ok(Some(nip_oa::parse_auth_tag(&raw)?))
}

fn resolve_relative_path(config_path: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

fn validate_file_config(config: &FileConfig) -> Result<()> {
    if config.channels.is_empty() {
        bail!("config must contain at least one channel mapping");
    }
    if config.replay_lookback_secs == 0 || config.replay_lookback_secs > MAX_REPLAY_LOOKBACK_SECS {
        bail!("replay_lookback_secs must be between 1 and {MAX_REPLAY_LOOKBACK_SECS}");
    }

    let mut slack_routes = HashSet::new();
    let mut buzz_routes = HashSet::new();
    for route in &config.channels {
        validate_slack_id("slack_team_id", &route.slack_team_id, &['T'])?;
        validate_slack_id("slack_channel_id", &route.slack_channel_id, &['C', 'G'])?;
        if !slack_routes.insert((route.slack_team_id.clone(), route.slack_channel_id.clone())) {
            bail!(
                "duplicate Slack route {}:{}",
                route.slack_team_id,
                route.slack_channel_id
            );
        }
        if !buzz_routes.insert(route.buzz_channel_id) {
            bail!(
                "Buzz channel {} is mapped more than once; one-to-many fan-out must be explicit in a future bridge",
                route.buzz_channel_id
            );
        }
    }
    Ok(())
}

fn validate_slack_id(field: &str, value: &str, prefixes: &[char]) -> Result<()> {
    let mut chars = value.chars();
    let prefix = chars.next();
    if value.len() < 9
        || value.len() > 32
        || !prefix.is_some_and(|p| prefixes.contains(&p))
        || !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        bail!("{field} has an invalid Slack ID");
    }
    Ok(())
}

fn required_env(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_config() -> FileConfig {
        FileConfig {
            listen_addr: default_listen_addr(),
            state_path: default_state_path(),
            allow_non_shared_channels: false,
            replay_lookback_secs: DEFAULT_REPLAY_LOOKBACK_SECS,
            channels: vec![ChannelMapping {
                slack_team_id: "T12345678".into(),
                slack_channel_id: "C12345678".into(),
                buzz_channel_id: Uuid::new_v4(),
            }],
        }
    }

    #[test]
    fn accepts_one_to_one_routes() {
        validate_file_config(&file_config()).unwrap();
    }

    #[test]
    fn rejects_duplicate_buzz_destinations() {
        let mut config = file_config();
        config.channels.push(ChannelMapping {
            slack_team_id: "T12345678".into(),
            slack_channel_id: "C87654321".into(),
            buzz_channel_id: config.channels[0].buzz_channel_id,
        });
        let error = validate_file_config(&config).unwrap_err().to_string();
        assert!(error.contains("mapped more than once"), "{error}");
    }

    #[test]
    fn rejects_invalid_slack_ids_without_echoing_secrets() {
        let mut config = file_config();
        config.channels[0].slack_channel_id = "not-a-channel".into();
        let error = validate_file_config(&config).unwrap_err().to_string();
        assert_eq!(error, "slack_channel_id has an invalid Slack ID");
        assert!(!error.contains("not-a-channel"));
    }

    #[test]
    fn relative_state_path_is_relative_to_config() {
        assert_eq!(
            resolve_relative_path(
                Path::new("/opt/buzz/slack-bridge.json"),
                Path::new("state.json")
            ),
            PathBuf::from("/opt/buzz/state.json")
        );
    }
}
