//! Reading NIP-FP evidence out of raw event tags.
//!
//! Everything here is a pure function of one event's claimed fields. Whether
//! the event is validly signed is decided elsewhere.

use std::fmt;
use std::str::FromStr;

use nostr::{Event, EventId, PublicKey};

use super::{OwnerLink, LABEL_NAMESPACE};
use crate::nip_oa::verify_auth_tag_for_event;

/// Status kinds (NIP-34): open, applied/merged/resolved, closed, draft.
pub(super) const STATUS_KINDS: [u16; 4] = [1630, 1631, 1632, 1633];
/// Kinds of outcomes discovered by work event id.
pub(super) const OUTCOME_KINDS: [u16; 6] = [5, 1630, 1631, 1632, 1633, 1985];
/// Kinds of outcomes discovered by subject key.
pub(super) const KEYED_OUTCOME_KINDS: [u16; 5] = [1630, 1631, 1632, 1633, 1985];
/// Replies, mentions, and reactions. Counted as attention, never outcomes.
pub(super) const ATTENTION_KINDS: [u16; 4] = [1, 7, 9, 1111];
/// Patch and pull-request kinds, whose `1631` means merged.
pub(super) const CHANGE_KINDS: [u16; 2] = [1617, 1618];

/// A repository coordinate, `30617:<owner>:<d>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Coord {
    pub(super) owner: PublicKey,
    pub(super) d: String,
}

impl fmt::Display for Coord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "30617:{}:{}", self.owner.to_hex(), self.d)
    }
}

impl FromStr for Coord {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix("30617:").ok_or(())?;
        let (owner, d) = rest.split_once(':').ok_or(())?;
        if d.is_empty() {
            return Err(());
        }
        let owner = PublicKey::from_hex(owner).map_err(|_| ())?;
        Ok(Self {
            owner,
            d: d.to_owned(),
        })
    }
}

/// Every tag of `event` named `name`, as string slices.
pub(super) fn tags<'a>(event: &'a Event, name: &'a str) -> impl Iterator<Item = &'a [String]> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(move |parts| parts.first().is_some_and(|n| n == name))
}

/// The first value of the first tag named `name`.
pub(super) fn tag_value<'a>(event: &'a Event, name: &'a str) -> Option<&'a str> {
    tags(event, name)
        .next()
        .and_then(|parts| parts.get(1))
        .map(String::as_str)
}

fn e_tag(parts: &[String]) -> Option<EventId> {
    parts.get(1).and_then(|id| EventId::from_hex(id).ok())
}

fn marked_e(event: &Event, marker: &str) -> Option<EventId> {
    tags(event, "e")
        .find(|parts| parts.get(3).is_some_and(|m| m == marker))
        .and_then(e_tag)
}

/// The event a work event answers (NIP-10 `reply`, else `root`). Its author
/// is the requester.
pub(super) fn parent(event: &Event) -> Option<EventId> {
    marked_e(event, "reply").or_else(|| marked_e(event, "root"))
}

/// The work event an outcome is about: the `root`-marked `e` tag, else the
/// first `e` tag.
pub(super) fn target(event: &Event) -> Option<EventId> {
    marked_e(event, "root").or_else(|| tags(event, "e").find_map(e_tag))
}

/// Every event id named by an `e` tag.
pub(super) fn e_ids(event: &Event) -> Vec<EventId> {
    tags(event, "e").filter_map(e_tag).collect()
}

/// The repository a work event belongs to, from its first `30617` `a` tag.
pub(super) fn repo(event: &Event) -> Option<Coord> {
    tags(event, "a").find_map(|parts| parts.get(1).and_then(|a| a.parse().ok()))
}

/// The `buzz.wot` value of a trust label, when it is one this NIP defines.
pub(super) fn label(event: &Event) -> Option<&'static str> {
    if event.kind.as_u16() != 1985
        || !tags(event, "L").any(|p| p.get(1).is_some_and(|ns| ns == LABEL_NAMESPACE))
    {
        return None;
    }
    tags(event, "l")
        .filter(|p| p.get(2).is_some_and(|ns| ns == LABEL_NAMESPACE))
        .find_map(|p| match p.get(1).map(String::as_str) {
            Some("accepted") => Some("accepted"),
            Some("reused") => Some("reused"),
            Some("corrected") => Some("corrected"),
            Some("rejected") => Some("rejected"),
            Some("disputed") => Some("disputed"),
            _ => None,
        })
}

/// Maintainer keys listed by a `30617`.
pub(super) fn maintainers(event: &Event) -> Vec<PublicKey> {
    tags(event, "maintainers")
        .flat_map(|parts| parts.iter().skip(1))
        .filter_map(|pk| PublicKey::from_hex(pk).ok())
        .collect()
}

/// The branch a `30618` names as `HEAD`, and that branch's commit.
pub(super) fn head_commit(event: &Event) -> Option<&str> {
    let branch = tag_value(event, "HEAD")?.strip_prefix("ref: ")?;
    tag_value(event, branch)
}

/// The NIP-OA owner link of a validly signed event. More than one `auth` tag
/// is no link.
pub(super) fn owner_link(event: &Event) -> OwnerLink {
    let mut auth = tags(event, "auth");
    let (Some(first), None) = (auth.next(), auth.next()) else {
        return if tags(event, "auth").next().is_some() {
            OwnerLink::Invalid
        } else {
            OwnerLink::None
        };
    };
    let Ok(json) = serde_json::to_string(first) else {
        return OwnerLink::Invalid;
    };
    match verify_auth_tag_for_event(&json, event) {
        Ok(owner) => OwnerLink::Verified(owner.to_hex()),
        Err(_) => OwnerLink::Invalid,
    }
}
