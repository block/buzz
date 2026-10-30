pub mod migrate;

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

/// Collect `["emoji", shortcode, url]` tags for the `:shortcode:` patterns in
/// `content`, mirroring the desktop's `customEmojiTags.ts` algorithm:
///
/// - Pattern: `:([a-z0-9_-]+):` (case-insensitive; canonical lowercase emitted)
/// - One tag per distinct first-appearing shortcode
/// - Unknown shortcodes silently ignored
///
/// Lookup order: one batched `#d` query for catalog entries (kind:30180),
/// then, only when some name is still unresolved, the legacy workspace palette
/// (kind:30030 union). Messages whose names all resolve in the catalog never
/// fetch the legacy palette. A catalog entry wins over a legacy entry for the
/// same name.
///
/// Returns an empty `Vec` without a relay round-trip if no candidates appear.
/// A failed lookup returns `Err` rather than "no emoji"; the message-send
/// caller degrades that to a stderr warning and sends without emoji tags.
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

    let catalog_names: Vec<String> = candidates
        .iter()
        .filter(|sc| buzz_sdk::normalize_custom_emoji_shortcode(sc).is_ok())
        .cloned()
        .collect();
    let mut url_by_shortcode: std::collections::HashMap<String, String> =
        fetch_catalog_by_names(client, &catalog_names)
            .await?
            .into_iter()
            .map(|e| (e.shortcode, e.url))
            .collect();

    if candidates
        .iter()
        .any(|sc| !url_by_shortcode.contains_key(sc))
    {
        for entry in fetch_legacy_palette(client).await? {
            url_by_shortcode.entry(entry.shortcode).or_insert(entry.url);
        }
    }

    Ok(candidates
        .iter()
        .filter_map(|sc| {
            url_by_shortcode
                .get(sc)
                .map(|url| vec!["emoji".to_string(), sc.clone(), url.clone()])
        })
        .collect())
}

/// Fetch the legacy workspace palette (union of every member's kind:30030 set).
async fn fetch_legacy_palette(client: &BuzzClient) -> Result<Vec<EmojiEntry>, CliError> {
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_EMOJI_SET],
        "#d": [CUSTOM_EMOJI_SET_D_TAG],
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse emoji set query: {e}")))?;
    Ok(union_custom_emoji(&events))
}

/// Names per `#d` filter. Keeps each query bounded.
const CATALOG_NAMES_PER_FILTER: usize = 50;
/// Events requested per `#d` filter. Reaching it is reported as truncation.
const CATALOG_EVENTS_PER_FILTER: usize = 500;
/// Shortest accepted search prefix, in characters.
const CATALOG_MIN_PREFIX_CHARS: usize = 2;
const CATALOG_MAX_SEARCH_LIMIT: u32 = 50;

/// One catalog emoji (kind:30180) in CLI output.
#[derive(Debug, serde::Serialize, PartialEq, Eq)]
struct CatalogEntry {
    shortcode: String,
    url: String,
    author: String,
    event_id: String,
    created_at: i64,
    /// Other authors that published the same shortcode; only the newest
    /// event is returned.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    other_authors: Vec<String>,
}

fn catalog_entry_of(event: &serde_json::Value) -> Option<CatalogEntry> {
    if event.get("kind").and_then(|v| v.as_u64()) != Some(buzz_sdk::kind::KIND_EMOJI_ENTRY as u64) {
        return None;
    }
    let mut d = None;
    let mut emoji = None;
    for tag in event.get("tags")?.as_array()? {
        let parts = tag.as_array()?;
        match parts.first().and_then(|v| v.as_str()) {
            Some("d") if d.is_none() => d = parts.get(1)?.as_str(),
            Some("emoji") if emoji.is_none() => {
                emoji = Some((parts.get(1)?.as_str()?, parts.get(2)?.as_str()?));
            }
            _ => {}
        }
    }
    let (shortcode, url) = emoji?;
    let shortcode = buzz_sdk::normalize_custom_emoji_shortcode(shortcode).ok()?;
    if d? != shortcode || buzz_sdk::check_custom_emoji_url(url).is_err() {
        return None;
    }
    Some(CatalogEntry {
        shortcode,
        url: url.to_string(),
        author: event.get("pubkey")?.as_str()?.to_string(),
        event_id: event.get("id")?.as_str()?.to_string(),
        created_at: event.get("created_at")?.as_i64()?,
        other_authors: Vec::new(),
    })
}

/// Collapse kind:30180 events to one entry per shortcode: the newest
/// `created_at` wins (smallest event id on a tie), and authors of the losing
/// events are listed in `other_authors`. With `wanted`, entries outside the
/// set are dropped. Malformed events are skipped. Sorted by shortcode.
fn collapse_catalog(
    events: &[serde_json::Value],
    wanted: Option<&std::collections::HashSet<&str>>,
) -> Vec<CatalogEntry> {
    let mut by_shortcode: std::collections::HashMap<String, Vec<CatalogEntry>> =
        std::collections::HashMap::new();
    for entry in events.iter().filter_map(catalog_entry_of) {
        if wanted.is_some_and(|w| !w.contains(entry.shortcode.as_str())) {
            continue;
        }
        by_shortcode
            .entry(entry.shortcode.clone())
            .or_default()
            .push(entry);
    }
    let mut out: Vec<CatalogEntry> = by_shortcode
        .into_values()
        .map(|mut group| {
            group.sort_by(|a, b| {
                b.created_at
                    .cmp(&a.created_at)
                    .then_with(|| a.event_id.cmp(&b.event_id))
            });
            let mut winner = group.remove(0);
            let mut others: Vec<String> = group
                .into_iter()
                .map(|e| e.author)
                .filter(|a| *a != winner.author)
                .collect();
            others.sort();
            others.dedup();
            winner.other_authors = others;
            winner
        })
        .collect();
    out.sort_by(|a, b| a.shortcode.cmp(&b.shortcode));
    out
}

fn parse_events(raw: &str, what: &str) -> Result<Vec<serde_json::Value>, CliError> {
    serde_json::from_str(raw).map_err(|e| CliError::Other(format!("failed to parse {what}: {e}")))
}

/// Exact `#d` lookup of catalog entries, `CATALOG_NAMES_PER_FILTER` names per
/// query. `names` must already be normalized. A filter that reaches
/// `CATALOG_EVENTS_PER_FILTER` events is an error, not a partial answer.
async fn fetch_catalog_by_names(
    client: &BuzzClient,
    names: &[String],
) -> Result<Vec<CatalogEntry>, CliError> {
    let mut events = Vec::new();
    for chunk in names.chunks(CATALOG_NAMES_PER_FILTER) {
        let filter = serde_json::json!({
            "kinds": [buzz_sdk::kind::KIND_EMOJI_ENTRY],
            "#d": chunk,
            "limit": CATALOG_EVENTS_PER_FILTER,
        });
        let raw = client.query(&filter).await?;
        let page = parse_events(&raw, "emoji entry query")?;
        if page.len() >= CATALOG_EVENTS_PER_FILTER {
            return Err(CliError::Other(format!(
                "emoji entry lookup returned {} events and may be truncated; retry with fewer names",
                page.len()
            )));
        }
        events.extend(page);
    }
    let wanted: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    Ok(collapse_catalog(&events, Some(&wanted)))
}

/// Resolve exact shortcodes against the catalog. Returns the found entries and
/// the normalized names with no entry, in input order.
async fn resolve_catalog(
    client: &BuzzClient,
    names: &[String],
) -> Result<(Vec<CatalogEntry>, Vec<String>), CliError> {
    let mut normalized = Vec::with_capacity(names.len());
    for name in names {
        let n = buzz_sdk::normalize_custom_emoji_shortcode(name)
            .map_err(|e| CliError::Usage(format!("invalid shortcode '{name}': {e}")))?;
        if !normalized.contains(&n) {
            normalized.push(n);
        }
    }
    let found = fetch_catalog_by_names(client, &normalized).await?;
    let missing = normalized
        .into_iter()
        .filter(|n| !found.iter().any(|e| e.shortcode == *n))
        .collect();
    Ok((found, missing))
}

/// Prefix search over catalog entries via the relay's `search_mode: prefix`.
/// The relay indexes the shortcode with `_` and `-` as spaces, so the prefix
/// is mapped the same way.
async fn search_catalog(
    client: &BuzzClient,
    prefix: &str,
    limit: u32,
) -> Result<Vec<CatalogEntry>, CliError> {
    let trimmed = prefix.trim().trim_matches(':');
    if trimmed.chars().count() < CATALOG_MIN_PREFIX_CHARS {
        return Err(CliError::Usage(format!(
            "search prefix must be at least {CATALOG_MIN_PREFIX_CHARS} characters"
        )));
    }
    if !(1..=CATALOG_MAX_SEARCH_LIMIT).contains(&limit) {
        return Err(CliError::Usage(format!(
            "--limit must be between 1 and {CATALOG_MAX_SEARCH_LIMIT}"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CliError::Usage(
            "search prefix may only contain ASCII letters, digits, hyphens, and underscores".into(),
        ));
    }
    let search = trimmed.to_ascii_lowercase().replace(['_', '-'], " ");
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_EMOJI_ENTRY],
        "search": search,
        "search_mode": "prefix",
        "limit": limit,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw, "emoji search")?;
    Ok(collapse_catalog(&events, None))
}

async fn cmd_search(client: &BuzzClient, prefix: &str, limit: u32) -> Result<(), CliError> {
    let entries = search_catalog(client, prefix, limit).await?;
    print_json(&entries)
}

async fn cmd_resolve(client: &BuzzClient, names: &[String]) -> Result<(), CliError> {
    let (found, missing) = resolve_catalog(client, names).await?;
    print_json(&serde_json::json!({ "found": found, "missing": missing }))
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), CliError> {
    let out = serde_json::to_string(value)
        .map_err(|e| CliError::Other(format!("serialization failed: {e}")))?;
    println!("{out}");
    Ok(())
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
        EmojiCmd::Search { prefix, limit } => cmd_search(client, &prefix, limit).await,
        EmojiCmd::Resolve { names } => cmd_resolve(client, &names).await,
        EmojiCmd::Migrate(sub) => migrate::dispatch(sub, client).await,
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

    // ── resolve_emoji_tags_for_content — send-path lookup seam ──────────────
    //
    // These tests drive the production `resolve_emoji_tags_for_content` and
    // the catalog commands through a real `BuzzClient` against an axum fake
    // `/query` server that routes by the filter's first kind and records every
    // request.

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

    type Requests = Arc<Mutex<Vec<serde_json::Value>>>;

    /// Status and body for one kind of query.
    #[derive(Clone)]
    struct Reply(u16, String);

    impl Reply {
        fn ok(body: String) -> Self {
            Reply(200, body)
        }
        fn empty() -> Self {
            Reply(200, "[]".into())
        }
    }

    /// Fake relay: kind:30180 filters get `entries`, everything else gets
    /// `palette`. Returns the base URL and the recorded first filters.
    async fn fake_relay(entries: Reply, palette: Reply) -> (String, Requests) {
        let requests: Requests = Arc::new(Mutex::new(Vec::new()));
        type S = (Requests, Reply, Reply);
        let app =
            Router::new()
                .route(
                    "/query",
                    post(
                        |State((log, entries, palette)): State<S>,
                         _headers: HeaderMap,
                         req: Bytes| async move {
                            let filters: Vec<serde_json::Value> =
                                serde_json::from_slice(&req).unwrap();
                            let filter = filters[0].clone();
                            let is_entry = filter["kinds"][0]
                                == serde_json::json!(buzz_sdk::kind::KIND_EMOJI_ENTRY);
                            log.lock().unwrap().push(filter);
                            let Reply(status, body) = if is_entry { entries } else { palette };
                            (
                                StatusCode::from_u16(status).unwrap(),
                                [("content-type", "application/json")],
                                body,
                            )
                        },
                    ),
                )
                .with_state((requests.clone(), entries, palette));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), requests)
    }

    fn entry_event(
        shortcode: &str,
        url: &str,
        author: &str,
        id: &str,
        created_at: i64,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "pubkey": author,
            "kind": buzz_sdk::kind::KIND_EMOJI_ENTRY,
            "created_at": created_at,
            "content": shortcode.replace(['_', '-'], " "),
            "tags": [["d", shortcode], ["emoji", shortcode, url]],
        })
    }

    fn entries_reply(events: &[serde_json::Value]) -> Reply {
        Reply::ok(serde_json::to_string(events).unwrap())
    }

    /// Legacy palette response: two custom emoji — `wave` and `sweatblob`.
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

    fn legacy_only() -> (Reply, Reply) {
        (Reply::empty(), Reply::ok(palette_response()))
    }

    fn kinds_of(requests: &Requests) -> Vec<u64> {
        requests
            .lock()
            .unwrap()
            .iter()
            .map(|f| f["kinds"][0].as_u64().unwrap())
            .collect()
    }

    const ENTRY: u64 = buzz_sdk::kind::KIND_EMOJI_ENTRY as u64;
    const SET: u64 = buzz_sdk::kind::KIND_EMOJI_SET as u64;

    #[tokio::test]
    async fn resolve_tags_known_shortcode_returns_correct_tag() {
        let (entries, palette) = legacy_only();
        let (url, _reqs) = fake_relay(entries, palette).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), "hello :wave:")
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
        let (entries, palette) = legacy_only();
        let (url, _reqs) = fake_relay(entries, palette).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":notarealemoji:")
            .await
            .unwrap();
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn resolve_tags_deduplicates_repeated_shortcode() {
        let (entries, palette) = legacy_only();
        let (url, _reqs) = fake_relay(entries, palette).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":wave: and :wave: again")
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0][1], "wave");
    }

    #[tokio::test]
    async fn resolve_tags_first_appearance_order() {
        let (entries, palette) = legacy_only();
        let (url, _reqs) = fake_relay(entries, palette).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":sweatblob: :wave:")
            .await
            .unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0][1], "sweatblob");
        assert_eq!(tags[1][1], "wave");
    }

    #[tokio::test]
    async fn resolve_tags_case_insensitive_match_emits_lowercase() {
        let (entries, palette) = legacy_only();
        let (url, _reqs) = fake_relay(entries, palette).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":WAVE:")
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0][1], "wave",
            "canonical tag shortcode must be lowercase"
        );
    }

    #[tokio::test]
    async fn resolve_tags_no_colon_content_skips_all_queries() {
        let (entries, palette) = legacy_only();
        let (url, reqs) = fake_relay(entries, palette).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), "Hello world, no colons")
            .await
            .unwrap();
        assert!(tags.is_empty());
        assert!(
            reqs.lock().unwrap().is_empty(),
            "must not query when content has no shortcode"
        );
    }

    #[tokio::test]
    async fn resolve_tags_non_canonical_palette_key_resolves() {
        // The relay stores the original signed tag, so a relay-valid legacy key
        // like "  :WAVE:  " must resolve for `:wave:` (emoji_tags_of uses the
        // SDK normalizer, not a plain lowercase conversion).
        let palette = serde_json::json!([{
            "created_at": 100,
            "tags": [
                ["d", "buzz:custom-emoji"],
                ["emoji", "  :WAVE:  ", "https://cdn.example.com/wave.png"],
            ]
        }])
        .to_string();
        let (url, _reqs) = fake_relay(Reply::empty(), Reply::ok(palette)).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), "hello :wave:")
            .await
            .unwrap();
        assert_eq!(
            tags,
            vec![vec!["emoji", "wave", "https://cdn.example.com/wave.png"]]
        );
    }

    #[tokio::test]
    async fn send_path_catalog_hit_never_fetches_legacy_palette() {
        let events = [
            entry_event(
                "parrot",
                "https://cdn.example.com/parrot.gif",
                "aa",
                "e1",
                10,
            ),
            entry_event("dance", "https://cdn.example.com/dance.gif", "aa", "e2", 10),
        ];
        let (url, reqs) = fake_relay(entries_reply(&events), Reply::ok(palette_response())).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":dance: x :parrot:")
            .await
            .unwrap();
        assert_eq!(
            tags,
            vec![
                vec!["emoji", "dance", "https://cdn.example.com/dance.gif"],
                vec!["emoji", "parrot", "https://cdn.example.com/parrot.gif"],
            ]
        );
        assert_eq!(kinds_of(&reqs), vec![ENTRY], "one batched #d query only");
        let filter = reqs.lock().unwrap()[0].clone();
        assert_eq!(filter["#d"], serde_json::json!(["dance", "parrot"]));
    }

    #[tokio::test]
    async fn send_path_mixed_names_query_catalog_then_legacy_once() {
        let events = [entry_event(
            "parrot",
            "https://cdn.example.com/parrot.gif",
            "aa",
            "e1",
            10,
        )];
        let (url, reqs) = fake_relay(entries_reply(&events), Reply::ok(palette_response())).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":parrot: :wave: :nope:")
            .await
            .unwrap();
        let names: Vec<&str> = tags.iter().map(|t| t[1].as_str()).collect();
        assert_eq!(names, vec!["parrot", "wave"]);
        assert_eq!(kinds_of(&reqs), vec![ENTRY, SET]);
    }

    #[tokio::test]
    async fn send_path_catalog_entry_wins_over_legacy_entry() {
        let events = [entry_event(
            "wave",
            "https://cdn.example.com/catalog-wave.png",
            "aa",
            "e1",
            10,
        )];
        let (url, _reqs) = fake_relay(entries_reply(&events), Reply::ok(palette_response())).await;
        let tags = resolve_emoji_tags_for_content(&test_client(&url), ":wave: :sweatblob:")
            .await
            .unwrap();
        assert_eq!(tags[0][2], "https://cdn.example.com/catalog-wave.png");
        assert_eq!(tags[1][1], "sweatblob");
    }

    #[tokio::test]
    async fn send_path_lookup_failures_are_errors_not_empty() {
        let (url, _reqs) =
            fake_relay(Reply(500, "boom".into()), Reply::ok(palette_response())).await;
        let err = resolve_emoji_tags_for_content(&test_client(&url), ":wave:")
            .await
            .unwrap_err();
        assert!(
            matches!(err, CliError::Relay { status: 500, .. }),
            "{err:?}"
        );

        let (url, _reqs) = fake_relay(Reply::empty(), Reply(500, "boom".into())).await;
        let err = resolve_emoji_tags_for_content(&test_client(&url), ":wave:")
            .await
            .unwrap_err();
        assert!(
            matches!(err, CliError::Relay { status: 500, .. }),
            "{err:?}"
        );

        let (url, _reqs) = fake_relay(Reply::ok("not json".into()), Reply::empty()).await;
        assert!(resolve_emoji_tags_for_content(&test_client(&url), ":wave:")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn send_path_truncated_catalog_page_is_an_error() {
        let events: Vec<_> = (0..CATALOG_EVENTS_PER_FILTER)
            .map(|i| {
                entry_event(
                    "wave",
                    "https://cdn.example.com/w.png",
                    "aa",
                    &format!("e{i}"),
                    i as i64,
                )
            })
            .collect();
        let (url, _reqs) = fake_relay(entries_reply(&events), Reply::empty()).await;
        let err = resolve_emoji_tags_for_content(&test_client(&url), ":wave:")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    // ── catalog commands ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_batches_names_in_chunks_of_fifty() {
        let (url, reqs) = fake_relay(Reply::empty(), Reply::empty()).await;
        let names: Vec<String> = (0..120).map(|i| format!("emoji{i}")).collect();
        let (found, missing) = resolve_catalog(&test_client(&url), &names).await.unwrap();
        assert!(found.is_empty());
        assert_eq!(missing, names);
        let sizes: Vec<usize> = reqs
            .lock()
            .unwrap()
            .iter()
            .map(|f| f["#d"].as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, vec![50, 50, 20]);
    }

    #[tokio::test]
    async fn resolve_reports_found_missing_and_newest_of_two_authors() {
        let events = [
            entry_event("parrot", "https://cdn.example.com/old.gif", "aa", "e1", 10),
            entry_event("parrot", "https://cdn.example.com/new.gif", "bb", "e2", 20),
            // not requested: must be dropped even if the relay returns it
            entry_event("other", "https://cdn.example.com/o.gif", "aa", "e3", 10),
        ];
        let (url, _reqs) = fake_relay(entries_reply(&events), Reply::empty()).await;
        let names = vec![
            "PARROT".to_string(),
            ":ghost:".to_string(),
            "parrot".to_string(),
        ];
        let (found, missing) = resolve_catalog(&test_client(&url), &names).await.unwrap();
        assert_eq!(
            found,
            vec![CatalogEntry {
                shortcode: "parrot".into(),
                url: "https://cdn.example.com/new.gif".into(),
                author: "bb".into(),
                event_id: "e2".into(),
                created_at: 20,
                other_authors: vec!["aa".into()],
            }]
        );
        assert_eq!(missing, vec!["ghost"]);
    }

    #[tokio::test]
    async fn resolve_rejects_invalid_name_before_any_query() {
        let (url, reqs) = fake_relay(Reply::empty(), Reply::empty()).await;
        let err = resolve_catalog(&test_client(&url), &["bad name".to_string()])
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert!(reqs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn search_sends_prefix_filter_and_shapes_output() {
        let events = [entry_event(
            "party_parrot",
            "https://cdn.example.com/pp.gif",
            "aa",
            "e1",
            10,
        )];
        let (url, reqs) = fake_relay(entries_reply(&events), Reply::empty()).await;
        let entries = search_catalog(&test_client(&url), "Party_Pa", 7)
            .await
            .unwrap();
        let filter = reqs.lock().unwrap()[0].clone();
        assert_eq!(
            filter,
            serde_json::json!({
                "kinds": [buzz_sdk::kind::KIND_EMOJI_ENTRY],
                "search": "party pa",
                "search_mode": "prefix",
                "limit": 7,
            })
        );
        let out = serde_json::to_value(&entries).unwrap();
        assert_eq!(
            out,
            serde_json::json!([{
                "shortcode": "party_parrot",
                "url": "https://cdn.example.com/pp.gif",
                "author": "aa",
                "event_id": "e1",
                "created_at": 10,
            }])
        );
    }

    #[tokio::test]
    async fn search_rejects_bad_usage_before_any_query() {
        let (url, reqs) = fake_relay(Reply::empty(), Reply::empty()).await;
        let client = test_client(&url);
        for (prefix, limit) in [
            ("p", 20),
            ("::", 20),
            ("pa rrot", 20),
            ("pa", 0),
            ("pa", 51),
        ] {
            let err = search_catalog(&client, prefix, limit).await.unwrap_err();
            assert!(
                matches!(err, CliError::Usage(_)),
                "{prefix:?}/{limit}: {err:?}"
            );
        }
        assert!(search_catalog(&client, "pa", 50).await.is_ok());
        assert_eq!(reqs.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_relay_failure_is_an_error() {
        let (url, _reqs) = fake_relay(Reply(500, "boom".into()), Reply::empty()).await;
        assert!(search_catalog(&test_client(&url), "pa", 20).await.is_err());
    }

    #[test]
    fn sdk_built_entry_parses_as_catalog_entry() {
        let keys = Keys::generate();
        let event = buzz_sdk::build_emoji_entry("Party_Parrot", "https://cdn.example.com/pp.gif")
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        let json = serde_json::to_value(&event).unwrap();
        let entry = catalog_entry_of(&json).expect("SDK-built entry must parse");
        assert_eq!(entry.shortcode, "party_parrot");
        assert_eq!(entry.url, "https://cdn.example.com/pp.gif");
        assert_eq!(entry.author, keys.public_key().to_hex());
        assert_eq!(entry.event_id, event.id.to_hex());
    }

    #[test]
    fn collapse_ties_break_to_smallest_event_id_and_skips_malformed() {
        let mut bad_url = entry_event("bad", "ftp://x/y.png", "aa", "e0", 99);
        bad_url["kind"] = serde_json::json!(buzz_sdk::kind::KIND_EMOJI_ENTRY);
        let mut wrong_kind = entry_event("wrongkind", "https://x/y.png", "aa", "e9", 99);
        wrong_kind["kind"] = serde_json::json!(1);
        let mut d_mismatch = entry_event("mismatch", "https://x/y.png", "aa", "e8", 99);
        d_mismatch["tags"][0] = serde_json::json!(["d", "other"]);
        let events = [
            bad_url,
            wrong_kind,
            d_mismatch,
            entry_event("tie", "https://x/b.png", "bb", "e2", 5),
            entry_event("tie", "https://x/a.png", "aa", "e1", 5),
        ];
        let out = collapse_catalog(&events, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event_id, "e1");
        assert_eq!(out[0].other_authors, vec!["bb"]);
    }

    #[test]
    fn collapse_same_author_replacement_is_not_a_conflict() {
        let events = [
            entry_event("wave", "https://x/old.png", "aa", "e1", 5),
            entry_event("wave", "https://x/new.png", "aa", "e2", 6),
        ];
        let out = collapse_catalog(&events, None);
        assert_eq!(out[0].url, "https://x/new.png");
        assert!(out[0].other_authors.is_empty());
    }
}
