//! Optional, output-only NIP-FI name claims. Never used for authorization.

use serde::{Deserialize, Serialize};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// The names an issuer explicitly authorizes the relay to publish.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicProfile {
    /// NFC display text, bounded to 64 Unicode scalar values.
    pub display_name: String,
    /// Lowercase ASCII handle.
    pub username: String,
    /// Optional full name, with the same Unicode and length bounds as display text.
    #[serde(
        default,
        deserialize_with = "optional_full_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub full_name: Option<String>,
}

fn optional_full_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

impl std::fmt::Debug for PublicProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublicProfile([REDACTED])")
    }
}

impl PublicProfile {
    /// Malformed optional claims are absent, never an admission error.
    pub fn from_json(value: &str) -> Option<Self> {
        let profile: Self = serde_json::from_str(value).ok()?;
        (valid_display_name(&profile.display_name)
            && valid_username(&profile.username)
            && profile.full_name.as_deref().is_none_or(valid_display_name))
        .then_some(profile)
    }
}

/// Validate the protocol's display-name and public-issuer-label text rules.
pub fn valid_display_name(value: &str) -> bool {
    use GeneralCategory::{
        Control, Format, LineSeparator, ParagraphSeparator, PrivateUse, Surrogate, Unassigned,
    };
    !value.is_empty()
        && value.chars().count() <= 64
        && value.trim() == value
        && unicode_normalization::is_nfc(value)
        && !value.chars().any(|c| {
            matches!(
                c.general_category(),
                Control
                    | Format
                    | Surrogate
                    | PrivateUse
                    | Unassigned
                    | LineSeparator
                    | ParagraphSeparator
            )
        })
}

/// Validate a NIP-FI public username.
pub fn valid_username(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}

/// Verified projection input, kept separate from authoritative capabilities.
#[derive(Clone, PartialEq, Eq)]
pub struct ProfileAssertion {
    /// Authenticated assertion issuance time, including fractional seconds.
    pub issued_at: chrono::DateTime<chrono::Utc>,
    /// None for agents and malformed/missing human claims: publish nothing.
    pub profile: Option<PublicProfile>,
}

impl std::fmt::Debug for ProfileAssertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProfileAssertion([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_full_name_is_validated_without_requiring_it_for_older_issuers() {
        let profile = PublicProfile::from_json(
            r#"{"display_name":"alice","username":"alice","full_name":"Alice Example"}"#,
        )
        .unwrap();
        assert_eq!(profile.full_name.as_deref(), Some("Alice Example"));
        for full_name in [
            serde_json::json!(null),
            serde_json::json!(42),
            serde_json::json!(""),
            serde_json::json!(" A"),
            serde_json::json!("A\nB"),
        ] {
            assert!(PublicProfile::from_json(&serde_json::json!({"display_name":"alice","username":"alice","full_name":full_name}).to_string()).is_none());
        }
    }

    #[test]
    fn claim_is_closed_and_unicode_rules_are_exact() {
        for name in ["Alice", "Élodie", "李明", "🦀", &"a".repeat(64)] {
            assert!(valid_display_name(name), "{name:?}");
        }
        for name in [
            "",
            " Alice",
            "Alice\u{a0}",
            "E\u{301}",
            "a\n",
            "a\u{200d}b",
            "a\u{e000}",
            "a\u{378}",
            "a\u{2028}",
            &"a".repeat(65),
        ] {
            assert!(!valid_display_name(name), "{name:?}");
        }
        for value in [
            serde_json::json!(null),
            serde_json::json!({"display_name":"Alice"}),
            serde_json::json!({"display_name":"Alice","username":"Alice"}),
            serde_json::json!({"display_name":"Alice","username":"alice","email":"private@example.com"}),
        ] {
            assert!(PublicProfile::from_json(&value.to_string()).is_none());
        }
        assert!(PublicProfile::from_json(
            r#"{"display_name":"Alice","display_name":"Bob","username":"alice"}"#
        )
        .is_none());
        assert!(PublicProfile::from_json(
            &serde_json::json!({"display_name":"Alice","username":"alice.s_1-"}).to_string()
        )
        .is_some());
    }
}
