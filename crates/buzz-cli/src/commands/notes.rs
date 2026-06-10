//! `sprout notes` — NIP-23 long-form editable notes (kind:30023).
//!
//! Skill-sharing knowledge base for the team. Notes are parameterized-replaceable
//! events keyed by `(kind=30023, pubkey, d-tag)`; the `d` tag is the human slug.
//!
//! ## Verbs
//! - `notes set --name <s> --title T [--summary S] [--tag t]... --content -`
//!   Idempotent upsert. Read-before-write preserves `published_at` and carries
//!   forward the existing title when `--title` is omitted on an update.
//! - `notes get (--naddr <n> | --name <slug> [--author <ref>] [--latest]) [--content-only]`
//!   `--naddr` / coordinate form is exact. `--name` does a cross-author `#d`
//!   query; >1 hit prints candidates and exits 1, unless `--latest` picks the
//!   most recently updated one. `--latest` conflicts with `--author`/`--naddr`.
//! - `notes ls [--author <ref>] [--tag t] [--limit N]` — own notes by default.
//! - `notes rm --name <s>` — NIP-09 deletion (kind:5) targeting the addressable
//!   coordinate via an `a` tag only (no `e` tag — see [`build_rm_event`]).
//!
//! ## Implementation state
//! `set`/`get`/`ls`/`rm` are implemented. `set` is the write path: stdin handling,
//! read-before-write via [`fetch_own_note`], pure carry-forward via
//! [`build_set_event`], sign + publish, and the durable-reference output line.
//! `get`/`ls` are the read paths (coordinate / cross-author `#d` resolution and
//! bounded listing). `rm` is the delete path: read-before-write for a clear
//! "nothing to delete" signal, then an a-tag-only kind:5 (the relay's #714
//! coordinate soft-delete lands in this same change).

use std::io::Read;
use std::str::FromStr;
use std::time::SystemTime;

use nostr::{Event, EventBuilder, Kind, PublicKey, Tag, Timestamp, ToBech32};

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// NIP-23 long-form content kind.
pub const KIND_LONG_FORM: u16 = 30023;

/// Hard cap on slug length. NIP-23 doesn't bound it; we pick a value that's
/// comfortably URL/filename-safe and matches `mem` slug ergonomics.
pub const SLUG_MAX_LEN: usize = 80;

// ---------------------------------------------------------------------------
// Slug validation
// ---------------------------------------------------------------------------

/// Validate and normalize a slug for use as a NIP-23 `d` tag.
///
/// Rules: 1..=80 chars, `[a-z0-9._-]` only. Lowercase ascii keeps memory
/// pointers and shell-safe filenames trivial; the protocol allows more but
/// being strict here costs us nothing and prevents the "is `Dco-Recipe` the
/// same slug as `dco-recipe`?" ambiguity.
pub fn parse_slug(raw: &str) -> Result<String, CliError> {
    if raw.is_empty() {
        return Err(CliError::Usage("slug cannot be empty".into()));
    }
    if raw.len() > SLUG_MAX_LEN {
        return Err(CliError::Usage(format!(
            "slug too long ({} > {SLUG_MAX_LEN} chars)",
            raw.len()
        )));
    }
    for c in raw.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.');
        if !ok {
            return Err(CliError::Usage(format!(
                "slug contains invalid character {c:?}; allowed: a-z 0-9 - _ ."
            )));
        }
    }
    Ok(raw.to_string())
}

// ---------------------------------------------------------------------------
// NoteSnapshot — parsed view of a kind:30023 event, derived once
// ---------------------------------------------------------------------------

/// Parsed view of a NIP-23 long-form event. Built once via
/// [`NoteSnapshot::from_event`] so the tag-parsing footgun lives in exactly
/// one place; `set` (carry-forward), `get`/`ls` (output shaping), and the
/// ambiguous-candidate path all consume the same derived fields.
#[derive(Debug, Clone)]
pub struct NoteSnapshot {
    /// Event id of this specific incarnation (not the addressable coordinate).
    pub id: nostr::EventId,
    /// Author pubkey.
    pub pubkey: PublicKey,
    /// `d` tag — the slug.
    pub slug: String,
    /// NIP-23 `title` tag (required by spec; empty if author omitted it).
    pub title: String,
    /// NIP-23 `summary` tag.
    pub summary: Option<String>,
    /// Repeated `t` topic tags.
    pub tags: Vec<String>,
    /// NIP-23 `published_at` tag, unix seconds. `None` if absent on this event.
    pub published_at: Option<u64>,
    /// `created_at` of this incarnation — the LWW key for the addressable
    /// coordinate; reasonable proxy for "last updated".
    pub updated_at: u64,
    /// Raw markdown body.
    pub content: String,
}

impl NoteSnapshot {
    /// Parse a kind:30023 event into a snapshot. Returns `Err` if the event
    /// is the wrong kind or missing the mandatory `d` tag.
    pub fn from_event(event: &Event) -> Result<Self, CliError> {
        if event.kind != Kind::Custom(KIND_LONG_FORM) {
            return Err(CliError::Other(format!(
                "expected kind:{KIND_LONG_FORM}, got {}",
                event.kind.as_u16()
            )));
        }

        let mut slug: Option<String> = None;
        let mut title = String::new();
        let mut summary: Option<String> = None;
        let mut tags: Vec<String> = Vec::new();
        let mut published_at: Option<u64> = None;

        for tag in event.tags.iter() {
            let parts = tag.as_slice();
            let Some(name) = parts.first().map(String::as_str) else {
                continue;
            };
            let val = parts.get(1).map(String::as_str).unwrap_or("");
            match name {
                "d" => slug = Some(val.to_string()),
                "title" => title = val.to_string(),
                "summary" => summary = Some(val.to_string()),
                "t" if !val.is_empty() => tags.push(val.to_string()),
                "published_at" => {
                    published_at = val.parse::<u64>().ok();
                }
                _ => {}
            }
        }

        let slug = slug.ok_or_else(|| {
            CliError::Other("kind:30023 event is missing the required `d` tag".into())
        })?;

        Ok(NoteSnapshot {
            id: event.id,
            pubkey: event.pubkey,
            slug,
            title,
            summary,
            tags,
            published_at,
            updated_at: event.created_at.as_secs(),
            content: event.content.clone(),
        })
    }

    /// Canonical addressable coordinate `(kind:30023, pubkey, slug)`.
    pub fn coordinate(&self) -> nostr::nips::nip01::Coordinate {
        coord_for(&self.pubkey, &self.slug)
    }
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str::<Vec<Event>>(json)
        .map_err(|e| CliError::Other(format!("failed to parse relay response: {e}")))
}

/// Read-before-write: fetch the caller's current `(kind:30023, me, d=slug)`
/// event. Returns `Ok(None)` when the slug doesn't exist for the caller.
///
/// Stays protocol-pure (returns a raw `Event`): `set` calls
/// `NoteSnapshot::from_event` on the result for carry-forward, and `rm` only
/// needs its presence to decide first-publish vs. update / deletable-or-not.
/// (Quinn's option (b) — single tag-parser, isolated.)
pub async fn fetch_own_note(client: &SproutClient, slug: &str) -> Result<Option<Event>, CliError> {
    let me = client.keys().public_key();
    let filter = serde_json::json!({
        "kinds": [KIND_LONG_FORM],
        "authors": [me.to_hex()],
        "#d": [slug],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let mut events = parse_events(&raw)?;
    // Defensive: a parameterized-replaceable coordinate has at most one live
    // event, but if the relay returns multiple we take the newest.
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    Ok(events.into_iter().next())
}

/// Cross-author `#d` lookup for `get --name`. The relay pushes the `#d`
/// filter into SQL for NIP-33 kinds (`req.rs`), so this is a single
/// indexed query, not a fan-out.
pub async fn fetch_by_slug(client: &SproutClient, slug: &str) -> Result<Vec<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_LONG_FORM],
        "#d": [slug],
        "limit": 50,
    });
    let raw = client.query(&filter).await?;
    parse_events(&raw)
}

// ---------------------------------------------------------------------------
// Author resolution
// ---------------------------------------------------------------------------

/// Resolve an `--author` flag value to a `PublicKey`.
///
/// Accepts:
/// - `"me"` → the CLI's own keypair.
/// - 64-hex pubkey → parsed directly.
/// - anything else → treated as a petname / display name, searched against
///   kind:0 profiles. Exact-one match required; ambiguity is a hard error.
pub async fn resolve_author(
    client: &SproutClient,
    author_flag: &str,
) -> Result<PublicKey, CliError> {
    if author_flag == "me" {
        return Ok(client.keys().public_key());
    }
    if validate_hex64(author_flag).is_ok() {
        return PublicKey::from_hex(author_flag)
            .map_err(|e| CliError::Usage(format!("invalid pubkey: {e}")));
    }
    // Petname lookup: NIP-50 search on kind:0, then exact-name filter.
    let filter = serde_json::json!({
        "kinds": [0],
        "search": author_flag,
        "limit": 100,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw)?;
    let lower = author_flag.to_ascii_lowercase();
    let matches: Vec<&Event> = events
        .iter()
        .filter(|e| {
            let Ok(meta) = serde_json::from_str::<serde_json::Value>(&e.content) else {
                return false;
            };
            let name = meta
                .get("display_name")
                .or_else(|| meta.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            name.to_ascii_lowercase() == lower
        })
        .collect();
    match matches.len() {
        0 => Err(CliError::Usage(format!(
            "no user found with display_name {author_flag:?}; pass a 64-hex pubkey or \"me\""
        ))),
        1 => Ok(matches[0].pubkey),
        n => Err(CliError::Usage(format!(
            "{n} users match display_name {author_flag:?}; disambiguate with --author <hex-pubkey>"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Coordinate parsing (naddr / kind:pk:d / NIP-21)
// ---------------------------------------------------------------------------

/// Parse a `--naddr` flag. Accepts:
/// - bech32 `naddr1…`
/// - `<kind>:<pubkey-hex>:<d-tag>` (KPI format)
/// - `nostr:naddr1…` (NIP-21 URI)
///
/// Errors if the parsed kind is not 30023 — `notes` doesn't address any other
/// kind, and silently succeeding on (say) a kind:30024 coordinate would just
/// confuse the user later.
pub fn parse_naddr(raw: &str) -> Result<nostr::nips::nip01::Coordinate, CliError> {
    let coord = nostr::nips::nip01::Coordinate::from_str(raw)
        .map_err(|e| CliError::Usage(format!("invalid naddr/coordinate: {e}")))?;
    if coord.kind != Kind::Custom(KIND_LONG_FORM) {
        return Err(CliError::Usage(format!(
            "coordinate kind is {}, expected {KIND_LONG_FORM}",
            coord.kind.as_u16()
        )));
    }
    if coord.identifier.is_empty() {
        return Err(CliError::Usage(
            "kind:30023 coordinate is missing its d-tag/slug".into(),
        ));
    }
    Ok(coord)
}

pub fn coord_for(author: &PublicKey, slug: &str) -> nostr::nips::nip01::Coordinate {
    nostr::nips::nip01::Coordinate::new(Kind::Custom(KIND_LONG_FORM), *author)
        .identifier(slug.to_string())
}

// ---------------------------------------------------------------------------
// Candidate formatting (used when --name resolves to >1 author)
// ---------------------------------------------------------------------------

/// Format a list of candidate notes for the "ambiguous slug" error path.
/// One line per candidate; sorted newest-first. Designed so the user can
/// paste a pubkey into a follow-up `--author <hex>` invocation.
pub fn format_note_candidates(snapshots: &[NoteSnapshot]) -> String {
    let mut rows: Vec<&NoteSnapshot> = snapshots.iter().collect();
    rows.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    let mut out = String::new();
    for s in rows {
        let title = if s.title.is_empty() {
            "(untitled)"
        } else {
            s.title.as_str()
        };
        out.push_str(&format!(
            "  {} {} {}\n",
            s.pubkey.to_hex(),
            s.updated_at,
            title
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Output helpers for `get` / `ls`
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct NoteOutput {
    id: String,
    pubkey: String,
    naddr: String,
    coordinate: String,
    slug: String,
    title: String,
    summary: Option<String>,
    tags: Vec<String>,
    published_at: Option<u64>,
    updated_at: u64,
    content: String,
}

impl TryFrom<&NoteSnapshot> for NoteOutput {
    type Error = CliError;

    fn try_from(snapshot: &NoteSnapshot) -> Result<Self, Self::Error> {
        let coordinate = snapshot.coordinate();
        let naddr = coordinate
            .to_bech32()
            .map_err(|e| CliError::Other(format!("failed to encode naddr: {e}")))?;
        Ok(Self {
            id: snapshot.id.to_hex(),
            pubkey: snapshot.pubkey.to_hex(),
            naddr,
            coordinate: coordinate.to_string(),
            slug: snapshot.slug.clone(),
            title: snapshot.title.clone(),
            summary: snapshot.summary.clone(),
            tags: snapshot.tags.clone(),
            published_at: snapshot.published_at,
            updated_at: snapshot.updated_at,
            content: snapshot.content.clone(),
        })
    }
}

fn snapshot_from_event(event: &Event) -> Result<NoteSnapshot, CliError> {
    NoteSnapshot::from_event(event)
}

fn snapshots_from_events(events: Vec<Event>) -> Result<Vec<NoteSnapshot>, CliError> {
    events
        .iter()
        .map(snapshot_from_event)
        .collect::<Result<Vec<_>, _>>()
}

async fn fetch_by_coord(
    client: &SproutClient,
    coord: &nostr::nips::nip01::Coordinate,
) -> Result<Option<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_LONG_FORM],
        "authors": [coord.public_key.to_hex()],
        "#d": [coord.identifier],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let mut events = parse_events(&raw)?;
    events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
    Ok(events.into_iter().next())
}

fn print_snapshot_json(snapshot: &NoteSnapshot) -> Result<(), CliError> {
    let output = NoteOutput::try_from(snapshot)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&output)
            .map_err(|e| CliError::Other(format!("failed to serialize note: {e}")))?
    );
    Ok(())
}

fn print_snapshot_list_json(snapshots: &[NoteSnapshot]) -> Result<(), CliError> {
    let output = snapshots
        .iter()
        .map(NoteOutput::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&output)
            .map_err(|e| CliError::Other(format!("failed to serialize notes: {e}")))?
    );
    Ok(())
}

fn sort_snapshots_newest_first(snapshots: &mut [NoteSnapshot]) {
    snapshots.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
}

// ---------------------------------------------------------------------------
// Event builder for `set` — pure, unit-testable carry-forward logic
// ---------------------------------------------------------------------------

/// Build the unsigned `EventBuilder` for `notes set`. Pure function — no I/O,
/// no clock — so every carry/clear/first-publish case is unit-testable.
///
/// # Carry-forward semantics (ratified)
/// All three of `title` / `summary` / `tags` use the same omit-vs-clear
/// pattern: `None` means "omit" (carry on update; spec-driven default on
/// create), `Some(empty)` means "explicit clear".
///
/// - `title: None` → carry from `prior.title`; on first publish (`prior=None`)
///   this is a usage error (NIP-23 requires `title`).
/// - `title: Some("")` → explicit clear; emit empty `title` tag.
/// - `title: Some(s)` → use `s`.
/// - `summary`: same shape as `title`, but `None` on first publish is allowed
///   (summary is optional in NIP-23).
/// - `tags: None` → carry from `prior.tags` (on first publish: emit no
///   `t` tags).
/// - `tags: Some(&[])` → explicit clear; emit no `t` tags.
/// - `tags: Some(slice)` → replace existing `t` tags with `slice` verbatim
///   (not merged).
/// - `published_at`: preserved from `prior` if present; otherwise set to `now`
///   on first publish.
///
/// `now` is **Unix seconds** (not millis). Used for the event's `created_at`
/// and, on first publish, for the `published_at` tag value. Injectable so
/// tests can assert `published_at` preservation deterministically.
///
pub fn build_set_event(
    prior: Option<&NoteSnapshot>,
    slug: &str,
    title: Option<&str>,
    summary: Option<&str>,
    tags: Option<&[String]>,
    content: &str,
    now: u64,
) -> Result<EventBuilder, CliError> {
    // Resolve `title` against the carry-forward / clear / require-on-create matrix.
    let title_value: &str = match (title, prior) {
        (Some(t), _) => t,                   // explicit (empty = clear)
        (None, Some(p)) => p.title.as_str(), // carry
        (None, None) => {
            return Err(CliError::Usage(
                "--title is required on first publish (NIP-23)".into(),
            ));
        }
    };

    // `summary` is optional on create, so `None` on first-publish carries no value.
    let summary_value: Option<&str> = match (summary, prior) {
        (Some(s), _) => Some(s),                 // explicit (empty = clear)
        (None, Some(p)) => p.summary.as_deref(), // carry if prior had one
        (None, None) => None,                    // first publish, no summary
    };

    // `tags`: None = carry-on-edit (or empty-on-create); Some(&[]) = clear; Some(slice) = replace.
    let topic_tags: Vec<String> = match (tags, prior) {
        (Some(ts), _) => ts.to_vec(),
        (None, Some(p)) => p.tags.clone(),
        (None, None) => Vec::new(),
    };

    // `published_at`: preserve prior if present, otherwise set to `now` on first publish.
    let published_at: u64 = prior.and_then(|p| p.published_at).unwrap_or(now);

    let mut evt_tags: Vec<Tag> = Vec::with_capacity(4 + topic_tags.len());
    evt_tags.push(Tag::parse(["d", slug]).map_err(tag_err)?);
    evt_tags.push(Tag::parse(["title", title_value]).map_err(tag_err)?);
    if let Some(s) = summary_value {
        evt_tags.push(Tag::parse(["summary", s]).map_err(tag_err)?);
    }
    for t in &topic_tags {
        evt_tags.push(Tag::parse(["t", t.as_str()]).map_err(tag_err)?);
    }
    evt_tags.push(Tag::parse(["published_at", &published_at.to_string()]).map_err(tag_err)?);

    Ok(EventBuilder::new(Kind::Custom(KIND_LONG_FORM), content)
        .tags(evt_tags)
        .custom_created_at(Timestamp::from(now)))
}

fn tag_err(e: impl std::fmt::Display) -> CliError {
    CliError::Other(format!("failed to build tag: {e}"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Bound on stdin reads for `--content -`. NIP-23 doesn't cap event size;
/// this is a guardrail against runaway producers OOMing the CLI. 1 MiB is
/// far above any realistic skill-KB note.
pub const SET_STDIN_MAX_BYTES: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Dispatch — stubs for verb implementations (filled by follow-up commits).
// ---------------------------------------------------------------------------

pub async fn cmd_set(
    client: &SproutClient,
    slug: &str,
    title: Option<&str>,
    summary: Option<&str>,
    tags: Option<&[String]>,
    content: &str,
    allow_empty: bool,
) -> Result<(), CliError> {
    // Resolve `--content -` from stdin with a hard byte cap. Mirrors the
    // `mem set` hygiene: empty stdin without `--allow-empty` is refused
    // (almost always an upstream pipeline failure).
    let body: String = if content == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .take((SET_STDIN_MAX_BYTES as u64) + 1)
            .read_to_string(&mut buf)
            .map_err(|e| CliError::Other(format!("stdin read failed: {e}")))?;
        if buf.len() > SET_STDIN_MAX_BYTES {
            return Err(CliError::Usage(format!(
                "stdin body exceeds {SET_STDIN_MAX_BYTES}-byte limit"
            )));
        }
        if buf.is_empty() && !allow_empty {
            return Err(CliError::Usage(
                "refusing to publish an empty body from stdin (an upstream pipeline step \
                 likely failed). Pass --allow-empty to confirm."
                    .into(),
            ));
        }
        buf
    } else {
        content.to_string()
    };

    // Read-before-write: fetch the current event for (me, slug), if any.
    // We use the raw event from `fetch_own_note` and parse once via
    // `NoteSnapshot::from_event` so the tag-handling lives in exactly one
    // place. `None` here means this is a first-publish.
    let prior_event = fetch_own_note(client, slug).await?;
    let prior_snapshot = prior_event
        .as_ref()
        .map(NoteSnapshot::from_event)
        .transpose()?;

    let builder = build_set_event(
        prior_snapshot.as_ref(),
        slug,
        title,
        summary,
        tags,
        &body,
        now_secs(),
    )?;

    let event = client.sign_event(builder)?;
    let event_id = event.id;
    let me = event.pubkey;
    let raw = client.submit_event(event).await?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e} ({raw})")))?;
    let accepted = parsed
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let message = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    // NIP-33 LWW: the relay accepts but reports `duplicate:` when this write was
    // dominated by a newer (or same-second) head and did NOT become the live
    // note. Surface a Conflict so we don't print success for a write that
    // didn't take. (Mirrors `mem`'s engram submit.)
    if message.starts_with("duplicate:") || message == "duplicate" {
        return Err(CliError::Conflict(
            "relay reported event as duplicate / dominated by a newer head".into(),
        ));
    }

    // Print the durable references the user can paste into memory.
    let coord = coord_for(&me, slug);
    let naddr = coord
        .to_bech32()
        .map_err(|e| CliError::Other(format!("naddr encoding failed: {e}")))?;
    println!("event_id   {}", event_id.to_hex());
    println!("naddr      {naddr}");
    println!("coordinate {KIND_LONG_FORM}:{}:{slug}", me.to_hex());
    println!("slug       {slug}");
    // Resolved title actually written: explicit `--title` wins, else carried
    // from prior. (Printing prior.title unconditionally was wrong on edits.)
    let resolved_title = title
        .or_else(|| prior_snapshot.as_ref().map(|p| p.title.as_str()))
        .unwrap_or_default();
    println!("title      {resolved_title}");
    Ok(())
}

/// Validate the flag combination for `notes get`. Pure (booleans only) so the
/// full matrix is unit-testable without a relay. `--naddr` and `--name` are
/// exclusive-or; `--author` and `--latest` only refine `--name`, and they
/// disambiguate the same multi-author case in opposite ways, so they conflict.
fn validate_get_args(naddr: bool, name: bool, author: bool, latest: bool) -> Result<(), CliError> {
    if naddr == name {
        return Err(CliError::Usage(
            "exactly one of --naddr or --name is required".into(),
        ));
    }
    if naddr && author {
        return Err(CliError::Usage(
            "--author only applies with --name; --naddr already identifies the author".into(),
        ));
    }
    if naddr && latest {
        return Err(CliError::Usage(
            "--latest only applies with --name; --naddr already identifies one note".into(),
        ));
    }
    if author && latest {
        return Err(CliError::Usage(
            "--latest and --author are mutually exclusive".into(),
        ));
    }
    Ok(())
}

pub async fn cmd_get(
    client: &SproutClient,
    naddr: Option<&str>,
    name: Option<&str>,
    author: Option<&str>,
    latest: bool,
    content_only: bool,
) -> Result<(), CliError> {
    let snapshot = if let Some(raw) = naddr {
        let coord = parse_naddr(raw)?;
        let event = fetch_by_coord(client, &coord).await?.ok_or_else(|| {
            CliError::NotFound(format!(
                "note not found: {}:{}:{}",
                coord.kind.as_u16(),
                coord.public_key.to_hex(),
                coord.identifier
            ))
        })?;
        snapshot_from_event(&event)?
    } else {
        let slug = parse_slug(name.expect("dispatch enforces --name xor --naddr"))?;
        if let Some(author_flag) = author {
            let author_pk = resolve_author(client, author_flag).await?;
            let coord = coord_for(&author_pk, &slug);
            let event = fetch_by_coord(client, &coord).await?.ok_or_else(|| {
                CliError::NotFound(format!("note not found: {}/{}", author_pk.to_hex(), slug))
            })?;
            snapshot_from_event(&event)?
        } else {
            let mut snapshots = snapshots_from_events(fetch_by_slug(client, &slug).await?)?;
            match snapshots.len() {
                0 => return Err(CliError::NotFound(format!("note not found: {slug}"))),
                1 => snapshots.remove(0),
                _ => {
                    sort_snapshots_newest_first(&mut snapshots);
                    if latest {
                        snapshots.remove(0)
                    } else {
                        return Err(CliError::Usage(format!(
                            "note name {slug:?} is ambiguous; pass --author <pubkey> or --latest\n{}",
                            format_note_candidates(&snapshots)
                        )));
                    }
                }
            }
        }
    };

    if content_only {
        print!("{}", snapshot.content);
        if !snapshot.content.ends_with('\n') {
            println!();
        }
    } else {
        print_snapshot_json(&snapshot)?;
    }
    Ok(())
}

pub async fn cmd_ls(
    client: &SproutClient,
    author: Option<&str>,
    tag: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    let limit = limit.unwrap_or(50).min(200);
    let author = author.unwrap_or("me");

    let mut filter = serde_json::json!({
        "kinds": [KIND_LONG_FORM],
        "limit": limit,
    });

    if author != "all" {
        let author_pk = resolve_author(client, author).await?;
        filter["authors"] = serde_json::json!([author_pk.to_hex()]);
    }

    if let Some(tag) = tag {
        if tag.is_empty() {
            return Err(CliError::Usage("--tag cannot be empty".into()));
        }
        filter["#t"] = serde_json::json!([tag]);
    }

    let raw = client.query(&filter).await?;
    let mut snapshots = snapshots_from_events(parse_events(&raw)?)?;
    sort_snapshots_newest_first(&mut snapshots);
    print_snapshot_list_json(&snapshots)?;
    Ok(())
}

/// Build the NIP-09 deletion event (kind:5) for an addressable coordinate.
///
/// The deletion carries **only** an `a` tag (`30023:<pubkey>:<slug>`) and no
/// `e` tag. This is load-bearing: the relay routes to its coordinate
/// soft-delete path *only* when the kind:5 has no `e` target ids
/// (`handle_standard_deletion_event` → `handle_a_tag_deletion`). An `e` tag
/// would route to the per-event path and leave the live replaceable row
/// intact — the note would survive the "deletion". Pure and unit-testable.
pub fn build_rm_event(coord: &nostr::nips::nip01::Coordinate) -> Result<EventBuilder, CliError> {
    let a_tag = Tag::parse(["a", &coord.to_string()]).map_err(tag_err)?;
    Ok(EventBuilder::new(Kind::EventDeletion, "").tags(vec![a_tag]))
}

pub async fn cmd_rm(client: &SproutClient, slug: &str) -> Result<(), CliError> {
    // Read-before-write: only the author can delete their own note, and we
    // want a clear "nothing to delete" signal rather than emitting a kind:5
    // for a coordinate that was never published.
    let me = client.keys().public_key();
    if fetch_own_note(client, slug).await?.is_none() {
        return Err(CliError::NotFound(format!(
            "no note {slug:?} found for you ({}); nothing to delete",
            me.to_hex()
        )));
    }

    let coord = coord_for(&me, slug);
    let builder = build_rm_event(&coord)?;
    let event = client.sign_event(builder)?;
    let event_id = event.id;
    let raw = client.submit_event(event).await?;
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e} ({raw})")))?;
    let accepted = parsed
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let message = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if !accepted {
        return Err(CliError::Other(format!(
            "relay rejected deletion: {message}"
        )));
    }

    println!("deleted    {KIND_LONG_FORM}:{}:{slug}", me.to_hex());
    println!("deletion   {}", event_id.to_hex());
    Ok(())
}

pub async fn dispatch(cmd: crate::NotesCmd, client: &SproutClient) -> Result<(), CliError> {
    use crate::NotesCmd;
    match cmd {
        NotesCmd::Set {
            name,
            title,
            summary,
            tags,
            clear_tags,
            content,
            allow_empty,
        } => {
            let slug = parse_slug(&name)?;
            // Map (Vec<String>, --clear-tags) → Option<&[String]>:
            //   --clear-tags         → Some(&[])  (explicit clear)
            //   --tag a --tag b      → Some(&["a","b"])  (replace)
            //   neither              → None  (carry on update, empty on create)
            // `--clear-tags` + any `--tag` is contradictory; reject loudly.
            if clear_tags && !tags.is_empty() {
                return Err(CliError::Usage(
                    "--clear-tags is mutually exclusive with --tag; pick one".into(),
                ));
            }
            let tags_arg: Option<&[String]> = if clear_tags {
                Some(&[])
            } else if tags.is_empty() {
                None
            } else {
                Some(&tags)
            };
            cmd_set(
                client,
                &slug,
                title.as_deref(),
                summary.as_deref(),
                tags_arg,
                &content,
                allow_empty,
            )
            .await
        }
        NotesCmd::Get {
            naddr,
            name,
            author,
            latest,
            content_only,
        } => {
            validate_get_args(naddr.is_some(), name.is_some(), author.is_some(), latest)?;
            cmd_get(
                client,
                naddr.as_deref(),
                name.as_deref(),
                author.as_deref(),
                latest,
                content_only,
            )
            .await
        }
        NotesCmd::Ls { author, tag, limit } => {
            cmd_ls(client, author.as_deref(), tag.as_deref(), limit).await
        }
        NotesCmd::Rm { name } => {
            let slug = parse_slug(&name)?;
            cmd_rm(client, &slug).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slug_accepts_dco_recipe() {
        assert_eq!(parse_slug("dco-recipe").unwrap(), "dco-recipe");
    }

    #[test]
    fn parse_slug_accepts_dots_and_underscores() {
        assert!(parse_slug("v1.2_notes").is_ok());
    }

    #[test]
    fn parse_slug_rejects_empty() {
        assert!(matches!(parse_slug(""), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_slug_rejects_uppercase() {
        let err = parse_slug("DCO-Recipe").unwrap_err();
        assert!(matches!(err, CliError::Usage(msg) if msg.contains("invalid character")));
    }

    #[test]
    fn parse_slug_rejects_spaces() {
        assert!(matches!(parse_slug("dco recipe"), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_slug_rejects_overlong() {
        let s = "a".repeat(SLUG_MAX_LEN + 1);
        assert!(matches!(parse_slug(&s), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_naddr_rejects_wrong_kind() {
        // kind:1 with a fake pubkey + slug — well-formed KPI, wrong kind.
        let pk = "0000000000000000000000000000000000000000000000000000000000000001";
        let err = parse_naddr(&format!("1:{pk}:hello")).unwrap_err();
        assert!(matches!(err, CliError::Usage(msg) if msg.contains("expected 30023")));
    }

    #[test]
    fn parse_naddr_accepts_kpi_form() {
        let pk = "0000000000000000000000000000000000000000000000000000000000000001";
        let c = parse_naddr(&format!("30023:{pk}:my-note")).expect("parse");
        assert_eq!(c.identifier, "my-note");
        assert_eq!(c.kind.as_u16(), 30023);
    }

    // -- NoteSnapshot --

    use nostr::{EventBuilder, Keys, Tag, Timestamp};

    fn build_30023(
        keys: &Keys,
        ts: u64,
        slug: &str,
        title: &str,
        extra: Vec<Tag>,
        content: &str,
    ) -> Event {
        let mut tags = vec![
            Tag::parse(["d", slug]).unwrap(),
            Tag::parse(["title", title]).unwrap(),
        ];
        tags.extend(extra);
        EventBuilder::new(Kind::Custom(KIND_LONG_FORM), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(ts))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn note_snapshot_parses_all_standard_tags() {
        let keys = Keys::generate();
        let event = build_30023(
            &keys,
            2_000,
            "my-slug",
            "My Title",
            vec![
                Tag::parse(["summary", "a short summary"]).unwrap(),
                Tag::parse(["t", "rust"]).unwrap(),
                Tag::parse(["t", "cli"]).unwrap(),
                Tag::parse(["published_at", "1700000000"]).unwrap(),
            ],
            "# body",
        );
        let snap = NoteSnapshot::from_event(&event).expect("parse");
        assert_eq!(snap.slug, "my-slug");
        assert_eq!(snap.title, "My Title");
        assert_eq!(snap.summary.as_deref(), Some("a short summary"));
        assert_eq!(snap.tags, vec!["rust".to_string(), "cli".to_string()]);
        assert_eq!(snap.published_at, Some(1_700_000_000));
        assert_eq!(snap.updated_at, 2_000);
        assert_eq!(snap.content, "# body");
        assert_eq!(snap.id, event.id);
        assert_eq!(snap.pubkey, event.pubkey);
    }

    #[test]
    fn note_snapshot_missing_d_tag_is_err() {
        let keys = Keys::generate();
        // Synthesize an event without a `d` tag — has to be done with EventBuilder
        // directly since `build_30023` always inserts one.
        let event = EventBuilder::new(Kind::Custom(KIND_LONG_FORM), "body")
            .tags(vec![Tag::parse(["title", "no-d"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let err = NoteSnapshot::from_event(&event).unwrap_err();
        assert!(matches!(err, CliError::Other(m) if m.contains("missing the required `d` tag")));
    }

    #[test]
    fn note_snapshot_rejects_wrong_kind() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hi")
            .tags(vec![Tag::parse(["d", "ignored"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let err = NoteSnapshot::from_event(&event).unwrap_err();
        assert!(matches!(err, CliError::Other(m) if m.contains("expected kind:30023")));
    }

    #[test]
    fn note_snapshot_garbage_published_at_yields_none() {
        // We don't want a malformed `published_at` to fail the entire parse;
        // it should fall back to `None` so the carry-forward logic treats it
        // as "no prior value" and sets a fresh one.
        let keys = Keys::generate();
        let event = build_30023(
            &keys,
            1_000,
            "x",
            "T",
            vec![Tag::parse(["published_at", "not-a-number"]).unwrap()],
            "",
        );
        let snap = NoteSnapshot::from_event(&event).unwrap();
        assert_eq!(snap.published_at, None);
    }

    #[test]
    fn format_note_candidates_sorts_newest_first() {
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let older =
            NoteSnapshot::from_event(&build_30023(&keys_a, 1_000, "shared", "older", vec![], ""))
                .unwrap();
        let newer =
            NoteSnapshot::from_event(&build_30023(&keys_b, 2_000, "shared", "newer", vec![], ""))
                .unwrap();
        let out = format_note_candidates(&[older, newer]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("newer"));
        assert!(lines[1].contains("older"));
    }

    #[test]
    fn format_note_candidates_uses_untitled_for_empty_title() {
        let keys = Keys::generate();
        let snap =
            NoteSnapshot::from_event(&build_30023(&keys, 1_000, "x", "", vec![], "")).unwrap();
        let out = format_note_candidates(&[snap]);
        assert!(out.contains("(untitled)"));
    }

    // -- build_set_event --
    //
    // We can't introspect an unsigned `EventBuilder` for its tags, so each
    // test signs with a throwaway key and inspects the resulting `Event`.
    // The signature is irrelevant — we're asserting the tag set and timing.

    /// Sign `build_set_event` output with a fresh key and return the event
    /// for tag inspection. `now` and signing key are local to the test;
    /// nothing about the result depends on the wall clock.
    fn build_and_sign(
        prior: Option<&NoteSnapshot>,
        slug: &str,
        title: Option<&str>,
        summary: Option<&str>,
        tags: Option<&[String]>,
        content: &str,
        now: u64,
    ) -> Result<Event, CliError> {
        let builder = build_set_event(prior, slug, title, summary, tags, content, now)?;
        let keys = Keys::generate();
        Ok(builder.sign_with_keys(&keys).unwrap())
    }

    fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
        event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(String::as_str) == Some(name))
            .and_then(|t| t.as_slice().get(1).map(String::as_str))
    }

    fn t_tags(event: &Event) -> Vec<&str> {
        event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(String::as_str) == Some("t"))
            .filter_map(|t| t.as_slice().get(1).map(String::as_str))
            .collect()
    }

    fn prior_snapshot(
        ts: u64,
        slug: &str,
        title: &str,
        summary: Option<&str>,
        tags: &[&str],
        published_at: Option<u64>,
        content: &str,
    ) -> NoteSnapshot {
        let keys = Keys::generate();
        let mut extra: Vec<Tag> = Vec::new();
        if let Some(s) = summary {
            extra.push(Tag::parse(["summary", s]).unwrap());
        }
        for t in tags {
            extra.push(Tag::parse(["t", t]).unwrap());
        }
        if let Some(p) = published_at {
            extra.push(Tag::parse(["published_at", &p.to_string()]).unwrap());
        }
        NoteSnapshot::from_event(&build_30023(&keys, ts, slug, title, extra, content)).unwrap()
    }

    #[test]
    fn set_first_publish_requires_title() {
        // No prior, no --title → usage error (NIP-23 mandates `title`).
        let err = build_set_event(None, "x", None, None, None, "body", 1_000).unwrap_err();
        assert!(matches!(err, CliError::Usage(m) if m.contains("title is required")));
    }

    #[test]
    fn set_first_publish_sets_published_at_to_now() {
        let event =
            build_and_sign(None, "x", Some("Hello"), None, None, "body", 1_700_000_000).unwrap();
        assert_eq!(tag_value(&event, "title"), Some("Hello"));
        assert_eq!(tag_value(&event, "d"), Some("x"));
        assert_eq!(tag_value(&event, "published_at"), Some("1700000000"));
        assert_eq!(event.created_at.as_secs(), 1_700_000_000);
        // No `summary` tag should be present when none was specified.
        assert!(tag_value(&event, "summary").is_none());
        assert!(t_tags(&event).is_empty());
    }

    #[test]
    fn set_update_preserves_published_at_and_carries_title() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "Original Title",
            None,
            &[],
            Some(1_650_000_000),
            "old body",
        );
        // Omit --title; `now` advances.
        let event = build_and_sign(
            Some(&prior),
            "x",
            None,
            None,
            None,
            "new body",
            1_700_001_000,
        )
        .unwrap();
        assert_eq!(tag_value(&event, "title"), Some("Original Title"));
        assert_eq!(tag_value(&event, "published_at"), Some("1650000000"));
        assert_eq!(event.created_at.as_secs(), 1_700_001_000);
        assert_eq!(event.content, "new body");
    }

    #[test]
    fn set_update_clears_title_when_explicit_empty() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "Old",
            None,
            &[],
            Some(1_650_000_000),
            "",
        );
        let event = build_and_sign(
            Some(&prior),
            "x",
            Some(""),
            None,
            None,
            "body",
            1_700_001_000,
        )
        .unwrap();
        assert_eq!(tag_value(&event, "title"), Some(""));
    }

    #[test]
    fn set_update_carries_tags_when_omitted() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "T",
            None,
            &["rust", "cli"],
            Some(1_650_000_000),
            "",
        );
        // None = omit; expect prior tags carried through.
        let event =
            build_and_sign(Some(&prior), "x", None, None, None, "body", 1_700_001_000).unwrap();
        let mut got = t_tags(&event);
        got.sort();
        assert_eq!(got, vec!["cli", "rust"]);
    }

    #[test]
    fn set_update_clears_tags_when_explicit_empty_slice() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "T",
            None,
            &["rust", "cli"],
            Some(1_650_000_000),
            "",
        );
        let event = build_and_sign(
            Some(&prior),
            "x",
            None,
            None,
            Some(&[]),
            "body",
            1_700_001_000,
        )
        .unwrap();
        assert!(t_tags(&event).is_empty());
    }

    #[test]
    fn set_update_replaces_tags_when_provided() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "T",
            None,
            &["old1", "old2"],
            Some(1_650_000_000),
            "",
        );
        let new = ["fresh".to_string(), "tags".to_string()];
        let event = build_and_sign(
            Some(&prior),
            "x",
            None,
            None,
            Some(&new),
            "body",
            1_700_001_000,
        )
        .unwrap();
        let mut got = t_tags(&event);
        got.sort();
        assert_eq!(got, vec!["fresh", "tags"]);
    }

    #[test]
    fn set_update_carries_summary_when_omitted() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "T",
            Some("old summary"),
            &[],
            Some(1_650_000_000),
            "",
        );
        let event =
            build_and_sign(Some(&prior), "x", None, None, None, "body", 1_700_001_000).unwrap();
        assert_eq!(tag_value(&event, "summary"), Some("old summary"));
    }

    #[test]
    fn set_update_clears_summary_when_explicit_empty() {
        let prior = prior_snapshot(
            1_700_000_000,
            "x",
            "T",
            Some("old summary"),
            &[],
            Some(1_650_000_000),
            "",
        );
        let event = build_and_sign(
            Some(&prior),
            "x",
            None,
            Some(""),
            None,
            "body",
            1_700_001_000,
        )
        .unwrap();
        // Explicit clear: an empty `summary` tag is emitted (vs no tag).
        assert_eq!(tag_value(&event, "summary"), Some(""));
    }

    // -- build_rm_event --
    //
    // The relay only honours an addressable (a-tag) deletion when the kind:5
    // carries no `e` target ids. These tests pin both halves of that contract:
    // exactly one `a` tag with the right coordinate, and zero `e` tags.

    #[test]
    fn rm_event_is_kind5_with_a_tag_only() {
        let keys = Keys::generate();
        let coord = coord_for(&keys.public_key(), "dco-recipe");
        let event = build_rm_event(&coord)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(event.kind, Kind::EventDeletion);

        let a_tags: Vec<&str> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(String::as_str) == Some("a"))
            .filter_map(|t| t.as_slice().get(1).map(String::as_str))
            .collect();
        assert_eq!(
            a_tags,
            vec![format!(
                "{KIND_LONG_FORM}:{}:dco-recipe",
                keys.public_key().to_hex()
            )]
        );

        // Load-bearing: an `e` tag would route the relay to the per-event
        // delete path and leave the replaceable coordinate row alive.
        assert!(
            !event
                .tags
                .iter()
                .any(|t| t.as_slice().first().map(String::as_str) == Some("e")),
            "rm deletion must not carry an `e` tag"
        );
    }

    #[test]
    fn set_first_publish_with_no_prior_published_at_uses_now_even_after_a_garbage_prior() {
        // Edge case: an existing event whose `published_at` was malformed
        // would round-trip to `published_at: None`. On the next `set`, we
        // treat that as "no prior `published_at`" and stamp `now`.
        let prior = prior_snapshot(1_000, "x", "T", None, &[], None, "");
        let event = build_and_sign(Some(&prior), "x", None, None, None, "body", 2_000).unwrap();
        assert_eq!(tag_value(&event, "published_at"), Some("2000"));
    }

    // -- validate_get_args --

    #[test]
    fn validate_get_args_accepts_minimal_forms() {
        // (naddr, name, author, latest)
        assert!(validate_get_args(true, false, false, false).is_ok()); // --naddr
        assert!(validate_get_args(false, true, false, false).is_ok()); // --name
        assert!(validate_get_args(false, true, true, false).is_ok()); // --name --author
        assert!(validate_get_args(false, true, false, true).is_ok()); // --name --latest
    }

    #[test]
    fn validate_get_args_requires_exactly_one_selector() {
        let neither = validate_get_args(false, false, false, false);
        let both = validate_get_args(true, true, false, false);
        for err in [neither, both] {
            assert!(matches!(err, Err(CliError::Usage(m)) if m.contains("exactly one")));
        }
    }

    #[test]
    fn validate_get_args_rejects_naddr_with_refiners() {
        assert!(matches!(
            validate_get_args(true, false, true, false),
            Err(CliError::Usage(m)) if m.contains("--author only applies with --name")
        ));
        assert!(matches!(
            validate_get_args(true, false, false, true),
            Err(CliError::Usage(m)) if m.contains("--latest only applies with --name")
        ));
    }

    #[test]
    fn validate_get_args_rejects_author_and_latest_together() {
        assert!(matches!(
            validate_get_args(false, true, true, true),
            Err(CliError::Usage(m)) if m.contains("mutually exclusive")
        ));
    }
}
