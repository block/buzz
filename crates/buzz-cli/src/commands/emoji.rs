use std::io::Read;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use buzz_sdk::CustomEmoji;

/// d-tag for a member's own custom emoji set (kind:30030). Mirrors the SDK
/// constant; the workspace palette is the union of every member's own set.
const CUSTOM_EMOJI_SET_D_TAG: &str = buzz_sdk::CUSTOM_EMOJI_SET_D_TAG;

/// Custom emoji entry in CLI output.
#[derive(Debug, serde::Serialize)]
struct EmojiEntry {
    shortcode: String,
    url: String,
}

/// Parse `["emoji", shortcode, url]` tags from one event into entries.
fn emoji_tags_of(event: &serde_json::Value) -> Vec<EmojiEntry> {
    let Some(tags) = event.get("tags").and_then(|v| v.as_array()) else {
        return vec![];
    };
    let mut out = Vec::new();
    for tag in tags {
        let Some(parts) = tag.as_array() else {
            continue;
        };
        if parts.first().and_then(|v| v.as_str()) != Some("emoji") {
            continue;
        }
        let (Some(shortcode), Some(url)) = (
            parts.get(1).and_then(|v| v.as_str()),
            parts.get(2).and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        out.push(EmojiEntry {
            shortcode: shortcode.to_string(),
            url: url.to_string(),
        });
    }
    out
}

/// Union every member's kind:30030 set, deduped by `(shortcode, url)`.
/// Stable, sorted by shortcode then url, so identical input yields identical output.
fn union_custom_emoji(events: &[serde_json::Value]) -> Vec<EmojiEntry> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<EmojiEntry> = Vec::new();
    for event in events {
        for entry in emoji_tags_of(event) {
            if seen.insert((entry.shortcode.clone(), entry.url.clone())) {
                out.push(entry);
            }
        }
    }
    out.sort_by(|a, b| a.shortcode.cmp(&b.shortcode).then(a.url.cmp(&b.url)));
    out
}

/// List the workspace custom emoji palette: the union of every member's
/// own kind:30030 set (d=`buzz:custom-emoji`).
async fn cmd_list(client: &BuzzClient) -> Result<(), CliError> {
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_EMOJI_SET],
        "#d": [CUSTOM_EMOJI_SET_D_TAG],
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse emoji set query: {e}")))?;
    let emojis = union_custom_emoji(&events);
    let output = serde_json::json!({ "emojis": emojis });
    println!("{}", serde_json::to_string(&output).unwrap_or_default());
    Ok(())
}

/// Fetch the caller's own current custom emoji set (latest kind:30030 under
/// the d-tag, authored by the caller). Empty when none published yet.
async fn fetch_own_emoji(client: &BuzzClient) -> Result<Vec<CustomEmoji>, CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_EMOJI_SET],
        "#d": [CUSTOM_EMOJI_SET_D_TAG],
        "authors": [me],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse own emoji set: {e}")))?;
    // The relay keeps only the latest per (pubkey, d_tag), but be defensive.
    let Some(event) = events.last() else {
        return Ok(vec![]);
    };
    Ok(emoji_tags_of(event)
        .into_iter()
        .map(|e| CustomEmoji {
            shortcode: e.shortcode,
            url: e.url,
        })
        .collect())
}

/// Publish the caller's own (replaced) kind:30030 set, signed as the caller.
async fn publish_own_set(client: &BuzzClient, emojis: &[CustomEmoji]) -> Result<(), CliError> {
    let builder = buzz_sdk::build_custom_emoji_set(emojis)
        .map_err(|e| CliError::Other(format!("build_custom_emoji_set failed: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Add/update a shortcode in the caller's own set (read-modify-write).
async fn cmd_set(client: &BuzzClient, shortcode: &str, url: &str) -> Result<(), CliError> {
    let normalized = buzz_sdk::normalize_custom_emoji_shortcode(shortcode)
        .map_err(|e| CliError::Other(format!("invalid shortcode: {e}")))?;
    let mut emojis = fetch_own_emoji(client).await?;
    emojis.retain(|e| e.shortcode != normalized);
    emojis.push(CustomEmoji {
        shortcode: normalized,
        url: url.to_string(),
    });
    publish_own_set(client, &emojis).await
}

/// Remove a shortcode from the caller's own set (read-modify-write).
async fn cmd_rm(client: &BuzzClient, shortcode: &str) -> Result<(), CliError> {
    let normalized = buzz_sdk::normalize_custom_emoji_shortcode(shortcode)
        .map_err(|e| CliError::Other(format!("invalid shortcode: {e}")))?;
    let mut emojis = fetch_own_emoji(client).await?;
    let before = emojis.len();
    emojis.retain(|e| e.shortcode != normalized);
    if emojis.len() == before {
        // Nothing to remove; avoid republishing an unchanged set.
        println!(
            "{}",
            serde_json::json!({"accepted": true, "message": "not present"})
        );
        return Ok(());
    }
    publish_own_set(client, &emojis).await
}

/// 10 MiB — a safety rail against runaway producers. An emoji manifest will
/// never approach this size in practice.
const STDIN_MAX_BYTES: u64 = 10_000_000;

/// Read from a file path or stdin. Returns `CliError::Usage` on empty stdin,
/// `CliError::Other` on I/O failure.
fn read_source(file: Option<&str>) -> Result<String, CliError> {
    match file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| CliError::Other(format!("failed to read file '{path}': {e}"))),
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .take(STDIN_MAX_BYTES)
                .read_to_string(&mut buf)
                .map_err(|e| CliError::Other(format!("stdin read failed: {e}")))?;
            if buf.is_empty() {
                return Err(CliError::Usage(
                    "no input: provide --file or pipe JSON to stdin".into(),
                ));
            }
            Ok(buf)
        }
    }
}

/// Write to a file path or stdout.
fn write_output(output: &str, file: Option<&str>) -> Result<(), CliError> {
    match file {
        Some(path) => std::fs::write(path, output)
            .map_err(|e| CliError::Other(format!("failed to write file '{path}': {e}"))),
        None => {
            println!("{output}");
            Ok(())
        }
    }
}

/// Export custom emojis to stdout or a file.
async fn cmd_export(
    client: &BuzzClient,
    file: Option<&str>,
    scope: &crate::EmojiScope,
) -> Result<(), CliError> {
    let entries: Vec<EmojiEntry> = match scope {
        crate::EmojiScope::Own => {
            let mut entries: Vec<EmojiEntry> = fetch_own_emoji(client)
                .await?
                .into_iter()
                .map(|e| EmojiEntry {
                    shortcode: e.shortcode,
                    url: e.url,
                })
                .collect();
            // Sort to match union_custom_emoji output order so repeated
            // export | import --replace cycles are stable.
            entries.sort_by(|a, b| a.shortcode.cmp(&b.shortcode).then(a.url.cmp(&b.url)));
            entries
        }
        crate::EmojiScope::Workspace => {
            let filter = serde_json::json!({
                "kinds": [buzz_sdk::kind::KIND_EMOJI_SET],
                "#d": [CUSTOM_EMOJI_SET_D_TAG],
            });
            let raw = client.query(&filter).await?;
            let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
                .map_err(|e| CliError::Other(format!("failed to parse emoji set query: {e}")))?;
            union_custom_emoji(&events)
        }
    };
    let output = serde_json::to_string(&serde_json::json!({ "emojis": entries }))
        .map_err(|e| CliError::Other(format!("serialization failed: {e}")))?;
    write_output(&output, file)
}

/// Import custom emojis from stdin or a file into the caller's own set.
async fn cmd_import(
    client: &BuzzClient,
    file: Option<&str>,
    replace: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    // 1. Read raw JSON
    let raw = read_source(file)?;

    // 2. Parse and extract ["emojis"] array
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| CliError::Usage(format!("invalid JSON: {e}")))?;
    let arr = parsed
        .get("emojis")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CliError::Usage("input must be a JSON object with an \"emojis\" array".into())
        })?;

    // 3–4. Parse each element and normalize shortcodes
    let mut import_entries: Vec<CustomEmoji> = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let shortcode = item
            .get("shortcode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CliError::Usage(format!("emojis[{i}]: missing \"shortcode\" field")))?;
        let url = item
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CliError::Usage(format!("emojis[{i}]: missing \"url\" field")))?;
        let normalized = buzz_sdk::normalize_custom_emoji_shortcode(shortcode)
            .map_err(|e| CliError::Usage(format!("emojis[{i}]: invalid shortcode: {e}")))?;
        import_entries.push(CustomEmoji {
            shortcode: normalized,
            url: url.to_string(),
        });
    }

    // 5. Deduplicate within the import batch (first occurrence wins)
    let mut seen = std::collections::HashSet::new();
    import_entries.retain(|e| seen.insert(e.shortcode.clone()));

    // 6. Build final set
    let final_set: Vec<CustomEmoji> = if replace {
        import_entries
    } else {
        let mut existing = fetch_own_emoji(client).await?;
        let existing_shortcodes: std::collections::HashSet<String> =
            existing.iter().map(|e| e.shortcode.clone()).collect();
        for entry in import_entries {
            if !existing_shortcodes.contains(&entry.shortcode) {
                existing.push(entry);
            }
        }
        existing
    };

    // 7. Dry-run: print final set to stdout, warn to stderr
    if dry_run {
        let entries: Vec<EmojiEntry> = final_set
            .iter()
            .map(|e| EmojiEntry {
                shortcode: e.shortcode.clone(),
                url: e.url.clone(),
            })
            .collect();
        let output = serde_json::to_string(&serde_json::json!({ "emojis": entries }))
            .map_err(|e| CliError::Other(format!("serialization failed: {e}")))?;
        println!("{output}");
        eprintln!("(dry run — not published)");
        return Ok(());
    }

    // 8. Publish
    publish_own_set(client, &final_set).await
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn dispatch(cmd: crate::EmojiCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::EmojiCmd;
    match cmd {
        EmojiCmd::List => cmd_list(client).await,
        EmojiCmd::Set { shortcode, url } => cmd_set(client, &shortcode, &url).await,
        EmojiCmd::Rm { shortcode } => cmd_rm(client, &shortcode).await,
        EmojiCmd::Export { file, scope } => cmd_export(client, file.as_deref(), &scope).await,
        EmojiCmd::Import {
            file,
            replace,
            dry_run,
        } => cmd_import(client, file.as_deref(), replace, dry_run).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_dedups_by_shortcode_and_url_across_members() {
        let events = vec![
            serde_json::json!({
                "tags": [
                    ["d", "buzz:custom-emoji"],
                    ["emoji", "zort", "https://example.com/zort.png"],
                    ["emoji", "narf", "https://example.com/narf.png"]
                ]
            }),
            serde_json::json!({
                "tags": [
                    ["d", "buzz:custom-emoji"],
                    // exact duplicate (same shortcode+url) — collapses
                    ["emoji", "narf", "https://example.com/narf.png"],
                    // same shortcode, different url — both kept (distinct pair)
                    ["emoji", "zort", "https://example.com/zort2.png"]
                ]
            }),
        ];
        let emojis = union_custom_emoji(&events);
        let pairs: Vec<(&str, &str)> = emojis
            .iter()
            .map(|e| (e.shortcode.as_str(), e.url.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("narf", "https://example.com/narf.png"),
                ("zort", "https://example.com/zort.png"),
                ("zort", "https://example.com/zort2.png"),
            ]
        );
    }
}
