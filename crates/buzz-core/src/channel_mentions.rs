//! NIP-CM channel-wide mentions — the `["notify", …]` marker tag.
//!
//! A channel-wide mention is carried by a single marker tag on the message
//! event itself; there is no per-member `p` tag expansion, so the roster is
//! never written into the event and agents are never woken by `@channel` or
//! `@here`.
//!
//! ```text
//! ["notify", "channel"]   // every member of the channel
//! ["notify", "here"]      // members who are online right now (live-only)
//! ```
//!
//! Validation here is pure (no I/O): it covers tag shape, mode spelling,
//! at-most-one-tag, and the allowed kinds. The DM-channel rejection needs the
//! channel row and therefore lives at the relay ingest seam, which calls
//! [`validate_notify_tag`] first and then applies
//! [`NotifyTagError::DirectMessage`] itself.

use std::fmt;
use std::str::FromStr;

use crate::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_EDIT,
};

/// Tag name carrying a channel-wide mention.
pub const NOTIFY_TAG: &str = "notify";

/// Reserved mention tokens that must never resolve to a member identity.
///
/// Parsers compare case-insensitively: a member whose display name is
/// literally `here` still loses to the reserved token.
pub const RESERVED_MENTION_TOKENS: [&str; 2] = ["channel", "here"];

/// Event kinds that may carry a [`NOTIFY_TAG`].
///
/// `40003` (message edit) is accepted for render continuity only — it never
/// escalates a notification and never persists a feed row (see
/// [`persists_channel_notification`]).
pub const NOTIFY_ALLOWED_KINDS: [u32; 4] = [
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_EDIT,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];

/// Returns whether `token` is a reserved channel-wide mention token.
///
/// Comparison is ASCII case-insensitive and the token must be given without
/// its leading `@`.
pub fn is_reserved_mention_token(token: &str) -> bool {
    RESERVED_MENTION_TOKENS
        .iter()
        .any(|reserved| token.eq_ignore_ascii_case(reserved))
}

/// Who a `["notify", …]` tag notifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyMode {
    /// Every member of the channel; persistent (feed row, badge, offline catch-up).
    Channel,
    /// Members who are online at delivery time; live-only, never persisted.
    Here,
}

impl NotifyMode {
    /// Canonical string representation (the tag's second element).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Here => "here",
        }
    }
}

impl fmt::Display for NotifyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NotifyMode {
    type Err = NotifyTagError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "channel" => Ok(Self::Channel),
            "here" => Ok(Self::Here),
            other => Err(NotifyTagError::InvalidMode(other.to_string())),
        }
    }
}

/// Why a `["notify", …]` tag was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyTagError {
    /// The tag has no mode element (`["notify"]`).
    MissingMode,
    /// The mode is not `channel` or `here`. Carries the offending value.
    InvalidMode(String),
    /// More than one notify tag on a single event.
    Duplicate,
    /// The event kind may not carry a notify tag. Carries the kind.
    KindNotAllowed(u32),
    /// Channel-wide mentions are meaningless in a DM channel.
    DirectMessage,
}

impl fmt::Display for NotifyTagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMode => write!(f, "notify tag requires a mode value"),
            Self::InvalidMode(value) => {
                write!(
                    f,
                    "invalid notify mode {value:?} (expected channel or here)"
                )
            }
            Self::Duplicate => write!(f, "at most one notify tag is allowed per event"),
            Self::KindNotAllowed(kind) => {
                write!(f, "kind {kind} may not carry a notify tag")
            }
            Self::DirectMessage => {
                write!(f, "channel-wide mentions are not allowed in DM channels")
            }
        }
    }
}

impl std::error::Error for NotifyTagError {}

/// Validate the notify tag (if any) carried by an event's tags.
///
/// Returns `Ok(None)` when the event carries no notify tag, `Ok(Some(mode))`
/// when it carries exactly one well-formed tag on an allowed kind, and an
/// error otherwise. Extra elements past the mode are ignored, matching Nostr's
/// forward-compatible tag convention.
///
/// This function performs no I/O; the DM-channel rule is applied by the caller
/// that can see the channel row.
pub fn validate_notify_tag<'a, I, T>(
    kind: u32,
    tags: I,
) -> Result<Option<NotifyMode>, NotifyTagError>
where
    I: IntoIterator<Item = &'a T>,
    T: AsRef<[String]> + 'a,
{
    let mut found: Option<NotifyMode> = None;
    for tag in tags {
        let parts = tag.as_ref();
        let Some(name) = parts.first() else {
            continue;
        };
        if name != NOTIFY_TAG {
            continue;
        }
        if found.is_some() {
            return Err(NotifyTagError::Duplicate);
        }
        let value = parts.get(1).ok_or(NotifyTagError::MissingMode)?;
        found = Some(value.parse::<NotifyMode>()?);
    }

    if found.is_some() && !NOTIFY_ALLOWED_KINDS.contains(&kind) {
        return Err(NotifyTagError::KindNotAllowed(kind));
    }

    Ok(found)
}

/// Validate the notify tag carried by a signed Nostr event.
///
/// Thin wrapper over [`validate_notify_tag`] for callers holding an event.
pub fn event_notify_mode(event: &nostr::Event) -> Result<Option<NotifyMode>, NotifyTagError> {
    let tags: Vec<&[String]> = event.tags.iter().map(|tag| tag.as_slice()).collect();
    validate_notify_tag(event.kind.as_u16() as u32, &tags)
}

/// Whether an accepted notify tag persists a `channel_notifications` feed row.
///
/// Only `mode = channel` persists, and only on the kinds that create new
/// content: edits (`40003`) re-carry the tag for rendering but must not
/// re-notify, and `here` is live-only by design.
pub fn persists_channel_notification(kind: u32, mode: NotifyMode) -> bool {
    mode == NotifyMode::Channel
        && matches!(
            kind,
            KIND_STREAM_MESSAGE | KIND_FORUM_POST | KIND_FORUM_COMMENT
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(raw: &[&[&str]]) -> Vec<Vec<String>> {
        raw.iter()
            .map(|tag| tag.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn no_notify_tag_is_ok() {
        let t = tags(&[&["h", "abc"], &["p", "deadbeef"]]);
        assert_eq!(validate_notify_tag(KIND_STREAM_MESSAGE, &t), Ok(None));
    }

    #[test]
    fn parses_both_modes() {
        for (value, expected) in [("channel", NotifyMode::Channel), ("here", NotifyMode::Here)] {
            let t = tags(&[&["notify", value]]);
            assert_eq!(
                validate_notify_tag(KIND_STREAM_MESSAGE, &t),
                Ok(Some(expected))
            );
        }
    }

    #[test]
    fn rejects_unknown_mode() {
        let t = tags(&[&["notify", "everyone"]]);
        assert_eq!(
            validate_notify_tag(KIND_STREAM_MESSAGE, &t),
            Err(NotifyTagError::InvalidMode("everyone".into()))
        );
    }

    #[test]
    fn mode_is_case_sensitive() {
        let t = tags(&[&["notify", "Channel"]]);
        assert!(matches!(
            validate_notify_tag(KIND_STREAM_MESSAGE, &t),
            Err(NotifyTagError::InvalidMode(_))
        ));
    }

    #[test]
    fn rejects_missing_mode() {
        let t = tags(&[&["notify"]]);
        assert_eq!(
            validate_notify_tag(KIND_STREAM_MESSAGE, &t),
            Err(NotifyTagError::MissingMode)
        );
    }

    #[test]
    fn rejects_duplicate_tags() {
        let t = tags(&[&["notify", "channel"], &["notify", "here"]]);
        assert_eq!(
            validate_notify_tag(KIND_STREAM_MESSAGE, &t),
            Err(NotifyTagError::Duplicate)
        );
        let same = tags(&[&["notify", "channel"], &["notify", "channel"]]);
        assert_eq!(
            validate_notify_tag(KIND_STREAM_MESSAGE, &same),
            Err(NotifyTagError::Duplicate)
        );
    }

    #[test]
    fn duplicate_check_precedes_kind_check() {
        let t = tags(&[&["notify", "channel"], &["notify", "channel"]]);
        assert_eq!(
            validate_notify_tag(1, &t),
            Err(NotifyTagError::Duplicate),
            "shape errors are reported before the kind gate"
        );
    }

    #[test]
    fn allows_only_the_four_kinds() {
        let t = tags(&[&["notify", "channel"]]);
        for kind in NOTIFY_ALLOWED_KINDS {
            assert!(validate_notify_tag(kind, &t).is_ok(), "kind {kind}");
        }
        for kind in [1u32, 7, 40002, 45002, 9735] {
            assert_eq!(
                validate_notify_tag(kind, &t),
                Err(NotifyTagError::KindNotAllowed(kind)),
                "kind {kind}"
            );
        }
    }

    #[test]
    fn disallowed_kind_without_tag_is_fine() {
        let t = tags(&[&["e", "abc"]]);
        assert_eq!(validate_notify_tag(1, &t), Ok(None));
    }

    #[test]
    fn extra_tag_elements_are_ignored() {
        let t = tags(&[&["notify", "here", "future-field"]]);
        assert_eq!(
            validate_notify_tag(KIND_FORUM_POST, &t),
            Ok(Some(NotifyMode::Here))
        );
    }

    #[test]
    fn only_channel_mode_persists_and_never_on_edits() {
        assert!(persists_channel_notification(
            KIND_STREAM_MESSAGE,
            NotifyMode::Channel
        ));
        assert!(persists_channel_notification(
            KIND_FORUM_POST,
            NotifyMode::Channel
        ));
        assert!(persists_channel_notification(
            KIND_FORUM_COMMENT,
            NotifyMode::Channel
        ));
        assert!(!persists_channel_notification(
            KIND_STREAM_MESSAGE_EDIT,
            NotifyMode::Channel
        ));
        for kind in NOTIFY_ALLOWED_KINDS {
            assert!(
                !persists_channel_notification(kind, NotifyMode::Here),
                "here is live-only (kind {kind})"
            );
        }
    }

    #[test]
    fn mode_round_trips_through_str() {
        for mode in [NotifyMode::Channel, NotifyMode::Here] {
            assert_eq!(mode.as_str().parse::<NotifyMode>(), Ok(mode));
        }
    }

    #[test]
    fn reserved_tokens_are_case_insensitive() {
        for token in ["channel", "Channel", "HERE", "here"] {
            assert!(is_reserved_mention_token(token), "{token}");
        }
        for token in ["chan", "everyone", "here2", ""] {
            assert!(!is_reserved_mention_token(token), "{token}");
        }
    }

    #[test]
    fn event_helper_reads_tags_from_signed_event() {
        use nostr::{EventBuilder, Keys, Kind, Tag};
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "hi")
            .tags([Tag::parse(["notify", "channel"]).expect("tag")])
            .sign_with_keys(&keys)
            .expect("sign");
        assert_eq!(event_notify_mode(&event), Ok(Some(NotifyMode::Channel)));
    }
}
