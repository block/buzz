use std::cmp::Reverse;

use crate::client::{normalize_events, BuzzClient};
use crate::error::CliError;

const VALID_FEED_TYPES: &[&str] = &["mentions", "needs_action", "activity", "agent_activity"];

/// Get activity feed — query events mentioning our pubkey (via p-tag).
pub async fn cmd_get_feed(
    client: &BuzzClient,
    since: Option<i64>,
    limit: Option<u32>,
    types: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let limit = limit.unwrap_or(20).min(50);

    let mut filter = serde_json::json!({
        "#p": [my_pk],
        "limit": limit
    });

    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }

    if let Some(types_str) = types {
        let type_list: Vec<&str> = types_str.split(',').map(str::trim).collect();
        for t in &type_list {
            if !VALID_FEED_TYPES.contains(t) {
                return Err(crate::error::CliError::Usage(format!(
                    "invalid feed type {t:?} — must be one of: {}",
                    VALID_FEED_TYPES.join(", ")
                )));
            }
        }
        filter["feed_types"] = serde_json::json!(type_list);
    }

    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| Reverse(e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0)));
    let normalized = normalize_events(&events);
    // One projection, shared with `messages`. `buzz feed` filters on `#p` across
    // everything, so its results are cross-channel and cross-author by
    // construction — a hit that carries neither is the least actionable output
    // of the three, and a second copy of this shape is how the two drifted.
    let output = crate::commands::messages::format_events(&normalized, format);
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
mod feed_compact_projection_tests {
    use crate::commands::messages::format_events;
    use crate::OutputFormat;

    const AUTHOR: &str = "ff04bc24c4a5fd1b6450d3bf62c049b106bb701777adc8f1f51716fde45550c3";
    const CHANNEL_A: &str = "fb298e44-68b0-46f9-988c-f994b938deb9";
    const CHANNEL_B: &str = "740851fb-09dd-4e7c-b507-bd1f9106d1b6";

    fn feed_events() -> String {
        serde_json::json!([
            {"id": "a", "pubkey": AUTHOR, "content": "one", "created_at": 1,
             "tags": [["h", CHANNEL_A]]},
            {"id": "b", "pubkey": AUTHOR, "content": "two", "created_at": 2,
             "tags": [["h", CHANNEL_B]]}
        ])
        .to_string()
    }

    /// `buzz feed` filters on `#p` across everything, so its results are
    /// cross-channel and cross-author by construction. A hit that carries
    /// neither is the least actionable output of the three commands: there is
    /// nothing to reply to and nothing to open.
    #[test]
    fn compact_feed_rows_carry_author_and_channel() {
        let out = format_events(&feed_events(), &OutputFormat::Compact);
        let rows: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid json");
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row["pubkey"], AUTHOR, "a feed hit with no author");
        }
        assert_eq!(rows[0]["channel"], CHANNEL_A);
        assert_eq!(
            rows[1]["channel"], CHANNEL_B,
            "distinct channels must survive"
        );
    }

    /// The projection is shared with `messages` rather than copied. The two
    /// copies had already drifted once, which is what this test guards.
    #[test]
    fn feed_and_messages_agree_on_the_compact_shape() {
        let out = format_events(&feed_events(), &OutputFormat::Compact);
        let rows: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid json");
        let keys: Vec<&str> = rows[0]
            .as_object()
            .expect("object")
            .keys()
            .map(|k| k.as_str())
            .collect();
        for expected in ["id", "pubkey", "channel", "content", "created_at"] {
            assert!(
                keys.contains(&expected),
                "compact row lost `{expected}`: {keys:?}"
            );
        }
    }
}
