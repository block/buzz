//! Fold specs: the saved definition of one accumulator.
//!
//! A spec is a named, saveable JSON definition; the daemon persists it in
//! local SQLite, keyed by name. There is deliberately no cadence field:
//! *when* to run belongs to the caller — a human pressing Run today, a
//! scheduler later. The spec only says what to read and how to fold it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::selection::Selection;

/// Maximum fold-name length (also the `d` tag value).
pub const MAX_NAME_LEN: usize = 64;

/// Default fold *task* instructions when the caller supplies none.
pub const DEFAULT_INSTRUCTIONS: &str = "Maintain a running digest of the selection: rewrite the \
prior version in light of the new events. Keep it concise, concrete, and factual.";

/// One fold definition: a factory for artifacts.
///
/// A fold is exactly name + selection + model + instructions — nothing else.
/// The selection's own window decides its lifecycle: frozen (pinned end) folds
/// run until their set is covered and are then done forever; live folds are
/// never done.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoldSpec {
    /// Fold name: 1–64 chars of `[a-z0-9-]`, no leading/trailing dash.
    pub name: String,
    /// What to read (who × what × when; frozen or live).
    pub selection: Selection,
    /// Model alias passed to the runner and priced by [`crate::estimate`].
    pub model: String,
    /// Fold instructions (the prompt).
    pub instructions: String,
    /// Free-form client-owned JSON, persisted verbatim with the spec.
    ///
    /// The engine never reads it and it is deliberately absent from the
    /// cached-run comparison ([`crate::plan_run`] compares model,
    /// prompt hash, and selection field-by-field) — so a client can stash its
    /// strategy metadata here without invalidating chains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl FoldSpec {
    /// Validate the spec and canonicalize its selection in place.
    pub fn validate(&mut self) -> Result<(), Error> {
        let name_ok = !self.name.is_empty()
            && self.name.len() <= MAX_NAME_LEN
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !self.name.starts_with('-')
            && !self.name.ends_with('-');
        if !name_ok {
            return Err(Error::InvalidSpec(format!(
                "name {:?} must be 1-{MAX_NAME_LEN} chars of [a-z0-9-] with no edge dashes",
                self.name
            )));
        }
        if self.model.trim().is_empty() {
            return Err(Error::InvalidSpec(
                "model is required — estimates and runs must name the model".into(),
            ));
        }
        if self.instructions.trim().is_empty() {
            return Err(Error::InvalidSpec("instructions must be non-empty".into()));
        }
        self.selection.canonicalize()
    }

    /// SHA-256 hex of the instructions. Stored on every artifact version so a
    /// prompt change is visible in provenance and invalidates the cached-run
    /// shortcut.
    pub fn prompt_sha256(&self) -> String {
        let digest = Sha256::digest(self.instructions.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> FoldSpec {
        FoldSpec {
            name: "team-digest".to_string(),
            selection: Selection {
                channels: vec!["59ca5528-71ea-4a53-a7f5-90c9fb2b1729".to_string()],
                ..Selection::default()
            },
            model: "haiku".to_string(),
            instructions: "Maintain the digest.".to_string(),
            meta: None,
        }
    }

    #[test]
    fn valid_spec_passes() {
        assert!(valid().validate().is_ok());
    }

    #[test]
    fn bad_names_are_rejected() {
        for name in ["", "-lead", "trail-", "Has Caps", "way!", &"x".repeat(65)] {
            let mut s = valid();
            s.name = name.to_string();
            assert!(s.validate().is_err(), "name {name:?} should be rejected");
        }
    }

    #[test]
    fn empty_fields_are_rejected() {
        let mut s = valid();
        s.model = " ".to_string();
        assert!(s.validate().is_err());
        let mut s = valid();
        s.instructions = "".to_string();
        assert!(s.validate().is_err());
    }

    #[test]
    fn legacy_spec_json_still_loads() {
        // Specs persisted before `meta` existed — and before `schema` was
        // removed — must keep loading (unknown fields are ignored).
        let legacy = serde_json::to_string(&valid()).expect("serialize");
        assert!(!legacy.contains("meta"), "None meta must not serialize");
        let loaded: FoldSpec = serde_json::from_str(&legacy).expect("deserialize");
        assert_eq!(loaded.meta, None);
        let with_schema = r#"{
            "name": "old",
            "selection": { "channels": ["59ca5528-71ea-4a53-a7f5-90c9fb2b1729"] },
            "schema": "channel-digest@v1",
            "model": "haiku",
            "instructions": "digest"
        }"#;
        let loaded: FoldSpec = serde_json::from_str(with_schema).expect("legacy schema field");
        assert_eq!(loaded.name, "old");
    }

    #[test]
    fn prompt_hash_tracks_instructions_only() {
        let a = valid();
        let mut b = valid();
        b.model = "opus".to_string();
        assert_eq!(a.prompt_sha256(), b.prompt_sha256());
        let mut c = valid();
        c.instructions.push('!');
        assert_ne!(a.prompt_sha256(), c.prompt_sha256());
        assert_eq!(a.prompt_sha256().len(), 64);
    }
}
