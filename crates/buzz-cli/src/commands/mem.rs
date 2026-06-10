//! `sprout mem` — agent-side engram management (NIP-AE).
//!
//! Subcommands:
//! - `sprout mem ls`                   — list non-tombstoned memories
//! - `sprout mem get <slug>`            — print the value to stdout
//! - `sprout mem hash <slug>`           — print sha256(value) hex
//! - `sprout mem set <slug> <value|-> ` — write a value (use `-` for stdin)
//! - `sprout mem patch <slug>`          — apply a unified diff to the current value
//! - `sprout mem rm <slug>`             — publish a tombstone
//!
//! By default, the caller's `SPROUT_PRIVATE_KEY` is the agent's nsec. The
//! agent's owner pubkey is resolved from `SPROUT_AUTH_TAG` (NIP-OA attestation)
//! or the `--owner` flag. Read commands also support owner-side recovery via
//! `--agent <pubkey>`: the CLI identity is treated as the owner and decrypts
//! the agent's engrams through the same agent↔owner NIP-44 conversation key.

use std::io::Read;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use nostr::PublicKey;
use sprout_core::engram::{
    self, conversation_key, d_tag, normalize_slug, select_head, validate_and_decrypt, Body, Listing,
};
use sprout_core::kind::KIND_AGENT_ENGRAM;

use crate::client::SproutClient;
use crate::error::CliError;

/// Resolve the agent's owner pubkey: explicit `--owner` flag wins, otherwise
/// fall back to the NIP-OA `auth_tag` (which carries owner pubkey in slot 1).
fn resolve_owner(client: &SproutClient, owner_flag: Option<&str>) -> Result<PublicKey, CliError> {
    if let Some(s) = owner_flag {
        return PublicKey::from_hex(s)
            .map_err(|e| CliError::Usage(format!("--owner must be a 64-hex pubkey: {e}")));
    }
    let tag = client.auth_tag_owner_hex().ok_or_else(|| {
        CliError::Usage(
            "owner pubkey required (set SPROUT_AUTH_TAG with a NIP-OA attestation or pass --owner)"
                .into(),
        )
    })?;
    PublicKey::from_hex(&tag)
        .map_err(|e| CliError::Other(format!("auth_tag owner pubkey is not valid hex: {e}")))
}

/// Resolve the read perspective for `mem ls/get/hash`.
///
/// Normal agent-side reads use the CLI identity as the agent and resolve the
/// owner from `--owner` / SPROUT_AUTH_TAG. Owner-side recovery passes
/// `--agent <pubkey>`; the CLI identity is then the owner and the supplied
/// pubkey is the agent author to query/decrypt.
fn resolve_reader(
    client: &SproutClient,
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
        if agent == client.keys().public_key() {
            return Err(CliError::Usage(
                "--agent must differ from the CLI identity; omit --agent for agent-side reads"
                    .into(),
            ));
        }
        return Ok((agent, client.keys().public_key(), agent));
    }

    let agent = client.keys().public_key();
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
async fn submit_engram(client: &SproutClient, event: nostr::Event) -> Result<(), CliError> {
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

/// Fetch the head event for `slug`, returning `(Option<Event>, Option<Body>)`.
async fn fetch_head(
    client: &SproutClient,
    agent: &PublicKey,
    owner: &PublicKey,
    slug: &str,
) -> Result<(Option<nostr::Event>, Option<Body>), CliError> {
    let their_pubkey = if client.keys().public_key() == *agent {
        owner
    } else {
        agent
    };
    let k_c = conversation_key(client.keys().secret_key(), their_pubkey);
    let d = d_tag(&k_c, slug);

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
        match validate_and_decrypt(&ev, agent, owner, client.keys().secret_key(), their_pubkey) {
            Ok(body) => valid_with_body.push((ev, body)),
            Err(_) => continue,
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

/// `sprout mem ls` — list non-tombstoned memory entries.
pub async fn cmd_ls(
    client: &SproutClient,
    owner_flag: Option<&str>,
    agent_flag: Option<&str>,
    json: bool,
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
        let body = match validate_and_decrypt(
            &ev,
            &agent,
            &owner,
            client.keys().secret_key(),
            &their_pubkey,
        ) {
            Ok(b) => b,
            Err(_) => continue,
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
        // Drop tombstones and the core entry (per spec: listing excludes core).
        match &body {
            Body::Core { .. } => continue,
            Body::Memory { value: None, .. } => continue,
            Body::Memory { slug, .. } => {
                listings.push(Listing {
                    slug: slug.clone(),
                    event_id: head.id.to_hex(),
                    created_at: head.created_at.as_secs(),
                });
            }
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

/// `sprout mem get <slug>` — print value (memory) or profile (core) to stdout.
///
/// Exit codes: 0 on found, 1 on absent or tombstoned.
pub async fn cmd_get(
    client: &SproutClient,
    raw_slug: &str,
    owner_flag: Option<&str>,
    agent_flag: Option<&str>,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    let (agent, owner, _) = resolve_reader(client, owner_flag, agent_flag)?;
    let (_head, body) = fetch_head(client, &agent, &owner, &slug).await?;
    use std::io::Write;
    match body {
        None => Err(CliError::NotFound(format!("not found: {slug}"))),
        Some(Body::Memory { value: None, .. }) => {
            Err(CliError::NotFound(format!("tombstoned: {slug}")))
        }
        Some(Body::Memory { value: Some(v), .. }) => {
            // Raw stdout, no trailing newline — round-trips with `sprout mem set foo -`.
            std::io::stdout()
                .write_all(v.as_bytes())
                .map_err(|e| CliError::Other(e.to_string()))
        }
        Some(Body::Core { profile }) => std::io::stdout()
            .write_all(profile.as_bytes())
            .map_err(|e| CliError::Other(e.to_string())),
    }
}

/// `sprout mem set <slug> <value|->` — write a value or core profile.
///
/// Pass `-` to read the value from stdin.
///
/// Guardrail: when reading from stdin, an empty read is **rejected** unless
/// `--allow-empty` is passed. This catches the common failure mode where an
/// upstream pipeline step errors out, closes its stdout, and `mem set` would
/// otherwise commit an empty value — silently destroying the slug.
/// A literal `""` positional argument is still accepted (explicit intent).
pub async fn cmd_set(
    client: &SproutClient,
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
                 failed). Pass --allow-empty to confirm, or use `sprout mem rm <slug>` to \
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
    let agent_pubkey = client.keys().public_key();
    let (head, _) = fetch_head(client, &agent_pubkey, &owner, &slug).await?;
    let prior_created_at = head.map(|e| e.created_at.as_secs());
    let created_at = engram::monotonic_created_at(now_secs(), prior_created_at);

    let agent = client.keys();
    let event = engram::build_event(agent, &owner, &body, created_at)
        .map_err(|e| CliError::Other(format!("build event failed: {e}")))?;
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
    client: &SproutClient,
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

/// `sprout mem hash <slug>` — print sha256(value) in hex to stdout.
///
/// The output is a 64-character hex digest followed by a newline (line-
/// oriented for shell use). Use this to capture a base-hash before editing,
/// then pass it to `sprout mem patch --base-hash <hex>` to make the edit
/// safe against concurrent writes.
pub async fn cmd_hash(
    client: &SproutClient,
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

/// `sprout mem patch <slug>` — apply a unified diff to the current value.
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
    client: &SproutClient,
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
                "missing --base-hash <hex> (run `sprout mem hash <slug>` to get it). \
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
    let agent_pubkey = client.keys().public_key();
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
             Pass --allow-empty to confirm, or use `sprout mem rm <slug>` to tombstone."
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

    let agent = client.keys();
    let event = engram::build_event(agent, &owner, &body, created_at)
        .map_err(|e| CliError::Other(format!("build event failed: {e}")))?;
    let id = event.id.to_hex();
    submit_engram(client, event).await?;
    eprintln!("wrote {slug} (event {id}, created_at {created_at}, sha256 {new_hash})");
    Ok(())
}

/// `sprout mem rm <slug>` — publish a tombstone (`value: null`).
///
/// `rm core` writes a tombstone-shaped body, but a core tombstone has no
/// well-defined semantics in NIP-AE (the spec only defines tombstones for
/// memory entries). We refuse it and tell the operator to overwrite `core`
/// with an empty profile instead.
pub async fn cmd_rm(
    client: &SproutClient,
    raw_slug: &str,
    owner_flag: Option<&str>,
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    if slug == engram::CORE_SLUG {
        return Err(CliError::Usage(
            "core cannot be tombstoned; overwrite it with `sprout mem set core ''` instead".into(),
        ));
    }
    let owner = resolve_owner(client, owner_flag)?;
    let body = Body::Memory {
        slug: slug.clone(),
        value: None,
    };
    let agent_pubkey = client.keys().public_key();
    let (head, _) = fetch_head(client, &agent_pubkey, &owner, &slug).await?;
    let prior_created_at = head.map(|e| e.created_at.as_secs());
    let created_at = engram::monotonic_created_at(now_secs(), prior_created_at);

    let agent = client.keys();
    let event = engram::build_event(agent, &owner, &body, created_at)
        .map_err(|e| CliError::Other(format!("build event failed: {e}")))?;
    let id = event.id.to_hex();
    submit_engram(client, event).await?;
    eprintln!("tombstoned {slug} (event {id}, created_at {created_at})");
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn dispatch(cmd: crate::MemCmd, client: &SproutClient) -> Result<(), CliError> {
    use crate::MemCmd;
    match cmd {
        MemCmd::Ls { owner, agent, json } => {
            cmd_ls(client, owner.as_deref(), agent.as_deref(), json).await
        }
        MemCmd::Get { slug, owner, agent } => {
            cmd_get(client, &slug, owner.as_deref(), agent.as_deref()).await
        }
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

    fn test_client(keys: nostr::Keys) -> SproutClient {
        SproutClient::new("http://127.0.0.1:9".into(), keys, None, None).unwrap()
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
