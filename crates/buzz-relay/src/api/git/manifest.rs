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
    /// **Bare hex digest** of the manifest this one supersedes (64 chars,
    /// SHA-256), or `None` for the first push to a fresh repo. Contrast with
    /// `packs` which carries full store keys (`packs/<hex>`); `parent` is the
    /// digest alone, matching the pointer-body shape so `Inv_RefDerivedFromParent`
    /// reads literally as `parent = pointer.digest`. Writers must strip any
    /// `manifests/` prefix before assigning. Enforced by `validate()`.
    pub parent: Option<String>,
}

/// Errors from manifest (de)serialization or validation.
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
    /// A refname in `refs` (or `head`) violates `is_safe_refname` — must start
    /// with `refs/`, no traversal, no control chars. Symmetric write-side check
    /// for the reader-side validation in `api::git::hydrate`.
    #[error("manifest contains unsafe ref name {0:?}")]
    UnsafeRefName(String),
    /// An object id in `refs` is not a valid hex SHA-1 (40) or SHA-256 (64).
    #[error("manifest ref {refname:?} has malformed oid {oid:?}")]
    MalformedOid {
        /// The ref carrying the bad oid.
        refname: String,
        /// The oid that failed validation.
        oid: String,
    },
    /// Manifest `head` is empty — pre-CAS validation must reject this so we
    /// never commit an un-clone-able manifest (read side `is_safe_refname("")`
    /// returns false).
    #[error("manifest head is empty")]
    EmptyHead,
    /// `parent` is not a bare 64-char hex digest. Common mistake: storing the
    /// full store key (`manifests/<hex>`) instead of stripping the prefix.
    /// Breaks the "manifest.parent == pointer.digest" model invariant
    /// (`Inv_RefDerivedFromParent`).
    #[error("manifest parent is not a bare 64-char hex digest: {0:?}")]
    MalformedParent(String),
}

/// Conservative refname validation, used symmetrically on both the write side
/// (in `Manifest::validate`, before `put_manifest`) and the read side (in
/// `api::git::hydrate`, before writing the ref to disk).
///
/// Refuses traversal (`..`), null/newline/control chars, non-`refs/` prefixes,
/// and leading/trailing/double slashes. Allowed alphabet:
/// `[a-zA-Z0-9_./-]`.
///
/// Sharing one predicate is load-bearing: any divergence creates the
/// "valid CAS, un-clone-able output" hazard.
pub fn is_safe_refname(s: &str) -> bool {
    if !s.starts_with("refs/") {
        return false;
    }
    if s.contains("..") || s.contains("//") || s.starts_with('/') || s.ends_with('/') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
}

/// Hex-OID predicate. Accepts both SHA-1 (40 chars) and SHA-256 (64 chars) —
/// sprout pins SHA-1 today but the predicate is forward-looking. Used
/// symmetrically by write-side validation and read-side hydration.
pub fn is_hex_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Bare manifest-digest predicate (64-char hex SHA-256).
///
/// Distinct from `is_hex_oid` (which also accepts 40-char SHA-1 for ref OIDs):
/// manifest digests are *always* SHA-256, so this is the tighter predicate
/// for the `Manifest::parent` field.
fn is_manifest_digest(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The canonical pointer key for a repo: `repos/<owner>/<repo>/pointer`.
///
/// Single source of truth shared by `cas_publish` (write side) and `hydrate`
/// (read side). Strips a trailing `.git` if the caller passed it. The
/// `repos/<owner>/<repo>/` namespace leaves room for future sibling keys
/// (archive flag, gc state, etc.) co-located under each repo.
pub fn pointer_key(owner: &str, repo: &str) -> String {
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    format!("repos/{owner}/{repo}/pointer")
}

impl Manifest {
    /// Validate pre-commit invariants.
    ///
    /// **Writers must call this before `canonical_bytes` → `put_manifest`.**
    /// A manifest that hydrators reject (unsafe refname, malformed oid, empty
    /// HEAD) MUST NOT be written: it would CAS successfully and then 5xx every
    /// subsequent clone — "valid CAS, un-clone-able output". Pre-CAS rejection
    /// turns those into push-time 4xx, which is the right surface for the bug.
    ///
    /// Checks:
    /// - `head` is non-empty and passes `is_safe_refname`.
    /// - Every key in `refs` passes `is_safe_refname`.
    /// - Every value in `refs` is a hex OID per `is_hex_oid`.
    /// - `parent`, if `Some`, is a bare 64-char hex digest (not a store key).
    ///
    /// Read-side `hydrate` runs the same predicates as defense-in-depth.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.head.is_empty() {
            return Err(ManifestError::EmptyHead);
        }
        if !is_safe_refname(&self.head) {
            return Err(ManifestError::UnsafeRefName(self.head.clone()));
        }
        for (refname, oid) in &self.refs {
            if !is_safe_refname(refname) {
                return Err(ManifestError::UnsafeRefName(refname.clone()));
            }
            if !is_hex_oid(oid) {
                return Err(ManifestError::MalformedOid {
                    refname: refname.clone(),
                    oid: oid.clone(),
                });
            }
        }
        if let Some(p) = &self.parent {
            if !is_manifest_digest(p) {
                return Err(ManifestError::MalformedParent(p.clone()));
            }
        }
        Ok(())
    }

    /// Serialize to canonical bytes suitable for `put_manifest`.
    ///
    /// Sorts `packs` defensively (writer is responsible for keeping them
    /// sorted, but a misuse should not silently break content-addressing).
    ///
    /// Does NOT call `validate()` — callers must invoke it explicitly so a
    /// validation failure is visible at the write seam, not buried inside
    /// serialization.
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
    fn safe_refnames_predicate() {
        assert!(is_safe_refname("refs/heads/main"));
        assert!(is_safe_refname("refs/tags/v1.0.0"));
        assert!(is_safe_refname("refs/heads/feat/cas-publish"));
        assert!(!is_safe_refname("refs/heads/../escape"));
        assert!(!is_safe_refname("HEAD"));
        assert!(!is_safe_refname(""));
        assert!(!is_safe_refname("refs/heads/"));
        assert!(!is_safe_refname("/refs/heads/main"));
        assert!(!is_safe_refname("refs/heads/main\nrefs/heads/evil"));
        assert!(!is_safe_refname("refs/heads/main\0"));
    }

    #[test]
    fn hex_oid_predicate() {
        assert!(is_hex_oid(&"a".repeat(40)));
        assert!(is_hex_oid(&"a".repeat(64)));
        assert!(!is_hex_oid(&"a".repeat(39)));
        assert!(!is_hex_oid(&"g".repeat(40)));
        assert!(!is_hex_oid(""));
    }

    #[test]
    fn validate_happy_path() {
        sample().validate().expect("sample manifest must validate");
    }

    /// The empty manifest is the announce-time seed (`side_effects.rs::
    /// seed_manifest_pointer`). It must validate — otherwise repo announce
    /// would fail before the pointer can be seeded, and the read path would
    /// 404 every freshly-announced repo. This pins that contract.
    #[test]
    fn empty_manifest_validates() {
        let m = Manifest {
            version: MANIFEST_VERSION,
            head: "refs/heads/main".into(),
            refs: BTreeMap::new(),
            packs: Vec::new(),
            parent: None,
        };
        m.validate().expect("empty manifest is the announce-seed");
        // Canonical bytes must be deterministic + stable so all empty manifests
        // share one digest (idempotent put_manifest, one shared S3 object).
        let bytes = m.canonical_bytes().expect("serialize");
        let s = std::str::from_utf8(&bytes).expect("utf8");
        assert_eq!(
            s,
            r#"{"version":1,"head":"refs/heads/main","refs":{},"packs":[],"parent":null}"#
        );
    }

    #[test]
    fn validate_rejects_empty_head() {
        let mut m = sample();
        m.head = String::new();
        assert!(matches!(m.validate(), Err(ManifestError::EmptyHead)));
    }

    #[test]
    fn validate_rejects_unsafe_head() {
        let mut m = sample();
        m.head = "refs/heads/..".into();
        assert!(matches!(m.validate(), Err(ManifestError::UnsafeRefName(_))));
    }

    #[test]
    fn validate_rejects_non_refs_head() {
        let mut m = sample();
        m.head = "HEAD".into();
        assert!(matches!(m.validate(), Err(ManifestError::UnsafeRefName(_))));
    }

    #[test]
    fn validate_rejects_unsafe_ref_name() {
        let mut m = sample();
        m.refs.insert("refs/heads/bad\nname".into(), "a".repeat(40));
        assert!(matches!(m.validate(), Err(ManifestError::UnsafeRefName(_))));
    }

    #[test]
    fn validate_rejects_malformed_oid() {
        let mut m = sample();
        m.refs
            .insert("refs/heads/ok".into(), "not-a-hex-oid".into());
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MalformedOid { .. })
        ));
    }

    #[test]
    fn validate_rejects_parent_with_store_prefix() {
        // The common bug Perci named: storing the full key in `parent` instead
        // of the bare digest. `Inv_RefDerivedFromParent` reads `parent =
        // pointer.digest`; carrying the prefix breaks the model literal.
        let mut m = sample();
        m.parent = Some(format!("manifests/{}", "a".repeat(64)));
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MalformedParent(_))
        ));
    }

    #[test]
    fn validate_rejects_short_parent() {
        let mut m = sample();
        m.parent = Some("abc".into());
        assert!(matches!(
            m.validate(),
            Err(ManifestError::MalformedParent(_))
        ));
    }

    #[test]
    fn validate_accepts_no_parent() {
        let mut m = sample();
        m.parent = None;
        m.validate().expect("no parent is fine (first push)");
    }

    #[test]
    fn pointer_key_strips_dot_git() {
        assert_eq!(pointer_key("alice", "myrepo"), "repos/alice/myrepo/pointer");
        assert_eq!(
            pointer_key("alice", "myrepo.git"),
            "repos/alice/myrepo/pointer"
        );
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
