use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use nostr::Timestamp;
use sha2::{Digest, Sha256};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use buzz_sdk::CustomEmoji;

/// Events per `/query` page when reading emoji set parts.
const PAGE: usize = 100;
/// Pages one catalog read may take (100k parts) before giving up.
const MAX_PAGES: usize = 1_000;
/// Parts one publish run signs per second: half a read page.
const PARTS_PER_SECOND: usize = PAGE / 2;

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
    emoji_tags_where(event, |_| true)
}

/// [`emoji_tags_of`], keeping only tags whose raw shortcode passes `keep`
/// (checked before normalizing, so skipped tags cost no allocation).
fn emoji_tags_where(event: &serde_json::Value, keep: impl Fn(&str) -> bool) -> Vec<EmojiEntry> {
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
        if url.is_empty() || !keep(raw_shortcode) {
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
    union_custom_emoji_where(events, |_| true)
}

/// [`union_custom_emoji`] over only the tags whose raw shortcode passes `keep`.
fn union_custom_emoji_where(
    events: &[serde_json::Value],
    keep: impl Fn(&str) -> bool,
) -> Vec<EmojiEntry> {
    let mut by_shortcode: std::collections::HashMap<String, (String, i64)> =
        std::collections::HashMap::new();
    for event in events {
        let created_at = event
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        for entry in emoji_tags_where(event, &keep) {
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

fn created_at_of(event: &serde_json::Value) -> i64 {
    event
        .get("created_at")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

fn str_field<'a>(event: &'a serde_json::Value, key: &str) -> &'a str {
    event.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Part number of a custom emoji set event, or `None` for any other
/// kind:30030 set (someone else's emoji pack, say).
fn part_of(event: &serde_json::Value) -> Option<u32> {
    let tags = event.get("tags")?.as_array()?;
    let d = tags.iter().find_map(|tag| {
        let parts = tag.as_array()?;
        (parts.first()?.as_str()? == "d").then(|| parts.get(1)?.as_str())?
    })?;
    buzz_sdk::custom_emoji_set_part(d)
}

/// Read every custom emoji set part (kind:30030), optionally for one author
/// or since a timestamp. Pages newest-first with an inclusive `until`,
/// dedupes by event id, and fails rather than return a silently truncated
/// catalog when a full page cannot advance (more than a page of parts share
/// one second) or the catalog exceeds [`MAX_PAGES`].
async fn read_parts(
    client: &BuzzClient,
    author: Option<&str>,
    since: Option<i64>,
) -> Result<Vec<serde_json::Value>, CliError> {
    let mut ids = HashSet::new();
    let mut out = Vec::new();
    let mut until: Option<i64> = None;
    for _ in 0..MAX_PAGES {
        let mut filter = serde_json::json!({
            "kinds": [buzz_sdk::kind::KIND_EMOJI_SET],
            "limit": PAGE,
        });
        if let Some(author) = author {
            filter["authors"] = serde_json::json!([author]);
        }
        if let Some(since) = since {
            filter["since"] = serde_json::json!(since);
        }
        if let Some(until) = until {
            filter["until"] = serde_json::json!(until);
        }
        let raw = client.query(&filter).await?;
        let page: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse emoji set query: {e}")))?;
        let full = page.len() >= PAGE;
        let oldest = page.iter().map(created_at_of).min();
        let mut advanced = false;
        for event in page {
            let id = str_field(&event, "id");
            if id.is_empty() || ids.insert(id.to_string()) {
                advanced = true;
                if part_of(&event).is_some() {
                    out.push(event);
                }
            }
        }
        let Some(oldest) = oldest.filter(|_| full) else {
            return Ok(out);
        };
        if !advanced {
            return Err(CliError::Other(
                "emoji catalog read stalled: more than 100 emoji sets share one second".into(),
            ));
        }
        until = Some(oldest);
    }
    Err(CliError::Other(format!(
        "emoji catalog exceeds {} sets",
        PAGE * MAX_PAGES
    )))
}

/// Keep only the newest event per `(pubkey, part)`; ties go to the smaller id.
fn latest_parts(events: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut latest: HashMap<(String, u32), serde_json::Value> = HashMap::new();
    for event in events {
        let Some(part) = part_of(&event) else {
            continue;
        };
        let key = (str_field(&event, "pubkey").to_string(), part);
        let newer = latest.get(&key).is_none_or(|old| {
            let (at, old_at) = (created_at_of(&event), created_at_of(old));
            at > old_at || (at == old_at && str_field(&event, "id") < str_field(old, "id"))
        });
        if newer {
            latest.insert(key, event);
        }
    }
    latest.into_values().collect()
}

/// The workspace palette's set parts, read in full.
async fn workspace_parts(client: &BuzzClient) -> Result<Vec<serde_json::Value>, CliError> {
    Ok(latest_parts(read_parts(client, None, None).await?))
}

/// List the workspace custom emoji palette: the union of every member's
/// custom emoji set parts.
async fn cmd_list(client: &BuzzClient) -> Result<(), CliError> {
    let emojis = union_custom_emoji(&workspace_parts(client).await?);
    let output = serde_json::json!({ "emojis": emojis });
    println!("{}", serde_json::to_string(&output).unwrap_or_default());
    Ok(())
}

/// One part of the caller's own set.
#[derive(Debug, Clone, Default, PartialEq)]
struct OwnPart {
    created_at: i64,
    emojis: Vec<CustomEmoji>,
}

/// The caller's own set, by part number.
type OwnParts = BTreeMap<u32, OwnPart>;

fn own_parts_of(events: Vec<serde_json::Value>) -> OwnParts {
    latest_parts(events)
        .into_iter()
        .filter_map(|event| {
            let part = part_of(&event)?;
            let emojis = emoji_tags_of(&event)
                .into_iter()
                .map(|e| CustomEmoji {
                    shortcode: e.shortcode,
                    url: e.url,
                })
                .collect();
            Some((
                part,
                OwnPart {
                    created_at: created_at_of(&event),
                    emojis,
                },
            ))
        })
        .collect()
}

/// Fetch the caller's own set parts. Fails rather than return a partial set,
/// since every edit is read-modify-write.
async fn fetch_own_parts(client: &BuzzClient) -> Result<OwnParts, CliError> {
    let me = client.keys().public_key().to_hex();
    Ok(own_parts_of(read_parts(client, Some(&me), None).await?))
}

/// The caller's own emoji in part order; a shortcode a legacy client left in
/// several parts appears once, from its lowest part.
fn own_emoji(parts: &OwnParts) -> Vec<CustomEmoji> {
    let mut seen = HashSet::new();
    parts
        .values()
        .flat_map(|part| part.emojis.iter())
        .filter(|e| seen.insert(e.shortcode.clone()))
        .cloned()
        .collect()
}

fn part_bytes(emojis: &[CustomEmoji]) -> usize {
    emojis.iter().map(buzz_sdk::custom_emoji_tag_bytes).sum()
}

/// Parts to republish, in publish order, for adding (or re-pointing) `adds`
/// and removing `removes`. Untouched parts are never republished.
///
/// A re-pointed shortcode stays in the lowest part that held it when it
/// fits; new shortcodes go to the first part with room, then to a new part
/// after the highest. Every other copy of a touched shortcode is cleared, so
/// a duplicate a legacy client left behind cannot shadow the edit. Parts that
/// gain a shortcode publish before parts that only lose one, so a shortcode
/// moving between parts is never missing from the palette.
fn plan_edit(
    parts: &OwnParts,
    adds: &[CustomEmoji],
    removes: &HashSet<String>,
) -> Vec<(u32, Vec<CustomEmoji>)> {
    let mut working: BTreeMap<u32, Vec<CustomEmoji>> = parts
        .iter()
        .map(|(part, own)| (*part, own.emojis.clone()))
        .collect();
    let mut holders: HashMap<String, BTreeSet<u32>> = HashMap::new();
    for (part, emojis) in &working {
        for emoji in emojis {
            holders
                .entry(emoji.shortcode.clone())
                .or_default()
                .insert(*part);
        }
    }
    let mut sizes: BTreeMap<u32, usize> = working
        .iter()
        .map(|(part, emojis)| (*part, part_bytes(emojis)))
        .collect();
    let mut gained = BTreeSet::new();
    let mut lost = BTreeSet::new();
    // Clears a shortcode from every part holding it; returns each holder
    // with the index it sat at, so a re-point can keep its place.
    let mut clear = |shortcode: &str,
                     working: &mut BTreeMap<u32, Vec<CustomEmoji>>,
                     sizes: &mut BTreeMap<u32, usize>|
     -> Vec<(u32, usize)> {
        let held = holders.remove(shortcode).unwrap_or_default();
        let mut at = Vec::with_capacity(held.len());
        for part in held {
            let emojis = working.entry(part).or_default();
            if let Some(index) = emojis.iter().position(|e| e.shortcode == shortcode) {
                at.push((part, index));
            }
            emojis.retain(|e| e.shortcode != shortcode);
            sizes.insert(part, part_bytes(emojis));
            lost.insert(part);
        }
        at
    };
    for shortcode in removes {
        clear(shortcode, &mut working, &mut sizes);
    }
    for emoji in adds {
        let held = clear(&emoji.shortcode, &mut working, &mut sizes);
        let cost = buzz_sdk::custom_emoji_tag_bytes(emoji);
        let fits = |part: &u32| {
            sizes.get(part).copied().unwrap_or(0) + cost <= buzz_sdk::CUSTOM_EMOJI_PART_BYTES
        };
        let (target, index) = held
            .iter()
            .copied()
            .find(|(part, _)| fits(part))
            .map(|(part, index)| (part, Some(index)))
            .or_else(|| sizes.keys().copied().find(fits).map(|part| (part, None)))
            .unwrap_or_else(|| (sizes.keys().next_back().map_or(0, |last| last + 1), None));
        let emojis = working.entry(target).or_default();
        emojis.insert(
            index.unwrap_or(emojis.len()).min(emojis.len()),
            emoji.clone(),
        );
        *sizes.entry(target).or_default() += cost;
        gained.insert(target);
    }
    gained
        .iter()
        .chain(lost.difference(&gained))
        .filter_map(|part| {
            let next = working.get(part).cloned().unwrap_or_default();
            let unchanged = match parts.get(part) {
                Some(own) => own.emojis == next,
                None => next.is_empty(),
            };
            (!unchanged).then_some((*part, next))
        })
        .collect()
}

/// Pack `emojis` into parts 0..n under the part budget, then clear every
/// other part the caller still holds. Only parts that change are returned.
fn plan_replace(parts: &OwnParts, emojis: &[CustomEmoji]) -> Vec<(u32, Vec<CustomEmoji>)> {
    let mut packed: Vec<Vec<CustomEmoji>> = vec![Vec::new()];
    let mut size = 0;
    for emoji in emojis {
        let cost = buzz_sdk::custom_emoji_tag_bytes(emoji);
        if size + cost > buzz_sdk::CUSTOM_EMOJI_PART_BYTES && !packed[packed.len() - 1].is_empty() {
            packed.push(Vec::new());
            size = 0;
        }
        size += cost;
        let last = packed.len() - 1;
        packed[last].push(emoji.clone());
    }
    let mut plan: Vec<(u32, Vec<CustomEmoji>)> = packed
        .into_iter()
        .enumerate()
        .map(|(part, emojis)| (part as u32, emojis))
        .collect();
    let count = plan.len() as u32;
    plan.extend(
        parts
            .iter()
            .filter(|(part, own)| **part >= count && !own.emojis.is_empty())
            .map(|(part, _)| (*part, Vec::new())),
    );
    plan.retain(|(part, emojis)| match parts.get(part) {
        Some(own) => own.emojis != *emojis,
        None => !emojis.is_empty(),
    });
    // Parts receiving a shortcode another part held publish first, so a
    // shortcode moving between parts is never missing from the palette.
    let mut held_in: HashMap<&str, u32> = HashMap::new();
    for (part, own) in parts {
        for e in &own.emojis {
            held_in.entry(e.shortcode.as_str()).or_insert(*part);
        }
    }
    let gains = |(part, emojis): &(u32, Vec<CustomEmoji>)| {
        emojis.iter().any(|e| {
            held_in
                .get(e.shortcode.as_str())
                .is_some_and(|from| from != part)
        })
    };
    plan.sort_by_key(|entry| !gains(entry));
    plan
}

fn now_secs() -> i64 {
    Timestamp::now().as_secs() as i64
}

/// Publish planned parts in order, each signed as the caller with a
/// `created_at` past the copy it replaces. Updates `parts` as each lands so a
/// failure part-way leaves `parts` matching the relay.
async fn publish_parts(
    client: &BuzzClient,
    parts: &mut OwnParts,
    plan: Vec<(u32, Vec<CustomEmoji>)>,
) -> Result<Vec<serde_json::Value>, CliError> {
    let mut responses = Vec::with_capacity(plan.len());
    let mut per_second: HashMap<i64, usize> = HashMap::new();
    for (part, emojis) in plan {
        let mut created_at = now_secs().max(parts.get(&part).map_or(0, |own| own.created_at + 1));
        // Readers page newest-first by second; keep every second under half
        // a page so a read can always advance past it.
        while per_second.get(&created_at).copied().unwrap_or(0) >= PARTS_PER_SECOND {
            if created_at > now_secs() {
                // Already ahead of the clock (bumped past an old copy).
                created_at += 1;
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                created_at = now_secs().max(created_at);
            }
        }
        *per_second.entry(created_at).or_default() += 1;
        let builder = buzz_sdk::build_custom_emoji_set_part(part, &emojis)
            .map_err(|e| CliError::Other(format!("build_custom_emoji_set_part failed: {e}")))?
            .custom_created_at(Timestamp::from_secs(created_at as u64));
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        parts.insert(part, OwnPart { created_at, emojis });
        let mut resp: serde_json::Value = serde_json::from_str(&normalize_write_response(&resp))
            .unwrap_or_else(|_| serde_json::json!({ "accepted": true, "message": resp }));
        resp["part"] = serde_json::json!(part);
        responses.push(resp);
    }
    Ok(responses)
}

/// Print one publish result: the relay's response for a single part, or
/// every part's response when an edit spans several.
fn print_published(responses: Vec<serde_json::Value>) {
    match responses.as_slice() {
        [] => println!(
            "{}",
            serde_json::json!({"accepted": true, "message": "unchanged"})
        ),
        [one] => println!("{one}"),
        _ => println!(
            "{}",
            serde_json::json!({ "accepted": true, "parts": responses })
        ),
    }
}

/// Add/update a shortcode in the caller's own set (read-modify-write of the
/// parts it touches).
async fn cmd_set(client: &BuzzClient, shortcode: &str, url: &str) -> Result<(), CliError> {
    let normalized = buzz_sdk::normalize_custom_emoji_shortcode(shortcode)
        .map_err(|e| CliError::Other(format!("invalid shortcode: {e}")))?;
    let mut parts = fetch_own_parts(client).await?;
    let adds = [CustomEmoji {
        shortcode: normalized,
        url: url.to_string(),
    }];
    let plan = plan_edit(&parts, &adds, &HashSet::new());
    print_published(publish_parts(client, &mut parts, plan).await?);
    Ok(())
}

/// Remove a shortcode from the caller's own set (read-modify-write of the
/// parts that hold it).
async fn cmd_rm(client: &BuzzClient, shortcode: &str) -> Result<(), CliError> {
    let normalized = buzz_sdk::normalize_custom_emoji_shortcode(shortcode)
        .map_err(|e| CliError::Other(format!("invalid shortcode: {e}")))?;
    let mut parts = fetch_own_parts(client).await?;
    let plan = plan_edit(&parts, &[], &HashSet::from([normalized]));
    if plan.is_empty() {
        // Nothing to remove; avoid republishing an unchanged set.
        println!(
            "{}",
            serde_json::json!({"accepted": true, "message": "not present"})
        );
        return Ok(());
    }
    print_published(publish_parts(client, &mut parts, plan).await?);
    Ok(())
}

/// 64 MiB — a safety rail against runaway producers. A 100k-emoji manifest
/// or Slack `emoji.list` export is ~10–15 MB.
const STDIN_MAX_BYTES: u64 = 64 * 1024 * 1024;

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
            let mut entries: Vec<EmojiEntry> = own_emoji(&fetch_own_parts(client).await?)
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
        crate::EmojiScope::Workspace => union_custom_emoji(&workspace_parts(client).await?),
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

    // 6. Plan the parts to publish
    let mut parts = fetch_own_parts(client).await?;
    let plan = if replace {
        plan_replace(&parts, &import_entries)
    } else {
        let existing: HashSet<String> =
            own_emoji(&parts).into_iter().map(|e| e.shortcode).collect();
        import_entries.retain(|e| !existing.contains(&e.shortcode));
        plan_edit(&parts, &import_entries, &HashSet::new())
    };

    // 7. Dry-run: print the resulting set to stdout, warn to stderr
    if dry_run {
        let mut preview = parts.clone();
        for (part, emojis) in &plan {
            preview.insert(
                *part,
                OwnPart {
                    created_at: 0,
                    emojis: emojis.clone(),
                },
            );
        }
        let entries: Vec<EmojiEntry> = own_emoji(&preview)
            .into_iter()
            .map(|e| EmojiEntry {
                shortcode: e.shortcode,
                url: e.url,
            })
            .collect();
        let output = serde_json::to_string(&serde_json::json!({ "emojis": entries }))
            .map_err(|e| CliError::Other(format!("serialization failed: {e}")))?;
        println!("{output}");
        eprintln!(
            "(dry run — not published; would publish {} part(s))",
            plan.len()
        );
        return Ok(());
    }

    // 8. Publish
    print_published(publish_parts(client, &mut parts, plan).await?);
    Ok(())
}

/// New emoji uploaded per published batch: one part's worth, so an
/// interrupted import loses at most one batch of uploads.
const SLACK_BATCH: usize = 450;
/// Largest Slack image the importer downloads.
const SLACK_MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Upload attempts per image while the relay rate-limits.
const SLACK_UPLOAD_ATTEMPTS: u32 = 60;

/// One Slack emoji to import: its Buzz shortcode and source image URL.
#[derive(Debug, Clone, PartialEq)]
struct SlackEmoji {
    shortcode: String,
    source: String,
}

/// Parse a Slack `emoji.list` response (or its bare `emoji` map) into
/// importable emoji, resolving `alias:` entries to their target's image.
/// Returns the emoji sorted by shortcode and the names it skipped, with why.
fn slack_emoji(
    json: &serde_json::Value,
) -> Result<(Vec<SlackEmoji>, Vec<serde_json::Value>), CliError> {
    let map = json
        .get("emoji")
        .unwrap_or(json)
        .as_object()
        .ok_or_else(|| {
            CliError::Usage(
                "input must be a Slack emoji.list response or a {name: url} object".into(),
            )
        })?;
    let mut names: Vec<&String> = map.keys().collect();
    names.sort();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for name in names {
        let mut value = map[name].as_str().unwrap_or("");
        // Follow alias chains a few hops; Slack allows one, be lenient.
        for _ in 0..8 {
            match value.strip_prefix("alias:") {
                Some(target) => value = map.get(target).and_then(|v| v.as_str()).unwrap_or(""),
                None => break,
            }
        }
        let reason = if value.starts_with("alias:") || value.is_empty() {
            Some("alias of a standard emoji".to_string())
        } else if !value.starts_with("https://") && !value.starts_with("http://") {
            Some("not an image URL".to_string())
        } else {
            None
        };
        let shortcode = buzz_sdk::normalize_custom_emoji_shortcode(name);
        match (reason, shortcode) {
            (Some(reason), _) => skipped.push(serde_json::json!({"name": name, "reason": reason})),
            (None, Err(e)) => {
                skipped.push(serde_json::json!({"name": name, "reason": e.to_string()}))
            }
            (None, Ok(shortcode)) => {
                if seen.insert(shortcode.clone()) {
                    out.push(SlackEmoji {
                        shortcode,
                        source: value.to_string(),
                    });
                } else {
                    skipped
                        .push(serde_json::json!({"name": name, "reason": "duplicate shortcode"}));
                }
            }
        }
    }
    Ok((out, skipped))
}

async fn download_image(http: &reqwest::Client, source: &str) -> Result<Vec<u8>, CliError> {
    let mut last = None;
    for attempt in 0..3u64 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(attempt * 2)).await;
        }
        let result = async {
            let resp = http.get(source).send().await?.error_for_status()?;
            if resp
                .content_length()
                .is_some_and(|len| len as usize > SLACK_MAX_IMAGE_BYTES)
            {
                return Ok(Err(format!("image exceeds {SLACK_MAX_IMAGE_BYTES} bytes")));
            }
            let bytes = resp.bytes().await?;
            Ok::<_, reqwest::Error>(if bytes.len() > SLACK_MAX_IMAGE_BYTES {
                Err(format!("image exceeds {SLACK_MAX_IMAGE_BYTES} bytes"))
            } else {
                Ok(bytes.to_vec())
            })
        }
        .await;
        match result {
            Ok(Ok(bytes)) => return Ok(bytes),
            Ok(Err(reason)) => return Err(CliError::Other(reason)),
            Err(e) if e.status().is_some_and(|s| s.is_client_error()) => {
                return Err(CliError::Other(format!("download failed: {e}")))
            }
            Err(e) => last = Some(e),
        }
    }
    Err(CliError::Other(format!(
        "download failed: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    )))
}

/// Upload, waiting out the relay's per-pubkey upload rate limit.
async fn upload_paced(client: &BuzzClient, bytes: Vec<u8>) -> Result<String, CliError> {
    for _ in 0..SLACK_UPLOAD_ATTEMPTS {
        match client.upload_bytes(bytes.clone()).await {
            Ok(blob) => return Ok(blob.url),
            Err(CliError::Relay { status: 429, body }) => {
                let wait = crate::client::parse_retry_hint_text(&body)
                    .unwrap_or(10)
                    .clamp(1, 120);
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(CliError::Other(
        "upload still rate-limited; rerun to resume".into(),
    ))
}

/// Import a Slack workspace's custom emoji into the caller's own set.
///
/// Downloads each image, re-hosts it on the relay, and publishes the new
/// shortcodes one part-sized batch at a time. Resumable: shortcodes already
/// in the caller's set are skipped, so a rerun picks up where an interrupted
/// import stopped.
async fn cmd_import_slack(
    client: &BuzzClient,
    file: Option<&str>,
    limit: Option<usize>,
    concurrency: usize,
    dry_run: bool,
) -> Result<(), CliError> {
    let raw = read_source(file)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| CliError::Usage(format!("invalid JSON: {e}")))?;
    let (all, skipped) = slack_emoji(&parsed)?;
    let mut parts = fetch_own_parts(client).await?;
    let existing: HashSet<String> = own_emoji(&parts).into_iter().map(|e| e.shortcode).collect();
    let mut pending: Vec<SlackEmoji> = all
        .into_iter()
        .filter(|e| !existing.contains(&e.shortcode))
        .collect();
    let already = existing.len();
    if let Some(limit) = limit {
        pending.truncate(limit);
    }
    if dry_run {
        println!(
            "{}",
            serde_json::json!({
                "pending": pending.len(),
                "already_in_set": already,
                "skipped": skipped,
            })
        );
        eprintln!("(dry run — nothing uploaded or published)");
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| CliError::Other(format!("http client: {e}")))?;
    let total = pending.len();
    let mut imported = 0;
    let mut failed = Vec::new();
    // Aliases share an image: fetch and upload each source once per run.
    let mut hosted: HashMap<String, String> = HashMap::new();
    for batch in pending.chunks(SLACK_BATCH) {
        let mut by_source: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut adds = Vec::new();
        for emoji in batch {
            match hosted.get(&emoji.source) {
                Some(url) => adds.push(CustomEmoji {
                    shortcode: emoji.shortcode.clone(),
                    url: url.clone(),
                }),
                None => by_source
                    .entry(emoji.source.as_str())
                    .or_default()
                    .push(emoji.shortcode.as_str()),
            }
        }
        let results: Vec<_> = futures_util::stream::iter(by_source)
            .map(|(source, shortcodes)| {
                let http = &http;
                async move {
                    let url = match download_image(http, source).await {
                        Ok(bytes) => upload_paced(client, bytes).await,
                        Err(e) => Err(e),
                    };
                    (source, shortcodes, url)
                }
            })
            .buffer_unordered(concurrency.max(1))
            .collect()
            .await;
        for (source, shortcodes, url) in results {
            if let Ok(url) = &url {
                hosted.insert(source.to_string(), url.clone());
            }
            for shortcode in shortcodes {
                match &url {
                    Ok(url) => adds.push(CustomEmoji {
                        shortcode: shortcode.to_string(),
                        url: url.clone(),
                    }),
                    Err(e) => failed.push(serde_json::json!({
                        "shortcode": shortcode,
                        "source": source,
                        "error": e.to_string(),
                    })),
                }
            }
        }
        // Publish in source order so parts read alphabetically.
        adds.sort_by(|a, b| a.shortcode.cmp(&b.shortcode));
        let plan = plan_edit(&parts, &adds, &HashSet::new());
        publish_parts(client, &mut parts, plan).await?;
        imported += adds.len();
        eprintln!("imported {imported}/{total} ({} failed)", failed.len());
    }
    println!(
        "{}",
        serde_json::json!({
            "imported": imported,
            "already_in_set": already,
            "failed": failed,
            "skipped": skipped,
        })
    );
    if imported == 0 && !failed.is_empty() {
        return Err(CliError::Other(format!(
            "no emoji imported; all {} attempted failed",
            failed.len()
        )));
    }
    Ok(())
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

    // Workspace palette (union of all members' set parts), from the local
    // cache when one is fresh.
    // Only candidate shortcodes are unioned: a palette can hold 100k.
    let parts = cached_workspace_parts(client, palette_cache_path(client)).await?;
    let palette = union_custom_emoji_where(&parts, |raw| {
        let raw = raw.trim().trim_matches(':');
        candidates.iter().any(|c| c.eq_ignore_ascii_case(raw))
    });
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

/// Seconds of clock skew an incremental palette refresh re-reads. Parts
/// published within this window of the newest cached part are re-read on
/// every send, so it stays short.
const CACHE_SKEW_SECS: i64 = 120;
/// A cached palette older than this is re-read in full, which also drops
/// parts that were deleted rather than replaced.
const CACHE_FULL_READ_SECS: i64 = 3_600;

/// The send path's on-disk copy of the workspace palette's set parts.
#[derive(serde::Serialize, serde::Deserialize)]
struct PaletteCache {
    version: u32,
    /// When the cached parts were last read in full.
    full_read_at: i64,
    events: Vec<serde_json::Value>,
}

const PALETTE_CACHE_VERSION: u32 = 1;

/// Per relay and identity, so communities and keys never share a palette.
fn palette_cache_path(client: &BuzzClient) -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let key = Sha256::digest(format!(
        "{}\n{}",
        client.relay_url(),
        client.keys().public_key().to_hex()
    ));
    Some(
        dirs::cache_dir()?
            .join("buzz")
            .join("emoji")
            .join(format!("{}.json", &hex::encode(key)[..16])),
    )
}

/// The workspace palette's set parts for resolving a send. A 100k-emoji
/// palette is ~15 MB, so sends refresh a cached copy with only the parts
/// published since it was read (less a skew window) instead of re-reading
/// every part per message.
async fn cached_workspace_parts(
    client: &BuzzClient,
    cache: Option<PathBuf>,
) -> Result<Vec<serde_json::Value>, CliError> {
    let now = now_secs();
    let cached = cache
        .as_deref()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<PaletteCache>(&bytes).ok())
        .filter(|c| {
            c.version == PALETTE_CACHE_VERSION
                && (0..CACHE_FULL_READ_SECS).contains(&(now - c.full_read_at))
        });
    let (fresh, changed) = match cached {
        Some(mut cached) => {
            // A part dated ahead of now (a member's clock runs fast) must not
            // push `since` past parts published since the last refresh.
            let newest = cached
                .events
                .iter()
                .map(created_at_of)
                .filter(|at| *at <= now)
                .max()
                .unwrap_or(0);
            let since = (newest - CACHE_SKEW_SECS).max(0);
            let known: HashSet<&str> = cached.events.iter().map(|e| str_field(e, "id")).collect();
            let recent: Vec<serde_json::Value> = read_parts(client, None, Some(since))
                .await?
                .into_iter()
                .filter(|e| !known.contains(str_field(e, "id")))
                .collect();
            let changed = !recent.is_empty();
            if changed {
                cached.events.extend(recent);
                cached.events = latest_parts(cached.events);
            }
            (cached, changed)
        }
        None => (
            PaletteCache {
                version: PALETTE_CACHE_VERSION,
                full_read_at: now,
                events: workspace_parts(client).await?,
            },
            true,
        ),
    };
    if let Some(path) = cache.as_deref().filter(|_| changed) {
        // Best effort: a failed write only costs the next send a full read.
        let _ = write_cache(path, &fresh);
    }
    Ok(fresh.events)
}

fn write_cache(path: &Path, cache: &PaletteCache) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write then rename so a concurrent send never reads a torn file.
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(
        &tmp,
        serde_json::to_vec(cache).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
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
        EmojiCmd::ImportSlack {
            file,
            limit,
            concurrency,
            dry_run,
        } => cmd_import_slack(client, file.as_deref(), limit, concurrency, dry_run).await,
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

    // ── parts: planning, paging, cache, Slack import ────────────────────────

    fn emoji(shortcode: &str) -> CustomEmoji {
        CustomEmoji {
            shortcode: shortcode.to_string(),
            url: format!("https://cdn.example.com/{shortcode}.png"),
        }
    }

    fn own(parts: &[(u32, &[&str])]) -> OwnParts {
        parts
            .iter()
            .map(|(part, codes)| {
                (
                    *part,
                    OwnPart {
                        created_at: 10,
                        emojis: codes.iter().map(|c| emoji(c)).collect(),
                    },
                )
            })
            .collect()
    }

    fn codes(plan: &[(u32, Vec<CustomEmoji>)]) -> Vec<(u32, Vec<String>)> {
        plan.iter()
            .map(|(part, emojis)| (*part, emojis.iter().map(|e| e.shortcode.clone()).collect()))
            .collect()
    }

    #[test]
    fn plan_adds_into_a_new_part_when_the_legacy_set_is_full() {
        let full: Vec<String> = (0..buzz_sdk::CUSTOM_EMOJI_PART_BYTES / 40)
            .map(|i| format!("legacy_{i}"))
            .collect();
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        let parts = own(&[(0, &refs)]);
        let plan = plan_edit(&parts, &[emoji("wave")], &HashSet::new());
        assert_eq!(codes(&plan), vec![(1, vec!["wave".to_string()])]);
    }

    #[test]
    fn plan_repoints_in_place_and_removes_from_only_the_holder() {
        let parts = own(&[(0, &["a", "b"]), (1, &["c"])]);
        let repoint = CustomEmoji {
            shortcode: "c".into(),
            url: "https://cdn.example.com/c2.png".into(),
        };
        let plan = plan_edit(&parts, std::slice::from_ref(&repoint), &HashSet::new());
        assert_eq!(plan, vec![(1, vec![repoint])]);
        let plan = plan_edit(&parts, &[], &HashSet::from(["a".to_string()]));
        assert_eq!(codes(&plan), vec![(0, vec!["b".to_string()])]);
        assert!(plan_edit(&parts, &[], &HashSet::from(["zzz".to_string()])).is_empty());
    }

    #[test]
    fn plan_repoint_keeps_position_and_same_url_is_a_no_op() {
        let parts = own(&[(0, &["a", "b", "c"])]);
        assert!(plan_edit(&parts, &[emoji("b")], &HashSet::new()).is_empty());
        let b2 = CustomEmoji {
            shortcode: "b".into(),
            url: "https://cdn.example.com/b2.png".into(),
        };
        let plan = plan_edit(&parts, std::slice::from_ref(&b2), &HashSet::new());
        assert_eq!(plan, vec![(0, vec![emoji("a"), b2, emoji("c")])]);
    }

    #[test]
    fn plan_replace_publishes_gaining_parts_first() {
        // "x" moves from part 0 to part 1; part 1 must land first.
        let filler: Vec<String> = (0..buzz_sdk::CUSTOM_EMOJI_PART_BYTES / 50)
            .map(|i| format!("f{i:05}"))
            .collect();
        let refs: Vec<&str> = filler.iter().map(String::as_str).collect();
        let mut parts = own(&[(0, &refs)]);
        parts.get_mut(&0).unwrap().emojis.push(emoji("x"));
        let mut next: Vec<CustomEmoji> = vec![emoji("a0"), emoji("a1"), emoji("a2")];
        next.extend(refs.iter().map(|c| emoji(c)));
        next.push(emoji("x"));
        let plan = plan_replace(&parts, &next);
        assert_eq!(plan.iter().map(|(p, _)| *p).collect::<Vec<_>>(), vec![1, 0]);
        assert!(plan[0].1.iter().any(|e| e.shortcode == "x"));
    }

    #[test]
    fn plan_clears_legacy_duplicates_with_the_gaining_part_first() {
        let parts = own(&[(0, &["dup", "keep"]), (2, &["dup"])]);
        // Unchanged in part 0: only the stray copy is cleared.
        let plan = plan_edit(&parts, &[emoji("dup")], &HashSet::new());
        assert_eq!(codes(&plan), vec![(2, vec![])]);
        let dup2 = CustomEmoji {
            shortcode: "dup".into(),
            url: "https://cdn.example.com/dup2.png".into(),
        };
        let plan = plan_edit(&parts, std::slice::from_ref(&dup2), &HashSet::new());
        assert_eq!(plan, vec![(0, vec![dup2, emoji("keep")]), (2, vec![])]);
    }

    #[test]
    fn plan_fills_parts_in_order_for_a_bulk_import() {
        let adds: Vec<CustomEmoji> = (0..3_000).map(|i| emoji(&format!("e{i:04}"))).collect();
        let plan = plan_edit(&OwnParts::new(), &adds, &HashSet::new());
        assert!(plan.len() >= 3);
        for (index, (part, emojis)) in plan.iter().enumerate() {
            assert_eq!(*part, index as u32);
            assert!(part_bytes(emojis) <= buzz_sdk::CUSTOM_EMOJI_PART_BYTES);
        }
        let flat: Vec<CustomEmoji> = plan.into_iter().flat_map(|(_, e)| e).collect();
        assert_eq!(flat, adds);
    }

    #[test]
    fn plan_replace_repacks_and_empties_leftover_parts() {
        let parts = own(&[(0, &["a"]), (1, &["b"]), (5, &["c"]), (6, &[])]);
        let plan = plan_replace(&parts, &[emoji("a"), emoji("z")]);
        assert_eq!(
            codes(&plan),
            vec![
                (0, vec!["a".to_string(), "z".to_string()]),
                (1, vec![]),
                (5, vec![]),
            ]
        );
        // Replacing with the current set publishes nothing.
        assert!(plan_replace(&own(&[(0, &["a"])]), &[emoji("a")]).is_empty());
    }

    /// Filter-aware fake relay: serves `/query` from a store of events the
    /// way the relay does (newest first, inclusive `until`/`since`, `limit`,
    /// `authors`), recording every filter.
    async fn paging_server(
        events: Vec<serde_json::Value>,
    ) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        type S = (
            Arc<Mutex<Vec<serde_json::Value>>>,
            Arc<Vec<serde_json::Value>>,
        );
        let filters: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
        let state: S = (filters.clone(), Arc::new(events));
        let app = Router::new()
            .route(
                "/query",
                post(|State((seen, store)): State<S>, body: Bytes| async move {
                    let filter: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
                    let filter = filter[0].clone();
                    seen.lock().unwrap().push(filter.clone());
                    let num = |key: &str| filter.get(key).and_then(|v| v.as_i64());
                    let author = filter["authors"][0].as_str();
                    let mut out: Vec<serde_json::Value> = store
                        .iter()
                        .filter(|e| {
                            let at = created_at_of(e);
                            num("until").is_none_or(|u| at <= u)
                                && num("since").is_none_or(|s| at >= s)
                                && author.is_none_or(|a| str_field(e, "pubkey") == a)
                        })
                        .cloned()
                        .collect();
                    out.sort_by(|a, b| {
                        created_at_of(b)
                            .cmp(&created_at_of(a))
                            .then(str_field(a, "id").cmp(str_field(b, "id")))
                    });
                    out.truncate(num("limit").unwrap_or(500) as usize);
                    (
                        StatusCode::OK,
                        [("content-type", "application/json")],
                        serde_json::to_string(&out).unwrap(),
                    )
                }),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), filters)
    }

    fn part_event(pubkey: &str, part: u32, created_at: i64, codes: &[&str]) -> serde_json::Value {
        let mut tags = vec![serde_json::json!([
            "d",
            buzz_sdk::custom_emoji_set_d_tag(part)
        ])];
        tags.extend(
            codes.iter().map(|c| {
                serde_json::json!(["emoji", c, format!("https://cdn.example.com/{c}.png")])
            }),
        );
        serde_json::json!({
            "id": format!("{pubkey}-{part}-{created_at}"),
            "pubkey": pubkey,
            "kind": 30030,
            "created_at": created_at,
            "tags": tags,
        })
    }

    #[tokio::test]
    async fn reads_every_part_across_pages_and_ignores_other_sets() {
        // 250 parts, three per second, so pages split mid-second.
        let mut events: Vec<serde_json::Value> = (0..250u32)
            .map(|i| {
                let code = format!("code_{i}");
                part_event("member", i, 1_000 + i as i64 / 3, &[code.as_str()])
            })
            .collect();
        let mut foreign = part_event("member", 0, 5_000, &["foreign"]);
        foreign["tags"][0] = serde_json::json!(["d", "someone-elses-pack"]);
        foreign["id"] = serde_json::json!("foreign");
        events.push(foreign);
        let (url, filters) = paging_server(events).await;
        let palette = union_custom_emoji(&workspace_parts(&test_client(&url)).await.unwrap());
        assert_eq!(palette.len(), 250);
        assert!(palette.iter().all(|e| e.shortcode != "foreign"));
        let filters = filters.lock().unwrap();
        assert!(filters.len() >= 3);
        assert!(filters[0].get("#d").is_none());
        assert!(filters[1..].iter().all(|f| f.get("until").is_some()));
    }

    #[tokio::test]
    async fn refuses_a_read_that_cannot_advance_past_one_second() {
        let events: Vec<serde_json::Value> = (0..150u32)
            .map(|i| part_event("member", i, 7, &[]))
            .collect();
        let (url, _) = paging_server(events).await;
        let err = workspace_parts(&test_client(&url)).await.unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
    }

    #[tokio::test]
    async fn send_cache_refreshes_with_only_new_parts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("palette.json");
        let (url, filters) = paging_server(vec![part_event("member", 0, 1_000, &["wave"])]).await;
        let client = test_client(&url);
        let first = cached_workspace_parts(&client, Some(path.clone()))
            .await
            .unwrap();
        assert_eq!(union_custom_emoji(&first)[0].shortcode, "wave");
        // A newer cached copy of the part wins over the one the relay serves.
        let mut cache: PaletteCache =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        cache.events = vec![part_event("member", 0, 2_000, &["tada"])];
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let second = cached_workspace_parts(&client, Some(path.clone()))
            .await
            .unwrap();
        let palette = union_custom_emoji(&second);
        assert_eq!(palette.len(), 1);
        assert_eq!(palette[0].shortcode, "tada");
        let filters = filters.lock().unwrap();
        assert_eq!(filters.len(), 2);
        assert!(filters[0].get("since").is_none());
        assert_eq!(
            filters[1]["since"],
            serde_json::json!(2_000 - CACHE_SKEW_SECS)
        );
    }

    #[test]
    fn slack_export_resolves_aliases_and_skips_what_cannot_import() {
        let json = serde_json::json!({
            "ok": true,
            "emoji": {
                "parrot": "https://emoji.slack-edge.com/T1/parrot/1.gif",
                "Party-Parrot": "alias:parrot",
                "thumbsup_all": "alias:+1",
                "bad name": "https://emoji.slack-edge.com/T1/bad/1.png",
                "parrot2": "alias:Party-Parrot",
            }
        });
        let (emoji, skipped) = slack_emoji(&json).unwrap();
        let parrot = "https://emoji.slack-edge.com/T1/parrot/1.gif";
        assert_eq!(
            emoji,
            vec![
                SlackEmoji {
                    shortcode: "party-parrot".into(),
                    source: parrot.into()
                },
                SlackEmoji {
                    shortcode: "parrot".into(),
                    source: parrot.into()
                },
                SlackEmoji {
                    shortcode: "parrot2".into(),
                    source: parrot.into()
                },
            ]
        );
        let names: Vec<&str> = skipped
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["bad name", "thumbsup_all"]);
        // A bare {name: url} map works too.
        let (bare, _) = slack_emoji(&serde_json::json!({"a": "https://x.test/a.png"})).unwrap();
        assert_eq!(bare.len(), 1);
    }
}
