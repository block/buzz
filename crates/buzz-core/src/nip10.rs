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
    let mut markers = ThreadMarkers::default();
    for tag in tags.iter() {
        let parts = tag.as_slice();
        if parts.len() >= 4 && parts[0] == "e" && is_event_id_hex(&parts[1]) {
            match parts[3].as_str() {
                "root" => markers.root = Some(parts[1].to_string()),
                "reply" => markers.reply = Some(parts[1].to_string()),
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
}
