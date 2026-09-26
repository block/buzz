//! Signed, channel-scoped extension-panel manifest types.
//!
//! The manifest is a generic projection contract. It does not assign a Nostr
//! event kind or decide which integration owns a field; transport and domain
//! semantics remain outside `buzz-core`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

/// Current signed panel-manifest schema version.
pub const PANEL_MANIFEST_SCHEMA_VERSION: u16 = 1;
/// Maximum serialized UTF-8 manifest size, leaving room for an event envelope.
pub const MAX_PANEL_MANIFEST_BYTES: usize = 32 * 1024;
/// Maximum number of sections in a manifest.
pub const MAX_PANEL_SECTIONS: usize = 32;
/// Maximum number of fields in one section.
pub const MAX_PANEL_FIELDS_PER_SECTION: usize = 64;
/// Maximum number of links in one section.
pub const MAX_PANEL_LINKS_PER_SECTION: usize = 32;
/// Maximum number of source-event references in a manifest.
pub const MAX_PANEL_SOURCE_EVENTS: usize = 64;

const MAX_PANEL_ID_BYTES: usize = 128;
const MAX_PANEL_LABEL_BYTES: usize = 256;
const MAX_PANEL_VALUE_BYTES: usize = 4_096;

/// Validation failures for a [`PanelManifest`].
#[derive(Debug, Error)]
pub enum PanelManifestError {
    /// The JSON was not a manifest object or contained an unknown field.
    #[error("invalid panel manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A decoded manifest violated the bounded contract.
    #[error("invalid panel manifest: {0}")]
    Invalid(String),
}

/// A generic, read-only projection of signed channel state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelManifest {
    /// Exact schema version understood by this client.
    pub schema_version: u16,
    /// Stable identifier for this panel within its channel.
    pub panel_id: String,
    /// Canonical UUID of the channel that owns this projection.
    pub channel_id: String,
    /// Short human-readable panel title.
    pub title: String,
    /// Optional plain-text context for the panel.
    pub description: Option<String>,
    /// Overall panel status.
    pub status: PanelStatus,
    /// Unix timestamp in seconds for the latest source update.
    pub updated_at: u64,
    /// Structured sections rendered by the client.
    pub sections: Vec<PanelSection>,
    /// Signed source-event references supporting the projection.
    pub source_events: Vec<PanelSourceEvent>,
}

/// Bounded status vocabulary shared by a panel and its sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelStatus {
    /// Work has not started.
    Pending,
    /// Work is in progress.
    Active,
    /// Work completed successfully.
    Complete,
    /// Work needs an external decision or intervention.
    Blocked,
    /// Work failed.
    Failed,
    /// The projection may no longer reflect current source state.
    Stale,
    /// The source exists but is not currently available to the reader.
    Unavailable,
}

/// One named section of a panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelSection {
    /// Stable section identifier within the manifest.
    pub id: String,
    /// Short section heading.
    pub title: String,
    /// Current section status.
    pub status: PanelStatus,
    /// Human-readable fields in display order.
    pub fields: Vec<PanelField>,
    /// Typed links back to source events or an external HTTPS destination.
    pub links: Vec<PanelLink>,
}

/// One label/value pair in a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelField {
    /// Short field label.
    pub label: String,
    /// Plain-text field value.
    pub value: String,
    /// Allowlisted rendering hint.
    pub presentation: PanelPresentation,
}

/// Safe presentation hints understood by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelPresentation {
    /// Normal proportional text.
    Text,
    /// Text that benefits from fixed-width glyphs.
    Monospace,
    /// Unix seconds rendered as a localized timestamp.
    Timestamp,
    /// A value that should receive semantic status treatment.
    Status,
}

/// A typed link from a panel to a source event or external destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelLink {
    /// Short action label.
    pub label: String,
    /// Kind of destination the client should resolve.
    pub target: PanelLinkTarget,
    /// Source event for a Buzz-native destination.
    pub source_event_id: Option<String>,
    /// External destination. Only `https:` is permitted for this target.
    pub uri: Option<String>,
}

/// Allowlisted link destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelLinkTarget {
    /// A channel Canvas revision.
    Canvas,
    /// A workflow definition or run.
    Workflow,
    /// A structured job handoff/result.
    Handoff,
    /// A channel thread.
    Thread,
    /// A raw signed event.
    Event,
    /// A user-activated external HTTPS destination.
    External,
}

/// A signed event reference supporting the panel projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelSourceEvent {
    /// Lowercase 64-character event id.
    pub event_id: String,
    /// Nostr event kind.
    pub kind: u32,
    /// Channel containing the source event.
    pub channel_id: String,
    /// Short provenance label shown to the reader.
    pub label: String,
}

impl PanelManifest {
    /// Decode and validate a JSON manifest, including size limits.
    pub fn from_json(input: &str) -> Result<Self, PanelManifestError> {
        if input.len() > MAX_PANEL_MANIFEST_BYTES {
            return Err(PanelManifestError::Invalid(format!(
                "serialized content exceeds {MAX_PANEL_MANIFEST_BYTES} bytes"
            )));
        }

        let manifest: Self = serde_json::from_str(input)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate the manifest's structural, size, and identifier constraints.
    pub fn validate(&self) -> Result<(), PanelManifestError> {
        if self.schema_version != PANEL_MANIFEST_SCHEMA_VERSION {
            return invalid(format!(
                "unsupported schema version {}",
                self.schema_version
            ));
        }

        validate_identifier("panelId", &self.panel_id, MAX_PANEL_ID_BYTES)?;
        validate_channel_id("channelId", &self.channel_id)?;
        validate_text("title", &self.title, MAX_PANEL_LABEL_BYTES, true)?;
        if let Some(description) = &self.description {
            validate_text("description", description, MAX_PANEL_VALUE_BYTES, false)?;
        }

        if self.updated_at == 0 {
            return invalid("updatedAt must be a positive Unix timestamp");
        }
        if self.sections.len() > MAX_PANEL_SECTIONS {
            return invalid(format!("sections exceeds maximum of {MAX_PANEL_SECTIONS}"));
        }
        if self.source_events.len() > MAX_PANEL_SOURCE_EVENTS {
            return invalid(format!(
                "sourceEvents exceeds maximum of {MAX_PANEL_SOURCE_EVENTS}"
            ));
        }
        if self.source_events.is_empty() {
            return invalid("sourceEvents must contain at least one signed source");
        }

        let mut section_ids = HashSet::with_capacity(self.sections.len());
        for section in &self.sections {
            validate_identifier("section.id", &section.id, MAX_PANEL_ID_BYTES)?;
            validate_text("section.title", &section.title, MAX_PANEL_LABEL_BYTES, true)?;
            if !section_ids.insert(&section.id) {
                return invalid(format!("duplicate section id `{}`", section.id));
            }
            if section.fields.len() > MAX_PANEL_FIELDS_PER_SECTION {
                return invalid(format!(
                    "section `{}` fields exceeds maximum of {MAX_PANEL_FIELDS_PER_SECTION}",
                    section.id
                ));
            }
            if section.links.len() > MAX_PANEL_LINKS_PER_SECTION {
                return invalid(format!(
                    "section `{}` links exceeds maximum of {MAX_PANEL_LINKS_PER_SECTION}",
                    section.id
                ));
            }

            let mut field_labels = HashSet::with_capacity(section.fields.len());
            for field in &section.fields {
                validate_text("field.label", &field.label, MAX_PANEL_LABEL_BYTES, true)?;
                validate_text("field.value", &field.value, MAX_PANEL_VALUE_BYTES, false)?;
                if !field_labels.insert(&field.label) {
                    return invalid(format!(
                        "duplicate field label `{}` in section `{}`",
                        field.label, section.id
                    ));
                }
            }

            for link in &section.links {
                validate_text("link.label", &link.label, MAX_PANEL_LABEL_BYTES, true)?;
                validate_link(link)?;
            }
        }

        let mut source_event_ids = HashSet::with_capacity(self.source_events.len());
        for source in &self.source_events {
            validate_event_id("sourceEvents.eventId", &source.event_id)?;
            validate_channel_id("sourceEvents.channelId", &source.channel_id)?;
            if source.channel_id != self.channel_id {
                return invalid(format!(
                    "source event `{}` belongs to another channel",
                    source.event_id
                ));
            }
            validate_text(
                "sourceEvents.label",
                &source.label,
                MAX_PANEL_LABEL_BYTES,
                true,
            )?;
            if !source_event_ids.insert(&source.event_id) {
                return invalid(format!("duplicate source event id `{}`", source.event_id));
            }
        }

        let serialized = serde_json::to_vec(self)?;
        if serialized.len() > MAX_PANEL_MANIFEST_BYTES {
            return invalid(format!(
                "serialized content exceeds {MAX_PANEL_MANIFEST_BYTES} bytes"
            ));
        }
        Ok(())
    }

    /// Validate that the manifest is scoped to the reader's current channel.
    pub fn validate_for_channel(&self, channel_id: Uuid) -> Result<(), PanelManifestError> {
        self.validate()?;
        if self.channel_id != channel_id.to_string() {
            return invalid("manifest channel does not match the current channel");
        }
        Ok(())
    }
}

fn validate_link(link: &PanelLink) -> Result<(), PanelManifestError> {
    match link.target {
        PanelLinkTarget::External => {
            if link.source_event_id.is_some() {
                return invalid("external links cannot carry a source event id");
            }
            let Some(uri) = link.uri.as_deref() else {
                return invalid("external links require a URI");
            };
            let parsed = Url::parse(uri)
                .map_err(|_| PanelManifestError::Invalid("external URI is invalid".into()))?;
            if parsed.scheme() != "https" || parsed.host_str().is_none() {
                return invalid("external links require an HTTPS URI with a host");
            }
        }
        _ => {
            let Some(source_event_id) = link.source_event_id.as_deref() else {
                return invalid("Buzz-native links require a source event id");
            };
            validate_event_id("link.sourceEventId", source_event_id)?;
            if link.uri.is_some() {
                return invalid("Buzz-native links cannot carry an arbitrary URI");
            }
        }
    }
    Ok(())
}

fn validate_channel_id(field: &str, value: &str) -> Result<(), PanelManifestError> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| PanelManifestError::Invalid(format!("{field} is not a UUID")))?;
    if parsed.to_string() != value {
        return invalid(format!("{field} must use canonical lowercase UUID form"));
    }
    Ok(())
}

fn validate_event_id(field: &str, value: &str) -> Result<(), PanelManifestError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid(format!("{field} must be lowercase 64-character hex"));
    }
    Ok(())
}

fn validate_identifier(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), PanelManifestError> {
    validate_text(field, value, max_bytes, true)?;
    if !value.is_ascii() {
        return invalid(format!("{field} must contain ASCII characters only"));
    }
    Ok(())
}

fn validate_text(
    field: &str,
    value: &str,
    max_bytes: usize,
    require_non_empty: bool,
) -> Result<(), PanelManifestError> {
    if require_non_empty && value.is_empty() {
        return invalid(format!("{field} must not be empty"));
    }
    if value.len() > max_bytes {
        return invalid(format!("{field} exceeds maximum of {max_bytes} bytes"));
    }
    if value.chars().any(char::is_control) {
        return invalid(format!("{field} must not contain control characters"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, PanelManifestError> {
    Err(PanelManifestError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL_ID: &str = "00000000-0000-4000-8000-000000000001";

    fn valid_manifest() -> PanelManifest {
        PanelManifest {
            schema_version: PANEL_MANIFEST_SCHEMA_VERSION,
            panel_id: "panel".into(),
            channel_id: CHANNEL_ID.into(),
            title: "Panel".into(),
            description: Some("A projection".into()),
            status: PanelStatus::Active,
            updated_at: 1_760_000_000,
            sections: vec![PanelSection {
                id: "section".into(),
                title: "Section".into(),
                status: PanelStatus::Active,
                fields: vec![PanelField {
                    label: "State".into(),
                    value: "Active".into(),
                    presentation: PanelPresentation::Status,
                }],
                links: vec![PanelLink {
                    label: "Source".into(),
                    target: PanelLinkTarget::Event,
                    source_event_id: Some("1".repeat(64)),
                    uri: None,
                }],
            }],
            source_events: vec![PanelSourceEvent {
                event_id: "1".repeat(64),
                kind: 43004,
                channel_id: CHANNEL_ID.into(),
                label: "Result".into(),
            }],
        }
    }

    #[test]
    fn fixture_round_trips_and_validates() {
        let input = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/fixtures/signed-channel-panel.json"
        ));
        let manifest = PanelManifest::from_json(input).expect("fixture must validate");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.source_events.len(), 2);
    }

    #[test]
    fn valid_manifest_validates_for_current_channel() {
        let manifest = valid_manifest();
        assert!(manifest
            .validate_for_channel(Uuid::parse_str(CHANNEL_ID).expect("channel id"))
            .is_ok());
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let mut manifest = valid_manifest();
        manifest.schema_version = 2;
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn cross_channel_manifest_is_rejected() {
        let manifest = valid_manifest();
        assert!(manifest.validate_for_channel(Uuid::new_v4()).is_err());
    }

    #[test]
    fn unsafe_external_link_is_rejected() {
        let mut manifest = valid_manifest();
        manifest.sections[0].links = vec![PanelLink {
            label: "Remote".into(),
            target: PanelLinkTarget::External,
            source_event_id: None,
            uri: Some("javascript:alert(1)".into()),
        }];
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn duplicate_section_ids_are_rejected() {
        let mut manifest = valid_manifest();
        manifest.sections.push(manifest.sections[0].clone());
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn unknown_json_fields_are_rejected() {
        let mut value = serde_json::to_value(valid_manifest()).expect("serialize");
        value["unexpected"] = serde_json::Value::Bool(true);
        assert!(PanelManifest::from_json(&value.to_string()).is_err());
    }
}
