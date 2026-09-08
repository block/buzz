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
///
/// Mirrors desktop `customEmojiFromTags` (`desktop/src/shared/api/customEmoji.ts`):
/// - Shortcode is canonicalized via `buzz_sdk::normalize_custom_emoji_shortcode`
///   (trim whitespace/colons, validate charset/length, lowercase). The relay
///   validates with the same fn at ingest but stores the original signed tag,
///   so a relay-valid stored key like `"  :WAVE:  "` must be normalized here or
///   it will never resolve against `scan_shortcodes` output. Malformed tags
///   (where normalization returns `Err`) are skipped.
/// - Entries with a missing or empty URL are skipped.
/// - Within one event the first occurrence of a normalized shortcode wins;
///   later duplicates are dropped.
fn emoji_tags_of(event: &serde_json::Value) -> Vec<EmojiEntry> {
    let Some(tags) = event.get("tags").and_then(|v| v.as_array()) else {
        return vec![];
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for tag in tags {
        let Some(parts) = tag.as_array() else {
            continue;
        };
        if parts.first().and_then(|v| v.as_str()) != Some("emoji") {
            continue;
        }
        let (Some(raw_shortcode), Some(url)) = (
            parts.get(1).and_then(|v| v.as_str()),
            parts.get(2).and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        // Skip entries with empty URL — they are malformed and would silently
        // produce tags without a resolvable image.
        if url.is_empty() {
            continue;
        }
        // Canonicalize via the SDK normalizer: trim whitespace/colons, validate
        // charset/length, lowercase.  Relay validates with this same fn at
        // ingest but stores the original tag — so a relay-valid key like
        // "  :WAVE:  " must map to "wave" here or it will never resolve against
        // scan_shortcodes output.  Skip on Err (malformed tag).
        let shortcode = match buzz_sdk::normalize_custom_emoji_shortcode(raw_shortcode) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // First occurrence within this event wins; later duplicates are dropped.
        if seen.insert(shortcode.clone()) {
            out.push(EmojiEntry {
                shortcode,
                url: url.to_string(),
            });
        }
    }
    out
}

/// Union every member's kind:30030 set, collapsed to one entry per shortcode.
/// The most recently published set (`created_at`) wins; equal timestamps
/// tie-break to the lexicographically-smallest URL. Deterministic and
/// fetch-order-independent. Sorted by shortcode.
fn union_custom_emoji(events: &[serde_json::Value]) -> Vec<EmojiEntry> {
    let mut by_shortcode: std::collections::HashMap<String, (String, i64)> =
        std::collections::HashMap::new();
    for event in events {
        let created_at = event
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        for entry in emoji_tags_of(event) {
            match by_shortcode.get(&entry.shortcode) {
                Some((url, at)) if *at > created_at || (*at == created_at && *url <= entry.url) => {
                }
                _ => {
                    by_shortcode.insert(entry.shortcode, (entry.url, created_at));
                }
            }
        }
    }
    let mut out: Vec<EmojiEntry> = by_shortcode
        .into_iter()
        .map(|(shortcode, (url, _))| EmojiEntry { shortcode, url })
        .collect();
    out.sort_by(|a, b| a.shortcode.cmp(&b.shortcode));
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

/// Scan `content` for `:shortcode:` patterns, mirroring the desktop's
/// `customEmojiTags.ts` algorithm exactly:
///
/// - Pattern: `:([a-z0-9_-]+):` (case-insensitive; canonical lowercase emitted)
/// - One tag per distinct first-appearing shortcode
/// - Unknown shortcodes silently ignored
///
/// Returns NIP-30 `["emoji", shortcode, url]` tag vectors for every
/// shortcode that resolves in the workspace palette.  Returns an empty `Vec`
/// without a relay round-trip if no candidates appear in the content.
///
/// Callers must pre-screen with `content.contains(':')` to skip this
/// function entirely for the common case of plain content.
pub async fn resolve_emoji_tags_for_content(
    client: &BuzzClient,
    content: &str,
) -> Result<Vec<Vec<String>>, CliError> {
    let candidates = scan_shortcodes(content);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch workspace palette (union of all members' kind:30030 sets).
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_EMOJI_SET],
        "#d": [CUSTOM_EMOJI_SET_D_TAG],
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse emoji set query: {e}")))?;
    let palette = union_custom_emoji(&events);
    let url_by_shortcode: std::collections::HashMap<&str, &str> = palette
        .iter()
        .map(|e| (e.shortcode.as_str(), e.url.as_str()))
        .collect();

    let tags: Vec<Vec<String>> = candidates
        .iter()
        .filter_map(|sc| {
            url_by_shortcode
                .get(sc.as_str())
                .map(|url| vec!["emoji".to_string(), sc.clone(), url.to_string()])
        })
        .collect();

    Ok(tags)
}

/// Collect candidate shortcodes from `content` without a regex dependency.
///
/// Implements `:([a-z0-9_-]+):` (applied case-insensitively with lowercase
/// normalization) using a hand-rolled single-pass scanner.  Each distinct
/// shortcode appears exactly once in first-appearance order.
pub(crate) fn scan_shortcodes(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < len {
        if bytes[i] != b':' {
            i += 1;
            continue;
        }
        // Found opening `:`.  Scan forward for valid shortcode chars.
        let start = i + 1;
        let mut j = start;
        while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-')
        {
            j += 1;
        }
        // Require at least one char and a closing `:`.
        if j > start && j < len && bytes[j] == b':' {
            // SAFETY: `content` is valid UTF-8 and the slice covers only ASCII.
            let sc = content[start..j].to_lowercase();
            if seen.insert(sc.clone()) {
                out.push(sc);
            }
            // Advance past the closing `:` so overlapping patterns like `:a::b:`
            // are handled correctly (`:a:` consumed, next scan starts at `:`).
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

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
    fn union_latest_set_wins_per_shortcode() {
        let events = vec![
            serde_json::json!({
                "created_at": 100,
                "tags": [
                    ["d", "buzz:custom-emoji"],
                    ["emoji", "zort", "https://example.com/zort.png"],
                    ["emoji", "narf", "https://example.com/narf.png"]
                ]
            }),
            serde_json::json!({
                "created_at": 200,
                "tags": [
                    ["d", "buzz:custom-emoji"],
                    // newer set claims zort with a different url — newer wins
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
                ("zort", "https://example.com/zort2.png"),
            ]
        );
        // Order-independence: reversed input yields the identical palette.
        let reversed: Vec<_> = events.into_iter().rev().collect();
        let emojis_rev = union_custom_emoji(&reversed);
        let pairs_rev: Vec<(&str, &str)> = emojis_rev
            .iter()
            .map(|e| (e.shortcode.as_str(), e.url.as_str()))
            .collect();
        assert_eq!(pairs, pairs_rev);
    }

    #[test]
    fn union_equal_timestamps_tie_break_to_smallest_url() {
        let events = vec![
            serde_json::json!({
                "created_at": 100,
                "tags": [["emoji", "zort", "https://example.com/zort2.png"]]
            }),
            serde_json::json!({
                "created_at": 100,
                "tags": [["emoji", "zort", "https://example.com/zort.png"]]
            }),
        ];
        let emojis = union_custom_emoji(&events);
        assert_eq!(emojis.len(), 1);
        assert_eq!(emojis[0].shortcode, "zort");
        assert_eq!(emojis[0].url, "https://example.com/zort.png");
    }

    // ── scan_shortcodes ──────────────────────────────────────────────────────

    // ── emoji_tags_of — normalization and dedup ──────────────────────────────

    #[test]
    fn emoji_tags_of_normalizes_uppercase_shortcode_to_lowercase() {
        // Relay stores the original case; scanner always lowercases; so a
        // stored "WAVE" must map to "wave" for resolution to work.  Also
        // covers relay-valid keys with surrounding whitespace/colons.
        let event = serde_json::json!({
            "created_at": 100,
            "tags": [
                ["emoji", "WAVE", "https://example.com/wave.png"],
                ["emoji", "  :SweatBlob:  ", "https://example.com/sweatblob.gif"],
            ]
        });
        let entries = emoji_tags_of(&event);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].shortcode, "wave");
        assert_eq!(entries[1].shortcode, "sweatblob");
    }

    #[test]
    fn emoji_tags_of_skips_empty_url() {
        // An entry with a missing or empty URL is malformed; it must be
        // dropped so palette lookups never return an unusable image URL.
        let event = serde_json::json!({
            "created_at": 100,
            "tags": [
                ["emoji", "good", "https://example.com/good.png"],
                ["emoji", "bad", ""],
                ["emoji", "alsobad"],  // missing url field entirely
            ]
        });
        let entries = emoji_tags_of(&event);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].shortcode, "good");
    }

    #[test]
    fn emoji_tags_of_first_occurrence_wins_within_event() {
        // Within one event the first occurrence of a (normalized) shortcode
        // wins; a later duplicate tag for the same shortcode is dropped.
        let event = serde_json::json!({
            "created_at": 100,
            "tags": [
                ["emoji", "wave", "https://example.com/wave-first.png"],
                ["emoji", "wave", "https://example.com/wave-second.png"],
                ["emoji", "WAVE", "https://example.com/wave-uppercase.png"],
            ]
        });
        let entries = emoji_tags_of(&event);
        assert_eq!(
            entries.len(),
            1,
            "all three normalize to 'wave'; only first kept"
        );
        assert_eq!(entries[0].url, "https://example.com/wave-first.png");
    }

    #[test]
    fn scan_finds_basic_shortcode() {
        assert_eq!(scan_shortcodes(":wave:"), vec!["wave"]);
    }

    #[test]
    fn scan_finds_multiple_shortcodes_in_order() {
        let result = scan_shortcodes(":wave: hello :party_parrot: world :tada:");
        assert_eq!(result, vec!["wave", "party_parrot", "tada"]);
    }

    #[test]
    fn scan_deduplicates_shortcodes() {
        let result = scan_shortcodes(":wave: :wave: :wave:");
        assert_eq!(result, vec!["wave"]);
    }

    #[test]
    fn scan_normalizes_to_lowercase() {
        let result = scan_shortcodes(":WAVE: :Wave:");
        assert_eq!(result, vec!["wave"]);
    }

    #[test]
    fn scan_ignores_invalid_chars_in_shortcode() {
        // Spaces inside are not valid shortcode chars
        let result = scan_shortcodes(":hello world:");
        assert!(result.is_empty());
    }

    #[test]
    fn scan_empty_colons_not_matched() {
        // "::" has zero chars between — must not match
        assert!(scan_shortcodes("::").is_empty());
    }

    #[test]
    fn scan_no_candidates_in_plain_content() {
        assert!(scan_shortcodes("Hello world, no emoji here").is_empty());
    }

    #[test]
    fn scan_handles_adjacent_shortcodes() {
        // ":a::b:" — `:a:` consumed, then `:b:` starts at `:`
        let result = scan_shortcodes(":a::b:");
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn scan_allows_hyphens_and_underscores() {
        let result = scan_shortcodes(":party-parrot: :sweat_blob:");
        assert_eq!(result, vec!["party-parrot", "sweat_blob"]);
    }

    // ── resolve_emoji_tags_for_content — send-path palette seam ─────────────
    //
    // These tests drive the production `resolve_emoji_tags_for_content` through
    // a real `BuzzClient` against an axum fake `/query` server.  They verify
    // the full chain: scan → palette fetch → tag assembly.

    use crate::client::BuzzClient;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use nostr::Keys;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    fn test_client(base_url: &str) -> BuzzClient {
        BuzzClient::new(base_url.to_string(), Keys::generate(), None, None).unwrap()
    }

    /// Fake relay: serves a `/query` endpoint returning the given JSON body,
    /// and records how many times it was called.
    async fn fake_query_server(response_body: String) -> (String, Arc<Mutex<u32>>) {
        let call_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        type S = (Arc<Mutex<u32>>, String);
        let state: S = (call_count.clone(), response_body);

        let app = Router::new()
            .route(
                "/query",
                post(
                    |State((count, body)): State<S>, _headers: HeaderMap, _req: Bytes| async move {
                        *count.lock().unwrap() += 1;
                        (StatusCode::OK, [("content-type", "application/json")], body)
                    },
                ),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), call_count)
    }

    /// Palette response: two custom emoji — `wave` and `sweatblob`.
    fn palette_response() -> String {
        serde_json::json!([{
            "created_at": 100,
            "tags": [
                ["d", "buzz:custom-emoji"],
                ["emoji", "wave", "https://cdn.example.com/wave.png"],
                ["emoji", "sweatblob", "https://cdn.example.com/sweatblob.gif"]
            ]
        }])
        .to_string()
    }

    #[tokio::test]
    async fn resolve_tags_known_shortcode_returns_correct_tag() {
        let (url, _calls) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        let tags = resolve_emoji_tags_for_content(&client, "hello :wave:")
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0],
            vec!["emoji", "wave", "https://cdn.example.com/wave.png"]
        );
    }

    #[tokio::test]
    async fn resolve_tags_unknown_shortcode_is_filtered_out() {
        let (url, _calls) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        // :notarealemoji: is not in the palette — must produce no tags.
        let tags = resolve_emoji_tags_for_content(&client, ":notarealemoji:")
            .await
            .unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn resolve_tags_deduplicates_repeated_shortcode() {
        let (url, _calls) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        // `:wave:` appears twice; output must have exactly one tag for it.
        let tags = resolve_emoji_tags_for_content(&client, ":wave: and :wave: again")
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0][1], "wave");
    }

    #[tokio::test]
    async fn resolve_tags_first_appearance_order() {
        let (url, _calls) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        // `:sweatblob:` before `:wave:` — tags must appear in that order.
        let tags = resolve_emoji_tags_for_content(&client, ":sweatblob: :wave:")
            .await
            .unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0][1], "sweatblob");
        assert_eq!(tags[1][1], "wave");
    }

    #[tokio::test]
    async fn resolve_tags_case_insensitive_match_emits_lowercase() {
        let (url, _calls) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        // `:WAVE:` must resolve to the lowercase `wave` tag.
        let tags = resolve_emoji_tags_for_content(&client, ":WAVE:")
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0][1], "wave",
            "canonical tag shortcode must be lowercase"
        );
    }

    #[tokio::test]
    async fn resolve_tags_no_colon_content_skips_palette_query() {
        let (url, call_count) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        // Content with no `:` must return empty tags with ZERO relay queries.
        let tags = resolve_emoji_tags_for_content(&client, "Hello world, no colons here")
            .await
            .unwrap();
        assert!(tags.is_empty());
        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "must not query the palette when content has no colon"
        );
    }

    #[tokio::test]
    async fn resolve_tags_unknown_only_content_still_queries_once() {
        let (url, call_count) = fake_query_server(palette_response()).await;
        let client = test_client(&url);
        // Content has `:` but the shortcode is not in the palette.
        // One palette query should occur (candidates are non-empty), zero tags returned.
        let tags = resolve_emoji_tags_for_content(&client, ":notreal:")
            .await
            .unwrap();
        assert!(tags.is_empty());
        assert_eq!(
            *call_count.lock().unwrap(),
            1,
            "must query palette once even when no shortcodes resolve"
        );
    }

    #[tokio::test]
    async fn resolve_tags_non_canonical_palette_key_resolves() {
        // The relay validates shortcodes via normalize_custom_emoji_shortcode but
        // stores the original signed tag.  A relay-valid stored key like
        // "  :WAVE:  " must resolve when content contains `:wave:`.
        // This is the production-resolver regression that proves emoji_tags_of
        // uses the SDK normalizer rather than a plain lowercase conversion.
        let non_canonical_palette = serde_json::json!([{
            "created_at": 100,
            "tags": [
                ["d", "buzz:custom-emoji"],
                // Relay-valid but non-canonical: whitespace + surrounding colons + uppercase.
                ["emoji", "  :WAVE:  ", "https://cdn.example.com/wave.png"],
            ]
        }])
        .to_string();
        let (url, _calls) = fake_query_server(non_canonical_palette).await;
        let client = test_client(&url);
        let tags = resolve_emoji_tags_for_content(&client, "hello :wave:")
            .await
            .unwrap();
        assert_eq!(
            tags.len(),
            1,
            "non-canonical palette key must resolve; got tags: {tags:?}"
        );
        assert_eq!(
            tags[0],
            vec!["emoji", "wave", "https://cdn.example.com/wave.png"],
            "resolved tag must use the canonical lowercase shortcode"
        );
    }
}
