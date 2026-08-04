//! Repo → channel binding resolution, shared by the read gate and push policy.
//!
//! The `buzz-channel` tag on a kind:30617 announcement IS the git ACL: the
//! read gate (SEC-005, `transport::authorize_git_read`) and the push policy
//! endpoint (`policy::hook_callback`) both authorize against membership in
//! the bound channel. Before this module they each parsed the tag with their
//! own code that agreed only by coincidence; the resolver makes the
//! agreement structural.
//!
//! # Exact-one, fail-closed semantics
//!
//! Exactly one `buzz-channel` tag must exist and carry a valid UUID. A missing
//! tag resolves to [`RepoBinding::NotBound`]; a malformed or duplicate binding
//! resolves to [`RepoBinding::Broken`]. Duplicate tags are ambiguous even when
//! their values match, so every Git authorization door fails closed on them.
//!
//! # What this deliberately does NOT do
//!
//! No DB access. A well-formed UUID that names a nonexistent or deleted
//! channel still resolves to [`RepoBinding::Bound`]; each gate's own
//! membership lookup then denies (`get_member_role` joins
//! `channels … deleted_at IS NULL`, so a dead channel is indistinguishable
//! from a non-member — the info-leak-safe posture). Likewise each gate keeps
//! its own archived-channel policy: push denies on archived channels, read
//! does not, and this resolver must not unify that asymmetry as a side
//! effect.

use uuid::Uuid;

/// How a kind:30617 announcement binds (or fails to bind) a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoBinding {
    /// No `buzz-channel` tag at all. The announcement author (the only
    /// identity that can rebind — 30617 is keyed by `(author, d)`) may be
    /// offered remediation; everyone else gets the generic denial.
    NotBound,
    /// The sole `buzz-channel` tag carries a valid UUID.
    Bound(Uuid),
    /// The binding is malformed or more than one `buzz-channel` tag exists.
    /// Fail closed with the generic denial — never remediation, which
    /// would leak that the repo exists.
    Broken,
}

/// Resolve the channel binding of a kind:30617 announcement from its tags.
pub fn resolve_repo_binding(event: &nostr::Event) -> RepoBinding {
    let mut bindings = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(String::as_str) == Some("buzz-channel"));
    let Some(binding) = bindings.next() else {
        return RepoBinding::NotBound;
    };
    if bindings.next().is_some() {
        return RepoBinding::Broken;
    }
    match binding.as_slice().get(1).map(|v| Uuid::parse_str(v)) {
        Some(Ok(id)) => RepoBinding::Bound(id),
        _ => RepoBinding::Broken,
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::{resolve_repo_binding, RepoBinding};

    fn announcement(tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::Custom(30617), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign 30617")
    }

    #[test]
    fn extracts_valid_uuid() {
        let ch = uuid::Uuid::new_v4();
        let event = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", &ch.to_string()]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&event), RepoBinding::Bound(ch));
    }

    #[test]
    fn absent_tag_is_not_bound() {
        let event = announcement(vec![Tag::parse(["d", "repo"]).unwrap()]);
        assert_eq!(resolve_repo_binding(&event), RepoBinding::NotBound);
    }

    #[test]
    fn malformed_and_empty_values_are_broken_not_absent() {
        let malformed = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", "not-a-uuid"]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&malformed), RepoBinding::Broken);

        let empty = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel"]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&empty), RepoBinding::Broken);
    }

    #[test]
    fn fails_closed_on_ambiguous_duplicate_bindings() {
        let ch = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();

        // Malformed first + valid second: the ambiguity denies; the valid
        // duplicate must NOT win, or the duplicate picks the channel.
        let malformed_first = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", "not-a-uuid"]).unwrap(),
            Tag::parse(["buzz-channel", &ch.to_string()]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&malformed_first), RepoBinding::Broken);

        // Valid first + different second: duplicate bindings are ambiguous.
        let valid_first = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", &ch.to_string()]).unwrap(),
            Tag::parse(["buzz-channel", &other.to_string()]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&valid_first), RepoBinding::Broken);

        // Identical duplicates are still ambiguous; readers must not depend on
        // tag ordering or silently accept a non-canonical announcement.
        let identical = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", &ch.to_string()]).unwrap(),
            Tag::parse(["buzz-channel", &ch.to_string()]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&identical), RepoBinding::Broken);
    }
}
