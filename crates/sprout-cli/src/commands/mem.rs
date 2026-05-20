//! `sprout mem` — agent-side engram management (NIP-AE).
//!
//! Subcommands:
//! - `sprout mem ls`                   — list non-tombstoned memories
//! - `sprout mem get <slug>`            — print the value to stdout
//! - `sprout mem set <slug> <value|-> ` — write a value (use `-` for stdin)
//! - `sprout mem rm <slug>`             — publish a tombstone
//!
//! The caller's `SPROUT_PRIVATE_KEY` is the agent's nsec. The agent's owner
//! pubkey is resolved from `SPROUT_AUTH_TAG` (NIP-OA attestation) or the
//! `--owner` flag. Every record is encrypted under the agent↔owner NIP-44
//! conversation key; both parties can decrypt.

use std::io::Read;
use std::time::SystemTime;

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
    owner: &PublicKey,
    slug: &str,
) -> Result<(Option<nostr::Event>, Option<Body>), CliError> {
    let agent = client.keys();
    let k_c = conversation_key(agent.secret_key(), owner);
    let d = d_tag(&k_c, slug);

    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent.public_key().to_hex()],
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
        match validate_and_decrypt(&ev, &agent.public_key(), owner, agent.secret_key(), owner) {
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
    json: bool,
) -> Result<(), CliError> {
    let owner = resolve_owner(client, owner_flag)?;
    let agent = client.keys();

    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent.public_key().to_hex()],
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
            &agent.public_key(),
            &owner,
            agent.secret_key(),
            &owner,
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
                    created_at: head.created_at.as_u64(),
                });
            }
        }
    }
    listings.sort_by(|a, b| a.slug.cmp(&b.slug));

    if json {
        println!("{}", serde_json::to_string(&listings).unwrap_or_default());
    } else if listings.is_empty() {
        eprintln!("(no memories)");
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
) -> Result<(), CliError> {
    let slug =
        normalize_slug(raw_slug).map_err(|e| CliError::Usage(format!("invalid slug: {e}")))?;
    let owner = resolve_owner(client, owner_flag)?;
    let (_head, body) = fetch_head(client, &owner, &slug).await?;
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
pub async fn cmd_set(
    client: &SproutClient,
    raw_slug: &str,
    raw_value: &str,
    owner_flag: Option<&str>,
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
    let (head, _) = fetch_head(client, &owner, &slug).await?;
    let prior_created_at = head.map(|e| e.created_at.as_u64());
    let created_at = engram::monotonic_created_at(now_secs(), prior_created_at);

    let agent = client.keys();
    let event = engram::build_event(agent, &owner, &body, created_at)
        .map_err(|e| CliError::Other(format!("build event failed: {e}")))?;
    let id = event.id.to_hex();
    submit_engram(client, event).await?;
    eprintln!("wrote {slug} (event {id}, created_at {created_at})");
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
    let (head, _) = fetch_head(client, &owner, &slug).await?;
    let prior_created_at = head.map(|e| e.created_at.as_u64());
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
        MemCmd::Ls { owner, json } => cmd_ls(client, owner.as_deref(), json).await,
        MemCmd::Get { slug, owner } => cmd_get(client, &slug, owner.as_deref()).await,
        MemCmd::Set { slug, value, owner } => {
            cmd_set(client, &slug, &value, owner.as_deref()).await
        }
        MemCmd::Rm { slug, owner } => cmd_rm(client, &slug, owner.as_deref()).await,
    }
}
