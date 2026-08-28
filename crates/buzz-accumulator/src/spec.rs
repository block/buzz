//! Fold specs: the saved definition of one accumulator.
//!
//! A spec is the JSON content of an addressable relay event (`d` tag = fold
//! name, last-write-wins), NIP-44-encrypted to its author. There is
//! deliberately no cadence field: *when* to run belongs to the caller — a
//! human pressing Run today, a `buzz-workflow` schedule trigger later. The
//! spec only says what to read and how to fold it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::schema;
use crate::selection::Selection;

/// Maximum fold-name length (also the `d` tag value).
pub const MAX_NAME_LEN: usize = 64;

/// One fold definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldSpec {
    /// Fold name: 1–64 chars of `[a-z0-9-]`, no leading/trailing dash.
    pub name: String,
    /// What to read.
    pub selection: Selection,
    /// Built-in schema name (see [`crate::schema::BUILTIN_SCHEMAS`]).
    pub schema: String,
    /// Model alias passed to the runner and priced by [`crate::estimate`].
    pub model: String,
    /// Fold instructions (the prompt).
    pub instructions: String,
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
        if schema::builtin(&self.schema).is_none() {
            let known: Vec<&str> = schema::BUILTIN_SCHEMAS.iter().map(|s| s.name).collect();
            return Err(Error::InvalidSpec(format!(
                "unknown schema {:?}; built-ins: {known:?}",
                self.schema
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
                channels: vec!["ch1".to_string()],
                authors: vec![],
                kinds: vec![],
            },
            schema: "channel-digest@v1".to_string(),
            model: "haiku".to_string(),
            instructions: "Maintain the digest.".to_string(),
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
    fn unknown_schema_and_empty_fields_are_rejected() {
        let mut s = valid();
        s.schema = "nope@v1".to_string();
        assert!(s.validate().is_err());
        let mut s = valid();
        s.model = " ".to_string();
        assert!(s.validate().is_err());
        let mut s = valid();
        s.instructions = "".to_string();
        assert!(s.validate().is_err());
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
