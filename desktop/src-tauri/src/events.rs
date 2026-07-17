//! Signed-event builders for desktop write operations.
//!
//! Mirrors the buzz-sdk builder patterns but uses nostr 0.37 API
//! (the desktop is excluded from the workspace which pins nostr 0.36).
//!
//! Mental model:
//!   caller params → build_*() → EventBuilder → submit_event() signs + POSTs
//!
//! Each function validates inputs and returns a nostr::EventBuilder.
//! Signing and submission happen in relay::submit_event.

use buzz_core_pkg::kind::{KIND_IA_ARCHIVE_REQUEST, KIND_IA_UNARCHIVE_REQUEST};
use nostr::{EventBuilder, EventId, Kind, Tag};
use uuid::Uuid;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum content size — matches buzz-sdk (64 KiB).
const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// Maximum mention count — matches buzz-sdk.
const MAX_MENTIONS: usize = 50;

/// Maximum emoji length in characters — matches buzz-sdk.
const MAX_EMOJI_CHARS: usize = 64;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn tag(parts: Vec<&str>) -> Result<Tag, String> {
    Tag::parse(parts).map_err(|e| format!("invalid tag: {e}"))
}

fn check_content(content: &str) -> Result<(), String> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(format!(
            "content exceeds maximum size of {} bytes (got {})",
            MAX_CONTENT_BYTES,
            content.len()
        ));
    }
    Ok(())
}

/// NIP-10 thread reference.
pub struct ThreadRef {
    pub root_event_id: EventId,
    pub parent_event_id: EventId,
}

fn thread_tags(tr: &ThreadRef) -> Result<Vec<Tag>, String> {
    let root = tr.root_event_id.to_hex();
    let parent = tr.parent_event_id.to_hex();
    if root == parent {
        Ok(vec![tag(vec!["e", &root, "", "reply"])?])
    } else {
        Ok(vec![
            tag(vec!["e", &root, "", "root"])?,
            tag(vec!["e", &parent, "", "reply"])?,
        ])
    }
}

fn mention_tags(mentions: &[&str]) -> Result<Vec<Tag>, String> {
    if mentions.len() > MAX_MENTIONS {
        return Err(format!("too many mentions (max {MAX_MENTIONS})"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut tags = Vec::new();
    for &hex in mentions {
        check_pubkey(hex)?;
        let lower = hex.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            tags.push(tag(vec!["p", &lower])?);
        }
    }
    Ok(tags)
}

fn mention_reference_tags(mentions: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for mention in mentions {
        if mention.first().map(String::as_str) != Some("mention") {
            return Err(format!(
                "mention reference tags must use 'mention' prefix (got {:?})",
                mention.first()
            ));
        }
        let Some(pubkey) = mention.get(1) else {
            return Err("mention reference tag missing pubkey".into());
        };
        check_pubkey(pubkey)?;
        tags.push(tag(vec!["mention", &pubkey.to_ascii_lowercase()])?);
    }
    Ok(())
}

/// Validate and append imeta tags. Rejects any tag whose first element is not "imeta"
/// to prevent injection of arbitrary tags (e.g., forged "h", "e", or "p" tags).
fn imeta_tags(media_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for mt in media_tags {
        if mt.first().map(String::as_str) != Some("imeta") {
            return Err(format!(
                "media tags must use 'imeta' prefix (got {:?})",
                mt.first()
            ));
        }
        let parts: Vec<&str> = mt.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid imeta tag: {e}"))?);
    }
    Ok(())
}

/// Validate and append NIP-30 custom-emoji tags. Mirrors `imeta_tags`: rejects
/// any tag whose first element is not "emoji" so this path can't be used to
/// smuggle forged "h"/"e"/"p" tags. Each tag is `["emoji", shortcode, url]`.
fn emoji_tags(emoji_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for et in emoji_tags {
        if et.first().map(String::as_str) != Some("emoji") {
            return Err(format!(
                "emoji tags must use 'emoji' prefix (got {:?})",
                et.first()
            ));
        }
        let parts: Vec<&str> = et.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid emoji tag: {e}"))?);
    }
    Ok(())
}

/// Validate a hex pubkey is exactly 64 hex characters.
fn check_pubkey(pubkey: &str) -> Result<(), String> {
    if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "pubkey must be a 64-character hex string (got {} chars)",
            pubkey.len()
        ));
    }
    Ok(())
}

// ── Channel operations ───────────────────────────────────────────────────────

/// Kind 9007 — create channel.
pub fn build_create_channel(
    channel_id: Uuid,
    name: &str,
    visibility: &str,
    channel_type: &str,
    about: Option<&str>,
    ttl_seconds: Option<i32>,
) -> Result<EventBuilder, String> {
    let mut tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["name", name])?,
        tag(vec!["visibility", visibility])?,
        tag(vec!["channel_type", channel_type])?,
    ];
    if let Some(a) = about {
        tags.push(tag(vec!["about", a])?);
    }
    if let Some(ttl) = ttl_seconds {
        tags.push(tag(vec!["ttl", &ttl.to_string()])?);
    }
    Ok(EventBuilder::new(Kind::Custom(9007), "").tags(tags))
}

/// Kind 9021 — join channel.
pub fn build_join(channel_id: Uuid) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(9021), "").tags(tags))
}

/// Kind 9022 — leave channel.
pub fn build_leave(channel_id: Uuid) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(9022), "").tags(tags))
}

/// Kind 9002 — update channel name/description/visibility/ttl.
///
/// `ttl`: outer `None` leaves it unchanged; `Some(Some(secs))` sets the
/// ephemeral timeout; `Some(None)` clears it (emits `["ttl", ""]`).
pub fn build_update_channel(
    channel_id: Uuid,
    name: Option<&str>,
    about: Option<&str>,
    visibility: Option<&str>,
    ttl: Option<Option<i32>>,
) -> Result<EventBuilder, String> {
    if name.is_none() && about.is_none() && visibility.is_none() && ttl.is_none() {
        return Err("at least one of name, about, visibility, or ttl must be provided".into());
    }
    if let Some(v) = visibility {
        if v != "open" && v != "private" {
            return Err("visibility must be \"open\" or \"private\"".into());
        }
    }
    let mut tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    if let Some(n) = name {
        tags.push(tag(vec!["name", n])?);
    }
    if let Some(a) = about {
        tags.push(tag(vec!["about", a])?);
    }
    if let Some(v) = visibility {
        tags.push(tag(vec!["visibility", v])?);
    }
    if let Some(ttl) = ttl {
        match ttl {
            Some(secs) => tags.push(tag(vec!["ttl", &secs.to_string()])?),
            None => tags.push(tag(vec!["ttl", ""])?),
        }
    }
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Kind 9002 — set topic.
pub fn build_set_topic(channel_id: Uuid, topic: &str) -> Result<EventBuilder, String> {
    let tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["topic", topic])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Kind 9002 — set purpose.
pub fn build_set_purpose(channel_id: Uuid, purpose: &str) -> Result<EventBuilder, String> {
    let tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["purpose", purpose])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Kind 9002 — archive.
pub fn build_archive(channel_id: Uuid) -> Result<EventBuilder, String> {
    let tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["archived", "true"])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Kind 9002 — unarchive.
pub fn build_unarchive(channel_id: Uuid) -> Result<EventBuilder, String> {
    let tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["archived", "false"])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Kind 9008 — delete channel.
pub fn build_delete_channel(channel_id: Uuid) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(9008), "").tags(tags))
}

// ── Membership ───────────────────────────────────────────────────────────────

/// Kind 9000 — add member.
pub fn build_add_member(
    channel_id: Uuid,
    target_pubkey: &str,
    role: Option<&str>,
) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    let mut tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?,
    ];
    if let Some(r) = role {
        tags.push(tag(vec!["role", r])?);
    }
    Ok(EventBuilder::new(Kind::Custom(9000), "").tags(tags))
}

/// Kind 9001 — remove member.
pub fn build_remove_member(channel_id: Uuid, target_pubkey: &str) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    let tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9001), "").tags(tags))
}

// ── Messages ─────────────────────────────────────────────────────────────────

/// Kind 9 — stream message.
pub fn build_message(
    channel_id: Uuid,
    content: &str,
    thread_ref: Option<&ThreadRef>,
    mentions: &[&str],
    media_tags: &[Vec<String>],
    custom_emoji_tags: &[Vec<String>],
    mention_ref_tags: &[Vec<String>],
) -> Result<EventBuilder, String> {
    build_message_with_client_tags(
        channel_id,
        content,
        thread_ref,
        mentions,
        media_tags,
        custom_emoji_tags,
        mention_ref_tags,
        &[],
    )
}

/// Kind 9 — stream message with internal client marker tags.
///
/// This is intentionally narrower than arbitrary extra tags: callers can add
/// only `["client", ...]` tags, which are useful for idempotency markers but
/// cannot forge channel/thread/mention metadata.
#[allow(clippy::too_many_arguments)]
pub fn build_message_with_client_tags(
    channel_id: Uuid,
    content: &str,
    thread_ref: Option<&ThreadRef>,
    mentions: &[&str],
    media_tags: &[Vec<String>],
    custom_emoji_tags: &[Vec<String>],
    mention_ref_tags: &[Vec<String>],
    client_tags: &[Vec<String>],
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let mut tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    if let Some(tr) = thread_ref {
        tags.extend(thread_tags(tr)?);
    }
    tags.extend(mention_tags(mentions)?);
    imeta_tags(media_tags, &mut tags)?;
    emoji_tags(custom_emoji_tags, &mut tags)?;
    mention_reference_tags(mention_ref_tags, &mut tags)?;
    append_client_tags(client_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(9), content).tags(tags))
}

fn append_client_tags(client_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for client_tag in client_tags {
        if client_tag.first().map(String::as_str) != Some("client") {
            return Err(format!(
                "client tags must use 'client' prefix (got {:?})",
                client_tag.first()
            ));
        }
        if client_tag.len() < 2 {
            return Err("client tag missing marker".into());
        }
        let parts: Vec<&str> = client_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid client tag: {e}"))?);
    }
    Ok(())
}

/// Kind 45001 — forum post.
pub fn build_forum_post(
    channel_id: Uuid,
    content: &str,
    mentions: &[&str],
    media_tags: &[Vec<String>],
    mention_ref_tags: &[Vec<String>],
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let mut tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    tags.extend(mention_tags(mentions)?);
    imeta_tags(media_tags, &mut tags)?;
    mention_reference_tags(mention_ref_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(45001), content).tags(tags))
}

/// Kind 45003 — forum comment.
pub fn build_forum_comment(
    channel_id: Uuid,
    content: &str,
    thread_ref: &ThreadRef,
    mentions: &[&str],
    media_tags: &[Vec<String>],
    mention_ref_tags: &[Vec<String>],
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let mut tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    tags.extend(thread_tags(thread_ref)?);
    tags.extend(mention_tags(mentions)?);
    imeta_tags(media_tags, &mut tags)?;
    mention_reference_tags(mention_ref_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(45003), content).tags(tags))
}

/// Kind 40003 — edit a message. Carries the full new content AND a fresh
/// imeta tag set; the receiver overlays the imeta tags onto the original
/// event so the rendered message reflects exactly the edited state. NIP-30
/// custom-emoji tags ride along the same way so an edited body's `:shortcode:`s
/// stay resolvable (the send path attaches these too).
///
/// `mentions` carries the pubkeys of mentions that are *newly added* by this
/// edit (the caller diffs the edited body against the original). Only those get
/// a `p` tag so the newly-mentioned party is notified/woken, while a typo-fix
/// edit that leaves the mention set unchanged emits no `p` tags and never
/// re-wakes anyone. This mirrors the send path's `mention_tags` (dedup +
/// lowercase); the receiver overlays these onto the original event's audience.
pub fn build_message_edit(
    channel_id: Uuid,
    target_event_id: EventId,
    content: &str,
    media_tags: &[Vec<String>],
    custom_emoji_tags: &[Vec<String>],
    mentions: &[&str],
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let mut tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["e", &target_event_id.to_hex()])?,
    ];
    tags.extend(mention_tags(mentions)?);
    imeta_tags(media_tags, &mut tags)?;
    emoji_tags(custom_emoji_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(40003), content).tags(tags))
}

/// Kind 5 — NIP-09 deletion. The `h` tag is non-standard for NIP-09 but is
/// required so channel-scoped subscriptions observe the delete.
pub fn build_delete_compat(
    channel_id: Uuid,
    target_event_id: EventId,
) -> Result<EventBuilder, String> {
    let tags = vec![
        tag(vec!["h", &channel_id.to_string()])?,
        tag(vec!["e", &target_event_id.to_hex()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

// ── Reactions ────────────────────────────────────────────────────────────────

/// Kind 7 — NIP-25 reaction.
pub fn build_reaction(target_event_id: EventId, emoji: &str) -> Result<EventBuilder, String> {
    if emoji.chars().count() > MAX_EMOJI_CHARS {
        return Err(format!(
            "emoji exceeds maximum length of {MAX_EMOJI_CHARS} characters"
        ));
    }
    let tags = vec![tag(vec!["e", &target_event_id.to_hex()])?];
    Ok(EventBuilder::new(Kind::Custom(7), emoji).tags(tags))
}

/// Kind 5 — delete a reaction event.
pub fn build_remove_reaction(reaction_event_id: EventId) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["e", &reaction_event_id.to_hex()])?];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

// ── Canvas ───────────────────────────────────────────────────────────────────

/// Kind 40100 — set canvas.
pub fn build_set_canvas(channel_id: Uuid, content: &str) -> Result<EventBuilder, String> {
    check_content(content)?;
    let tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(40100), content).tags(tags))
}

// ── Profile ──────────────────────────────────────────────────────────────────

/// Kind 0 — NIP-01 profile metadata (full snapshot).
pub fn build_profile(
    display_name: Option<&str>,
    name: Option<&str>,
    picture: Option<&str>,
    about: Option<&str>,
    nip05: Option<&str>,
) -> Result<EventBuilder, String> {
    let mut map = serde_json::Map::new();
    if let Some(v) = display_name {
        map.insert("display_name".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = name {
        map.insert("name".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = picture {
        map.insert("picture".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = about {
        map.insert("about".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = nip05 {
        map.insert("nip05".into(), serde_json::Value::String(v.into()));
    }
    let content = serde_json::Value::Object(map).to_string();
    Ok(EventBuilder::new(Kind::Custom(0), content))
}

// ── Huddles ──────────────────────────────────────────────────────────────────

/// Validate that a string is a valid UUID (defense-in-depth for `&str` channel IDs).
fn validate_channel_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id).map_err(|_| format!("invalid channel UUID: {id}"))?;
    Ok(())
}

/// Shared builder for huddle lifecycle events (kinds 48100–48103).
/// All huddle events share: validate two channel IDs, JSON content with
/// `ephemeral_channel_id`, an `["h", parent_channel_id]` tag, and an
/// optional `["p", participant_pubkey]` tag for join/leave identity.
fn build_huddle_event(
    kind: u16,
    parent_channel_id: &str,
    ephemeral_channel_id: &str,
    extra_fields: &[(&str, &str)],
    participant_pubkey: Option<&str>,
) -> Result<EventBuilder, String> {
    validate_channel_id(parent_channel_id)?;
    validate_channel_id(ephemeral_channel_id)?;
    let mut content = serde_json::json!({
        "ephemeral_channel_id": ephemeral_channel_id,
    });
    for (k, v) in extra_fields {
        content[*k] = serde_json::Value::String(v.to_string());
    }
    let mut tags = vec![tag(vec!["h", parent_channel_id])?];
    if let Some(pk) = participant_pubkey {
        tags.push(tag(vec!["p", pk])?);
    }
    Ok(EventBuilder::new(Kind::Custom(kind), content.to_string()).tags(tags))
}

/// Kind 48100 — huddle started advisory posted to the parent channel.
pub fn build_huddle_started(
    parent_channel_id: &str,
    ephemeral_channel_id: &str,
) -> Result<EventBuilder, String> {
    build_huddle_event(48100, parent_channel_id, ephemeral_channel_id, &[], None)
}

/// Kind 48103 — huddle ended, posted to the parent channel.
pub fn build_huddle_ended(
    parent_channel_id: &str,
    ephemeral_channel_id: &str,
) -> Result<EventBuilder, String> {
    build_huddle_event(48103, parent_channel_id, ephemeral_channel_id, &[], None)
}

/// Kind 48106 — voice-mode guidelines for agents in a huddle.
///
/// Posted to the **ephemeral** channel (not the parent) so agents see it
/// via EOSE replay when they subscribe. Uses a dedicated kind so the TTS
/// pipeline can filter it out without fragile content-prefix matching.
pub fn build_huddle_guidelines(
    ephemeral_channel_id: &str,
    guidelines_text: &str,
) -> Result<EventBuilder, String> {
    validate_channel_id(ephemeral_channel_id)?;
    check_content(guidelines_text)?;
    let tags = vec![tag(vec!["h", ephemeral_channel_id])?];
    Ok(EventBuilder::new(Kind::Custom(48106), guidelines_text).tags(tags))
}

// ── Social notes ────────────────────────────────────────────────────────────

/// Kind 1 — NIP-01 short text note (global, no channel scope).
pub fn build_note(
    content: &str,
    reply_to_event_id: Option<EventId>,
    mentions: &[&str],
    media_tags: &[Vec<String>],
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let mut tags = Vec::new();
    if let Some(parent) = reply_to_event_id {
        tags.push(tag(vec!["e", &parent.to_hex(), "", "reply"])?);
    }
    tags.extend(mention_tags(mentions)?);
    imeta_tags(media_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::TextNote, content).tags(tags))
}

// ── Relay admin (NIP-43) ────────────────────────────────────────────────────

/// Allowed relay member roles for NIP-43 admin commands.
const VALID_RELAY_ROLES: &[&str] = &["owner", "admin", "member"];

fn check_relay_role(role: &str) -> Result<(), String> {
    if !VALID_RELAY_ROLES.contains(&role) {
        return Err(format!(
            "invalid relay role \"{role}\" (expected one of: {})",
            VALID_RELAY_ROLES.join(", ")
        ));
    }
    Ok(())
}

/// Kind 9030 — add a pubkey to the relay member list.
pub fn build_relay_admin_add(target_pubkey: &str, role: &str) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    check_relay_role(role)?;
    let tags = vec![
        tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?,
        tag(vec!["role", role])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9030), "").tags(tags))
}

/// Kind 9031 — remove a pubkey from the relay member list.
pub fn build_relay_admin_remove(target_pubkey: &str) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    let tags = vec![tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?];
    Ok(EventBuilder::new(Kind::Custom(9031), "").tags(tags))
}

/// Kind 9032 — change the role of an existing relay member.
pub fn build_relay_admin_change_role(
    target_pubkey: &str,
    new_role: &str,
) -> Result<EventBuilder, String> {
    check_pubkey(target_pubkey)?;
    check_relay_role(new_role)?;
    let tags = vec![
        tag(vec!["p", &target_pubkey.to_ascii_lowercase()])?,
        tag(vec!["role", new_role])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9032), "").tags(tags))
}

// ── NIP-IA identity archival ─────────────────────────────────────────────────
//
// kind:9035 archive request, kind:9036 unarchive request.
// Both protected by NIP-70 (`["-"]`), p-tag the target, and may carry
// optional `reason` (machine-readable code), `replaced-by` (9035 only),
// and a NIP-OA `auth` tag for owner-of-agent requests.
//
// See docs/nips/NIP-IA.md §Event Formats. The relay verifies; the desktop's
// job is to produce a well-formed, signed request — consent path is selected
// by the relay, not declared here.

fn check_reason(reason: &str) -> Result<(), String> {
    // Reason codes are machine-readable strings; the spec doesn't cap length
    // but we keep them short to discourage stuffing prose where `content` goes.
    if reason.len() > 64 {
        return Err(format!(
            "reason code exceeds maximum length of 64 chars (got {})",
            reason.len()
        ));
    }
    if reason.chars().any(|c| c.is_control()) {
        return Err("reason code must not contain control characters".into());
    }
    Ok(())
}

fn identity_archive_tags(
    target_pubkey: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
    auth_tag: Option<&[String; 4]>,
) -> Result<Vec<Tag>, String> {
    check_pubkey(target_pubkey)?;
    let target_lower = target_pubkey.to_ascii_lowercase();

    let mut tags = Vec::with_capacity(5);
    // NIP-70: mark as protected administrative state.
    tags.push(tag(vec!["-"])?);
    tags.push(tag(vec!["p", &target_lower])?);

    if let Some(r) = reason {
        check_reason(r)?;
        tags.push(tag(vec!["reason", r])?);
    }

    if let Some(rb) = replaced_by {
        check_pubkey(rb)?;
        let rb_lower = rb.to_ascii_lowercase();
        if rb_lower == target_lower {
            return Err("replaced-by must differ from the target".into());
        }
        tags.push(tag(vec!["replaced-by", &rb_lower])?);
    }

    if let Some(auth) = auth_tag {
        // Structural check only — the relay performs full NIP-OA verification.
        // We require the label, a 64-hex owner pubkey, and a 128-hex signature.
        if auth[0] != "auth" {
            return Err(format!(
                "auth tag label must be \"auth\" (got \"{}\")",
                auth[0]
            ));
        }
        check_pubkey(&auth[1])?;
        if auth[3].len() != 128 || !auth[3].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("auth tag signature must be 128-character hex".into());
        }
        tags.push(tag(vec!["auth", &auth[1], &auth[2], &auth[3]])?);
    }

    Ok(tags)
}

/// Kind 9035 — NIP-IA archive request.
///
/// `content` is an optional human-readable reason (clients MUST NOT parse
/// authorization semantics from it). `reason` is the machine-readable code
/// (`rotated`, `retired`, `bot-rebuilt`, `left-organization`, `spam`, ...).
/// `replaced_by` is the rotation pointer. `auth` is a NIP-OA owner-attestation
/// tag required only for the owner-of-agent consent path.
///
/// `.allow_self_tagging()` is required: NIP-IA's self path has `actor==target`,
/// which means the request's `["p", target]` matches the signer. nostr 0.44
/// strips matching `p` tags by default — we need the wire form intact.
pub fn build_archive_identity_request(
    target_pubkey: &str,
    content: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
    auth: Option<&[String; 4]>,
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let tags = identity_archive_tags(target_pubkey, reason, replaced_by, auth)?;
    Ok(
        EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVE_REQUEST as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

/// Kind 9036 — NIP-IA unarchive request.
///
/// Same shape as 9035 minus `replaced-by` (which has no defined meaning on
/// unarchive per spec). `auth` is used for owner-of-agent unarchive paths.
/// See `build_archive_identity_request` for the rationale on
/// `.allow_self_tagging()`.
pub fn build_unarchive_identity_request(
    target_pubkey: &str,
    content: &str,
    reason: Option<&str>,
    auth: Option<&[String; 4]>,
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let tags = identity_archive_tags(target_pubkey, reason, None, auth)?;
    Ok(
        EventBuilder::new(Kind::Custom(KIND_IA_UNARCHIVE_REQUEST as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

/// Maximum contacts per contact list event.
const MAX_CONTACTS: usize = 10_000;

/// Kind 3 — NIP-02 contact list (replaceable, full snapshot).
pub fn build_contact_list(
    contacts: &[(&str, Option<&str>, Option<&str>)],
) -> Result<EventBuilder, String> {
    if contacts.len() > MAX_CONTACTS {
        return Err(format!(
            "too many contacts (max {MAX_CONTACTS}, got {})",
            contacts.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut tags = Vec::new();
    for &(pubkey, relay_url, petname) in contacts {
        check_pubkey(pubkey)?;
        let lower = pubkey.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            tags.push(tag(vec![
                "p",
                &lower,
                relay_url.unwrap_or(""),
                petname.unwrap_or(""),
            ])?);
        }
    }
    Ok(EventBuilder::new(Kind::ContactList, "").tags(tags))
}

/// Kind 41010 — open (or surface) a DM channel with the given participants.
///
/// Each pubkey is added as a `p` tag. The relay derives the canonical
/// channel id and replies via OK message with `response:{channel_id}`.
pub fn build_dm_open(pubkeys: &[String]) -> Result<EventBuilder, String> {
    if pubkeys.is_empty() {
        return Err("dm_open requires at least one pubkey".into());
    }
    let mut tags: Vec<Tag> = Vec::with_capacity(pubkeys.len());
    for pk in pubkeys {
        check_pubkey(pk)?;
        tags.push(tag(vec!["p", &pk.to_ascii_lowercase()])?);
    }
    Ok(EventBuilder::new(Kind::Custom(41010), "").tags(tags))
}

/// Kind 41012 — hide a DM channel from the user's listing.
pub fn build_dm_hide(channel_id: &str) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["h", channel_id])?];
    Ok(EventBuilder::new(Kind::Custom(41012), "").tags(tags))
}

/// Kind 30620 — replaceable workflow definition.
///
/// The `d` tag carries the workflow id; `h` tag carries the channel id; the
/// content is the YAML definition. Same (pubkey, d) replaces the prior version.
pub fn build_workflow_definition(
    workflow_id: &str,
    channel_id: &str,
    yaml_definition: &str,
) -> Result<EventBuilder, String> {
    check_content(yaml_definition)?;
    let tags = vec![tag(vec!["d", workflow_id])?, tag(vec!["h", channel_id])?];
    Ok(EventBuilder::new(Kind::Custom(30620), yaml_definition.to_string()).tags(tags))
}

/// Kind 5 — NIP-09 deletion targeting a kind:30620 workflow definition.
pub fn build_workflow_delete(
    workflow_id: &str,
    owner_pubkey_hex: &str,
) -> Result<EventBuilder, String> {
    let coord = format!("30620:{owner_pubkey_hex}:{workflow_id}");
    let tags = vec![tag(vec!["a", &coord])?];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

/// Kind 46020 — trigger a workflow run by id.
pub fn build_workflow_trigger(workflow_id: &str) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["d", workflow_id])?];
    Ok(EventBuilder::new(Kind::Custom(46020), "").tags(tags))
}

/// Kind 46030 — grant an approval token (with optional note).
pub fn build_approval_grant(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["t", token])?];
    Ok(EventBuilder::new(Kind::Custom(46030), note.unwrap_or("")).tags(tags))
}

/// Kind 46031 — deny an approval token (with optional note).
pub fn build_approval_deny(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["t", token])?];
    Ok(EventBuilder::new(Kind::Custom(46031), note.unwrap_or("")).tags(tags))
}

// ── Transport ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    /// Builder layout regression for the NIP-IA owner-of-agent archive flow.
    /// Compares against `docs/nips/NIP-IA.md` §Vector 1.
    #[test]
    fn archive_identity_request_matches_spec_vector_1_layout() {
        const OWNER_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        const TARGET_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        const CONDITIONS: &str = "kind=1&created_at<1713957000";
        const SIG: &str = "8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369";

        let auth: [String; 4] = [
            "auth".into(),
            OWNER_HEX.into(),
            CONDITIONS.into(),
            SIG.into(),
        ];
        let builder = build_archive_identity_request(
            TARGET_HEX,
            "Archiving zombie agent after rebuild.",
            Some("bot-rebuilt"),
            None,
            Some(&auth),
        )
        .expect("build_archive_identity_request");

        let owner_secret = nostr::SecretKey::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
        let owner_keys = Keys::new(owner_secret);
        let event = builder.sign_with_keys(&owner_keys).unwrap();

        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();

        assert_eq!(event.kind, Kind::Custom(KIND_IA_ARCHIVE_REQUEST as u16));
        // Spec layout: ["-"], ["p", target], ["reason", code], ["auth", ...]
        assert_eq!(tags[0], vec!["-"]);
        assert_eq!(tags[1], vec!["p", TARGET_HEX]);
        assert_eq!(tags[2], vec!["reason", "bot-rebuilt"]);
        assert_eq!(tags[3], vec!["auth", OWNER_HEX, CONDITIONS, SIG]);
    }

    #[test]
    fn archive_request_rejects_replaced_by_equal_target() {
        const TARGET_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        let err = build_archive_identity_request(TARGET_HEX, "", None, Some(TARGET_HEX), None)
            .unwrap_err();
        assert!(err.contains("replaced-by"));
    }

    #[test]
    fn unarchive_request_layout_self_path() {
        const TARGET_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        let builder = build_unarchive_identity_request(
            TARGET_HEX,
            "I am active again.",
            Some("returned"),
            None,
        )
        .unwrap();
        let target_secret = nostr::SecretKey::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .unwrap();
        let event = builder.sign_with_keys(&Keys::new(target_secret)).unwrap();
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(event.kind, Kind::Custom(KIND_IA_UNARCHIVE_REQUEST as u16));
        // Self-unarchive: the `p` tag MUST point at the signer. Verifies our
        // `.allow_self_tagging()` call survives nostr 0.44's default scrub.
        assert_eq!(tags[0], vec!["-"]);
        assert_eq!(tags[1], vec!["p", TARGET_HEX]);
        assert_eq!(tags[2], vec!["reason", "returned"]);
        assert_eq!(tags.len(), 3, "self unarchive must not carry auth tag");
        assert_eq!(event.pubkey.to_hex(), TARGET_HEX);
    }

    // ── build_message_edit `p`-tag emission (lane 8ace8eed) ──────────────
    //
    // The composer diffs the edited body's mentions against the original and
    // hands `build_message_edit` only the *newly added* pubkeys. These tests
    // pin the builder's contract given that contract: emit a `p` per added
    // mention (deduped, lowercased), and none when the added set is empty
    // (typo-fix edit) — so an unchanged mention set re-wakes nobody.

    const CH_ID: &str = "11111111-1111-4111-8111-111111111111";
    const ALICE_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const BOB_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

    fn edit_tags(mentions: &[&str]) -> Vec<Vec<String>> {
        let channel = Uuid::parse_str(CH_ID).unwrap();
        let target =
            EventId::from_hex("d24da132115ca0a46233cf4c2ad8338fbf914250cbcaa9181a6dd59533cb5ac1")
                .unwrap();
        let builder = build_message_edit(channel, target, "hi @alice", &[], &[], mentions).unwrap();
        let secret = nostr::SecretKey::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000003",
        )
        .unwrap();
        let event = builder.sign_with_keys(&Keys::new(secret)).unwrap();
        event.tags.iter().map(|t| t.as_slice().to_vec()).collect()
    }

    #[test]
    fn edit_with_added_mention_emits_p_tag() {
        let tags = edit_tags(&[ALICE_HEX]);
        assert_eq!(tags[0][0], "h");
        assert_eq!(tags[1][0], "e");
        // The `p` tag rides right after the `e` tag (insertion order).
        assert_eq!(tags[2], vec!["p".to_string(), ALICE_HEX.to_string()]);
    }

    #[test]
    fn edit_with_no_added_mentions_emits_no_p_tag() {
        // Typo-fix edit: mention set unchanged, so the composer passes `&[]`.
        // The edit event must carry no `p` tag and re-wake nobody.
        let tags = edit_tags(&[]);
        assert!(
            !tags
                .iter()
                .any(|t| t.first().map(String::as_str) == Some("p")),
            "unchanged-mention edit must not emit any `p` tag, got {tags:?}"
        );
    }

    #[test]
    fn edit_mentions_are_deduped_and_lowercased() {
        let alice_upper = ALICE_HEX.to_ascii_uppercase();
        let tags = edit_tags(&[ALICE_HEX, &alice_upper, BOB_HEX]);
        let p_tags: Vec<&Vec<String>> = tags
            .iter()
            .filter(|t| t.first().map(String::as_str) == Some("p"))
            .collect();
        // ALICE appears twice (mixed case) but collapses to one lowercase tag.
        assert_eq!(
            p_tags.len(),
            2,
            "duplicate mention must collapse, got {p_tags:?}"
        );
        assert_eq!(p_tags[0], &vec!["p".to_string(), ALICE_HEX.to_string()]);
        assert_eq!(p_tags[1], &vec!["p".to_string(), BOB_HEX.to_string()]);
    }
}
