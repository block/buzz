use buzz_core_pkg::kind::KIND_WINDOW_BOUNDS;
use nostr::Event;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PageCursor {
    pub(super) created_at: u64,
    pub(super) event_id: String,
}

#[derive(Deserialize)]
struct WindowBoundsPayload {
    has_more: bool,
    next_cursor: Option<WindowBoundsCursor>,
}

#[derive(Deserialize)]
struct WindowBoundsCursor {
    created_at: u64,
    id: String,
}

fn expected_bounds_key(channel_id: &str, cursor: Option<&PageCursor>) -> String {
    let suffix = cursor.map_or_else(
        || "head".to_string(),
        |cursor| {
            format!(
                "{}:{}",
                cursor.created_at,
                cursor.event_id.to_ascii_lowercase()
            )
        },
    );
    format!("{}:{suffix}", channel_id.to_ascii_lowercase())
}

pub(super) fn parse_top_level_page(
    page: Vec<Event>,
    channel_id: &str,
    cursor: Option<&PageCursor>,
) -> Result<(Vec<Event>, Option<PageCursor>), String> {
    let mut rows = Vec::new();
    let mut bounds = None;
    for event in page {
        if event.kind.as_u16() as u32 == KIND_WINDOW_BOUNDS {
            if bounds.replace(event).is_some() {
                return Err("unread catch-up window returned multiple bounds events".to_string());
            }
        } else {
            rows.push(event);
        }
    }
    let bounds =
        bounds.ok_or_else(|| "unread catch-up window returned no bounds event".to_string())?;
    let bounds_key = bounds
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().is_some_and(|part| part == "d"))
        .and_then(|tag| tag.as_slice().get(1));
    let expected_bounds_key = expected_bounds_key(channel_id, cursor);
    if bounds_key.map(String::as_str) != Some(expected_bounds_key.as_str()) {
        return Err("unread catch-up window bounds did not match its request cursor".to_string());
    }
    let payload: WindowBoundsPayload = serde_json::from_str(&bounds.content)
        .map_err(|error| format!("invalid unread catch-up window bounds: {error}"))?;
    let next = payload.next_cursor.map(|next| PageCursor {
        created_at: next.created_at,
        event_id: next.id,
    });
    if payload.has_more != next.is_some() {
        return Err("unread catch-up window bounds cursor disagreed with has_more".to_string());
    }
    if cursor == next.as_ref() {
        return Err("unread catch-up window cursor did not advance".to_string());
    }
    Ok((rows, next))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    fn signed_event(kind: u16, content: &str, tags: &[&[&str]]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("parse tag"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::from_u16(kind), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    #[test]
    fn uses_bounds_cursor_and_excludes_overlay() {
        let row = signed_event(9, "message", &[&["h", "ch"]]);
        let next_id = "ab".repeat(32);
        let bounds = signed_event(
            KIND_WINDOW_BOUNDS as u16,
            &serde_json::json!({
                "has_more": true,
                "next_cursor": {"created_at": 500, "id": next_id},
            })
            .to_string(),
            &[&["d", "ch:head"], &["h", "ch"]],
        );

        let (rows, next) =
            parse_top_level_page(vec![row.clone(), bounds], "ch", None).expect("parse page");

        assert_eq!(rows, [row]);
        assert_eq!(
            next,
            Some(PageCursor {
                created_at: 500,
                event_id: "ab".repeat(32),
            })
        );
    }

    #[test]
    fn terminal_page_requires_matching_bounds_contract() {
        let cursor = PageCursor {
            created_at: 500,
            event_id: "AB".repeat(32),
        };
        let bounds_key = format!("ch:500:{}", "ab".repeat(32));
        let bounds = signed_event(
            KIND_WINDOW_BOUNDS as u16,
            r#"{"has_more":false,"next_cursor":null}"#,
            &[&["d", bounds_key.as_str()]],
        );

        let (rows, next) =
            parse_top_level_page(vec![bounds], "CH", Some(&cursor)).expect("parse terminal page");
        assert!(rows.is_empty());
        assert_eq!(next, None);
        assert!(parse_top_level_page(Vec::new(), "ch", None).is_err());
    }
}
