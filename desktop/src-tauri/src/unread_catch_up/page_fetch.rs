use std::time::Duration;

use nostr::{Event, Keys};

use super::{AppState, PageCursor, CATCH_UP_LIMIT};

const PAGE_TIMEOUT: Duration = Duration::from_secs(10);

fn next_page_cursor(
    page_len: usize,
    last: Option<PageCursor>,
    previous: Option<&PageCursor>,
) -> Result<Option<PageCursor>, String> {
    if page_len < CATCH_UP_LIMIT {
        return Ok(None);
    }
    let next = last.ok_or_else(|| "full unread catch-up page had no cursor event".to_string())?;
    if previous == Some(&next) {
        return Err("unread catch-up pagination cursor did not advance".to_string());
    }
    Ok(Some(next))
}

pub(super) fn apply_page_cursor(filter: &mut serde_json::Value, cursor: &PageCursor) {
    filter["until"] = serde_json::json!(cursor.created_at);
    filter["before_id"] = serde_json::json!(cursor.event_id);
}

pub(super) async fn query_page(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    filter: serde_json::Value,
) -> Result<Vec<Event>, String> {
    tokio::time::timeout(
        PAGE_TIMEOUT,
        crate::relay::query_relay_at_with_keys(state, api_base, &[filter], keys, None),
    )
    .await
    .map_err(|_| "unread catch-up relay page timed out".to_string())?
}

pub(super) async fn fetch_filter_pages(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    base: &serde_json::Value,
) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let mut filter = base.clone();
        if let Some(current) = &cursor {
            apply_page_cursor(&mut filter, current);
        }
        let page = query_page(state, api_base, keys, filter).await?;
        let next = next_page_cursor(
            page.len(),
            page.last().map(|event| PageCursor {
                created_at: event.created_at.as_secs(),
                event_id: event.id.to_hex(),
            }),
            cursor.as_ref(),
        )?;
        events.extend(page);
        let Some(next) = next else { break };
        cursor = Some(next);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_pages_advance_the_composite_cursor_without_skipping_ties() {
        let first = PageCursor {
            created_at: 500,
            event_id: "0001".into(),
        };
        let tied = PageCursor {
            created_at: 500,
            event_id: "0002".into(),
        };
        assert_eq!(
            next_page_cursor(CATCH_UP_LIMIT, Some(first.clone()), None),
            Ok(Some(first.clone()))
        );
        assert_eq!(
            next_page_cursor(CATCH_UP_LIMIT, Some(tied.clone()), Some(&first)),
            Ok(Some(tied))
        );
        assert_eq!(
            next_page_cursor(CATCH_UP_LIMIT - 1, Some(first.clone()), None),
            Ok(None)
        );
        assert!(next_page_cursor(CATCH_UP_LIMIT, Some(first.clone()), Some(&first)).is_err());
    }

    #[test]
    fn page_filter_carries_timestamp_and_event_id_tiebreak() {
        let mut filter = serde_json::json!({"limit": CATCH_UP_LIMIT});
        apply_page_cursor(
            &mut filter,
            &PageCursor {
                created_at: 500,
                event_id: "ab".repeat(32),
            },
        );
        assert_eq!(filter["until"], 500);
        assert_eq!(filter["before_id"], "ab".repeat(32));
    }
}
