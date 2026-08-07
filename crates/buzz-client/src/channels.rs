use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{BuzzClient, ClientError};

/// Which channel population a listing should query.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ChannelScope {
    /// Every channel visible to the authenticated identity.
    #[default]
    Visible,
    /// Only channels whose membership snapshot includes the active signer.
    Member,
}

/// NIP-29 channel visibility classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelVisibility {
    /// Publicly visible channel.
    Open,
    /// Membership-restricted channel.
    Private,
}

/// Typed request for a complete bounded channel listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListChannelsRequest {
    /// Population to query.
    pub scope: ChannelScope,
    /// Optional visibility filter.
    pub visibility: Option<ChannelVisibility>,
    /// Maximum number of membership and metadata records to inspect and return.
    pub limit: u32,
}

impl Default for ListChannelsRequest {
    fn default() -> Self {
        Self {
            scope: ChannelScope::Visible,
            visibility: None,
            limit: 500,
        }
    }
}

/// Validated channel metadata returned by a Buzz community.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChannelRecord {
    /// Stable channel UUID from the NIP-29 `d` tag.
    pub channel_id: Uuid,
    /// Human-readable channel name.
    pub name: String,
    /// Human-readable channel description, when present.
    pub description: Option<String>,
    /// Relay event creation timestamp.
    pub created_at: u64,
    /// Relay-declared visibility, when present.
    pub visibility: Option<ChannelVisibility>,
}

impl BuzzClient {
    /// List channels from the bound community, following composite pagination.
    pub async fn list_channels(
        &self,
        request: ListChannelsRequest,
    ) -> Result<Vec<ChannelRecord>, ClientError> {
        if request.limit == 0 {
            return Err(ClientError::InvalidInput(
                "channel list limit must be greater than zero".into(),
            ));
        }
        let events = match request.scope {
            ChannelScope::Visible => {
                self.query_paginated(serde_json::json!({"kinds": [39000]}), request.limit)
                    .await?
            }
            ChannelScope::Member => {
                let memberships = self
                    .query_paginated(
                        serde_json::json!({
                            "kinds": [39002],
                            "#p": [self.public_key().to_hex()],
                        }),
                        request.limit,
                    )
                    .await?;
                let mut seen = HashSet::new();
                let channel_ids: Vec<String> = memberships
                    .iter()
                    .filter_map(|event| tag_value(event, "d"))
                    .filter(|id| seen.insert(id.clone()))
                    .collect();
                if channel_ids.is_empty() {
                    return Ok(Vec::new());
                }
                self.query_paginated(
                    serde_json::json!({"kinds": [39000], "#d": channel_ids}),
                    request.limit,
                )
                .await?
            }
        };

        let mut seen = HashSet::new();
        let mut channels = Vec::new();
        for event in events {
            let Some(channel) = channel_record(&event) else {
                continue;
            };
            if request
                .visibility
                .is_some_and(|visibility| channel.visibility != Some(visibility))
            {
                continue;
            }
            if seen.insert(channel.channel_id) {
                channels.push(channel);
            }
        }
        Ok(channels)
    }
}

fn channel_record(event: &serde_json::Value) -> Option<ChannelRecord> {
    let channel_id = Uuid::parse_str(&tag_value(event, "d")?).ok()?;
    let name = tag_value(event, "name").unwrap_or_default();
    let description = tag_value(event, "about");
    let created_at = event
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let visibility = if has_marker(event, "public") {
        Some(ChannelVisibility::Open)
    } else if has_marker(event, "private") {
        Some(ChannelVisibility::Private)
    } else {
        None
    };
    Some(ChannelRecord {
        channel_id,
        name,
        description,
        created_at,
        visibility,
    })
}

fn tag_value(event: &serde_json::Value, key: &str) -> Option<String> {
    event
        .get("tags")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_array)
        .find(|parts| parts.first().and_then(serde_json::Value::as_str) == Some(key))?
        .get(1)?
        .as_str()
        .map(str::to_string)
}

fn has_marker(event: &serde_json::Value, marker: &str) -> bool {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_array().is_some_and(|parts| {
                    parts.len() == 1
                        && parts.first().and_then(serde_json::Value::as_str) == Some(marker)
                })
            })
        })
}
