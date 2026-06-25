//! Manifest → kind:30618 NIP-34 ref-state event.
//!
//! Pure function. No subprocess, no disk read, no S3.
//! Source of truth is the in-memory `Manifest` (loaded from S3 by the caller).
//!
//! NIP-34 reference (kind:30618 "Repository state announcements"):
//!   tags = [
//!     ["d", "<repo-id>"],                     // matches kind:30617 d-tag
//!     ["refs/heads/<branch>", "<commit-id>"], // zero or more
//!     ["refs/tags/<tag>",     "<commit-id>"], // zero or more
//!     ["HEAD", "ref: refs/heads/<branch>"],   // symbolic HEAD
//!   ]
//!
//! Buzz extension: a `p` tag carrying the pusher's pubkey (or the repo
//! owner on creation events) so subscribers can filter by author of state
//! transition. Not part of NIP-34 but consistent with the rest of buzz's
//! event-publishing conventions.

use std::collections::BTreeMap;

use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, TagKind};

/// Subset of `Manifest` needed to emit a kind:30618 event.
///
/// Deliberately a borrowed slice of fields (not the whole `Manifest`) so this
/// module doesn't take a hard dep on `s3_repo::Manifest`'s exact shape. Caller
/// constructs this from the loaded manifest.
pub struct RefStateInputs<'a> {
    /// The kind:30617 d-tag identifier. == repo_id, NOT `<repo>.git`.
    pub repo_id: &'a str,
    /// Symbolic HEAD ref, e.g. "refs/heads/main". NO "ref: " prefix here —
    /// the prefix is added when emitting the tag. Storing it unprefixed in
    /// the manifest keeps Git-protocol formatting out of the storage schema.
    pub head: &'a str,
    /// `ref_name -> oid_hex`. Only `refs/heads/*` and `refs/tags/*` will be
    /// emitted; other ref namespaces are filtered.
    pub refs: &'a BTreeMap<String, String>,
    /// Pubkey to include in the `p` tag (buzz extension). On push, this is
    /// the pusher's pubkey from the receive-pack hook. On repo-creation, this
    /// is the kind:30617 author (repo owner). Hex-encoded (64 chars).
    pub actor_pubkey_hex: &'a str,
}

/// Errors from building a kind:30618 ref-state event.
#[derive(thiserror::Error, Debug)]
pub enum BuildError {
    /// `actor_pubkey_hex` did not parse as a valid 64-char hex pubkey.
    #[error("invalid actor_pubkey_hex: {0}")]
    InvalidActor(String),
    /// `nostr` event signing returned an error.
    #[error("nostr event signing failed: {0}")]
    Sign(String),
}

/// Build & sign a kind:30618 event from the manifest's ref state.
///
/// Signed with `relay_keys` — the relay is the authoritative source of ref
/// state for repos it hosts.
///
/// Invariants enforced inside this function:
/// - HEAD tag is wrapped as `"ref: <head>"` per NIP-34, even though the
///   manifest stores HEAD bare.
/// - Only `refs/heads/*` and `refs/tags/*` are emitted (NIP-34 §"Repository
///   state announcements" semantics).
/// - OIDs validated as 40-hex (SHA-1) or 64-hex (SHA-256). Invalid OIDs are
///   skipped, not failed — same conservative behavior as the legacy code.
/// - Ref names validated (no `//`, no leading `/`, alphanumeric + `/_.-`).
/// - Output tag ordering is deterministic for testability: `d`, refs (sorted
///   by `BTreeMap` iteration), HEAD, p.
pub fn build_ref_state_event(
    inputs: &RefStateInputs<'_>,
    relay_keys: &Keys,
) -> Result<Event, BuildError> {
    // Validate actor pubkey first so we error before any tag construction.
    let actor = PublicKey::from_hex(inputs.actor_pubkey_hex)
        .map_err(|e| BuildError::InvalidActor(e.to_string()))?;

    let mut tags: Vec<Tag> = Vec::with_capacity(inputs.refs.len() + 3);

    // d-tag: kind:30617 identifier.
    tags.push(Tag::custom(TagKind::custom("d"), [inputs.repo_id]));

    // ref tags: refs/heads/* and refs/tags/* only.
    for (ref_name, oid) in inputs.refs {
        if !is_emittable_ref(ref_name) {
            continue;
        }
        if !is_valid_oid(oid) {
            continue;
        }
        tags.push(Tag::custom(
            TagKind::custom(ref_name.clone()),
            [oid.clone()],
        ));
    }

    // HEAD tag — note the "ref: " prefix required by NIP-34.
    if !inputs.head.is_empty() && is_emittable_ref(inputs.head) {
        tags.push(Tag::custom(
            TagKind::custom("HEAD"),
            [format!("ref: {}", inputs.head)],
        ));
    }

    // p-tag: buzz extension (pusher or owner pubkey).
    tags.push(Tag::public_key(actor));

    let event = EventBuilder::new(Kind::Custom(30618), "")
        .tags(tags)
        .sign_with_keys(relay_keys)
        .map_err(|e| BuildError::Sign(e.to_string()))?;

    Ok(event)
}

/// NIP-34 kind:30618 only emits refs under heads/ and tags/.
fn is_emittable_ref(name: &str) -> bool {
    if !(name.starts_with("refs/heads/") || name.starts_with("refs/tags/")) {
        return false;
    }
    if name.starts_with('/') || name.contains("//") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.-".contains(c))
}

/// Accept SHA-1 (40 hex) and SHA-256 (64 hex) OIDs.
fn is_valid_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::SecretKey;

    fn relay_keys() -> Keys {
        // Deterministic test key.
        Keys::new(
            SecretKey::from_hex("0000000000000000000000000000000000000000000000000000000000000001")
                .unwrap(),
        )
    }

    fn owner_hex() -> String {
        // 64-hex pubkey for the test "actor".
        "f4a42a97e594b77bdbd8ee35191c8b28a94a4cb871d96f32921558275421fb68".to_string()
    }

    fn refs_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // Helper: get tag values by tag-name prefix.
    fn tags_with_kind(ev: &Event, kind: &str) -> Vec<Vec<String>> {
        ev.tags
            .iter()
            .filter_map(|t| {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some(kind) {
                    Some(s.to_vec())
                } else {
                    None
                }
            })
            .collect()
    }

    fn first_tag(ev: &Event, kind: &str) -> Option<Vec<String>> {
        tags_with_kind(ev, kind).into_iter().next()
    }

    #[test]
    fn empty_repo_emits_d_head_p_only() {
        let owner = owner_hex();
        let refs = refs_with(&[]);
        let inputs = RefStateInputs {
            repo_id: "myrepo",
            head: "refs/heads/main",
            refs: &refs,
            actor_pubkey_hex: &owner,
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();

        assert_eq!(ev.kind, Kind::Custom(30618));
        assert_eq!(ev.content, "");

        // d-tag
        assert_eq!(first_tag(&ev, "d").unwrap()[1], "myrepo");
        // HEAD tag — note "ref: " prefix
        assert_eq!(
            first_tag(&ev, "HEAD").unwrap()[1],
            "ref: refs/heads/main",
            "HEAD tag MUST be wrapped 'ref: <ref>' per NIP-34",
        );
        // p-tag = owner
        assert_eq!(first_tag(&ev, "p").unwrap()[1], owner);
        // no ref tags
        assert!(tags_with_kind(&ev, "refs/heads/main").is_empty());
    }

    #[test]
    fn head_tag_always_wraps_with_ref_prefix() {
        // Even if a caller is sloppy and passes head with the prefix already...
        // we shouldn't double-wrap. (Bare ref expected; doc the precondition.)
        // This test pins that bare ref → wrapped output.
        let refs = refs_with(&[]);
        let inputs = RefStateInputs {
            repo_id: "x",
            head: "refs/heads/dev",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();
        assert_eq!(first_tag(&ev, "HEAD").unwrap()[1], "ref: refs/heads/dev");
    }

    #[test]
    fn emits_branches_and_tags() {
        let refs = refs_with(&[
            (
                "refs/heads/main",
                "1111111111111111111111111111111111111111",
            ),
            ("refs/heads/dev", "2222222222222222222222222222222222222222"),
            ("refs/tags/v1.0", "3333333333333333333333333333333333333333"),
        ]);
        let inputs = RefStateInputs {
            repo_id: "r",
            head: "refs/heads/main",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();

        assert_eq!(
            first_tag(&ev, "refs/heads/main").unwrap()[1],
            "1111111111111111111111111111111111111111",
        );
        assert_eq!(
            first_tag(&ev, "refs/heads/dev").unwrap()[1],
            "2222222222222222222222222222222222222222",
        );
        assert_eq!(
            first_tag(&ev, "refs/tags/v1.0").unwrap()[1],
            "3333333333333333333333333333333333333333",
        );
    }

    #[test]
    fn skips_non_heads_or_tags_refs() {
        let refs = refs_with(&[
            (
                "refs/heads/main",
                "1111111111111111111111111111111111111111",
            ),
            (
                "refs/notes/commits",
                "2222222222222222222222222222222222222222",
            ),
            (
                "refs/remotes/origin/x",
                "3333333333333333333333333333333333333333",
            ),
            ("refs/stash", "4444444444444444444444444444444444444444"),
            (
                "refs/pull/1/head",
                "5555555555555555555555555555555555555555",
            ),
        ]);
        let inputs = RefStateInputs {
            repo_id: "r",
            head: "refs/heads/main",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();

        assert!(first_tag(&ev, "refs/heads/main").is_some());
        assert!(first_tag(&ev, "refs/notes/commits").is_none());
        assert!(first_tag(&ev, "refs/remotes/origin/x").is_none());
        assert!(first_tag(&ev, "refs/stash").is_none());
        assert!(first_tag(&ev, "refs/pull/1/head").is_none());
    }

    #[test]
    fn accepts_sha1_and_sha256_oids() {
        let sha1 = "1111111111111111111111111111111111111111"; // 40 hex
        let sha256 = "1111111111111111111111111111111111111111111111111111111111111111"; // 64
        let refs = refs_with(&[
            ("refs/heads/sha1-branch", sha1),
            ("refs/heads/sha256-branch", sha256),
        ]);
        let inputs = RefStateInputs {
            repo_id: "r",
            head: "refs/heads/sha1-branch",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();
        assert!(first_tag(&ev, "refs/heads/sha1-branch").is_some());
        assert!(first_tag(&ev, "refs/heads/sha256-branch").is_some());
    }

    #[test]
    fn rejects_invalid_oids() {
        let refs = refs_with(&[
            ("refs/heads/short", "1234"), // too short
            (
                "refs/heads/non-hex",
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            ), // non-hex
            (
                "refs/heads/midlen",
                "11111111111111111111111111111111111111111111111111",
            ), // 50, between
            ("refs/heads/ok", "1111111111111111111111111111111111111111"),
        ]);
        let inputs = RefStateInputs {
            repo_id: "r",
            head: "refs/heads/ok",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();
        assert!(first_tag(&ev, "refs/heads/short").is_none());
        assert!(first_tag(&ev, "refs/heads/non-hex").is_none());
        assert!(first_tag(&ev, "refs/heads/midlen").is_none());
        assert!(first_tag(&ev, "refs/heads/ok").is_some());
    }

    #[test]
    fn rejects_malformed_ref_names() {
        let refs = refs_with(&[
            (
                "refs/heads//double",
                "1111111111111111111111111111111111111111",
            ), // //
            (
                "/refs/heads/leading",
                "1111111111111111111111111111111111111111",
            ), // leading /
            (
                "refs/heads/space ref",
                "1111111111111111111111111111111111111111",
            ), // space
            (
                "refs/heads/legit",
                "1111111111111111111111111111111111111111",
            ),
        ]);
        let inputs = RefStateInputs {
            repo_id: "r",
            head: "refs/heads/legit",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();
        assert!(first_tag(&ev, "refs/heads/legit").is_some());
        // Malformed refs should not appear.
        assert_eq!(tags_with_kind(&ev, "refs/heads//double").len(), 0);
    }

    #[test]
    fn rejects_invalid_actor_pubkey() {
        let refs = refs_with(&[]);
        let inputs = RefStateInputs {
            repo_id: "r",
            head: "refs/heads/main",
            refs: &refs,
            actor_pubkey_hex: "not-a-pubkey",
        };
        let err = build_ref_state_event(&inputs, &relay_keys()).unwrap_err();
        assert!(matches!(err, BuildError::InvalidActor(_)));
    }

    #[test]
    fn d_tag_is_repo_id_not_repo_dot_git() {
        let refs = refs_with(&[]);
        let inputs = RefStateInputs {
            repo_id: "myrepo", // caller MUST strip .git before passing
            head: "refs/heads/main",
            refs: &refs,
            actor_pubkey_hex: &owner_hex(),
        };
        let ev = build_ref_state_event(&inputs, &relay_keys()).unwrap();
        assert_eq!(first_tag(&ev, "d").unwrap()[1], "myrepo");
        // Pin: caller responsibility — if they pass "myrepo.git", that's what
        // ends up in the d-tag and won't match the kind:30617 announcement.
    }
}
