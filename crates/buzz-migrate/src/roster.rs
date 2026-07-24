//! The Slack export roster: the trusted mapping from an email address to a
//! Slack user id, loaded from `users.json`.
//!
//! This mapping is what makes the email channel an *identity* proof rather than
//! just an email-ownership proof: the operator's own export says "this email
//! belongs to Slack user U060", so proving control of the email proves control
//! of U060. The roster is loaded once at service start from the same export the
//! history import used.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
struct RawUser {
    id: String,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    profile: RawProfile,
}

#[derive(Default, Deserialize)]
struct RawProfile {
    #[serde(default)]
    email: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    real_name: String,
}

/// Email→subject and subject→name lookups for the active migration.
#[derive(Debug, Default, Clone)]
pub struct Roster {
    /// lowercased, trimmed email → `slack:<team id>:<user id>`.
    email_to_subject: HashMap<String, String>,
    /// `slack:<team id>:<user id>` → best human-readable name (for email copy).
    subject_to_name: HashMap<String, String>,
    /// Emails shared by more than one active Slack user. These are never
    /// eligible for magic-link attribution because ownership is ambiguous.
    ambiguous_emails: HashSet<String>,
}

impl Roster {
    /// Build a roster from the bytes of a Slack export `users.json`.
    ///
    /// Deactivated (`deleted`) users are skipped: their email can no longer
    /// receive the magic link, so they can only be attributed by the OIDC
    /// channel or a manual `buzz import bind`.
    pub fn from_users_json(bytes: &[u8], team_id: &str) -> Result<Self, serde_json::Error> {
        let users: Vec<RawUser> = serde_json::from_slice(bytes)?;
        let mut email_to_subject = HashMap::new();
        let mut subject_to_name = HashMap::new();
        let mut ambiguous_emails = HashSet::new();
        for u in users {
            if u.deleted {
                continue;
            }
            let subject = format!("slack:{team_id}:{}", u.id);
            let name = if !u.profile.display_name.is_empty() {
                u.profile.display_name
            } else if !u.profile.real_name.is_empty() {
                u.profile.real_name
            } else {
                u.id.clone()
            };
            subject_to_name.insert(subject.clone(), name);
            let email = u.profile.email.trim().to_lowercase();
            if email.is_empty() || ambiguous_emails.contains(&email) {
                continue;
            }
            if email_to_subject.insert(email.clone(), subject).is_some() {
                email_to_subject.remove(&email);
                ambiguous_emails.insert(email);
            }
        }
        Ok(Self {
            email_to_subject,
            subject_to_name,
            ambiguous_emails,
        })
    }

    /// The `slack:<team>:<user>` subject for an email, if the export knows it. Matching
    /// is case-insensitive and whitespace-trimmed.
    pub fn subject_for_email(&self, email: &str) -> Option<&str> {
        let email = email.trim().to_lowercase();
        // Defense in depth: shared emails are already absent from the map, but
        // reject them explicitly so an ambiguous address can never resolve.
        if self.ambiguous_emails.contains(&email) {
            return None;
        }
        self.email_to_subject.get(&email).map(String::as_str)
    }

    /// The display name for a subject, for personalizing the email.
    pub fn name_for_subject(&self, subject: &str) -> Option<&str> {
        self.subject_to_name.get(subject).map(String::as_str)
    }

    /// Number of mailable (non-deactivated, has-email) users.
    pub fn mailable_count(&self) -> usize {
        self.email_to_subject.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERS: &str = r#"[
      {"id":"U060","profile":{"email":"Alice@Corp.com","display_name":"Alice"}},
      {"id":"U081","profile":{"email":" bob@corp.com ","real_name":"Bob B"}},
      {"id":"U099","deleted":true,"profile":{"email":"ghost@corp.com","display_name":"Ghost"}},
      {"id":"U100","profile":{"display_name":"NoEmail"}}
    ]"#;

    #[test]
    fn maps_email_to_subject_case_insensitively() {
        let r = Roster::from_users_json(USERS.as_bytes(), "T1").unwrap();
        assert_eq!(r.subject_for_email("alice@corp.com"), Some("slack:T1:U060"));
        // Original casing and surrounding whitespace both normalize.
        assert_eq!(
            r.subject_for_email("  BOB@CORP.COM "),
            Some("slack:T1:U081")
        );
    }

    #[test]
    fn deactivated_users_are_excluded() {
        let r = Roster::from_users_json(USERS.as_bytes(), "T1").unwrap();
        assert_eq!(r.subject_for_email("ghost@corp.com"), None);
    }

    #[test]
    fn users_without_email_are_not_mailable_but_named() {
        let r = Roster::from_users_json(USERS.as_bytes(), "T1").unwrap();
        assert_eq!(r.mailable_count(), 2); // U060, U081
        assert_eq!(r.name_for_subject("slack:T1:U100"), Some("NoEmail"));
    }

    #[test]
    fn best_name_falls_back_display_then_real_then_id() {
        let r = Roster::from_users_json(USERS.as_bytes(), "T1").unwrap();
        assert_eq!(r.name_for_subject("slack:T1:U060"), Some("Alice"));
        assert_eq!(r.name_for_subject("slack:T1:U081"), Some("Bob B"));
    }

    #[test]
    fn unknown_email_is_none() {
        let r = Roster::from_users_json(USERS.as_bytes(), "T1").unwrap();
        assert_eq!(r.subject_for_email("nobody@corp.com"), None);
    }

    #[test]
    fn duplicate_email_is_ambiguous_and_not_mailable() {
        let users = r#"[
          {"id":"U1","profile":{"email":"shared@corp.com"}},
          {"id":"U2","profile":{"email":"SHARED@corp.com"}}
        ]"#;
        let r = Roster::from_users_json(users.as_bytes(), "T1").unwrap();
        assert_eq!(r.subject_for_email("shared@corp.com"), None);
        assert_eq!(r.mailable_count(), 0);
    }
}
