//! Manifest schema for git-on-object-storage.
//!
//! The manifest is the immutable, content-addressed snapshot of a repo's
//! published state at a single point in time (§System Model). A push commits
//! by CAS-installing a new pointer to a new manifest digest; readers resolve
//! pointer → manifest → packs to hydrate (§Read).
//!
//! ## Canonical serialization
//!
//! `Manifest::canonical_bytes()` produces a deterministic byte sequence so
//! that `key == sha256(bytes)` (A1 detectability):
//!
//! - `refs: BTreeMap` — sorted ref names at serialization.
//! - `packs: Vec<String>` — sorted by `canonical_bytes()` before writing.
//! - Struct field order: `version`, `head`, `refs`, `packs`, `parent`
//!   (matches declaration; serde emits in this order).
//! - `serde_json::to_vec` — no whitespace.
//!
//! Round-trip + byte-stability are pinned in unit tests.
//!
//! ## Why HEAD is in the manifest
//!
//! HEAD is *published* ref state (§Implementation Correspondence), not a
//! read-time default. Deriving it ("default to main, fallback to first head")
//! would let a clone advertise a different default branch than the writer
//! intended — `Inv_RefEffectApplied` would not hold.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Current manifest schema version. Bump on incompatible change.
pub const MANIFEST_VERSION: u32 = 1;

/// A repository's published state.
///
/// Field order is significant for canonical JSON — do not reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version. Must equal [`MANIFEST_VERSION`] on read.
    pub version: u32,
    /// Symbolic HEAD ref, unprefixed (e.g. `"refs/heads/main"`). No `"ref: "`
    /// — that's a Git-protocol formatting concern, applied at hydrate time.
    pub head: String,
    /// All refs in the published state: refname → 40-char hex oid.
    pub refs: BTreeMap<String, String>,
    /// Store keys of every pack covering `refs`. Sorted ascending —
    /// `canonical_bytes` enforces this on serialize.
    pub packs: Vec<String>,
    /// Digest of the manifest this one supersedes, or `None` for the first
    /// push to a fresh repo.
    pub parent: Option<String>,
}

/// Errors from manifest (de)serialization.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// `serde_json` failed to encode or decode.
    #[error("manifest serde: {0}")]
    Serde(#[from] serde_json::Error),
    /// On-the-wire manifest carried a `version` we don't understand.
    #[error("unsupported manifest version {got} (expected {expected})")]
    UnsupportedVersion {
        /// The version we read.
        got: u32,
        /// The version we support.
        expected: u32,
    },
}

impl Manifest {
    /// Serialize to canonical bytes suitable for `put_manifest`.
    ///
    /// Sorts `packs` defensively (writer is responsible for keeping them
    /// sorted, but a misuse should not silently break content-addressing).
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        let mut owned = self.clone();
        owned.packs.sort();
        owned.packs.dedup();
        Ok(serde_json::to_vec(&owned)?)
    }

    /// Parse from bytes; reject unknown schema versions.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let m: Manifest = serde_json::from_slice(bytes)?;
        if m.version != MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                got: m.version,
                expected: MANIFEST_VERSION,
            });
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let mut refs = BTreeMap::new();
        refs.insert(
            "refs/heads/main".into(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        );
        refs.insert(
            "refs/heads/feature".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        Manifest {
            version: MANIFEST_VERSION,
            head: "refs/heads/main".into(),
            refs,
            packs: vec!["packs/cc".into(), "packs/dd".into()],
            parent: Some("ee".repeat(32)),
        }
    }

    #[test]
    fn canonical_bytes_round_trip() {
        let m = sample();
        let bytes = m.canonical_bytes().unwrap();
        let back = Manifest::from_bytes(&bytes).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn canonical_bytes_byte_stable_across_ref_insertion_order() {
        // Insert refs in opposite orders; canonical bytes must match because
        // BTreeMap iterates sorted.
        let mut a = sample();
        a.refs.clear();
        a.refs.insert("refs/heads/zzz".into(), "11".repeat(20));
        a.refs.insert("refs/heads/aaa".into(), "22".repeat(20));
        let mut b = sample();
        b.refs.clear();
        b.refs.insert("refs/heads/aaa".into(), "22".repeat(20));
        b.refs.insert("refs/heads/zzz".into(), "11".repeat(20));
        assert_eq!(a.canonical_bytes().unwrap(), b.canonical_bytes().unwrap());
    }

    #[test]
    fn canonical_bytes_sorts_and_dedups_packs() {
        let mut m = sample();
        m.packs = vec!["packs/dd".into(), "packs/cc".into(), "packs/dd".into()];
        let bytes = m.canonical_bytes().unwrap();
        let back = Manifest::from_bytes(&bytes).unwrap();
        assert_eq!(back.packs, vec!["packs/cc", "packs/dd"]);
    }

    #[test]
    fn rejects_unknown_version() {
        let mut m = sample();
        m.version = 999;
        let bytes = serde_json::to_vec(&m).unwrap();
        let err = Manifest::from_bytes(&bytes).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::UnsupportedVersion { got: 999, .. }
        ));
    }

    #[test]
    fn first_push_has_no_parent() {
        let mut m = sample();
        m.parent = None;
        let bytes = m.canonical_bytes().unwrap();
        let back = Manifest::from_bytes(&bytes).unwrap();
        assert!(back.parent.is_none());
    }

    /// Pin the exact byte shape so any unintended change to serialization
    /// (field order, whitespace, key ordering) triggers a failure rather than
    /// silently shifting the manifest digest.
    #[test]
    fn canonical_bytes_pinned() {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".into(), "a".repeat(40));
        let m = Manifest {
            version: 1,
            head: "refs/heads/main".into(),
            refs,
            packs: vec!["packs/p1".into()],
            parent: None,
        };
        let bytes = m.canonical_bytes().unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(
            s,
            r#"{"version":1,"head":"refs/heads/main","refs":{"refs/heads/main":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"packs":["packs/p1"],"parent":null}"#
        );
    }
}
