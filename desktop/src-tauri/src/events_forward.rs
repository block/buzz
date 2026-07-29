//! Signed-event builder for message forwarding (kind 40009).
//!
//! Split out of `events.rs` (which is at its file-size ceiling); follows the
//! same pattern: validate inputs, return a `nostr::EventBuilder`, and let
//! `relay::submit_event` sign + POST.

use nostr::{EventBuilder, Kind, Tag};
use uuid::Uuid;

use crate::events::{check_content, mention_tags, tag, MAX_CONTENT_BYTES};

/// Forward metadata tag families the desktop send path accepts. Everything
/// else is rejected so this path cannot forge channel/thread/mention metadata
/// (mirrors the `imeta_tags`/`emoji_tags` allowlist pattern in `events.rs`).
fn forward_tags(forward_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    let mut fwd_count = 0usize;
    let mut k_count = 0usize;
    let mut src_count = 0usize;
    for ft in forward_tags {
        match ft.first().map(String::as_str) {
            Some("fwd") => {
                fwd_count += 1;
                let embedded = ft.get(1).ok_or("fwd tag missing embedded event")?;
                if embedded.len() > MAX_CONTENT_BYTES {
                    return Err(format!(
                        "embedded event exceeds maximum size of {} bytes (got {})",
                        MAX_CONTENT_BYTES,
                        embedded.len()
                    ));
                }
            }
            Some("k") => k_count += 1,
            Some("fwd-src") => src_count += 1,
            Some("q") | Some("imeta") => {}
            other => {
                return Err(format!(
                    "forward tags must use fwd/k/fwd-src/q/imeta prefix (got {other:?})"
                ));
            }
        }
        let parts: Vec<&str> = ft.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid forward tag: {e}"))?);
    }
    if fwd_count != 1 || k_count != 1 || src_count != 1 {
        return Err(format!(
            "forward requires exactly one fwd, k, and fwd-src tag (got {fwd_count}/{k_count}/{src_count})"
        ));
    }
    Ok(())
}

/// Kind 40009 — forward a message snapshot into a channel or DM.
///
/// `note` is the forwarder's optional note (empty string when none);
/// the original message rides in the `fwd` tag untouched. `mentions` are
/// pubkeys the forwarder mentioned in the note — never the original author.
/// Deep validation of the embedded event (id/sig recompute, source-channel
/// access) is the relay's per-kind validator; this builder only guarantees a
/// well-formed tag shape.
pub fn build_forward(
    channel_id: Uuid,
    note: &str,
    fwd_tags: &[Vec<String>],
    mentions: &[&str],
) -> Result<EventBuilder, String> {
    check_content(note)?;
    let mut tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    forward_tags(fwd_tags, &mut tags)?;
    tags.extend(mention_tags(mentions)?);
    Ok(EventBuilder::new(
        Kind::Custom(buzz_core_pkg::kind::KIND_STREAM_MESSAGE_FORWARD as u16),
        note,
    )
    .tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn forward_builder_layout_and_tag_allowlist() {
        let channel_id = Uuid::new_v4();
        let dest = channel_id.to_string();
        let src = Uuid::new_v4().to_string();
        let fwd_tags = vec![
            vec!["fwd".to_string(), "{\"id\":\"ab\"}".to_string()],
            vec!["k".to_string(), "9".to_string()],
            vec!["fwd-src".to_string(), src.clone(), "channel".to_string()],
            vec![
                "q".to_string(),
                "ab".to_string(),
                String::new(),
                "cd".to_string(),
            ],
            vec!["imeta".to_string(), "url https://x/y.png".to_string()],
        ];
        let builder = build_forward(channel_id, "a note", &fwd_tags, &[]).unwrap();
        let keys = Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(
            event.kind,
            Kind::Custom(buzz_core_pkg::kind::KIND_STREAM_MESSAGE_FORWARD as u16)
        );
        assert_eq!(event.content, "a note");
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(tags[0], vec!["h", &dest]);
        assert_eq!(tags[1], vec!["fwd", "{\"id\":\"ab\"}"]);
        assert_eq!(tags[2], vec!["k", "9"]);
        assert_eq!(tags[3], vec!["fwd-src", &src, "channel"]);

        // Forged non-allowlisted tags are rejected.
        let smuggled = vec![
            vec!["fwd".to_string(), "{}".to_string()],
            vec!["k".to_string(), "9".to_string()],
            vec!["fwd-src".to_string(), src.clone(), "channel".to_string()],
            vec!["e".to_string(), "ab".to_string()],
        ];
        assert!(build_forward(channel_id, "", &smuggled, &[]).is_err());

        // Exactly one fwd tag.
        let doubled = vec![
            vec!["fwd".to_string(), "{}".to_string()],
            vec!["fwd".to_string(), "{}".to_string()],
            vec!["k".to_string(), "9".to_string()],
            vec!["fwd-src".to_string(), src, "channel".to_string()],
        ];
        assert!(build_forward(channel_id, "", &doubled, &[]).is_err());
    }
}
