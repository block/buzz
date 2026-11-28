//! `buzz mem` — agent-side engram management (NIP-AE).
//!
//! Subcommands:
//! - `buzz mem ls`                   — list non-tombstoned memories
//! - `buzz mem get <slug>`            — print the value to stdout
//! - `buzz mem hash <slug>`           — print sha256(value) hex
//! - `buzz mem set <slug> <value|-> ` — write a value (use `-` for stdin)
//! - `buzz mem patch <slug>`          — apply a unified diff to the current value
//! - `buzz mem rm <slug>`             — publish a tombstone
//!
//! By default, the caller's `BUZZ_PRIVATE_KEY` is the agent's nsec. The
//! agent's owner pubkey is resolved from `BUZZ_AUTH_TAG` (NIP-OA attestation)
//! or the `--owner` flag. Read commands also support owner-side recovery via
//! `--agent <pubkey>`: the CLI identity is treated as the owner and decrypts
//! the agent's engrams through the same agent↔owner NIP-44 conversation key.

use std::io::Read;
use std::time::SystemTime;

use serde::Serialize;
use sha2::{Digest, Sha256};

use buzz_core::engram::{self, normalize_slug, select_head, Body, Listing};
use buzz_core::kind::KIND_AGENT_ENGRAM;
use nostr::PublicKey;

use crate::client::BuzzClient;
use crate::error::CliError;

/// Resolve the agent's owner pubkey: explicit `--owner` flag wins, otherwise
/// fall back to the NIP-OA `auth_tag` (which carries owner pubkey in slot 1).
fn resolve_owner(client: &BuzzClient, owner_flag: Option<&str>) -> Result<PublicKey, CliError> {
    if let Some(s) = owner_flag {
        return PublicKey::from_hex(s)
            .map_err(|e| CliError::Usage(format!("--owner must be a 64-hex pubkey: {e}")));
    }
    let tag = client.auth_tag_owner_hex().ok_or_else(|| {
        CliError::Usage(
            "owner pubkey required (set BUZZ_AUTH_TAG with a NIP-OA attestation or pass --owner)"
                .into(),
        )
    })?;
    PublicKey::from_hex(&tag)
        .map_err(|e| CliError::Other(format!("auth_tag owner pubkey is not valid hex: {e}")))
}

/// Resolve the read perspective for `mem ls/get/hash`.
///
/// Normal agent-side reads use the CLI identity as the agent and resolve the
/// owner from `--owner` / BUZZ_AUTH_TAG. Owner-side recovery passes
/// `--agent <pubkey>`; the CLI identity is then the owner and the supplied
/// pubkey is the agent author to query/decrypt.
fn resolve_reader(
    client: &BuzzClient,
    owner_flag: Option<&str>,
    agent_flag: Option<&str>,
) -> Result<(PublicKey, PublicKey, PublicKey), CliError> {
    if let Some(agent) = agent_flag {
        if owner_flag.is_some() {
            return Err(CliError::Usage(
                "--owner and --agent are mutually exclusive for read commands".into(),
            ));
        }
        let agent = PublicKey::from_hex(agent)
            .map_err(|e| CliError::Usage(format!("--agent must be a 64-hex pubkey: {e}")))?;
        if agent == client.public_key() {
            return Err(CliError::Usage(
                "--agent must differ from the CLI identity; omit --agent for agent-side reads"
                    .into(),
            ));
        }
        return Ok((agent, client.public_key(), agent));
    }

    let agent = client.public_key();
    let owner = resolve_owner(client, owner_flag)?;
    Ok((agent, owner, owner))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Submit a signed engram event and confirm the relay treated it as
/// authoritative. The relay returns `{accepted, message}` where the
/// `message` field starts with `"duplicate:"` when the write was rejected
/// as already-superseded by a later head (NIP-33 LWW). In that case we
/// surface a `Conflict` so callers don't lie about success.
async fn submit_engram(client: &BuzzClient, event: nostr::Event) -> Result<(), CliError> {
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
    if message.starts_with("duplicate:") || message == "duplicate" {
        return Err(CliError::Conflict(
            "relay reported event as duplicate / dominated by a newer head".into(),
        ));
    }
    Ok(())
}

/// Parse a relay-response JSON array of events.
fn parse_events(json: &str) -> Result<Vec<nostr::Event>, CliError> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| CliError::Other(format!("relay returned invalid JSON: {e}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| CliError::Other("relay response is not an array".into()))?;
    // Per NIP-AE head selection: discard events that fail any validation
    // step and pick the head from the survivors. A single garbled response
    // entry must not deny-of-service the whole listing.
    let mut out = Vec::with_capacity(arr.len());
    for ev in arr {
        // Skip any event that fails to deserialize. Downstream validation
        // (signature, decrypt, slug↔d) will discard further bad apples; we
        // never want a single corrupt record to fail `mem ls`/`mem get`.
        if let Ok(event) = serde_json::from_value::<nostr::Event>(ev.clone()) {
            out.push(event);
        }
    }
    Ok(out)
}

/// NIP-AE read validation through the client's credentials: the envelope,
/// then NIP-44 decryption, then the slug re-deriving to the `d` tag.
/// `Ok(None)` means the event is not a valid engram and is skipped; `Err` is
/// a credential failure, which aborts rather than reading as "no entries".
async fn open_engram(
    client: &BuzzClient,
    ev: &nostr::Event,
    agent: &PublicKey,
    owner: &PublicKey,
    their_pubkey: &PublicKey,
) -> Result<Option<Body>, CliError> {
    let Ok(d_value) = engram::validate_envelope(ev, agent, owner) else {
        return Ok(None);
    };
    let credentials = client.credentials();
    let Some(plaintext) = credentials.nip44_decrypt(their_pubkey, &ev.content).await? else {
        return Ok(None);
    };
    let Ok(body) = Body::from_json_bytes(plaintext.as_bytes()) else {
        return Ok(None);
    };
    let derived = credentials.engram_d_tag(their_pubkey, body.slug()).await?;
    Ok(engram::check_d_tag(&d_value, &derived).ok().map(|()| body))
}

/// Encrypt `body` to `owner` and sign the `kind:30174` event with the
/// client's credentials (NIP-AE write path).
async fn seal_engram(
    client: &BuzzClient,
    owner: &PublicKey,
    body: &Body,
    created_at: u64,
) -> Result<nostr::Event, CliError> {
    let build_failed = |e: engram::EngramError| CliError::Other(format!("build event failed: {e}"));
    let credentials = client.credentials();
    let plaintext = engram::encode_body(body).map_err(build_failed)?;
    let ciphertext = credentials.nip44_encrypt(owner, &plaintext).await?;
    let d = credentials.engram_d_tag(owner, body.slug()).await?;
    let builder = engram::event_builder(owner, &d, ciphertext, created_at).map_err(build_failed)?;
    credentials.sign(builder).await
}

/// Fetch the head event for `slug`, returning `(Option<Event>, Option<Body>)`.
async fn fetch_head(
    client: &BuzzClient,
    agent: &PublicKey,
    owner: &PublicKey,
    slug: &str,
) -> Result<(Option<nostr::Event>, Option<Body>), CliError> {
    let their_pubkey = if client.public_key() == *agent {
        owner
    } else {
        agent
    };
    let d = client
        .credentials()
        .engram_d_tag(their_pubkey, slug)
        .await?;

    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent.to_hex()],
        "#d": [d],
        "#p": [owner.to_hex()],
        "limit": 16,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw)?;

    let mut valid_with_body: Vec<(nostr::Event, Body)> = Vec::new();
    for ev in events {
        // Signature is validated by NIP-01 / NIP-44 — `nostr::Event::verify` is
        // the conservative belt-and-suspenders check before decrypting.
        if ev.verify().is_err() {
            continue;
        }
        if let Some(body) = open_engram(client, &ev, agent, owner, their_pubkey).await? {
            valid_with_body.push((ev, body));
        }
    }
    if valid_with_body.is_empty() {
        return Ok((None, None));
    }
    let events: Vec<nostr::Event> = valid_with_body.iter().map(|(e, _)| e.clone()).collect();
    // `select_head` returns `None` only on an empty iterator; we guarded
    // that above, so the head is always present.
    let Some(head) = select_head(events) else {
        return Ok((None, None));
    };
    let body = valid_with_body
        .into_iter()
        .find(|(e, _)| e.id == head.id)
        .map(|(_, b)| b);
    Ok((Some(head), body))
}

/// `buzz mem ls` — list non-tombstoned memory entries.
///
/// The listing excludes the `core` profile by specification (NIP-AE: `core`
/// is the agent identity surface, not a listed entry). Pass `--include-core`
/// to see it anyway.
pub async fn cmd_ls(
    client: &BuzzClient,
    owner_flag: Option<&str>,
    agent_flag: Option<&str>,
    json: bool,
    include_core: bool,
) -> Result<(), CliError> {
    let (agent, owner, their_pubkey) = resolve_reader(client, owner_flag, agent_flag)?;

    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent.to_hex()],
        "#p": [owner.to_hex()],
        "limit": 5000,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw)?;

    // Validate + decrypt + group by d tag.
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<(nostr::Event, Body)>> = HashMap::new();
    for ev in events {
        if ev.verify().is_err() {
            continue;
        }
        let Some(d_value) = ev
            .tags
            .iter()
            .find(|t| t.kind().to_string() == "d")
            .and_then(|t| t.content())
            .map(|s| s.to_string())
        else {
            continue;
        };
        let Some(body) = open_engram(client, &ev, &agent, &owner, &their_pubkey).await? else {
            continue;
        };
        groups.entry(d_value).or_default().push((ev, body));
    }

    let mut listings: Vec<Listing> = Vec::new();
    for (_d, members) in groups {
        let events: Vec<nostr::Event> = members.iter().map(|(e, _)| e.clone()).collect();
        let Some(head) = select_head(events) else {
            continue;
        };
        // `select_head` returns one of the events it received, so the head
        // is always present in `members`. If something pathological breaks
        // that invariant, skip the group rather than panic.
        let Some((_, body)) = members.into_iter().find(|(e, _)| e.id == head.id) else {
            continue;
        };
        // Drop tombstones. The `core` entry is excluded by spec (listing
        // excludes core) unless the operator asked for it explicitly.
        if let Some(entry) = listing_entry(&body, &head, &owner, include_core) {
            listings.push(entry);
        }
    }
    listings.sort_by(|a, b| a.slug.cmp(&b.slug));

    if json {
        println!("{}", serde_json::to_string(&listings).unwrap_or_default());
    } else if listings.is_empty() {
        eprintln!("(no memories besides core)");
    } else {
        for l in &listings {
            println!("{}\t{}\t{}", l.slug, l.created_at, l.event_id);
        }
    }
    Ok(())
}

/// The listing entry for a head, or `None` when the head is not listed.
///
/// A tombstone is never listed. The `core` profile is excluded by
/// specification — `core` is the agent identity surface, not a listed entry —
/// unless `include_core` is set.
fn listing_entry(
    body: &Body,
    head: &nostr::Event,
    owner: &PublicKey,
    include_core: bool,
) -> Option<Listing> {
    match body {
        Body::Core { .. } => {
            if !include_core {
                return None;
            }
        }
        Body::Memory { value: None, .. } => return None,
        Body::Memory { .. } => {}
    }
    Some(Listing {
        slug: body.slug().to_string(),
        event_id: head.id.to_hex(),
        created_at: head.created_at.as_secs(),
        author: head.pubkey.to_hex(),
        owner: owner.to_hex(),
    })
}

/// The JSON shape of `buzz mem get --json`: the value plus the provenance of
/// the copy that was returned — which event it came from, which agent
/// authored it, which owner it is addressed to and when it was written.
#[derive(Debug, Serialize)]
struct GetOutput {
    /// The slug that was read.
    slug: String,
    /// The stored value (the `core` profile for the `core` slug).
    value: String,
    /// Event id of the head that was read.
    event_id: String,
    /// Pubkey of the agent that authored the head.
    author: String,
    /// Owner pubkey the head is addressed to.
    owner: String,
    /// `created_at` of the head.
    created_at: u64,
}

/// Build the JSON shape of a read: the value plus the provenance of the copy
/// that was returned — which event it came from, which agent authored it,
/// which owner it is addressed to and when it was written.
fn get_output_json(
    slug: &str,
    value: &str,
    head: Option<&nostr::Event>,
    owner: &PublicKey,
) -> Result<String, CliError> {
    let (event_id, author, created_at) = match head {
        Some(ev) => (ev.id.to_hex(), ev.pubkey.to_hex(), ev.created_at.as_secs()),
        None => (String::new(), String::new(), 0),
    };
    let out = GetOutput {
        slug: slug.to_string(),
        value: value.to_string(),
        event_id,
        author,
        owner: owner.to_hex(),
        created_at,
    };
    serde_json::to_string(&out).map_err(|e| CliError::Other(e.to_string()))
}

/// `buzz mem get <slug>` — print value (memory) or profile (core) to stdout.
///
/// Exit codes: 0 on found, 1 on absent or tombstoned.
pub async fn cmd_get(
    client: &BuzzClient,
    raw_slug: &str,
    owner_flag: Option<&str>,
    agent_flag: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    let (agent, owner, _) = resolve_reader(client, owner_flag, agent_flag)?;
    let (head, body) = fetch_head(client, &agent, &owner, &slug).await?;
    use std::io::Write;
    match body {
        None => Err(CliError::NotFound(format!("not found: {slug}"))),
        Some(Body::Memory { value: None, .. }) => {
            Err(CliError::NotFound(format!("tombstoned: {slug}")))
        }
        Some(Body::Memory { value: Some(v), .. }) => {
            if json {
                println!("{}", get_output_json(&slug, &v, head.as_ref(), &owner)?);
                return Ok(());
            }
            // Raw stdout, no trailing newline — round-trips with `buzz mem set foo -`.
            std::io::stdout()
                .write_all(v.as_bytes())
                .map_err(|e| CliError::Other(e.to_string()))
        }
        Some(Body::Core { profile }) => {
            if json {
                println!(
                    "{}",
                    get_output_json(&slug, &profile, head.as_ref(), &owner)?
                );
                return Ok(());
            }
            std::io::stdout()
                .write_all(profile.as_bytes())
                .map_err(|e| CliError::Other(e.to_string()))
        }
    }
}

/// `buzz mem set <slug> <value|->` — write a value or core profile.
///
/// Pass `-` to read the value from stdin.
///
/// Guardrail: when reading from stdin, an empty read is **rejected** unless
/// `--allow-empty` is passed. This catches the common failure mode where an
/// upstream pipeline step errors out, closes its stdout, and `mem set` would
/// otherwise commit an empty value — silently destroying the slug.
/// A literal `""` positional argument is still accepted (explicit intent).
pub async fn cmd_set(
    client: &BuzzClient,
    raw_slug: &str,
    raw_value: &str,
    owner_flag: Option<&str>,
    allow_empty: bool,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    let value = if raw_value == "-" {
        // Bound the stdin read so a runaway producer can't OOM us. We allow
        // one extra byte over the NIP-44 plaintext cap so the build step can
        // surface an exact `BodyTooLarge` if the cap is breached.
        let limit = engram::NIP44_PLAINTEXT_MAX + 1;
        let mut buf = String::new();
        std::io::stdin()
            .take(limit as u64)
            .read_to_string(&mut buf)
            .map_err(|e| CliError::Other(format!("stdin read failed: {e}")))?;
        if buf.len() > engram::NIP44_PLAINTEXT_MAX {
            return Err(CliError::Usage(format!(
                "stdin value exceeds {}-byte NIP-44 plaintext limit",
                engram::NIP44_PLAINTEXT_MAX
            )));
        }
        if buf.is_empty() && !allow_empty {
            return Err(CliError::Usage(
                "refusing to write empty value from stdin (an upstream pipeline step likely \
                 failed). Pass --allow-empty to confirm, or use `buzz mem rm <slug>` to \
                 tombstone."
                    .into(),
            ));
        }
        buf
    } else {
        raw_value.to_string()
    };
    let owner = resolve_owner(client, owner_flag)?;
    let body = if slug == engram::CORE_SLUG {
        Body::Core { profile: value }
    } else {
        Body::Memory {
            slug: slug.clone(),
            value: Some(value),
        }
    };
    let agent_pubkey = client.public_key();
    let (head, _) = fetch_head(client, &agent_pubkey, &owner, &slug).await?;
    let prior_created_at = head.map(|e| e.created_at.as_secs());
    let created_at = engram::monotonic_created_at(now_secs(), prior_created_at);

    let event = seal_engram(client, &owner, &body, created_at).await?;
    let id = event.id.to_hex();
    submit_engram(client, event).await?;
    eprintln!("wrote {slug} (event {id}, created_at {created_at})");
    Ok(())
}

/// Compute the canonical hex-encoded SHA-256 of a UTF-8 string. Matches
/// `printf '%s' "$value" | sha256sum`, so operators can verify base-hash
/// from the shell.
fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// Verify that each hunk's preimage lines (Context + Delete) match the
/// current value byte-for-byte starting at the line number the hunk declares.
///
/// Diffy's `apply` is strict on context content but will *slide* a hunk
/// forward or backward through the file to find a position where the
/// preimage matches. For memory-edit safety we want the stronger property:
/// the patch must apply at exactly the line number it was generated against.
/// Drift in line numbers usually means lines were inserted or deleted before
/// the hunk — at which point regenerating the patch is the correct response,
/// not silently landing the change at a different position.
///
/// Returns `Ok(())` on a clean match, `Err(message)` otherwise.
///
/// Line-number convention: unified-diff `@@ -N,M @@` uses 1-based line
/// numbers. A pure-insertion hunk against an empty file is encoded as
/// `@@ -0,0 +1,M @@` (`start == 0`, `len == 0`), which we treat as
/// "apply at index 0 of an empty preimage."
fn verify_hunks_at_declared_position(
    current: &str,
    patch: &diffy::Patch<'_, str>,
) -> Result<(), String> {
    // `split_inclusive('\n')` preserves the trailing newline on each line,
    // matching diffy's own line representation. A value with no trailing
    // newline produces a last segment with no `\n`, which also matches how
    // diffy stores the "no newline at EOF" case (parser strips the `\n`).
    let current_lines: Vec<&str> = current.split_inclusive('\n').collect();

    for (i, hunk) in patch.hunks().iter().enumerate() {
        let preimage: Vec<&str> = hunk
            .lines()
            .iter()
            .filter_map(|l| match l {
                diffy::Line::Context(s) | diffy::Line::Delete(s) => Some(*s),
                diffy::Line::Insert(_) => None,
            })
            .collect();

        // Pure insertion at start of empty file: `@@ -0,0 +1,M @@`.
        //
        // Known limitation: a pure-insertion hunk into a non-empty value
        // (`@@ -N,0 +N,M @@` with `N > 0`) is currently rejected. With no
        // preimage lines there's nothing to position-check against, and the
        // safe-default for a strict mode is "refuse" rather than "land at an
        // unverified position." `diff -u` includes context lines by default,
        // so users hit this only if they hand-author a no-context insertion.
        // Failure mode is rejection, not corruption — see PR #627 review.
        if preimage.is_empty() {
            if hunk.old_range().start() == 0 {
                continue;
            }
            return Err(format!(
                "hunk #{} has empty preimage at line {}; \
                 pure no-context insertions into non-empty values are not \
                 supported (regenerate the patch with `diff -u` to include \
                 surrounding context)",
                i + 1,
                hunk.old_range().start()
            ));
        }

        // Convert 1-based line number to 0-based index.
        let declared_start = hunk
            .old_range()
            .start()
            .checked_sub(1)
            .ok_or_else(|| format!("hunk #{} has invalid line number 0", i + 1))?;

        let end = declared_start
            .checked_add(preimage.len())
            .ok_or_else(|| format!("hunk #{} line range overflows", i + 1))?;
        if end > current_lines.len() {
            return Err(format!(
                "hunk #{} expects {} preimage line(s) starting at line {}, \
                 but the value only has {} line(s)",
                i + 1,
                preimage.len(),
                declared_start + 1,
                current_lines.len()
            ));
        }

        for (offset, expected) in preimage.iter().enumerate() {
            let actual = current_lines[declared_start + offset];
            if *expected != actual {
                return Err(format!(
                    "hunk #{} preimage mismatch at line {}: \
                     patch expects {:?} but value has {:?}",
                    i + 1,
                    declared_start + offset + 1,
                    expected,
                    actual
                ));
            }
        }
    }
    Ok(())
}

/// Extract the current slug value as a `String` (or return `NotFound`).
/// Used by `mem hash` and `mem patch` — they both need "the value or fail".
/// Returns `(head_event, value)` so the caller can preserve monotonic ordering.
async fn fetch_value(
    client: &BuzzClient,
    agent: &PublicKey,
    owner: &PublicKey,
    slug: &str,
) -> Result<(nostr::Event, String), CliError> {
    let (head, body) = fetch_head(client, agent, owner, slug).await?;
    match (head, body) {
        (None, _) => Err(CliError::NotFound(format!("not found: {slug}"))),
        (_, None) => Err(CliError::NotFound(format!("not found: {slug}"))),
        (_, Some(Body::Memory { value: None, .. })) => {
            Err(CliError::NotFound(format!("tombstoned: {slug}")))
        }
        (Some(head), Some(Body::Memory { value: Some(v), .. })) => Ok((head, v)),
        (Some(head), Some(Body::Core { profile })) => Ok((head, profile)),
    }
}

/// `buzz mem hash <slug>` — print sha256(value) in hex to stdout.
///
/// The output is a 64-character hex digest followed by a newline (line-
/// oriented for shell use). Use this to capture a base-hash before editing,
/// then pass it to `buzz mem patch --base-hash <hex>` to make the edit
/// safe against concurrent writes.
pub async fn cmd_hash(
    client: &BuzzClient,
    raw_slug: &str,
    owner_flag: Option<&str>,
    agent_flag: Option<&str>,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    let (agent, owner, _) = resolve_reader(client, owner_flag, agent_flag)?;
    let (_head, value) = fetch_value(client, &agent, &owner, &slug).await?;
    println!("{}", sha256_hex(&value));
    Ok(())
}

/// `buzz mem patch <slug>` — apply a unified diff to the current value.
///
/// Reads a unified diff from stdin (or `--patch-file <path>`), fetches the
/// current head, applies the diff with **strict context matching** (no
/// content fuzz; diffy will refuse a hunk whose context lines don't match
/// the file verbatim), and writes the result.
///
/// Safety properties:
/// - `--base-hash <hex>` is **required** unless `--no-base-hash` is passed.
///   This makes concurrent edits safe: if the slug has changed since the
///   patch was generated, the write is refused.
/// - The result is rejected if it would be empty, unless `--allow-empty`.
/// - `--dry-run` prints the post-application diff and exits without writing.
/// - On a successful write, the new sha256 is printed to stderr so callers
///   can chain edits.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_patch(
    client: &BuzzClient,
    raw_slug: &str,
    patch_path: Option<&str>,
    base_hash: Option<&str>,
    no_base_hash: bool,
    dry_run: bool,
    allow_empty: bool,
    owner_flag: Option<&str>,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;

    // Require an explicit base-hash decision: this is the whole point of the
    // command vs. raw stdin pipelines.
    match (base_hash, no_base_hash) {
        (Some(_), true) => {
            return Err(CliError::Usage(
                "--base-hash and --no-base-hash are mutually exclusive".into(),
            ));
        }
        (None, false) => {
            return Err(CliError::Usage(
                "missing --base-hash <hex> (run `buzz mem hash <slug>` to get it). \
                 Pass --no-base-hash to skip this check at your own risk."
                    .into(),
            ));
        }
        _ => {}
    }
    if let Some(h) = base_hash {
        if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CliError::Usage(
                "--base-hash must be a 64-character hex sha256 digest".into(),
            ));
        }
    }

    // Read the diff (stdin or file). We bound it the same as `set` since the
    // resulting value can't exceed the NIP-44 cap anyway — and we don't want
    // a 4 GB malformed patch to OOM us.
    let limit = engram::NIP44_PLAINTEXT_MAX + 1;
    let diff_text = match patch_path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| CliError::Usage(format!("failed to read --patch-file {path}: {e}")))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .take(limit as u64)
                .read_to_string(&mut buf)
                .map_err(|e| CliError::Other(format!("stdin read failed: {e}")))?;
            if buf.is_empty() {
                return Err(CliError::Usage(
                    "refusing to apply empty patch from stdin (an upstream pipeline step likely \
                     failed)"
                        .into(),
                ));
            }
            buf
        }
    };

    let owner = resolve_owner(client, owner_flag)?;
    let agent_pubkey = client.public_key();
    let (head, current) = fetch_value(client, &agent_pubkey, &owner, &slug).await?;

    // Base-hash gate: concurrent-edit safety.
    if let Some(expected) = base_hash {
        let actual = sha256_hex(&current);
        if actual != expected.to_ascii_lowercase() {
            return Err(CliError::Conflict(format!(
                "slug `{slug}` has changed since patch was generated \
                 (expected sha256 {expected}, got {actual}). Re-fetch and regenerate the patch."
            )));
        }
    }

    // Reject multi-file patches. A memory slug is a single virtual file; a
    // patch with multiple `--- ` headers is ambiguous and almost certainly an
    // operator mistake (e.g. piping a multi-file `git diff` output here).
    let file_header_count = diff_text.lines().filter(|l| l.starts_with("--- ")).count();
    if file_header_count > 1 {
        return Err(CliError::Usage(format!(
            "multi-file patch not supported (found {file_header_count} `--- ` headers); \
             a memory slug is a single virtual file"
        )));
    }

    let patch = diffy::Patch::from_str(&diff_text)
        .map_err(|e| CliError::Usage(format!("malformed unified diff: {e}")))?;

    // Strict positional check — diffy's `apply` allows the hunk to slide
    // forward/backward in the file if it finds the preimage elsewhere. For
    // memory edits we want the stronger guarantee: the hunk must apply at
    // exactly the line number it claims. If the file has drifted enough that
    // the hunk's declared position no longer matches, we'd rather refuse and
    // make the operator regenerate the patch than risk landing the change
    // somewhere unintended.
    verify_hunks_at_declared_position(&current, &patch).map_err(|msg| {
        CliError::Usage(format!(
            "patch did not apply cleanly to slug `{slug}`: {msg}. \
             Context must match the current value verbatim at the declared \
             line numbers — no fuzz, no offset."
        ))
    })?;

    let new_value = diffy::apply(&current, &patch).map_err(|e| {
        CliError::Usage(format!(
            "patch did not apply cleanly to slug `{slug}`: {e}. \
             Context must match the current value verbatim — no fuzz, no offset."
        ))
    })?;

    if new_value.len() > engram::NIP44_PLAINTEXT_MAX {
        return Err(CliError::Usage(format!(
            "patched value would exceed {}-byte NIP-44 plaintext limit \
             (got {} bytes)",
            engram::NIP44_PLAINTEXT_MAX,
            new_value.len()
        )));
    }
    if new_value.is_empty() && !allow_empty {
        return Err(CliError::Usage(
            "refusing to write empty value (patch result is empty). \
             Pass --allow-empty to confirm, or use `buzz mem rm <slug>` to tombstone."
                .into(),
        ));
    }

    // Echo the *input* patch verbatim (not a regenerated form) plus the
    // resulting sha256, so the operator can review exactly what was applied
    // and chain follow-up edits with the new hash.
    let new_hash = sha256_hex(&new_value);
    eprintln!("{}", diff_text.trim_end_matches('\n'));
    eprintln!();
    if dry_run {
        eprintln!("(dry run — slug `{slug}` not modified; would write sha256 {new_hash})");
        return Ok(());
    }

    let body = if slug == engram::CORE_SLUG {
        Body::Core {
            profile: new_value.clone(),
        }
    } else {
        Body::Memory {
            slug: slug.clone(),
            value: Some(new_value.clone()),
        }
    };
    let prior_created_at = Some(head.created_at.as_secs());
    let created_at = engram::monotonic_created_at(now_secs(), prior_created_at);

    let event = seal_engram(client, &owner, &body, created_at).await?;
    let id = event.id.to_hex();
    submit_engram(client, event).await?;
    eprintln!("wrote {slug} (event {id}, created_at {created_at}, sha256 {new_hash})");
    Ok(())
}

/// One visible copy of a slug: which owner key it is addressed to, which
/// event is its head, and whether that head is already a tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlugCopy {
    /// The owner (the `p` tag) this copy is addressed to.
    owner: PublicKey,
    /// Event id of the head.
    event_id: String,
    /// `created_at` of the head.
    created_at: u64,
    /// `true` when the head is already a tombstone.
    tombstoned: bool,
}

/// Enumerate every copy of `slug` this seat can see, across owner keys.
///
/// The `d` tag of an engram is derived from the conversation key of the
/// writer and the owner it is addressed to, so the same slug under two
/// owner keys is two separate slots. A delete that only writes one tombstone
/// leaves the other copy readable — so the delete has to see every copy first.
///
/// Only copies this seat authored are visible (the relay admits an engram
/// filter for the authoring agent or the addressed owner), so this is the
/// complete set of copies this seat can tombstone.
async fn enumerate_slug_copies(
    client: &BuzzClient,
    agent: &PublicKey,
    slug: &str,
) -> Result<Vec<SlugCopy>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent.to_hex()],
        "limit": 5000,
    });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw)?;

    // The owner keys this seat has ever addressed an engram to.
    let mut owners: Vec<PublicKey> = Vec::new();
    for ev in &events {
        if ev.verify().is_err() {
            continue;
        }
        let Some(owner) = ev
            .tags
            .iter()
            .find(|t| t.kind().to_string() == "p")
            .and_then(|t| t.content())
            .and_then(|v| PublicKey::from_hex(v).ok())
        else {
            continue;
        };
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }

    let mut copies = Vec::new();
    for owner in owners {
        // The `d` tag for this slug under this owner key.
        let d = client.credentials().engram_d_tag(&owner, slug).await?;
        let members: Vec<nostr::Event> = events
            .iter()
            .filter(|ev| {
                ev.tags
                    .iter()
                    .find(|t| t.kind().to_string() == "d")
                    .and_then(|t| t.content())
                    .map(|v| v == d)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        if members.is_empty() {
            continue;
        }
        let Some(head) = select_head(members) else {
            continue;
        };
        // Agent-side read: the peer for decryption is the owner.
        let body = open_engram(client, &head, agent, &owner, &owner).await?;
        let Some(body) = body else { continue };
        if body.slug() != slug {
            continue;
        }
        copies.push(SlugCopy {
            owner,
            event_id: head.id.to_hex(),
            created_at: head.created_at.as_secs(),
            tombstoned: body.is_tombstone(),
        });
    }
    Ok(copies)
}

/// The delete plan for a set of visible copies of one slug.
///
/// `targets` are the copies to tombstone. `others` are the live copies that
/// stay readable — only non-empty when the delete was scoped to one owner.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeletePlan {
    targets: Vec<SlugCopy>,
    others: Vec<SlugCopy>,
}

/// Plan a delete over the visible copies of a slug.
///
/// Without a scope, every live copy is a target: a delete that leaves a
/// readable copy behind reads as a delete and is not one. With a scope, only
/// the copies under that owner are targeted and the surviving copies under
/// other owners are named.
fn plan_delete(copies: &[SlugCopy], scope: Option<&PublicKey>) -> DeletePlan {
    let live: Vec<SlugCopy> = copies.iter().filter(|c| !c.tombstoned).cloned().collect();
    match scope {
        None => DeletePlan {
            targets: live,
            others: Vec::new(),
        },
        Some(owner) => DeletePlan {
            targets: live.iter().filter(|c| &c.owner == owner).cloned().collect(),
            others: live.iter().filter(|c| &c.owner != owner).cloned().collect(),
        },
    }
}

/// `buzz mem rm <slug>` — publish a tombstone (`value: null`).
///
/// A slug can exist as several copies — one per owner key it was written
/// under — because the engram `d` tag is derived per (writer, owner) pair.
/// This command enumerates every copy the seat can see first: with no
/// `--owner` it tombstones all of them, so a delete does not leave a
/// readable copy behind. With `--owner` the delete is scoped to that owner
/// and any surviving copy under another owner is named on stderr.
///
/// `rm core` writes a tombstone-shaped body, but a core tombstone has no
/// well-defined semantics in NIP-AE (the spec only defines tombstones for
/// memory entries). We refuse it and tell the operator to overwrite `core`
/// with an empty profile instead.
pub async fn cmd_rm(
    client: &BuzzClient,
    raw_slug: &str,
    owner_flag: Option<&str>,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    if slug == engram::CORE_SLUG {
        return Err(CliError::Usage(
            "core cannot be tombstoned; overwrite it with `buzz mem set core ''` instead".into(),
        ));
    }
    let agent_pubkey = client.public_key();

    // Enumerate BEFORE writing anything: if the enumeration fails, nothing
    // is deleted and the operator is not told a delete happened.
    let copies = enumerate_slug_copies(client, &agent_pubkey, &slug).await?;
    let scope = match owner_flag {
        Some(flag) => Some(
            PublicKey::from_hex(flag)
                .map_err(|e| CliError::Usage(format!("--owner must be a 64-hex pubkey: {e}")))?,
        ),
        None => None,
    };
    let plan = plan_delete(&copies, scope.as_ref());
    let targets = plan.targets;
    let others = plan.others;

    if !others.is_empty() {
        eprintln!(
            "warning: {} live copy/copies of {slug} under other owner key(s) remain and were NOT deleted: {}",
            others.len(),
            others
                .iter()
                .map(|c| c.owner.to_hex())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if targets.is_empty() {
        if owner_flag.is_some() {
            return Err(CliError::NotFound(format!("not found: {slug}")));
        }
        // No live copy anywhere. Preserve the historical behaviour of writing
        // a tombstone against the resolved owner so the operation stays
        // idempotent, and say plainly that nothing was live.
        let owner = resolve_owner(client, owner_flag)?;
        let body = Body::Memory {
            slug: slug.clone(),
            value: None,
        };
        let created_at = engram::monotonic_created_at(now_secs(), None);
        let event = seal_engram(client, &owner, &body, created_at).await?;
        let id = event.id.to_hex();
        submit_engram(client, event).await?;
        eprintln!(
            "tombstoned {slug} (event {id}, created_at {created_at}); no live copy was found"
        );
        return Ok(());
    }

    let body = Body::Memory {
        slug: slug.clone(),
        value: None,
    };
    for copy in &targets {
        let created_at = engram::monotonic_created_at(now_secs(), Some(copy.created_at));
        let event = seal_engram(client, &copy.owner, &body, created_at).await?;
        let id = event.id.to_hex();
        submit_engram(client, event).await?;
        eprintln!(
            "tombstoned {slug} under owner {} (event {id}, created_at {created_at})",
            copy.owner.to_hex()
        );
    }
    Ok(())
}

pub async fn dispatch(cmd: crate::MemCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::MemCmd;
    match cmd {
        MemCmd::Ls {
            owner,
            agent,
            json,
            include_core,
        } => {
            cmd_ls(
                client,
                owner.as_deref(),
                agent.as_deref(),
                json,
                include_core,
            )
            .await
        }
        MemCmd::Get {
            slug,
            owner,
            agent,
            json,
        } => cmd_get(client, &slug, owner.as_deref(), agent.as_deref(), json).await,
        MemCmd::Hash { slug, owner, agent } => {
            cmd_hash(client, &slug, owner.as_deref(), agent.as_deref()).await
        }
        MemCmd::Set {
            slug,
            value,
            owner,
            allow_empty,
        } => cmd_set(client, &slug, &value, owner.as_deref(), allow_empty).await,
        MemCmd::Patch {
            slug,
            patch_file,
            base_hash,
            no_base_hash,
            dry_run,
            allow_empty,
            owner,
        } => {
            cmd_patch(
                client,
                &slug,
                patch_file.as_deref(),
                base_hash.as_deref(),
                no_base_hash,
                dry_run,
                allow_empty,
                owner.as_deref(),
            )
            .await
        }
        MemCmd::Rm { slug, owner } => cmd_rm(client, &slug, owner.as_deref()).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // sha256_hex must match `printf '%s' "$value" | sha256sum` so operators can
    // verify base-hash from the shell. Hard-coded vectors from the NIST and
    // common quick-check inputs.

    /// Through the desktop, an engram with bad ciphertext is skipped while
    /// a valid one opens; a credential failure aborts instead of reading as
    /// "no entries".
    #[cfg(unix)]
    #[tokio::test]
    async fn the_desktop_skips_bad_ciphertext_but_aborts_on_credential_failure() {
        use crate::desktop_provider::{tests as dp, Provider};
        const RELAY: &str = "wss://community.example";
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let body = Body::Memory {
            slug: "mem/a".into(),
            value: Some("v".into()),
        };
        let valid = engram::build_event(&agent, &owner.public_key(), &body, 1).unwrap();
        let k_c = engram::conversation_key(agent.secret_key(), &owner.public_key());
        let corrupt = engram::event_builder(
            &owner.public_key(),
            &engram::d_tag(&k_c, "mem/a"),
            "AAAA".into(),
            2,
        )
        .unwrap()
        .sign_with_keys(&agent)
        .unwrap();
        let open = |endpoint| async move {
            let p = Provider::open(endpoint, "t".into(), false, RELAY)
                .await
                .unwrap()
                .ok()
                .unwrap();
            let credentials = crate::credentials::Credentials::from_desktop(p)
                .await
                .unwrap();
            BuzzClient::new("http://127.0.0.1:9".into(), credentials).unwrap()
        };
        let (endpoint, _) = dp::desktop(owner.clone(), vec![RELAY.into()]).await;
        let client = open(endpoint).await;
        let (a, o) = (agent.public_key(), owner.public_key());
        let opened = open_engram(&client, &valid, &a, &o, &a).await.unwrap();
        assert!(matches!(opened, Some(Body::Memory { .. })));
        assert!(open_engram(&client, &corrupt, &a, &o, &a)
            .await
            .unwrap()
            .is_none());

        let (endpoint, _) = dp::provider(move |_, r| match r["method"].as_str() {
            Some("describe") => serde_json::json!({"result": serde_json::json!({
                "pubkey": owner.public_key().to_hex(), "relays": [RELAY], "methods": [],
            }).to_string()}),
            _ => serde_json::json!({"result": "", "error": "unavailable"}),
        })
        .await;
        let client = open(endpoint).await;
        assert!(open_engram(&client, &valid, &a, &o, &a).await.is_err());
    }

    /// A desktop that throttles partway through `buzz mem ls` is waited out,
    /// and the list completes.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_list_completes_when_the_desktop_throttles_partway() {
        use crate::desktop_provider::{tests as dp, Provider};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate().public_key();
        let events: Vec<serde_json::Value> = (0..3)
            .map(|i| {
                let body = Body::Memory {
                    slug: format!("mem/{i}"),
                    value: Some("v".into()),
                };
                let ev = engram::build_event(&agent, &owner, &body, 1).unwrap();
                serde_json::to_value(&ev).unwrap()
            })
            .collect();
        let app = axum::Router::new().route(
            "/query",
            axum::routing::post(move || async move { axum::Json(events) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        // The second decrypt is refused once with `rate_limited`.
        let decrypts = std::sync::Arc::new(AtomicUsize::new(0));
        let counted = decrypts.clone();
        let answer = dp::desktop_answers(agent.clone(), vec![relay.clone()]);
        let (endpoint, seen) = dp::provider(move |version, request| {
            if request["method"] == "nip44_decrypt" && counted.fetch_add(1, Ordering::SeqCst) == 1 {
                return serde_json::json!({"result": "", "error": "rate_limited", "retry_after": 1});
            }
            answer(version, request)
        })
        .await;
        let provider = Provider::open(endpoint, "t".into(), false, &relay)
            .await
            .unwrap()
            .ok()
            .unwrap();
        let credentials = crate::credentials::Credentials::from_desktop(provider)
            .await
            .unwrap();
        let client = BuzzClient::new(relay, credentials).unwrap();

        cmd_ls(&client, Some(&owner.to_hex()), None, true, false)
            .await
            .unwrap();
        assert_eq!(
            decrypts.load(Ordering::SeqCst),
            4,
            "three entries, one retry"
        );
        let d_tags = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, _, r)| r["method"] == "engram_d_tag")
            .count();
        assert_eq!(d_tags, 3);
    }

    fn test_client(keys: nostr::Keys) -> BuzzClient {
        BuzzClient::new(
            "http://127.0.0.1:9".into(),
            crate::credentials::Credentials::from_keys(keys),
        )
        .unwrap()
    }

    /// A read must be able to say which copy it returned: the event it came
    /// from, who authored it, which owner it is addressed to and when it was
    /// written. Two agents addressing the same owner and slug each keep
    /// their own copy, so the provenance is what makes a read self-describing.
    #[test]
    fn a_read_reports_the_provenance_of_the_copy_it_returned() {
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate().public_key();
        let body = Body::Memory {
            slug: "mem/a".into(),
            value: Some("v".into()),
        };
        let event = engram::build_event(&agent, &owner, &body, 1_700_000_000).unwrap();
        let json = get_output_json("mem/a", "v", Some(&event), &owner).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["slug"], "mem/a");
        assert_eq!(parsed["value"], "v");
        assert_eq!(parsed["event_id"], event.id.to_hex());
        assert_eq!(parsed["author"], agent.public_key().to_hex());
        assert_eq!(parsed["owner"], owner.to_hex());
        assert_eq!(parsed["created_at"], 1_700_000_000u64);
    }

    /// A self-owned copy reports the author and the owner as the same key.
    #[test]
    fn a_self_owned_read_reports_the_author_as_the_owner() {
        let agent = nostr::Keys::generate();
        let body = Body::Memory {
            slug: "mem/a".into(),
            value: Some("v".into()),
        };
        let event = engram::build_event(&agent, &agent.public_key(), &body, 1_700_000_000).unwrap();
        let json = get_output_json("mem/a", "v", Some(&event), &agent.public_key()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["author"], agent.public_key().to_hex());
        assert_eq!(parsed["owner"], agent.public_key().to_hex());
    }

    /// The listing excludes `core` by specification, and `--include-core`
    /// makes it visible. A tombstone is never listed either way.
    #[test]
    fn the_listing_excludes_core_unless_it_is_asked_for() {
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate().public_key();
        let core = engram::build_event(
            &agent,
            &owner,
            &Body::Core {
                profile: "p".into(),
            },
            1_700_000_000,
        )
        .unwrap();
        assert!(listing_entry(
            &Body::Core {
                profile: "p".into()
            },
            &core,
            &owner,
            false
        )
        .is_none());
        let entry = listing_entry(
            &Body::Core {
                profile: "p".into(),
            },
            &core,
            &owner,
            true,
        )
        .expect("core is listed when asked for");
        assert_eq!(entry.slug, "core");
        assert_eq!(entry.event_id, core.id.to_hex());
        assert_eq!(entry.owner, owner.to_hex());

        let tombstone = engram::build_event(
            &agent,
            &owner,
            &Body::Memory {
                slug: "mem/a".into(),
                value: None,
            },
            1_700_000_000,
        )
        .unwrap();
        assert!(listing_entry(
            &Body::Memory {
                slug: "mem/a".into(),
                value: None
            },
            &tombstone,
            &owner,
            true
        )
        .is_none());
    }

    /// The `d` tag of an engram is derived per (writer, owner) pair, so the
    /// same slug written under two owner keys is two separate slots with two
    /// different `d` tags. A tombstone under one owner key therefore cannot
    /// address the copy under the other — which is why the delete has to
    /// enumerate every owner key first.
    #[test]
    fn the_same_slug_under_two_owners_is_two_separate_slots() {
        let agent = nostr::Keys::generate();
        let owner_a = nostr::Keys::generate().public_key();
        let owner_b = nostr::Keys::generate().public_key();
        let body = Body::Memory {
            slug: "mem/a".into(),
            value: Some("v".into()),
        };
        let a = engram::build_event(&agent, &owner_a, &body, 1_700_000_000).unwrap();
        let b = engram::build_event(&agent, &owner_b, &body, 1_700_000_000).unwrap();
        let d_of = |ev: &nostr::Event| {
            ev.tags
                .iter()
                .find(|t| t.kind().to_string() == "d")
                .and_then(|t| t.content())
                .unwrap()
                .to_string()
        };
        assert_ne!(d_of(&a), d_of(&b));
        assert_ne!(a.id.to_hex(), b.id.to_hex());
    }

    fn copy(owner: PublicKey, tombstoned: bool) -> SlugCopy {
        SlugCopy {
            owner,
            event_id: "e".repeat(64),
            created_at: 1_700_000_000,
            tombstoned,
        }
    }

    /// A delete without a scope tombstones EVERY live copy: a delete that
    /// leaves a readable copy behind reads as a delete and is not one.
    #[test]
    fn an_unscoped_delete_tombstones_every_live_copy() {
        let a = nostr::Keys::generate().public_key();
        let b = nostr::Keys::generate().public_key();
        let copies = vec![copy(a, false), copy(b, false), copy(a, true)];
        let plan = plan_delete(&copies, None);
        assert_eq!(plan.targets.len(), 2);
        assert_eq!(plan.others.len(), 0);
        let mut owners: Vec<String> = plan.targets.iter().map(|c| c.owner.to_hex()).collect();
        owners.sort();
        let mut expected = vec![a.to_hex(), b.to_hex()];
        expected.sort();
        assert_eq!(owners, expected);
    }

    /// A scoped delete targets only that owner and NAMES the copies that
    /// stay readable, so the operator is never told a delete was complete
    /// while a copy survived.
    #[test]
    fn a_scoped_delete_names_the_copies_that_survive() {
        let a = nostr::Keys::generate().public_key();
        let b = nostr::Keys::generate().public_key();
        let copies = vec![copy(a, false), copy(b, false)];
        let plan = plan_delete(&copies, Some(&a));
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].owner, a);
        assert_eq!(plan.others.len(), 1);
        assert_eq!(plan.others[0].owner, b);
    }

    /// An already-tombstoned copy is not a target — deleting it again would
    /// publish a second tombstone for nothing.
    #[test]
    fn a_tombstoned_copy_is_not_a_delete_target() {
        let a = nostr::Keys::generate().public_key();
        let plan = plan_delete(&[copy(a, true)], None);
        assert!(plan.targets.is_empty());
        assert!(plan.others.is_empty());
    }

    #[test]
    fn resolve_reader_defaults_to_agent_identity() {
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate();
        let client = test_client(agent.clone());

        let (resolved_agent, resolved_owner, their_pubkey) =
            resolve_reader(&client, Some(&owner.public_key().to_hex()), None).unwrap();

        assert_eq!(resolved_agent, agent.public_key());
        assert_eq!(resolved_owner, owner.public_key());
        assert_eq!(their_pubkey, owner.public_key());
    }

    #[test]
    fn resolve_reader_agent_flag_uses_cli_identity_as_owner() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let client = test_client(owner.clone());

        let (resolved_agent, resolved_owner, their_pubkey) =
            resolve_reader(&client, None, Some(&agent.public_key().to_hex())).unwrap();

        assert_eq!(resolved_agent, agent.public_key());
        assert_eq!(resolved_owner, owner.public_key());
        assert_eq!(their_pubkey, agent.public_key());
    }

    #[test]
    fn resolve_reader_rejects_owner_with_agent_flag() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let client = test_client(owner.clone());

        let err = resolve_reader(
            &client,
            Some(&owner.public_key().to_hex()),
            Some(&agent.public_key().to_hex()),
        )
        .unwrap_err();

        assert!(err.to_string().contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn resolve_reader_rejects_agent_flag_matching_cli_identity() {
        let owner = nostr::Keys::generate();
        let client = test_client(owner.clone());

        let err = resolve_reader(&client, None, Some(&owner.public_key().to_hex())).unwrap_err();

        assert!(err.to_string().contains("must differ"), "got: {err}");
    }

    #[test]
    fn sha256_hex_empty() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_abc() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_hex_handles_newline_terminated_value() {
        // `mem get` writes the raw value with no trailing newline added; a
        // value that itself ends in '\n' must hash the newline as part of
        // the content. Confirms we hash the bytes verbatim.
        assert_eq!(
            sha256_hex("abc\n"),
            "edeaaff3f1774ad2888673770c6d64097e391bc362d7d6fb34982ddf0efd18cb"
        );
    }

    // Strict-context behavior: diffy::apply must refuse a hunk whose context
    // lines don't match the current value. This pins the property `mem patch`
    // depends on; if a future diffy upgrade loosens it, this test catches it.

    #[test]
    fn diffy_apply_refuses_mismatched_context() {
        let current = "alpha\nbeta\ngamma\n";
        // Patch references "BETA" (wrong case) as context — must fail.
        let bad_patch = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 alpha
-BETA
+delta
 gamma
";
        let patch = diffy::Patch::from_str(bad_patch).unwrap();
        assert!(diffy::apply(current, &patch).is_err());
    }

    #[test]
    fn diffy_apply_succeeds_on_exact_context() {
        let current = "alpha\nbeta\ngamma\n";
        let good_patch = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 alpha
-beta
+delta
 gamma
";
        let patch = diffy::Patch::from_str(good_patch).unwrap();
        let out = diffy::apply(current, &patch).unwrap();
        assert_eq!(out, "alpha\ndelta\ngamma\n");
    }

    // Round-trip: a patch generated from (current → new) must reproduce `new`
    // when applied to `current`. Guards against silent encoding drift between
    // `create_patch` and `apply` (e.g. line-ending handling).
    #[test]
    fn diffy_roundtrip_preserves_content() {
        let current = "one\ntwo\nthree\nfour\nfive\n";
        let new = "one\nTWO\nthree\nFOUR\nfive\n";
        let p = diffy::create_patch(current, new);
        let applied = diffy::apply(current, &p).unwrap();
        assert_eq!(applied, new);
    }

    // Max's offset-search case: a hunk declaring `@@ -1,3 @@ alpha/-beta/+delta/gamma`
    // against `zero\nalpha\nbeta\ngamma\n` must be rejected. Diffy's `apply`
    // would happily slide the hunk forward and land it at line 2; the
    // positional check refuses.
    #[test]
    fn strict_position_rejects_offset_slide() {
        let current = "zero\nalpha\nbeta\ngamma\n";
        let patch_text = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 alpha
-beta
+delta
 gamma
";
        let patch = diffy::Patch::from_str(patch_text).unwrap();
        // Sanity: diffy's apply *would* slide and produce a result.
        let diffy_result = diffy::apply(current, &patch).unwrap();
        assert_eq!(diffy_result, "zero\nalpha\ndelta\ngamma\n");
        // Our positional check rejects.
        let err = verify_hunks_at_declared_position(current, &patch).unwrap_err();
        assert!(err.contains("preimage mismatch"), "got: {err}");
    }

    #[test]
    fn strict_position_accepts_exact_match() {
        let current = "alpha\nbeta\ngamma\n";
        let patch_text = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 alpha
-beta
+delta
 gamma
";
        let patch = diffy::Patch::from_str(patch_text).unwrap();
        verify_hunks_at_declared_position(current, &patch).unwrap();
    }

    // Pure insertion against an empty value: `@@ -0,0 +1,N @@`.
    #[test]
    fn strict_position_accepts_pure_insertion_into_empty() {
        let current = "";
        let patch_text = "\
--- a/x
+++ b/x
@@ -0,0 +1,2 @@
+first
+second
";
        let patch = diffy::Patch::from_str(patch_text).unwrap();
        verify_hunks_at_declared_position(current, &patch).unwrap();
    }

    // Multi-hunk patches: each hunk's `@@ -N @@` references line numbers in
    // the *original* file, not in the file as modified by previous hunks. So
    // validating each hunk's preimage against the unmodified `current_lines`
    // at the declared position is correct — no cumulative-delta tracking
    // needed. This test pins that property.
    #[test]
    fn strict_position_accepts_multi_hunk_against_original() {
        let current = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
        let patch_text = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 a
-b
+B
 c
@@ -10,3 +10,3 @@
 j
-k
+K
 l
";
        let patch = diffy::Patch::from_str(patch_text).unwrap();
        verify_hunks_at_declared_position(current, &patch).unwrap();
        // And the actual apply produces the expected result.
        assert_eq!(
            diffy::apply(current, &patch).unwrap(),
            "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\n"
        );
    }

    // Value with no trailing newline must still round-trip. `split_inclusive`
    // produces a final segment without `\n`; diffy strips `\n` from the last
    // hunk line when the patch carries `\\ No newline at end of file`. Both
    // representations must match here.
    #[test]
    fn strict_position_handles_no_trailing_newline() {
        let current = "alpha\nbeta\ngamma"; // no trailing \n
        let patch_text = "\
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 alpha
-beta
+delta
 gamma
\\ No newline at end of file
";
        let patch = diffy::Patch::from_str(patch_text).unwrap();
        verify_hunks_at_declared_position(current, &patch).unwrap();
    }

    // Lock the multi-file detection. The check is a simple count of lines
    // starting with `--- ` because diffy's `parse()` will only consume the
    // first patch and silently treat everything after the last hunk as
    // junk until the next `@@` (which it then rejects). Counting `--- `
    // before parsing catches the ambiguous case up-front.
    #[test]
    fn multi_file_header_count() {
        let single = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(single.lines().filter(|l| l.starts_with("--- ")).count(), 1);

        let multi = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\
                     --- a/y\n+++ b/y\n@@ -1 +1 @@\n-c\n+d\n";
        assert_eq!(multi.lines().filter(|l| l.starts_with("--- ")).count(), 2);
    }
}
