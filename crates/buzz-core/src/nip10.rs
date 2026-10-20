//! Shared NIP-10 thread-marker parsing.
//!
//! One parser for the `root`/`reply` markers on an event's `e` tags, so every
//! consumer reads ancestry the same way. The relay ingest resolver
//! (`resolve_nip10_thread_meta`) and the workflow `trigger_is_reply` predicate
//! both call this — a second hand-rolled copy is exactly how the two drifted on
//! marker semantics and on id-validity.
//!
//! Validity mirrors ingest: a marker counts only when its event id is exactly
//! 64 ASCII-hex characters. A malformed id (e.g. `["e","bad","","reply"]`) is
//! ignored, never treated as a thread link.

/// The `root` and `reply` event ids parsed from an event's NIP-10 `e` tags.
///
/// Each is `Some(id_hex)` only when a marker of that kind carried a valid
/// 64-hex event id. The last valid occurrence of each marker wins, matching
/// the relay resolver's single-pass overwrite.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThreadMarkers {
    /// Event id from a valid `["e", <64-hex>, <relay>, "root"]` tag.
    pub root: Option<String>,
    /// Event id from a valid `["e", <64-hex>, <relay>, "reply"]` tag.
    pub reply: Option<String>,
}

impl ThreadMarkers {
    /// Collapse the `root`/`reply` markers into a reply's `(root_id, parent_id)`.
    ///
    /// This is the single definition of the NIP-10 resolution rule shared by
    /// consumers that classify a reply's own root/parent or recover a parent's
    /// ancestry (relay ingest, ACP anchoring, and the CLI).
    ///
    /// - `root` + `reply` → `(root, reply)` — a nested reply names both.
    /// - `reply` only → `(reply, reply)` — a direct reply to the root; the
    ///   reply target is itself the thread root.
    /// - `root` only or neither → `None` — no `reply` marker means the event is
    ///   top-level, matching ingest (a lone `root` tag never anchors a reply).
    pub fn resolve(&self) -> Option<(String, String)> {
        match (&self.root, &self.reply) {
            (Some(root), Some(reply)) => Some((root.clone(), reply.clone())),
            (None, Some(reply)) => Some((reply.clone(), reply.clone())),
            (Some(_), None) | (None, None) => None,
        }
    }
}

/// Return true when `id` is exactly 64 ASCII-hex characters — the shape a
/// Nostr event id must have to be a real thread link.
fn is_event_id_hex(id: &str) -> bool {
    id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse the NIP-10 `root`/`reply` markers from an event's tags.
///
/// Only `e` tags with a marker (`parts.len() >= 4`) and a valid 64-hex event id
/// are considered; everything else is ignored.
pub fn parse_thread_markers(tags: &nostr::Tags) -> ThreadMarkers {
    parse_thread_markers_from_parts(tags.iter().map(nostr::Tag::as_slice))
}

/// Same parser as [`parse_thread_markers`], for consumers that hold raw tag
/// arrays (e.g. decoded JSON `tags`) rather than a [`nostr::Tags`].
///
/// Each tag is a slice of string-like parts (`["e", <id>, <relay>, <marker>]`).
pub fn parse_thread_markers_from_parts<'a, S, I>(tags: I) -> ThreadMarkers
where
    S: AsRef<str> + 'a,
    I: IntoIterator<Item = &'a [S]>,
{
    let mut markers = ThreadMarkers::default();
    for parts in tags {
        if parts.len() >= 4 && parts[0].as_ref() == "e" && is_event_id_hex(parts[1].as_ref()) {
            match parts[3].as_ref() {
                "root" => markers.root = Some(parts[1].as_ref().to_string()),
                "reply" => markers.reply = Some(parts[1].as_ref().to_string()),
                _ => {}
            }
        }
    }
    markers
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn markers_for(tags: Vec<Tag>) -> ThreadMarkers {
        let event = EventBuilder::new(Kind::Custom(9), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        parse_thread_markers(&event.tags)
    }

    fn id() -> String {
        "a".repeat(64)
    }

    #[test]
    fn no_e_tags_yields_no_markers() {
        assert_eq!(markers_for(vec![]), ThreadMarkers::default());
    }

    #[test]
    fn root_and_reply_both_parsed() {
        let m = markers_for(vec![
            Tag::parse(["e", &id(), "", "root"]).unwrap(),
            Tag::parse(["e", &"b".repeat(64), "", "reply"]).unwrap(),
        ]);
        assert_eq!(m.root.as_deref(), Some(id().as_str()));
        assert_eq!(m.reply.as_deref(), Some("b".repeat(64).as_str()));
    }

    #[test]
    fn reply_only_marker_parsed() {
        let m = markers_for(vec![Tag::parse(["e", &id(), "", "reply"]).unwrap()]);
        assert_eq!(m.reply.as_deref(), Some(id().as_str()));
        assert!(m.root.is_none());
    }

    #[test]
    fn bare_e_tag_without_marker_is_ignored() {
        let m = markers_for(vec![Tag::parse(["e", &id()]).unwrap()]);
        assert_eq!(m, ThreadMarkers::default());
    }

    #[test]
    fn malformed_id_is_ignored_for_both_markers() {
        // Ingest gates the marker on a valid 64-hex id; a malformed id is not a
        // thread link, so neither marker is set.
        let m = markers_for(vec![
            Tag::parse(["e", "bad", "", "reply"]).unwrap(),
            Tag::parse(["e", "also-bad", "", "root"]).unwrap(),
        ]);
        assert_eq!(m, ThreadMarkers::default());
    }

    #[test]
    fn valid_root_with_malformed_reply_is_top_level() {
        // A valid root but a malformed reply id: reply is ignored, so this is
        // top-level to ingest (root-only) and must be so here too.
        let m = markers_for(vec![
            Tag::parse(["e", &id(), "", "root"]).unwrap(),
            Tag::parse(["e", "bad", "", "reply"]).unwrap(),
        ]);
        assert_eq!(m.root.as_deref(), Some(id().as_str()));
        assert!(m.reply.is_none());
    }

    #[test]
    fn resolve_root_and_reply_keeps_both() {
        let m = ThreadMarkers {
            root: Some("r".repeat(64)),
            reply: Some("p".repeat(64)),
        };
        assert_eq!(m.resolve(), Some(("r".repeat(64), "p".repeat(64))));
    }

    #[test]
    fn resolve_reply_only_is_direct_reply_to_root() {
        let m = ThreadMarkers {
            root: None,
            reply: Some(id()),
        };
        assert_eq!(m.resolve(), Some((id(), id())));
    }

    #[test]
    fn resolve_root_only_is_top_level() {
        let m = ThreadMarkers {
            root: Some(id()),
            reply: None,
        };
        assert_eq!(m.resolve(), None);
    }

    #[test]
    fn resolve_no_markers_is_top_level() {
        assert_eq!(ThreadMarkers::default().resolve(), None);
    }

    #[test]
    fn parse_from_parts_matches_tags_path() {
        // The slice-based entry point must gate id validity and select markers
        // identically to the `nostr::Tags` path.
        let tags: Vec<Vec<String>> = vec![
            vec!["e".into(), id(), "".into(), "root".into()],
            vec!["e".into(), "b".repeat(64), "".into(), "reply".into()],
            vec!["e".into(), "bad".into(), "".into(), "reply".into()],
            vec!["p".into(), "abc".into()],
        ];
        let m = parse_thread_markers_from_parts(tags.iter().map(Vec::as_slice));
        assert_eq!(m.root.as_deref(), Some(id().as_str()));
        assert_eq!(m.reply.as_deref(), Some("b".repeat(64).as_str()));
    }
}
