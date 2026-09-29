//! Community channel sections (kind:30625).
//!
//! Owner/admin-authored NIP-33 replaceable catalog. Members fetch the catalog
//! and opt in (subscribe) client-side so sections appear on their left nav.
//! Personal custom sections stay on kind:30078 and are unrelated.

use serde::{Deserialize, Serialize};

/// d-tag for the community sections catalog.
pub const COMMUNITY_SECTIONS_D_TAG: &str = "buzz:community-sections";

/// Maximum number of community sections in one catalog.
pub const MAX_COMMUNITY_SECTIONS: usize = 50;

/// Maximum channels listed in one section.
pub const MAX_SECTION_CHANNELS: usize = 200;

/// Maximum length of a section id or name.
pub const MAX_SECTION_NAME_LEN: usize = 80;

/// Maximum length of an optional icon string.
pub const MAX_SECTION_ICON_LEN: usize = 80;

/// Maximum length of a channel id string.
pub const MAX_CHANNEL_ID_LEN: usize = 80;

/// One admin-defined community section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySection {
    /// Stable section id (UUID or other unique string).
    pub id: String,
    /// Display name in the left nav / catalog.
    pub name: String,
    /// Optional Lucide/emoji icon hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Ascending display order (lower first).
    pub order: i32,
    /// Channel ids included in this section (same for every subscriber).
    #[serde(rename = "channelIds", default)]
    pub channel_ids: Vec<String>,
}

/// Versioned community sections catalog stored in event content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySectionsPayload {
    /// Payload version. Currently `1`.
    pub version: u32,
    /// Admin-defined sections.
    pub sections: Vec<CommunitySection>,
}

/// Validate a kind:30625 event's `d` tag and JSON content.
pub fn validate_community_sections_payload(
    d_tag: Option<&str>,
    content: &str,
) -> Result<CommunitySectionsPayload, String> {
    if d_tag != Some(COMMUNITY_SECTIONS_D_TAG) {
        return Err(format!("d tag must be {COMMUNITY_SECTIONS_D_TAG}"));
    }
    let payload: CommunitySectionsPayload = serde_json::from_str(content)
        .map_err(|error| format!("community-sections content is not valid JSON: {error}"))?;
    if payload.version != 1 {
        return Err(format!(
            "unsupported community-sections version {}",
            payload.version
        ));
    }
    if payload.sections.len() > MAX_COMMUNITY_SECTIONS {
        return Err(format!(
            "too many community sections (max {MAX_COMMUNITY_SECTIONS})"
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for section in &payload.sections {
        validate_section(section)?;
        if !seen.insert(section.id.as_str()) {
            return Err(format!("duplicate section id {}", section.id));
        }
    }
    Ok(payload)
}

fn validate_section(section: &CommunitySection) -> Result<(), String> {
    validate_id_token("section id", &section.id)?;
    let name = section.name.trim();
    if name.is_empty() || name.len() > MAX_SECTION_NAME_LEN {
        return Err(format!(
            "section name is required and must be at most {MAX_SECTION_NAME_LEN} characters"
        ));
    }
    if let Some(icon) = &section.icon {
        let trimmed = icon.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_SECTION_ICON_LEN {
            return Err(format!(
                "section icon must be at most {MAX_SECTION_ICON_LEN} characters when set"
            ));
        }
    }
    if section.channel_ids.len() > MAX_SECTION_CHANNELS {
        return Err(format!(
            "too many channels in section {} (max {MAX_SECTION_CHANNELS})",
            section.id
        ));
    }
    let mut seen_channels = std::collections::BTreeSet::new();
    for channel_id in &section.channel_ids {
        validate_id_token("channel id", channel_id)?;
        if !seen_channels.insert(channel_id.as_str()) {
            return Err(format!(
                "duplicate channel id {channel_id} in section {}",
                section.id
            ));
        }
    }
    Ok(())
}

fn validate_id_token(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > MAX_CHANNEL_ID_LEN {
        return Err(format!(
            "{label} is required and must be at most {MAX_CHANNEL_ID_LEN} characters"
        ));
    }
    if value
        .chars()
        .any(|ch| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
    {
        return Err(format!(
            "{label} may only contain ASCII letters, digits, hyphen, or underscore"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> String {
        serde_json::json!({
            "version": 1,
            "sections": [{
                "id": "eng",
                "name": "Engineering",
                "order": 0,
                "channelIds": ["chan-1", "chan-2"]
            }]
        })
        .to_string()
    }

    #[test]
    fn accepts_valid_payload() {
        validate_community_sections_payload(Some(COMMUNITY_SECTIONS_D_TAG), &valid_json())
            .expect("valid");
    }

    #[test]
    fn rejects_wrong_d_tag() {
        assert!(validate_community_sections_payload(Some("other"), &valid_json()).is_err());
    }

    #[test]
    fn rejects_duplicate_section_ids() {
        let json = serde_json::json!({
            "version": 1,
            "sections": [
                {"id": "eng", "name": "A", "order": 0, "channelIds": []},
                {"id": "eng", "name": "B", "order": 1, "channelIds": []}
            ]
        })
        .to_string();
        assert!(validate_community_sections_payload(Some(COMMUNITY_SECTIONS_D_TAG), &json).is_err());
    }

    #[test]
    fn rejects_bad_channel_id() {
        let json = serde_json::json!({
            "version": 1,
            "sections": [{
                "id": "eng",
                "name": "Engineering",
                "order": 0,
                "channelIds": ["bad id!"]
            }]
        })
        .to_string();
        assert!(validate_community_sections_payload(Some(COMMUNITY_SECTIONS_D_TAG), &json).is_err());
    }
}
