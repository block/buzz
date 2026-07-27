//! Huddle transcript segments — the typed view of `KIND_HUDDLE_TRANSCRIPT`
//! events (issue #2513).
//!
//! A huddle's spoken content is the one modality that never became part of the
//! event log: agents watching a channel could see that a huddle happened, but
//! not what was said. A transcript segment closes that gap. Each segment is a
//! signed channel event whose **content is the plain transcript text** — so it
//! is FTS-indexed and readable by agents exactly like a message — with the
//! structure carried in tags:
//!
//! ```text
//! kind:    48104  (KIND_HUDDLE_TRANSCRIPT)
//! content: "the quick brown fox"     ← plain text, FTS-indexed
//! tags:    ["h", <channel-uuid>]     ← channel scope (NIP-29 group)
//!          ["p", <speaker-pubkey>]   ← attribution (the relay knows who spoke)
//!          ["e", <huddle-started-id>]← links the segment to its huddle session
//!          ["start", <ms>]           ← offset from huddle start, milliseconds
//!          ["end",   <ms>]           ← offset from huddle start, milliseconds
//!          ["lang", <bcp47>]         ← optional detected/declared language
//! ```
//!
//! Per-speaker attribution is inherent server-side: the audio room keys every
//! participant by `(pubkey, peer_index)`, so a segment maps to a pubkey with no
//! diarization guesswork. The signed-event builder lives in `buzz-sdk`
//! (`build_huddle_transcript`); this module is the reader side used by agents
//! and any consumer that wants a typed segment back out of the log.

use uuid::Uuid;

/// Tag key: millisecond start offset from the huddle start.
pub const TAG_START_MS: &str = "start";
/// Tag key: millisecond end offset from the huddle start.
pub const TAG_END_MS: &str = "end";
/// Tag key: optional BCP-47 language of the segment (e.g. `"en"`, `"uk"`).
pub const TAG_LANG: &str = "lang";
/// Tag key: channel scope (NIP-29 group id).
pub const TAG_CHANNEL: &str = "h";
/// Tag key: speaker pubkey (attribution).
pub const TAG_SPEAKER: &str = "p";
/// Tag key: the `KIND_HUDDLE_STARTED` event id this segment belongs to.
pub const TAG_HUDDLE_EVENT: &str = "e";

/// One attributed transcript segment: who spoke, what they said, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuddleTranscriptSegment {
    /// Channel (NIP-29 group) the huddle belongs to — from the `h` tag.
    pub channel_id: Uuid,
    /// Hex pubkey of the speaker — from the `p` tag.
    pub speaker_pubkey: String,
    /// The `KIND_HUDDLE_STARTED` event id, when present — from the `e` tag.
    pub huddle_event_id: Option<String>,
    /// Plain transcript text — the event content.
    pub text: String,
    /// Segment start, milliseconds from huddle start — from the `start` tag.
    pub start_ms: u64,
    /// Segment end, milliseconds from huddle start — from the `end` tag.
    pub end_ms: u64,
    /// Detected/declared language (BCP-47), when present — from the `lang` tag.
    pub language: Option<String>,
}

impl HuddleTranscriptSegment {
    /// Parse a segment from an event's `content` and raw string tags.
    ///
    /// `tags` is a slice of raw tag arrays (`["h", "<uuid>"]`, …) — the shape
    /// yielded by `event.tags.iter().map(|t| t.as_slice())`. Kept free of the
    /// `nostr` event type so it is trivially testable and reusable.
    ///
    /// Returns `None` when a required tag (`h`, `p`, `start`, `end`) is missing
    /// or malformed, or when `end_ms < start_ms`. Content is taken verbatim
    /// (empty is allowed at the type level; the builder rejects it on write).
    pub fn from_content_and_tags<S: AsRef<str>>(content: &str, tags: &[&[S]]) -> Option<Self> {
        let find = |key: &str| -> Option<&str> {
            tags.iter().find_map(|t| {
                let k = t.first()?.as_ref();
                if k == key {
                    Some(t.get(1)?.as_ref())
                } else {
                    None
                }
            })
        };

        let channel_id = find(TAG_CHANNEL)?.parse::<Uuid>().ok()?;
        let speaker_pubkey = find(TAG_SPEAKER)?.to_string();
        let start_ms = find(TAG_START_MS)?.parse::<u64>().ok()?;
        let end_ms = find(TAG_END_MS)?.parse::<u64>().ok()?;
        if end_ms < start_ms {
            return None;
        }
        let huddle_event_id = find(TAG_HUDDLE_EVENT).map(str::to_string);
        let language = find(TAG_LANG).map(str::to_string);

        Some(Self {
            channel_id,
            speaker_pubkey,
            huddle_event_id,
            text: content.to_string(),
            start_ms,
            end_ms,
            language,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PK: &str = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    const HUDDLE: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn tags(pairs: &[[&str; 2]]) -> Vec<Vec<String>> {
        pairs
            .iter()
            .map(|[k, v]| vec![k.to_string(), v.to_string()])
            .collect()
    }

    fn as_slices(tags: &[Vec<String>]) -> Vec<&[String]> {
        tags.iter().map(Vec::as_slice).collect()
    }

    #[test]
    fn parses_a_full_segment() {
        let cid = Uuid::new_v4();
        let t = tags(&[
            [TAG_CHANNEL, &cid.to_string()],
            [TAG_SPEAKER, PK],
            [TAG_HUDDLE_EVENT, HUDDLE],
            [TAG_START_MS, "1200"],
            [TAG_END_MS, "3400"],
            [TAG_LANG, "uk"],
        ]);
        let seg = HuddleTranscriptSegment::from_content_and_tags("привіт світ", &as_slices(&t))
            .expect("parses");
        assert_eq!(seg.channel_id, cid);
        assert_eq!(seg.speaker_pubkey, PK);
        assert_eq!(seg.huddle_event_id.as_deref(), Some(HUDDLE));
        assert_eq!(seg.text, "привіт світ");
        assert_eq!(seg.start_ms, 1200);
        assert_eq!(seg.end_ms, 3400);
        assert_eq!(seg.language.as_deref(), Some("uk"));
    }

    #[test]
    fn optional_tags_may_be_absent() {
        let cid = Uuid::new_v4();
        let t = tags(&[
            [TAG_CHANNEL, &cid.to_string()],
            [TAG_SPEAKER, PK],
            [TAG_START_MS, "0"],
            [TAG_END_MS, "500"],
        ]);
        let seg =
            HuddleTranscriptSegment::from_content_and_tags("hi", &as_slices(&t)).expect("parses");
        assert_eq!(seg.huddle_event_id, None);
        assert_eq!(seg.language, None);
    }

    #[test]
    fn missing_required_tag_is_none() {
        let cid = Uuid::new_v4();
        // No speaker `p` tag.
        let t = tags(&[
            [TAG_CHANNEL, &cid.to_string()],
            [TAG_START_MS, "0"],
            [TAG_END_MS, "1"],
        ]);
        assert!(HuddleTranscriptSegment::from_content_and_tags("hi", &as_slices(&t)).is_none());
    }

    #[test]
    fn malformed_values_are_none() {
        let cid = Uuid::new_v4();
        // Non-numeric start.
        let bad_start = tags(&[
            [TAG_CHANNEL, &cid.to_string()],
            [TAG_SPEAKER, PK],
            [TAG_START_MS, "soon"],
            [TAG_END_MS, "1"],
        ]);
        assert!(
            HuddleTranscriptSegment::from_content_and_tags("hi", &as_slices(&bad_start)).is_none()
        );

        // Non-uuid channel.
        let bad_channel = tags(&[
            [TAG_CHANNEL, "not-a-uuid"],
            [TAG_SPEAKER, PK],
            [TAG_START_MS, "0"],
            [TAG_END_MS, "1"],
        ]);
        assert!(
            HuddleTranscriptSegment::from_content_and_tags("hi", &as_slices(&bad_channel))
                .is_none()
        );
    }

    #[test]
    fn end_before_start_is_rejected() {
        let cid = Uuid::new_v4();
        let t = tags(&[
            [TAG_CHANNEL, &cid.to_string()],
            [TAG_SPEAKER, PK],
            [TAG_START_MS, "5000"],
            [TAG_END_MS, "1000"],
        ]);
        assert!(HuddleTranscriptSegment::from_content_and_tags("hi", &as_slices(&t)).is_none());
    }
}
