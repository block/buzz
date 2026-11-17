//! Bounded, provider-owned profile details. These are display data, never authorization.

use serde::{Deserialize, Serialize};

/// Optional directory snapshot supplied by an authenticated issuer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedProfile {
    /// Version of this display contract.
    pub version: u8,
    /// Human-readable name of the directory managing these fields.
    pub source_label: String,
    /// HTTPS link to this person's directory profile.
    pub source_url: String,
    /// Ordered, read-only profile fields.
    pub fields: Vec<ManagedProfileField>,
    /// Optional city for compact local-time display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// Optional IANA timezone; clients must validate before calculating time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Suggested avatar; never overrides a person's explicit choice or removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}

/// One plain-text directory field, with an optional related directory username.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedProfileField {
    /// Stable provider field key.
    pub key: String,
    /// Display group: personal, team, or location.
    pub section: String,
    /// Display label.
    pub label: String,
    /// Plain-text value. An empty value represents a known empty field.
    pub value: String,
    /// Optional related person's verified username in this community.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl std::fmt::Debug for ManagedProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedProfile([REDACTED])")
    }
}

impl ManagedProfile {
    /// Invalid optional enrichment is omitted without affecting authentication or names.
    pub fn from_json(value: &str) -> Option<Self> {
        if value.len() > 16_384 {
            return None;
        }
        let profile: Self = serde_json::from_str(value).ok()?;
        profile.valid().then_some(profile)
    }

    /// Validate resource bounds and links before publishing issuer-provided display data.
    pub fn valid(&self) -> bool {
        let mut keys = std::collections::HashSet::new();
        self.version == 1
            && text(&self.source_label, 64, false)
            && https(&self.source_url)
            && self.fields.len() <= 16
            && self.fields.iter().all(|field| {
                handle(&field.key)
                    && keys.insert(&field.key)
                    && matches!(field.section.as_str(), "personal" | "team" | "location")
                    && text(&field.label, 64, false)
                    && text(&field.value, 512, true)
                    && field.username.as_deref().is_none_or(handle)
            })
            && self.city.as_deref().is_none_or(|v| text(v, 128, false))
            && self.timezone.as_deref().is_none_or(|v| text(v, 128, false))
            && self.picture.as_deref().is_none_or(https)
    }
}

fn text(value: &str, max: usize, empty: bool) -> bool {
    (empty || !value.is_empty())
        && value.trim() == value
        && value.chars().count() <= max
        && !value.chars().any(char::is_control)
}

fn handle(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}

fn https(value: &str) -> bool {
    value.len() <= 2048
        && url::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_directory_contract() {
        let valid = serde_json::json!({"version":1,"source_label":"Directory","source_url":"https://example.org/people/alice","fields":[{"key":"team","section":"team","label":"Team","value":"Design"}]});
        assert!(ManagedProfile::from_json(&valid.to_string()).is_some());
        for (key, value) in [
            ("version", serde_json::json!(2)),
            ("source_url", serde_json::json!("javascript:alert(1)")),
            (
                "source_url",
                serde_json::json!("https://user:pass@example.org"),
            ),
            ("picture", serde_json::json!("http://example.org/photo.png")),
            ("city", serde_json::json!("London\n")),
            (
                "fields",
                serde_json::json!([valid["fields"][0], valid["fields"][0]]),
            ),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(ManagedProfile::from_json(&invalid.to_string()).is_none());
        }
        assert!(ManagedProfile::from_json(&" ".repeat(16_385)).is_none());
        assert!(ManagedProfile::from_json(r#"{"version":1,"version":1,"source_label":"Directory","source_url":"https://example.org","fields":[]}"#).is_none());
    }
}
