//! Strict Sonar sticker validation shared by relay and clients.
//!
//! The upstream `sonar-stickers` crate owns the interoperable wire model. Buzz
//! adds envelope limits and canonical-shape checks before an event may replace
//! an existing pack or installed-list head.

use nostr::Event;
use sonar_stickers::{
    parse_installed_pack_list, parse_pack_event, parse_sticker_ref_tag, InstalledPackList,
    StickerPack, StickerRef,
};
use thiserror::Error;
use url::Url;

const MAX_EVENT_TAGS: usize = 1_024;
const MAX_TAG_FIELDS: usize = 8;
const MAX_TAG_FIELD_BYTES: usize = 2_048;
const MAX_INSTALLED_PACKS: usize = 200;
const MAX_LICENSE_CHARS: usize = 160;

/// Maximum number of exact pack revisions in one workspace catalog snapshot.
///
/// Kind `13536` carries one `a` tag per approved revision plus the required
/// protected-event marker, so this also keeps relay-authored events comfortably
/// below the common event tag limit.
pub const MAX_STICKER_CATALOG_PACKS: usize = 500;

/// Error returned when a Sonar event or sticker reference is not canonical.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StickerValidationError {
    /// The event content must be empty because all public pack data is tagged.
    #[error("content must be empty")]
    NonEmptyContent,
    /// The event contains too many tags.
    #[error("too many tags: {0} > {MAX_EVENT_TAGS}")]
    TooManyTags(usize),
    /// A tag has an invalid field count or field size.
    #[error("invalid {name} tag: {reason}")]
    InvalidTag {
        /// Tag name.
        name: String,
        /// Human-readable rejection reason.
        reason: String,
    },
    /// A singleton tag was missing or repeated.
    #[error("expected exactly one {0} tag")]
    SingletonTag(&'static str),
    /// The upstream Sonar model rejected the event.
    #[error("{0}")]
    Sonar(String),
}

/// Validate and parse a canonical Sonar kind `30031` sticker-pack event.
pub fn validate_sticker_pack_event(event: &Event) -> Result<StickerPack, StickerValidationError> {
    validate_common_envelope(event)?;
    require_singleton(event, "d")?;
    require_singleton(event, "title")?;
    require_singleton_value(event, "pack_format", sonar_stickers::PACK_FORMAT)?;
    require_optional_singleton_value(event, "t", sonar_stickers::PACK_FORMAT)?;
    require_at_most_one(event, "description")?;
    require_at_most_one(event, "image")?;
    require_at_most_one(event, "license")?;

    for tag in event.tags.iter() {
        let fields = tag.as_slice();
        let name = fields.first().map(String::as_str).unwrap_or_default();
        match name {
            "d" | "title" | "description" | "license" if fields.len() != 2 => {
                return Err(invalid_tag(name, "expected exactly one value"));
            }
            "sticker" if !(6..=8).contains(&fields.len()) => {
                return Err(invalid_tag(
                    name,
                    "expected shortcode, url, sha256, mime, dim, and optional alt and emoji",
                ));
            }
            "emoji" if fields.len() != 3 => {
                return Err(invalid_tag(name, "expected shortcode and URL"));
            }
            "image" if !(3..=4).contains(&fields.len()) => {
                return Err(invalid_tag(
                    name,
                    "expected URL, sha256, and optional dimensions",
                ));
            }
            _ => {}
        }

        if matches!(name, "sticker" | "image") {
            validate_asset_tag(fields)?;
        }
    }

    if event
        .tags
        .iter()
        .find_map(|tag| {
            let fields = tag.as_slice();
            (fields.first().map(String::as_str) == Some("license"))
                .then(|| fields.get(1))
                .flatten()
        })
        .is_some_and(|license| license.chars().count() > MAX_LICENSE_CHARS)
    {
        return Err(invalid_tag("license", "must be at most 160 characters"));
    }

    let pack = parse_pack_event(event)
        .map_err(|error| StickerValidationError::Sonar(error.to_string()))?;

    // NIP-30 compatibility tags are optional, but each one that is present
    // must uniquely and exactly name a sticker from this revision.
    for compatibility in event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("emoji"))
    {
        let fields = compatibility.as_slice();
        let matches = pack.stickers.iter().filter(|sticker| {
            fields.get(1).map(String::as_str) == Some(sticker.shortcode.as_str())
                && fields.get(2).map(String::as_str) == Some(sticker.url.as_str())
        });
        if matches.count() != 1 {
            return Err(invalid_tag(
                "emoji",
                "compatibility tag must exactly match a sticker shortcode and URL",
            ));
        }
        let duplicate_count = event
            .tags
            .iter()
            .filter(|tag| {
                let fields = tag.as_slice();
                fields.first().map(String::as_str) == Some("emoji")
                    && fields.get(1) == compatibility.as_slice().get(1)
                    && fields.get(2) == compatibility.as_slice().get(2)
            })
            .count();
        if duplicate_count != 1 {
            return Err(invalid_tag("emoji", "duplicate compatibility tag"));
        }
    }
    Ok(pack)
}

/// Validate and parse a canonical Sonar kind `10031` installed-pack list.
pub fn validate_installed_pack_list_event(
    event: &Event,
) -> Result<InstalledPackList, StickerValidationError> {
    validate_common_envelope(event)?;
    if event.tags.len() > MAX_INSTALLED_PACKS {
        return Err(StickerValidationError::InvalidTag {
            name: "a".into(),
            reason: format!(
                "too many installed packs: {} > {MAX_INSTALLED_PACKS}",
                event.tags.len()
            ),
        });
    }
    for tag in event.tags.iter() {
        let fields = tag.as_slice();
        if fields.len() != 2 || fields.first().map(String::as_str) != Some("a") {
            return Err(invalid_tag("a", "expected only exact [a, coordinate] tags"));
        }
        let coordinate = fields.get(1).map(String::as_str).unwrap_or_default();
        let parsed = sonar_stickers::PackAddress::parse(coordinate)
            .map_err(|error| StickerValidationError::Sonar(error.to_string()))?;
        if parsed.coordinate() != coordinate {
            return Err(invalid_tag(
                "a",
                "coordinate must use canonical lowercase hex",
            ));
        }
    }
    let list = parse_installed_pack_list(event)
        .map_err(|error| StickerValidationError::Sonar(error.to_string()))?;
    if list.packs.len() != event.tags.len() {
        return Err(invalid_tag("a", "duplicate pack coordinate"));
    }
    Ok(list)
}

/// Validate the optional sticker reference carried by an ordinary message.
///
/// Returns `Ok(None)` when the message is not a sticker message. A sticker
/// message may contain exactly one four-field tag:
/// `["sticker", "30031:<author>:<id>", "<shortcode>", "<sha256>"]`.
pub fn validate_message_sticker_ref(
    event: &Event,
) -> Result<Option<StickerRef>, StickerValidationError> {
    let mut refs = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("sticker"));
    let Some(tag) = refs.next() else {
        return Ok(None);
    };
    if refs.next().is_some() {
        return Err(invalid_tag("sticker", "at most one reference is allowed"));
    }
    if tag.as_slice().len() != 4 {
        return Err(invalid_tag(
            "sticker",
            "expected exactly pack coordinate, shortcode, and plaintext sha256",
        ));
    }
    let sticker_ref = parse_sticker_ref_tag(tag)
        .map_err(|error| StickerValidationError::Sonar(error.to_string()))?;
    if sticker_ref.pack.coordinate()
        != tag
            .as_slice()
            .get(1)
            .map(String::as_str)
            .unwrap_or_default()
        || sticker_ref.plaintext_sha256
            != tag
                .as_slice()
                .get(3)
                .map(String::as_str)
                .unwrap_or_default()
    {
        return Err(invalid_tag(
            "sticker",
            "coordinate and hash must use canonical lowercase hex",
        ));
    }
    Ok(Some(sticker_ref))
}

/// Return the declared APNG frame count when PNG chunks carry a coherent
/// animation-control sequence.
///
/// A raw search for the bytes `acTL` is unsafe because the same bytes may occur
/// inside compressed pixel data. This bounded parser only recognizes an
/// animation control chunk before image data, a matching number of frame
/// control chunks, and a complete PNG chunk sequence.
pub fn apng_frame_count(bytes: &[u8]) -> Option<u32> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.get(..PNG_SIGNATURE.len())? != PNG_SIGNATURE {
        return None;
    }

    let mut offset = PNG_SIGNATURE.len();
    let mut saw_ihdr = false;
    let mut saw_idat = false;
    let mut declared_frames = None;
    let mut frame_controls = 0u32;
    let mut saw_iend = false;

    while offset.checked_add(12)? <= bytes.len() {
        let length = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        let data_start = offset.checked_add(8)?;
        let data_end = data_start.checked_add(length)?;
        let chunk_end = data_end.checked_add(4)?;
        if chunk_end > bytes.len() {
            return None;
        }
        let chunk_type = bytes.get(offset + 4..offset + 8)?;
        let data = bytes.get(data_start..data_end)?;

        match chunk_type {
            b"IHDR" if !saw_ihdr && offset == PNG_SIGNATURE.len() && length == 13 => {
                saw_ihdr = true;
            }
            b"acTL" if saw_ihdr && !saw_idat && declared_frames.is_none() && length == 8 => {
                let frames = u32::from_be_bytes(data.get(..4)?.try_into().ok()?);
                if frames == 0 {
                    return None;
                }
                declared_frames = Some(frames);
            }
            b"fcTL" if declared_frames.is_some() && length == 26 => {
                frame_controls = frame_controls.checked_add(1)?;
            }
            b"IDAT" if saw_ihdr => saw_idat = true,
            b"IEND" if length == 0 => {
                saw_iend = true;
                offset = chunk_end;
                break;
            }
            _ => {}
        }
        offset = chunk_end;
    }

    declared_frames.filter(|frames| {
        saw_ihdr && saw_idat && saw_iend && offset == bytes.len() && frame_controls == *frames
    })
}

fn validate_common_envelope(event: &Event) -> Result<(), StickerValidationError> {
    if !event.content.is_empty() {
        return Err(StickerValidationError::NonEmptyContent);
    }
    if event.tags.len() > MAX_EVENT_TAGS {
        return Err(StickerValidationError::TooManyTags(event.tags.len()));
    }
    for tag in event.tags.iter() {
        let fields = tag.as_slice();
        let name = fields.first().cloned().unwrap_or_default();
        if fields.is_empty() || fields.len() > MAX_TAG_FIELDS {
            return Err(invalid_tag(&name, "invalid field count"));
        }
        if fields.iter().any(|field| field.len() > MAX_TAG_FIELD_BYTES) {
            return Err(invalid_tag(&name, "field exceeds 2048 bytes"));
        }
    }
    Ok(())
}

fn validate_asset_tag(fields: &[String]) -> Result<(), StickerValidationError> {
    let (url_index, hash_index) = if fields.first().map(String::as_str) == Some("sticker") {
        (2, 3)
    } else {
        (1, 2)
    };
    let url = fields
        .get(url_index)
        .map(String::as_str)
        .unwrap_or_default();
    let hash = fields
        .get(hash_index)
        .map(String::as_str)
        .unwrap_or_default();
    if hash.len() != 64
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        || hash.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(invalid_tag(
            "asset",
            "sha256 must be 64 lowercase hex characters",
        ));
    }
    let parsed = Url::parse(url).map_err(|_| invalid_tag("asset", "invalid URL"))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.port().is_some_and(|port| port != 443)
        || !parsed.path().to_ascii_lowercase().contains(hash)
    {
        return Err(invalid_tag(
            "asset",
            "URL must be credential-free HTTPS and contain the plaintext sha256 in its path",
        ));
    }
    Ok(())
}

fn require_singleton(event: &Event, name: &'static str) -> Result<(), StickerValidationError> {
    if tag_count(event, name) != 1 {
        return Err(StickerValidationError::SingletonTag(name));
    }
    Ok(())
}

fn require_singleton_value(
    event: &Event,
    name: &'static str,
    expected: &str,
) -> Result<(), StickerValidationError> {
    require_singleton(event, name)?;
    let valid = event.tags.iter().any(|tag| {
        let fields = tag.as_slice();
        fields.len() == 2
            && fields.first().map(String::as_str) == Some(name)
            && fields.get(1).map(String::as_str) == Some(expected)
    });
    if !valid {
        return Err(invalid_tag(name, "unexpected value or field count"));
    }
    Ok(())
}

fn require_optional_singleton_value(
    event: &Event,
    name: &'static str,
    expected: &str,
) -> Result<(), StickerValidationError> {
    match tag_count(event, name) {
        0 => Ok(()),
        1 => {
            let valid = event.tags.iter().any(|tag| {
                let fields = tag.as_slice();
                fields.len() == 2
                    && fields.first().map(String::as_str) == Some(name)
                    && fields.get(1).map(String::as_str) == Some(expected)
            });
            if valid {
                Ok(())
            } else {
                Err(invalid_tag(name, "unexpected value or field count"))
            }
        }
        _ => Err(invalid_tag(name, "tag may appear at most once")),
    }
}

fn require_at_most_one(event: &Event, name: &str) -> Result<(), StickerValidationError> {
    if tag_count(event, name) > 1 {
        return Err(invalid_tag(name, "tag may appear at most once"));
    }
    Ok(())
}

fn tag_count(event: &Event, name: &str) -> usize {
    event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
        .count()
}

fn invalid_tag(name: &str, reason: &str) -> StickerValidationError {
    StickerValidationError::InvalidTag {
        name: name.to_owned(),
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag, TagKind};
    use sonar_stickers::{build_pack_tags, PackAddress, Sticker, StickerPack};

    use super::*;

    const HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn pack_event() -> Event {
        let keys = Keys::generate();
        let sticker = Sticker::new(
            "wave",
            format!("https://cdn.example/{HASH}.webp"),
            HASH,
            "image/webp",
            Some(512),
            Some(512),
            Some("Wave".into()),
            Some("👋".into()),
        )
        .expect("valid fixture");
        let pack = StickerPack::new(
            PackAddress::new(keys.public_key().to_hex(), "waves").expect("valid address"),
            "Waves",
            None,
            None,
            vec![sticker],
            None,
        )
        .expect("valid pack");
        EventBuilder::new(Kind::Custom(sonar_stickers::STICKER_PACK_KIND), "")
            .tags(build_pack_tags(&pack))
            .sign_with_keys(&keys)
            .expect("sign fixture")
    }

    #[test]
    fn accepts_canonical_pack() {
        assert!(validate_sticker_pack_event(&pack_event()).is_ok());
    }

    #[test]
    fn accepts_minimal_pack_without_recommended_tags() {
        let original = pack_event();
        let keys = Keys::generate();
        let tags = original
            .tags
            .iter()
            .filter(|tag| {
                !matches!(
                    tag.as_slice().first().map(String::as_str),
                    Some("t" | "emoji")
                )
            })
            .map(|tag| {
                if tag.as_slice().first().map(String::as_str) == Some("sticker") {
                    Tag::parse(tag.as_slice()[..6].iter().map(String::as_str))
                        .expect("minimal sticker tag")
                } else {
                    tag.clone()
                }
            })
            .collect::<Vec<_>>();
        let event = EventBuilder::new(Kind::Custom(sonar_stickers::STICKER_PACK_KIND), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign fixture");
        assert!(validate_sticker_pack_event(&event).is_ok());
    }

    #[test]
    fn rejects_non_empty_pack_content() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(sonar_stickers::STICKER_PACK_KIND), "secret")
            .tags(pack_event().tags)
            .sign_with_keys(&keys)
            .expect("sign fixture");
        assert_eq!(
            validate_sticker_pack_event(&event),
            Err(StickerValidationError::NonEmptyContent)
        );
    }

    #[test]
    fn rejects_sticker_ref_with_extra_fields() {
        let keys = Keys::generate();
        let tag = Tag::custom(
            TagKind::Custom("sticker".into()),
            [
                format!("30031:{}:waves", keys.public_key().to_hex()),
                "wave".to_owned(),
                HASH.to_owned(),
                "extra".to_owned(),
            ],
        );
        let event = EventBuilder::new(Kind::TextNote, ":wave:")
            .tags([tag])
            .sign_with_keys(&keys)
            .expect("sign fixture");
        assert!(validate_message_sticker_ref(&event).is_err());
    }
}
