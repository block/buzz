//! Canvas event builders.

use nostr::{EventBuilder, Kind, Tag};
use uuid::Uuid;

fn tag(parts: Vec<&str>) -> Result<Tag, String> {
    Tag::parse(parts).map_err(|e| format!("invalid tag: {e}"))
}

fn check_content(content: &str) -> Result<(), String> {
    const MAX_CONTENT_BYTES: usize = 64 * 1024;
    if content.len() > MAX_CONTENT_BYTES {
        return Err(format!(
            "content exceeds maximum size of {} bytes (got {})",
            MAX_CONTENT_BYTES,
            content.len()
        ));
    }
    Ok(())
}

/// Kind 40100 — set canvas.
pub fn build_set_canvas(channel_id: Uuid, content: &str) -> Result<EventBuilder, String> {
    check_content(content)?;
    let tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(40100), content).tags(tags))
}
