use std::cmp::Reverse;

use crate::client::{normalize_events, BuzzClient};
use crate::error::CliError;

const VALID_FEED_TYPES: &[&str] = &["mentions", "needs_action", "activity", "agent_activity"];

/// Feed types requested when `--types` is omitted.
///
/// `agent_activity` is deliberately absent — the relay canonicalizes it to
/// `activity`, so including both would just dedupe to the same feed.
const DEFAULT_FEED_TYPES: &[&str] = &["mentions", "needs_action", "activity"];

/// Build the `POST /query` filter for the activity feed.
///
/// `feed_types` routes the query to the relay's bounded feed queries — direct
/// `p`-tag mentions UNION NIP-CM `@channel` notifications, membership- and
/// visibility-scoped server-side. Without it the bridge treats this as a raw
/// `#p` filter, and marker-only `@channel` events (which carry no `p` tag)
/// never produce a feed row. The raw `#p`/`limit` fields stay as a graceful
/// fallback: the bridge ignores them when `feed_types` is present, while a
/// relay predating the extension drops the unknown field and still serves
/// direct mentions.
fn build_feed_filter(
    my_pk: &str,
    since: Option<i64>,
    limit: u32,
    types: Option<&str>,
) -> Result<serde_json::Value, CliError> {
    let mut filter = serde_json::json!({
        "#p": [my_pk],
        "limit": limit
    });

    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }

    filter["feed_types"] = match types {
        Some(types_str) => {
            let type_list: Vec<&str> = types_str.split(',').map(str::trim).collect();
            for t in &type_list {
                if !VALID_FEED_TYPES.contains(t) {
                    return Err(CliError::Usage(format!(
                        "invalid feed type {t:?} — must be one of: {}",
                        VALID_FEED_TYPES.join(", ")
                    )));
                }
            }
            serde_json::json!(type_list)
        }
        None => serde_json::json!(DEFAULT_FEED_TYPES),
    };

    Ok(filter)
}

/// Get activity feed — mentions, needs-action, and activity rows addressed to us.
pub async fn cmd_get_feed(
    client: &BuzzClient,
    since: Option<i64>,
    limit: Option<u32>,
    types: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let limit = limit.unwrap_or(20).min(50);

    let filter = build_feed_filter(&my_pk, since, limit, types)?;

    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| Reverse(e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0)));
    let normalized = normalize_events(&events);
    let output = match format {
        crate::OutputFormat::Compact => {
            let evts: Vec<serde_json::Value> =
                serde_json::from_str(&normalized).unwrap_or_default();
            let compact: Vec<serde_json::Value> = evts
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.get("id").cloned().unwrap_or_default(),
                        "content": e.get("content").cloned().unwrap_or_default(),
                        "created_at": e.get("created_at").cloned().unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json => normalized,
    };
    println!("{output}");
    Ok(())
}

pub async fn dispatch(
    cmd: crate::FeedCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::FeedCmd;
    match cmd {
        FeedCmd::Get {
            since,
            limit,
            types,
        } => cmd_get_feed(client, since, limit, types.as_deref(), format).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn default_requests_all_bounded_feeds() {
        let filter = build_feed_filter(PK, None, 20, None).expect("default filter builds");
        assert_eq!(
            filter["feed_types"],
            serde_json::json!(["mentions", "needs_action", "activity"]),
            "without --types the query must still route to the bounded feeds so marker-only @channel rows appear"
        );
        assert_eq!(filter["#p"], serde_json::json!([PK]));
        assert_eq!(filter["limit"], serde_json::json!(20));
        assert!(filter.get("since").is_none());
    }

    #[test]
    fn explicit_types_are_passed_through() {
        let filter = build_feed_filter(PK, Some(1_700_000_000), 5, Some("mentions,activity"))
            .expect("explicit filter builds");
        assert_eq!(
            filter["feed_types"],
            serde_json::json!(["mentions", "activity"])
        );
        assert_eq!(filter["#p"], serde_json::json!([PK]));
        assert_eq!(filter["since"], serde_json::json!(1_700_000_000));
    }

    #[test]
    fn invalid_type_is_a_usage_error() {
        let err = build_feed_filter(PK, None, 20, Some("mentions,bogus"))
            .expect_err("invalid type must be rejected");
        match err {
            CliError::Usage(msg) => assert!(msg.contains("bogus"), "message names the bad type"),
            other => panic!("expected Usage error, got {other:?}"),
        }
    }

    #[test]
    fn p_tag_retained_for_every_type_selection() {
        for types in [None, Some("mentions"), Some("needs_action,agent_activity")] {
            let filter = build_feed_filter(PK, None, 20, types).expect("filter builds");
            assert_eq!(
                filter["#p"],
                serde_json::json!([PK]),
                "#p is the fallback for relays predating feed_types ({types:?})"
            );
        }
    }
}
