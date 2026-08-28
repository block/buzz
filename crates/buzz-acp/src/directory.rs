//! Kind:10100 agent-directory record publisher.
//!
//! The harness advertises the `respond_to` policy it actually enforces and the
//! channels it actually listens on, so member clients can decide whether this
//! agent is @-mentionable (`agentAutocompleteEligibility` in the desktop app,
//! `agent_identity_provider` on mobile). Publishing from the harness — rather
//! than the owner's client — guarantees the advertisement can never drift from
//! enforcement: a deployment that clamps `--respond-to` also clamps what gets
//! advertised.
//!
//! The record is a replaceable event authored by the agent key; consumers
//! (`agents_from_events` in the desktop Tauri backend) treat the event author
//! as the authoritative pubkey and parse the content as a flat JSON object.
//! `channel_add_policy` is carried forward from the current head record so a
//! policy set via `buzz channels set-add-policy` survives our republish (the
//! relay's kind:10100 side effect also requires the field).

use std::time::Duration;

use buzz_core::kind::{KIND_AGENT_PROFILE, KIND_NIP29_GROUP_METADATA};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::config::RespondTo;
use crate::relay::{merge_discovered_channels, ChannelInfo, RestClient};

/// Trailing-edge debounce for membership-churn republishes.
pub(crate) const DIRECTORY_DEBOUNCE: Duration = Duration::from_secs(2);
/// Bound on each relay round-trip (head fetch, metadata fetch, submit). Wide
/// enough for the RestClient's internal transient-error retries to complete.
const DIRECTORY_STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// Fallback add-policy when no head record exists — matches the relay's
/// database default for users without a kind:10100 record.
const DEFAULT_CHANNEL_ADD_POLICY: &str = "anyone";

/// Static identity and policy inputs for directory publishes, built once at
/// startup. Cheap to clone into spawned publish tasks.
#[derive(Clone)]
pub(crate) struct DirectoryPublisher {
    /// HTTP bridge client (NIP-98 signed as the agent).
    pub rest: RestClient,
    /// Agent keys — the record must be authored by the agent itself.
    pub keys: Keys,
    /// The author gate mode this harness enforces.
    pub respond_to: RespondTo,
    /// Allowlist pubkeys (relevant when `respond_to` is `allowlist`).
    pub respond_to_allowlist: Vec<String>,
    /// Normalized harness/runtime identity (e.g. `claude-agent-acp`).
    pub agent_type: String,
    /// Display name for the record; `None` omits the key so consumers fall
    /// back to the agent's npub / kind:0 profile.
    pub name: Option<String>,
}

impl DirectoryPublisher {
    /// Spawn a debounced, best-effort publish of the directory record.
    ///
    /// Callers hold at most one outstanding handle and abort it before
    /// spawning a replacement (trailing-edge debounce, same pattern as the
    /// presence heartbeat task).
    pub(crate) fn spawn_publish(
        &self,
        channel_ids: Vec<Uuid>,
        status: &'static str,
        debounce: Duration,
    ) -> JoinHandle<()> {
        let publisher = self.clone();
        tokio::spawn(async move {
            if !debounce.is_zero() {
                tokio::time::sleep(debounce).await;
            }
            publisher.publish_once(channel_ids, status).await;
        })
    }

    /// One publish attempt: carry forward `channel_add_policy` from the head
    /// record, resolve channel names, sign, submit. Every failure is logged
    /// and swallowed — directory publishing must never take the harness down.
    pub(crate) async fn publish_once(&self, channel_ids: Vec<Uuid>, status: &str) {
        let channel_add_policy = match self.fetch_head_add_policy().await {
            Ok(policy) => policy,
            Err(e) => {
                // Never overwrite a policy we could not read — skip entirely.
                tracing::warn!("directory publish skipped (head fetch failed): {e}");
                return;
            }
        };

        let channels = if channel_ids.is_empty() {
            Vec::new()
        } else {
            match self.resolve_channels(channel_ids).await {
                Ok(map) => advertisable_channels(map),
                Err(e) => {
                    tracing::warn!("directory publish skipped (channel metadata failed): {e}");
                    return;
                }
            }
        };

        let content = build_directory_content(
            self.name.as_deref(),
            &self.agent_type,
            status,
            &self.respond_to,
            &self.respond_to_allowlist,
            &channels,
            &channel_add_policy,
        );

        let event =
            match EventBuilder::new(Kind::Custom(KIND_AGENT_PROFILE as u16), content.to_string())
                .tags([])
                .sign_with_keys(&self.keys)
            {
                Ok(event) => event,
                Err(e) => {
                    tracing::warn!("directory record sign failed: {e}");
                    return;
                }
            };

        match tokio::time::timeout(DIRECTORY_STEP_TIMEOUT, self.rest.submit_event(&event)).await {
            Ok(Ok(_)) => tracing::info!(
                status,
                channels = channels.len(),
                respond_to = %self.respond_to,
                "published kind:10100 directory record"
            ),
            Ok(Err(e)) => tracing::warn!("directory record publish failed: {e}"),
            Err(_) => tracing::warn!("directory record publish timed out"),
        }
    }

    /// Fetch the agent's current head kind:10100 record and extract its
    /// `channel_add_policy`, defaulting when no record (or no field) exists.
    async fn fetch_head_add_policy(&self) -> Result<String, String> {
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_AGENT_PROFILE as u16))
            .author(self.keys.public_key())
            .limit(1);
        let head = tokio::time::timeout(DIRECTORY_STEP_TIMEOUT, self.rest.query(&[filter]))
            .await
            .map_err(|_| "head query timed out".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(parse_add_policy(&head))
    }

    /// Resolve channel names/types via kind:39000 metadata, reusing the
    /// discovery-time merge (which also drops archived channels).
    async fn resolve_channels(
        &self,
        channel_ids: Vec<Uuid>,
    ) -> Result<std::collections::HashMap<Uuid, ChannelInfo>, String> {
        let d_tag = SingleLetterTag::lowercase(Alphabet::D);
        let d_values: Vec<String> = channel_ids.iter().map(|u| u.to_string()).collect();
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_NIP29_GROUP_METADATA as u16))
            .custom_tags(d_tag, d_values);
        let meta = tokio::time::timeout(DIRECTORY_STEP_TIMEOUT, self.rest.query(&[filter]))
            .await
            .map_err(|_| "metadata query timed out".to_string())?
            .map_err(|e| e.to_string())?;
        Ok(merge_discovered_channels(channel_ids, &meta))
    }
}

/// Reduce discovered channels to the advertisable `(id, name)` set, sorted by
/// id for deterministic output.
///
/// DM channels are excluded (non-owner DM invocation is rejected by the author
/// gate regardless of `respond_to`), and so are channels whose type could not
/// be resolved — the author gate fails closed on unresolved types, so
/// advertising them would invite mentions the harness then silently drops.
fn advertisable_channels(map: std::collections::HashMap<Uuid, ChannelInfo>) -> Vec<(Uuid, String)> {
    let mut channels: Vec<(Uuid, String)> = map
        .into_iter()
        .filter(|(_, info)| info.channel_type != "dm" && info.channel_type != "unknown")
        .map(|(id, info)| (id, info.name))
        .collect();
    channels.sort_by_key(|(id, _)| *id);
    channels
}

/// Extract `content.channel_add_policy` from the newest head event in a
/// `/query` response, defaulting to the relay's database default.
fn parse_add_policy(head_events: &Value) -> String {
    head_events
        .as_array()
        .and_then(|events| events.first())
        .and_then(|event| event.get("content"))
        .and_then(Value::as_str)
        .and_then(|content| serde_json::from_str::<Value>(content).ok())
        .and_then(|content| {
            content
                .get("channel_add_policy")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_CHANNEL_ADD_POLICY.to_string())
}

/// Build the kind:10100 content object exactly as the desktop consumer
/// (`agents_from_events` → `RelayAgentInfo` → `RawRelayAgent`) parses it.
///
/// The allowlist is re-sorted and `channels`/`channel_ids` stay index-aligned;
/// `name` is omitted (not null) when absent so the consumer's npub fallback
/// engages.
fn build_directory_content(
    name: Option<&str>,
    agent_type: &str,
    status: &str,
    respond_to: &RespondTo,
    respond_to_allowlist: &[String],
    channels: &[(Uuid, String)],
    channel_add_policy: &str,
) -> Value {
    let mut allowlist: Vec<&str> = respond_to_allowlist.iter().map(String::as_str).collect();
    allowlist.sort_unstable();
    let mut content = serde_json::Map::new();
    if let Some(name) = name {
        content.insert("name".into(), json!(name));
    }
    content.insert("agent_type".into(), json!(agent_type));
    content.insert("status".into(), json!(status));
    content.insert("respond_to".into(), json!(respond_to.to_string()));
    content.insert("respond_to_allowlist".into(), json!(allowlist));
    content.insert(
        "channels".into(),
        json!(channels.iter().map(|(_, name)| name).collect::<Vec<_>>()),
    );
    content.insert(
        "channel_ids".into(),
        json!(channels
            .iter()
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>()),
    );
    content.insert("capabilities".into(), json!([]));
    content.insert("channel_add_policy".into(), json!(channel_add_policy));
    Value::Object(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_u128(n as u128)
    }

    #[test]
    fn content_matches_consumer_schema() {
        let channels = vec![
            (uuid(1), "general".to_string()),
            (uuid(2), "hq".to_string()),
        ];
        let content = build_directory_content(
            Some("Scout"),
            "claude-agent-acp",
            "online",
            &RespondTo::Anyone,
            &[],
            &channels,
            "anyone",
        );

        assert_eq!(content["name"], "Scout");
        assert_eq!(content["agent_type"], "claude-agent-acp");
        assert_eq!(content["status"], "online");
        assert_eq!(content["respond_to"], "anyone");
        assert_eq!(content["respond_to_allowlist"], json!([]));
        assert_eq!(content["channels"], json!(["general", "hq"]));
        assert_eq!(
            content["channel_ids"],
            json!([uuid(1).to_string(), uuid(2).to_string()])
        );
        assert_eq!(content["capabilities"], json!([]));
        assert_eq!(content["channel_add_policy"], "anyone");
    }

    #[test]
    fn respond_to_serializes_to_ts_literals() {
        for (mode, expected) in [
            (RespondTo::OwnerOnly, "owner-only"),
            (RespondTo::Allowlist, "allowlist"),
            (RespondTo::Anyone, "anyone"),
            (RespondTo::Nobody, "nobody"),
        ] {
            let content = build_directory_content(None, "acp", "online", &mode, &[], &[], "anyone");
            assert_eq!(content["respond_to"], expected);
        }
    }

    #[test]
    fn name_key_omitted_when_none() {
        let content = build_directory_content(
            None,
            "acp",
            "online",
            &RespondTo::Anyone,
            &[],
            &[],
            "anyone",
        );
        assert!(!content.as_object().is_some_and(|o| o.contains_key("name")));
    }

    #[test]
    fn allowlist_always_present_and_sorted() {
        let allowlist = vec!["bb".repeat(32), "aa".repeat(32)];
        let content = build_directory_content(
            None,
            "acp",
            "online",
            &RespondTo::Allowlist,
            &allowlist,
            &[],
            "anyone",
        );
        assert_eq!(
            content["respond_to_allowlist"],
            json!(["aa".repeat(32), "bb".repeat(32)])
        );
    }

    #[test]
    fn advertisable_channels_excludes_dm_and_unknown_and_sorts() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            uuid(3),
            ChannelInfo {
                name: "later".into(),
                channel_type: "stream".into(),
            },
        );
        map.insert(
            uuid(1),
            ChannelInfo {
                name: "first".into(),
                channel_type: "private".into(),
            },
        );
        map.insert(
            uuid(2),
            ChannelInfo {
                name: "direct".into(),
                channel_type: "dm".into(),
            },
        );
        map.insert(
            uuid(4),
            ChannelInfo {
                name: "mystery".into(),
                channel_type: "unknown".into(),
            },
        );

        let channels = advertisable_channels(map);
        assert_eq!(
            channels,
            vec![(uuid(1), "first".into()), (uuid(3), "later".into())]
        );
    }

    #[test]
    fn parse_add_policy_defaults_and_carries() {
        assert_eq!(parse_add_policy(&json!([])), "anyone");
        assert_eq!(parse_add_policy(&json!("not-an-array")), "anyone");
        assert_eq!(
            parse_add_policy(&json!([{ "content": "{\"channel_add_policy\":\"nobody\"}" }])),
            "nobody"
        );
        assert_eq!(
            parse_add_policy(&json!([{ "content": "not json" }])),
            "anyone"
        );
        assert_eq!(
            parse_add_policy(&json!([{ "content": "{\"name\":\"x\"}" }])),
            "anyone"
        );
    }
}
