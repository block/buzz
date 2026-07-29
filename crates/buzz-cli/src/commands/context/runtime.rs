use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use nostr::{Event, EventBuilder, Timestamp};

use super::profile::{ProfileEnvironment, ResolvedProfile};
use crate::client::BuzzClient;
use crate::error::CliError;

pub struct ContextRuntime<'a> {
    pub profile: &'a ResolvedProfile,
    pub environment: &'a ProfileEnvironment,
}

impl<'a> ContextRuntime<'a> {
    pub fn new(
        profile: &'a ResolvedProfile,
        environment: &'a ProfileEnvironment,
    ) -> Result<Self, CliError> {
        profile.require_ready()?;
        Ok(Self {
            profile,
            environment,
        })
    }

    pub fn local_journal_client(&self) -> Result<BuzzClient, CliError> {
        self.profile.client_for(
            "journal_author",
            &self.profile.file.relays.local,
            self.environment,
        )
    }

    pub fn local_event_client(&self) -> Result<(BuzzClient, &'static str), CliError> {
        let role = if self.profile.file.identities.agent.is_some() {
            "agent"
        } else {
            "journal_author"
        };
        Ok((
            self.profile
                .client_for(role, &self.profile.file.relays.local, self.environment)?,
            role,
        ))
    }

    pub fn cloud_reader_client(&self) -> Result<BuzzClient, CliError> {
        let relay = self.rendezvous()?;
        let role = if self.profile.file.identities.replication_transport.is_some() {
            "replication_transport"
        } else {
            "journal_author"
        };
        self.profile.client_for(role, relay, self.environment)
    }

    pub fn cloud_journal_client(&self) -> Result<BuzzClient, CliError> {
        self.profile
            .client_for("journal_author", self.rendezvous()?, self.environment)
    }

    pub fn cloud_artifact_client(&self) -> Result<BuzzClient, CliError> {
        self.profile.client_for(
            "artifact_source_reader",
            self.rendezvous()?,
            self.environment,
        )
    }

    pub fn local_artifact_client(&self) -> Result<BuzzClient, CliError> {
        self.profile.client_for(
            "artifact_destination_owner",
            &self.profile.file.relays.local,
            self.environment,
        )
    }

    pub fn cloud_artifact_uploader_client(&self) -> Result<BuzzClient, CliError> {
        let role = if self
            .profile
            .file
            .identities
            .artifact_rendezvous_uploader
            .is_some()
        {
            "artifact_rendezvous_uploader"
        } else {
            "journal_author"
        };
        self.profile
            .client_for(role, self.rendezvous()?, self.environment)
    }

    pub fn rendezvous(&self) -> Result<&str, CliError> {
        self.profile
            .file
            .relays
            .rendezvous
            .as_deref()
            .ok_or_else(|| {
                CliError::Usage(format!(
                    "profile {} does not configure relays.rendezvous",
                    self.profile.name
                ))
            })
    }

    pub fn node_label(&self) -> &str {
        self.profile
            .file
            .node
            .label
            .as_deref()
            .unwrap_or(&self.profile.name)
    }

    pub fn identity_label(&self, role: &str) -> String {
        self.profile
            .identity(role)
            .and_then(|identity| identity.label.as_deref())
            .unwrap_or(role)
            .to_string()
    }

    pub fn default_context(&self) -> Result<&str, CliError> {
        self.profile
            .file
            .context
            .default_h
            .as_deref()
            .ok_or_else(|| {
                CliError::Usage(format!(
                    "profile {} does not configure context.default_h",
                    self.profile.name
                ))
            })
    }

    pub async fn post_builder(&self, builder: EventBuilder) -> Result<String, CliError> {
        let (client, _) = self.local_event_client()?;
        let event = client.sign_event(builder)?;
        submit_checked(&client, event).await
    }

    pub async fn post_owner_builder(&self, builder: EventBuilder) -> Result<String, CliError> {
        let client = self.local_journal_client()?;
        let event = client.sign_event(builder)?;
        submit_checked(&client, event).await
    }

    pub async fn query_preferred(
        &self,
        filters: &[serde_json::Value],
    ) -> Result<Vec<Event>, CliError> {
        let local = self.local_journal_client()?;
        match query_events(&local, filters).await {
            Ok(events) => Ok(events),
            Err(local_error) => {
                let cloud = self.cloud_reader_client().map_err(|_| local_error)?;
                query_events(&cloud, filters).await
            }
        }
    }

    pub async fn query_union(&self, filters: &[serde_json::Value]) -> Result<Vec<Event>, CliError> {
        let local = self.local_journal_client()?;
        let local_events = query_events(&local, filters).await?;
        if self.profile.file.relays.rendezvous.is_none() {
            return Ok(local_events);
        }
        let cloud = self.cloud_reader_client()?;
        let cloud_events = query_events(&cloud, filters).await?;
        let mut unique = BTreeMap::new();
        for event in local_events.into_iter().chain(cloud_events) {
            unique.insert(event.id.to_hex(), event);
        }
        Ok(unique.into_values().collect())
    }
}

pub async fn query_events(
    client: &BuzzClient,
    filters: &[serde_json::Value],
) -> Result<Vec<Event>, CliError> {
    let raw = client.query_multi(filters).await?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("relay query returned invalid JSON: {error}")))?;
    let mut events = Vec::with_capacity(values.len());
    for value in values {
        let event: Event = serde_json::from_value(value).map_err(|error| {
            CliError::Other(format!("relay returned an invalid event: {error}"))
        })?;
        event.verify().map_err(|error| {
            CliError::Other(format!("relay returned an unsigned event: {error}"))
        })?;
        events.push(event);
    }
    Ok(events)
}

pub async fn submit_checked(client: &BuzzClient, event: Event) -> Result<String, CliError> {
    let expected_id = event.id.to_hex();
    let raw = client.submit_event(event).await?;
    let receipt: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("relay returned an invalid receipt: {error}")))?;
    if receipt.get("accepted").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(CliError::Other(format!(
            "relay rejected event: {}",
            receipt
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no reason")
        )));
    }
    let returned_id = receipt
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&expected_id);
    if returned_id != expected_id {
        return Err(CliError::Other(format!(
            "relay receipt changed event identity: expected {expected_id}, got {returned_id}"
        )));
    }
    Ok(expected_id)
}

pub fn event_builder(
    content: impl Into<String>,
    tags: impl IntoIterator<Item = nostr::Tag>,
    created_at: Option<u64>,
) -> EventBuilder {
    let builder = EventBuilder::new(nostr::Kind::TextNote, content).tags(tags);
    if let Some(created_at) = created_at {
        builder.custom_created_at(Timestamp::from_secs(created_at))
    } else {
        builder
    }
}

pub fn read_text(path: Option<&Path>) -> Result<String, CliError> {
    match path {
        Some(path) if path != Path::new("-") => std::fs::read_to_string(path).map_err(|error| {
            CliError::Usage(format!("could not read {}: {error}", path.display()))
        }),
        _ => {
            let mut value = String::new();
            std::io::stdin()
                .read_to_string(&mut value)
                .map_err(|error| CliError::Other(format!("could not read stdin: {error}")))?;
            Ok(value)
        }
    }
}

pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|hostname| hostname.trim().to_string())
        .filter(|hostname| !hostname.is_empty())
        .unwrap_or_else(|| "unknown-host".into())
}

pub fn tag(values: &[&str]) -> Result<nostr::Tag, CliError> {
    nostr::Tag::parse(values.iter().copied())
        .map_err(|error| CliError::Other(format!("could not build event tag: {error}")))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn parse_id(value: &str, label: &str) -> Result<String, CliError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CliError::Usage(format!(
            "{label} must be 64 lowercase hexadecimal characters"
        )));
    }
    Ok(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_canonical_lowercase() {
        assert!(parse_id(&"a".repeat(64), "id").is_ok());
        assert!(parse_id(&"A".repeat(64), "id").is_err());
        assert!(parse_id("short", "id").is_err());
    }
}
