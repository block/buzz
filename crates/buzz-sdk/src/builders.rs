//! Typed event builder functions (38 builders).
//!
//! All functions return `Result<nostr::EventBuilder, SdkError>`.
//! The caller signs: `builder.sign_with_keys(&keys)?`.

use buzz_core::{
    kind::{
        KIND_AGENT_OBSERVER_FRAME, KIND_APPROVAL_DENY, KIND_APPROVAL_GRANT, KIND_DELETION,
        KIND_DM_ADD_MEMBER, KIND_DM_OPEN, KIND_EMOJI_SET, KIND_GIT_ISSUE, KIND_GIT_PATCH,
        KIND_GIT_REPO_ANNOUNCEMENT, KIND_GIT_STATUS_CLOSED, KIND_GIT_STATUS_DRAFT,
        KIND_GIT_STATUS_MERGED, KIND_GIT_STATUS_OPEN, KIND_PRESENCE_UPDATE, KIND_WORKFLOW_DEF,
        KIND_WORKFLOW_TRIGGER,
    },
    observer::{
        content_looks_like_nip44, OBSERVER_AGENT_TAG, OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG,
        OBSERVER_FRAME_TELEMETRY,
    },
};
use nostr::{EventBuilder, Kind, Tag};
use uuid::Uuid;

use crate::{
    ChannelKind, CustomEmoji, DiffMeta, MemberRole, SdkError, ThreadRef, Visibility, VoteDirection,
};

/// Parse a tag slice, mapping errors to `SdkError::InvalidTag`.
fn tag(parts: &[&str]) -> Result<Tag, SdkError> {
    Tag::parse(parts.iter().copied()).map_err(|e| SdkError::InvalidTag(e.to_string()))
}

/// Validate content byte length.
fn check_content(content: &str, max: usize) -> Result<(), SdkError> {
    let got = content.len();
    if got > max {
        return Err(SdkError::ContentTooLarge { max, got });
    }
    Ok(())
}

/// Validate hex string has at least `min_len` hex characters.
fn check_hex_len(s: &str, min_len: usize, field: &str) -> Result<(), SdkError> {
    if s.len() < min_len || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidDiffMeta(format!(
            "{field} must be at least {min_len} hex characters (got {:?})",
            s
        )));
    }
    Ok(())
}

/// Validate a git commit-like hex id (commit, parent-commit, euc,
/// merge-commit, applied-as-commit). Git object ids are full SHA-1 (40 hex
/// chars) or SHA-256 (64 hex chars) — anything shorter is an abbreviated
/// ref that NIP-34 canonical tags shouldn't carry, since consumers resolve
/// these against the actual repo.
fn check_commit_hex(s: &str, field: &str) -> Result<(), SdkError> {
    if (s.len() != 40 && s.len() != 64) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a full 40-character (SHA-1) or 64-character (SHA-256) hex commit id (got {:?})",
            s
        )));
    }
    Ok(())
}

fn check_pubkey_hex(s: &str, field: &str) -> Result<String, SdkError> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a 64-character hex pubkey"
        )));
    }
    Ok(s.to_ascii_lowercase())
}

/// Validate an exact-length hex string (event ids), returning it lowercased.
fn check_hex_exact(s: &str, len: usize, field: &str) -> Result<String, SdkError> {
    if s.len() != len || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a {len}-character hex string"
        )));
    }
    Ok(s.to_ascii_lowercase())
}

/// Validate a git repo identifier: `[a-zA-Z0-9._-]{1,64}`, no leading dot,
/// no `..`. Shared by `build_repo_announcement` and `GitRepoCoord` so a
/// repo coordinate built directly through the SDK (bypassing CLI-side
/// `validate_repo_id`) can't slip an invalid `d`-tag into an `a`-tag value.
fn check_repo_id(repo_id: &str) -> Result<(), SdkError> {
    if repo_id.is_empty() {
        return Err(SdkError::InvalidInput("repo_id must not be empty".into()));
    }
    if repo_id.len() > 64 {
        return Err(SdkError::InvalidInput(format!(
            "repo_id exceeds 64 characters (got {})",
            repo_id.len()
        )));
    }
    if !repo_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(SdkError::InvalidInput(
            "repo_id may only contain [a-zA-Z0-9._-]".into(),
        ));
    }
    if repo_id.starts_with('.') {
        return Err(SdkError::InvalidInput(
            "repo_id must not start with a dot".into(),
        ));
    }
    if repo_id.contains("..") {
        return Err(SdkError::InvalidInput(
            "repo_id must not contain '..'".into(),
        ));
    }
    Ok(())
}

/// Validate and normalize a NIP-30 custom emoji shortcode.
///
/// Shortcodes are case-insensitive in Buzz's relay-global set; lowercase
/// normalization prevents `party_parrot` and `Party_Parrot` from colliding.
pub fn normalize_custom_emoji_shortcode(shortcode: &str) -> Result<String, SdkError> {
    let trimmed = shortcode.trim().trim_matches(':');
    if trimmed.is_empty() {
        return Err(SdkError::InvalidInput(
            "emoji shortcode must not be empty".into(),
        ));
    }
    if trimmed.len() > 64 {
        return Err(SdkError::InvalidInput(format!(
            "emoji shortcode exceeds 64 bytes (got {})",
            trimmed.len()
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(SdkError::InvalidInput(
            "emoji shortcode may only contain ASCII letters, digits, hyphens, and underscores"
                .into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn check_custom_emoji_url(url: &str) -> Result<(), SdkError> {
    if url.is_empty() {
        return Err(SdkError::InvalidInput(
            "emoji image URL must not be empty".into(),
        ));
    }
    if url.len() > 2048 {
        return Err(SdkError::InvalidInput(format!(
            "emoji image URL exceeds 2048 bytes (got {})",
            url.len()
        )));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(SdkError::InvalidInput(
            "emoji image URL must start with http:// or https://".into(),
        ));
    }
    Ok(())
}

/// Emit NIP-10 e-tags for a `ThreadRef`.
fn thread_tags(thread_ref: &ThreadRef, tags: &mut Vec<Tag>) -> Result<(), SdkError> {
    let root = thread_ref.root_event_id.to_hex();
    let parent = thread_ref.parent_event_id.to_hex();
    if root == parent {
        // Direct reply
        tags.push(tag(&["e", &root, "", "reply"])?);
    } else {
        // Nested reply
        tags.push(tag(&["e", &root, "", "root"])?);
        tags.push(tag(&["e", &parent, "", "reply"])?);
    }
    Ok(())
}

/// Deduplicate and cap mentions, emitting p-tags.
fn mention_tags(mentions: &[&str], tags: &mut Vec<Tag>) -> Result<(), SdkError> {
    if mentions.len() > crate::mentions::MENTION_CAP {
        return Err(SdkError::TooManyMentions);
    }
    let mut seen = std::collections::HashSet::new();
    for &hex in mentions {
        let lower = hex.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            tags.push(tag(&["p", &lower])?);
        }
    }
    Ok(())
}

/// Emit imeta tags from raw tag vectors.
fn imeta_tags(media_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), SdkError> {
    for mt in media_tags {
        let parts: Vec<&str> = mt.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| SdkError::InvalidTag(e.to_string()))?);
    }
    Ok(())
}

/// Build a stream message (kind 9).
///
/// - `channel_id`: target channel UUID
/// - `content`: message text (max 64 KiB)
/// - `thread_ref`: optional NIP-10 reply context
/// - `mentions`: pubkey hex strings to p-tag (deduped, max 50)
/// - `broadcast`: if true, adds `["broadcast", "1"]` tag
/// - `media_tags`: raw imeta tag vectors
pub fn build_message(
    channel_id: Uuid,
    content: &str,
    thread_ref: Option<&ThreadRef>,
    mentions: &[&str],
    broadcast: bool,
    media_tags: &[Vec<String>],
) -> Result<EventBuilder, SdkError> {
    check_content(content, 64 * 1024)?;
    let mut tags = vec![tag(&["h", &channel_id.to_string()])?];
    if let Some(tr) = thread_ref {
        thread_tags(tr, &mut tags)?;
    }
    mention_tags(mentions, &mut tags)?;
    if broadcast {
        tags.push(tag(&["broadcast", "1"])?);
    }
    imeta_tags(media_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(9), content).tags(tags))
}

/// Build an encrypted agent observer frame (kind 24200).
///
/// `recipient_pubkey` is the cleartext `p` tag used by the relay for owner-only
/// routing. `agent_pubkey` identifies the managed agent whose observer stream
/// this frame belongs to. `encrypted_content` must be NIP-44 v2 ciphertext.
pub fn build_agent_observer_frame(
    recipient_pubkey: &str,
    agent_pubkey: &str,
    frame: &str,
    encrypted_content: &str,
) -> Result<EventBuilder, SdkError> {
    if frame != OBSERVER_FRAME_TELEMETRY && frame != OBSERVER_FRAME_CONTROL {
        return Err(SdkError::InvalidInput(format!(
            "observer frame must be {OBSERVER_FRAME_TELEMETRY:?} or {OBSERVER_FRAME_CONTROL:?}"
        )));
    }
    if !content_looks_like_nip44(encrypted_content) {
        return Err(SdkError::InvalidInput(
            "observer frame content must be NIP-44 v2 ciphertext".into(),
        ));
    }

    let recipient_pubkey = check_pubkey_hex(recipient_pubkey, "recipient_pubkey")?;
    let agent_pubkey = check_pubkey_hex(agent_pubkey, "agent_pubkey")?;
    let tags = vec![
        tag(&["p", &recipient_pubkey])?,
        tag(&[OBSERVER_AGENT_TAG, &agent_pubkey])?,
        tag(&[OBSERVER_FRAME_TAG, frame])?,
    ];

    Ok(EventBuilder::new(
        Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16),
        encrypted_content,
    )
    .tags(tags))
}

/// Build a forum post thread root (kind 45001).
pub fn build_forum_post(
    channel_id: Uuid,
    content: &str,
    mentions: &[&str],
    media_tags: &[Vec<String>],
) -> Result<EventBuilder, SdkError> {
    check_content(content, 64 * 1024)?;
    let mut tags = vec![tag(&["h", &channel_id.to_string()])?];
    mention_tags(mentions, &mut tags)?;
    imeta_tags(media_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(45001), content).tags(tags))
}

/// Build a forum comment reply (kind 45003).
pub fn build_forum_comment(
    channel_id: Uuid,
    content: &str,
    thread_ref: &ThreadRef,
    mentions: &[&str],
    media_tags: &[Vec<String>],
) -> Result<EventBuilder, SdkError> {
    check_content(content, 64 * 1024)?;
    let mut tags = vec![tag(&["h", &channel_id.to_string()])?];
    thread_tags(thread_ref, &mut tags)?;
    mention_tags(mentions, &mut tags)?;
    imeta_tags(media_tags, &mut tags)?;
    Ok(EventBuilder::new(Kind::Custom(45003), content).tags(tags))
}

/// Build a diff/patch message (kind 40008).
pub fn build_diff_message(
    channel_id: Uuid,
    content: &str,
    diff_meta: &DiffMeta,
    thread_ref: Option<&ThreadRef>,
) -> Result<EventBuilder, SdkError> {
    check_content(content, 60 * 1024)?;

    // Validate DiffMeta
    if !diff_meta.repo_url.starts_with("http://") && !diff_meta.repo_url.starts_with("https://") {
        return Err(SdkError::InvalidDiffMeta(
            "repo_url must start with http:// or https://".into(),
        ));
    }
    check_hex_len(&diff_meta.commit_sha, 7, "commit_sha")?;
    if let Some(ref pc) = diff_meta.parent_commit {
        check_hex_len(pc, 7, "parent_commit")?;
    }
    match &diff_meta.branch {
        Some((src, tgt)) if src.is_empty() || tgt.is_empty() => {
            return Err(SdkError::InvalidDiffMeta(
                "branch requires both source and target to be non-empty".into(),
            ));
        }
        _ => {}
    }
    if let Some(pr) = diff_meta.pr_number {
        if pr == 0 {
            return Err(SdkError::InvalidDiffMeta(
                "pr_number must be positive".into(),
            ));
        }
    }

    let mut tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["repo", &diff_meta.repo_url])?,
        tag(&["commit", &diff_meta.commit_sha])?,
    ];
    if let Some(ref fp) = diff_meta.file_path {
        tags.push(tag(&["file", fp])?);
    }
    if let Some(ref pc) = diff_meta.parent_commit {
        tags.push(tag(&["parent-commit", pc])?);
    }
    if let Some((ref src, ref tgt)) = diff_meta.branch {
        tags.push(tag(&["branch", src, tgt])?);
    }
    if let Some(pr) = diff_meta.pr_number {
        tags.push(tag(&["pr", &pr.to_string()])?);
    }
    if let Some(ref lang) = diff_meta.language {
        tags.push(tag(&["l", lang])?);
    }
    if let Some(ref desc) = diff_meta.description {
        tags.push(tag(&["description", desc])?);
    }
    if diff_meta.truncated {
        tags.push(tag(&["truncated", "true"])?);
    }
    if let Some(ref alt) = diff_meta.alt_text {
        tags.push(tag(&["alt", alt])?);
    }
    if let Some(tr) = thread_ref {
        thread_tags(tr, &mut tags)?;
    }
    Ok(EventBuilder::new(Kind::Custom(40008), content).tags(tags))
}

/// Build an edit event targeting an existing message (kind 40003).
pub fn build_edit(
    channel_id: Uuid,
    target_event_id: nostr::EventId,
    new_content: &str,
) -> Result<EventBuilder, SdkError> {
    check_content(new_content, 64 * 1024)?;
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["e", &target_event_id.to_hex()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(40003), new_content).tags(tags))
}

/// Build a Buzz-native delete event (kind 9005).
pub fn build_delete_message(
    channel_id: Uuid,
    target_event_id: nostr::EventId,
) -> Result<EventBuilder, SdkError> {
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["e", &target_event_id.to_hex()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9005), "").tags(tags))
}

/// Build a NIP-09 deletion event (kind 5). The `h` tag is non-standard for
/// NIP-09 but is required so channel-scoped subscriptions observe the delete.
pub fn build_delete_compat(
    channel_id: Uuid,
    target_event_id: nostr::EventId,
) -> Result<EventBuilder, SdkError> {
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["e", &target_event_id.to_hex()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

/// Build a forum vote event (kind 45002). Content is `"+"` or `"-"`.
pub fn build_vote(
    channel_id: Uuid,
    target_event_id: nostr::EventId,
    direction: VoteDirection,
) -> Result<EventBuilder, SdkError> {
    let content = match direction {
        VoteDirection::Up => "+",
        VoteDirection::Down => "-",
    };
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["e", &target_event_id.to_hex()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(45002), content).tags(tags))
}

/// Build a NIP-25 reaction event (kind 7). Emoji max 64 chars.
pub fn build_reaction(
    target_event_id: nostr::EventId,
    emoji: &str,
) -> Result<EventBuilder, SdkError> {
    if emoji.chars().count() > 64 {
        return Err(SdkError::EmojiTooLong);
    }
    let tags = vec![tag(&["e", &target_event_id.to_hex()])?];
    Ok(EventBuilder::new(Kind::Custom(7), emoji).tags(tags))
}

/// Build a NIP-25 reaction event using a NIP-30 custom emoji.
///
/// The reaction content is `:shortcode:` and the event carries exactly one
/// `["emoji", shortcode, url]` tag, matching NIP-25's custom emoji reaction
/// guidance.
pub fn build_custom_emoji_reaction(
    target_event_id: nostr::EventId,
    shortcode: &str,
    url: &str,
) -> Result<EventBuilder, SdkError> {
    let shortcode = normalize_custom_emoji_shortcode(shortcode)?;
    check_custom_emoji_url(url)?;
    let content = format!(":{shortcode}:");
    let tags = vec![
        tag(&["e", &target_event_id.to_hex()])?,
        tag(&["emoji", &shortcode, url])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(7), content).tags(tags))
}

/// Build a deletion event targeting a reaction (kind 5).
pub fn build_remove_reaction(reaction_event_id: nostr::EventId) -> Result<EventBuilder, SdkError> {
    let tags = vec![tag(&["e", &reaction_event_id.to_hex()])?];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

/// d-tag for a member's own custom emoji set. Each member publishes one
/// user-signed kind:30030 under this d-tag; the workspace palette is the
/// client-side union of every member's set.
pub const CUSTOM_EMOJI_SET_D_TAG: &str = "buzz:custom-emoji";

/// Build a member's own custom emoji set event (kind:30030, NIP-30/NIP-51).
///
/// User-signed and parameterized-replaceable, keyed by `(pubkey, 30030,
/// "buzz:custom-emoji")`. Replaces the caller's prior set. The workspace
/// palette shown in clients is the union of every member's set, deduped by
/// `(shortcode, url)` on read. Add/remove is read-own-set → mutate → rebuild.
pub fn build_custom_emoji_set(emojis: &[CustomEmoji]) -> Result<EventBuilder, SdkError> {
    let mut seen = std::collections::HashSet::with_capacity(emojis.len());
    let mut tags = Vec::with_capacity(emojis.len() + 1);
    tags.push(tag(&["d", CUSTOM_EMOJI_SET_D_TAG])?);
    for emoji in emojis {
        let shortcode = normalize_custom_emoji_shortcode(&emoji.shortcode)?;
        check_custom_emoji_url(&emoji.url)?;
        if !seen.insert(shortcode.clone()) {
            return Err(SdkError::InvalidInput(format!(
                "duplicate emoji shortcode: {shortcode}"
            )));
        }
        tags.push(tag(&["emoji", &shortcode, &emoji.url])?);
    }
    Ok(EventBuilder::new(Kind::Custom(KIND_EMOJI_SET as u16), "").tags(tags))
}

/// Build a canvas update event (kind 40100).
pub fn build_set_canvas(channel_id: Uuid, content: &str) -> Result<EventBuilder, SdkError> {
    let tags = vec![tag(&["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(40100), content).tags(tags))
}

/// Build a NIP-01 profile metadata event (kind 0).
///
/// Only present (Some) fields are included in the JSON object.
pub fn build_profile(
    display_name: Option<&str>,
    name: Option<&str>,
    picture: Option<&str>,
    about: Option<&str>,
    nip05: Option<&str>,
) -> Result<EventBuilder, SdkError> {
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
    Ok(EventBuilder::new(Kind::Custom(0), content).tags([]))
}

/// Build a NIP-29 add-member event (kind 9000).
pub fn build_add_member(
    channel_id: Uuid,
    target_pubkey: &str,
    role: Option<MemberRole>,
) -> Result<EventBuilder, SdkError> {
    check_hex_len(target_pubkey, 64, "target_pubkey")?;
    let mut tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["p", &target_pubkey.to_ascii_lowercase()])?,
    ];
    if let Some(r) = role {
        tags.push(tag(&["role", r.as_str()])?);
    }
    Ok(EventBuilder::new(Kind::Custom(9000), "").tags(tags))
}

/// Build a NIP-29 remove-member event (kind 9001).
pub fn build_remove_member(
    channel_id: Uuid,
    target_pubkey: &str,
) -> Result<EventBuilder, SdkError> {
    check_hex_len(target_pubkey, 64, "target_pubkey")?;
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["p", &target_pubkey.to_ascii_lowercase()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9001), "").tags(tags))
}

/// Build a NIP-29 leave-request event (kind 9022).
pub fn build_leave(channel_id: Uuid) -> Result<EventBuilder, SdkError> {
    let tags = vec![tag(&["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(9022), "").tags(tags))
}

/// Build a NIP-29 edit-metadata event for name/about/visibility/ttl (kind 9002).
///
/// `ttl`: outer `None` leaves it unchanged; `Some(Some(secs))` sets the
/// ephemeral timeout; `Some(None)` clears it (emits `["ttl", ""]`).
pub fn build_update_channel(
    channel_id: Uuid,
    name: Option<&str>,
    about: Option<&str>,
    visibility: Option<&str>,
    ttl: Option<Option<i32>>,
) -> Result<EventBuilder, SdkError> {
    if name.is_none() && about.is_none() && visibility.is_none() && ttl.is_none() {
        return Err(SdkError::InvalidTag(
            "at least one of name, about, visibility, or ttl must be provided".into(),
        ));
    }
    if let Some(v) = visibility {
        if v != "open" && v != "private" {
            return Err(SdkError::InvalidTag(
                "visibility must be \"open\" or \"private\"".into(),
            ));
        }
    }
    let mut tags = vec![tag(&["h", &channel_id.to_string()])?];
    if let Some(n) = name {
        tags.push(tag(&["name", n])?);
    }
    if let Some(a) = about {
        tags.push(tag(&["about", a])?);
    }
    if let Some(v) = visibility {
        tags.push(tag(&["visibility", v])?);
    }
    if let Some(ttl) = ttl {
        match ttl {
            Some(secs) => tags.push(tag(&["ttl", &secs.to_string()])?),
            None => tags.push(tag(&["ttl", ""])?),
        }
    }
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Build a NIP-29 edit-metadata event for topic (kind 9002).
pub fn build_set_topic(channel_id: Uuid, topic: &str) -> Result<EventBuilder, SdkError> {
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["topic", topic])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Build a NIP-29 edit-metadata event for purpose (kind 9002).
pub fn build_set_purpose(channel_id: Uuid, purpose: &str) -> Result<EventBuilder, SdkError> {
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["purpose", purpose])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Build a NIP-29 create-group event (kind 9007).
///
/// `ttl`: `Some(secs)` makes the channel ephemeral with that lifetime in
/// seconds (the relay archives it once the deadline passes without activity);
/// `None` leaves it permanent.
pub fn build_create_channel(
    channel_id: Uuid,
    name: &str,
    visibility: Option<Visibility>,
    channel_type: Option<ChannelKind>,
    about: Option<&str>,
    ttl: Option<i32>,
) -> Result<EventBuilder, SdkError> {
    let mut tags = vec![tag(&["h", &channel_id.to_string()])?, tag(&["name", name])?];
    if let Some(v) = visibility {
        tags.push(tag(&["visibility", v.as_str()])?);
    }
    if let Some(ct) = channel_type {
        tags.push(tag(&["channel_type", ct.as_str()])?);
    }
    if let Some(a) = about {
        tags.push(tag(&["about", a])?);
    }
    if let Some(secs) = ttl {
        tags.push(tag(&["ttl", &secs.to_string()])?);
    }
    Ok(EventBuilder::new(Kind::Custom(9007), "").tags(tags))
}

/// Build a NIP-29 join-request event (kind 9021).
pub fn build_join(channel_id: Uuid) -> Result<EventBuilder, SdkError> {
    let tags = vec![tag(&["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(9021), "").tags(tags))
}

/// Build a NIP-29 archive event (kind 9002, `["archived", "true"]`).
pub fn build_archive(channel_id: Uuid) -> Result<EventBuilder, SdkError> {
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["archived", "true"])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Build a NIP-29 unarchive event (kind 9002, `["archived", "false"]`).
pub fn build_unarchive(channel_id: Uuid) -> Result<EventBuilder, SdkError> {
    let tags = vec![
        tag(&["h", &channel_id.to_string()])?,
        tag(&["archived", "false"])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(9002), "").tags(tags))
}

/// Build a NIP-29 delete-group event (kind 9008).
pub fn build_delete_channel(channel_id: Uuid) -> Result<EventBuilder, SdkError> {
    let tags = vec![tag(&["h", &channel_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(9008), "").tags(tags))
}

/// Build a global text note (kind:1, NIP-01).
///
/// `reply_to_event_id`: adds a single `["e", <id>, "", "reply"]` tag.
/// This is intentionally simpler than the full `ThreadRef` mechanism used
/// for channel messages — social notes use a flat reply model for now.
/// Full NIP-10 threading (root + reply + p-tags) is deferred.
pub fn build_note(
    content: &str,
    reply_to_event_id: Option<nostr::EventId>,
) -> Result<EventBuilder, SdkError> {
    check_content(content, 64 * 1024)?;
    let mut tags = vec![];
    if let Some(reply_id) = reply_to_event_id {
        tags.push(tag(&["e", &reply_id.to_hex(), "", "reply"])?);
    }
    Ok(EventBuilder::new(Kind::Custom(1), content).tags(tags))
}

/// Maximum number of contacts allowed in a single contact list event.
const MAX_CONTACTS: usize = 10_000;

/// Build a contact list replacement event (kind:3, NIP-02).
///
/// Each contact is `(pubkey_hex, relay_url, petname)`.
/// `pubkey_hex` must be exactly 64 hex characters (any case accepted, normalized
/// to lowercase before storage). Non-hex or wrong-length pubkeys are rejected
/// with `SdkError::InvalidInput`.
/// `relay_url` and `petname` may be `None` (stored as empty string per NIP-02).
///
/// Duplicate pubkeys are silently deduplicated — the first occurrence is kept.
///
/// Replaces the entire contact list — callers must read-before-write for deltas.
pub fn build_contact_list(
    contacts: &[(&str, Option<&str>, Option<&str>)],
) -> Result<EventBuilder, SdkError> {
    if contacts.len() > MAX_CONTACTS {
        return Err(SdkError::InvalidInput(format!(
            "contact list exceeds maximum of {} contacts (got {})",
            MAX_CONTACTS,
            contacts.len()
        )));
    }
    let mut seen = std::collections::HashSet::with_capacity(contacts.len());
    let mut tags = Vec::with_capacity(contacts.len());
    for &(pubkey_hex, relay_url, petname) in contacts {
        if pubkey_hex.len() != 64 || !pubkey_hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(SdkError::InvalidInput(format!(
                "contact pubkey must be exactly 64 hex chars, got len={}",
                pubkey_hex.len()
            )));
        }
        if let Some(url) = relay_url {
            if url.len() > 2048 {
                return Err(SdkError::InvalidInput(format!(
                    "relay_url exceeds 2048 bytes (got {})",
                    url.len()
                )));
            }
        }
        if let Some(name) = petname {
            if name.len() > 256 {
                return Err(SdkError::InvalidInput(format!(
                    "petname exceeds 256 bytes (got {})",
                    name.len()
                )));
            }
        }
        let lower = pubkey_hex.to_ascii_lowercase();
        if !seen.insert(lower.clone()) {
            continue;
        }
        tags.push(tag(&[
            "p",
            &lower,
            relay_url.unwrap_or(""),
            petname.unwrap_or(""),
        ])?);
    }
    Ok(EventBuilder::new(Kind::Custom(3), "").tags(tags))
}

/// Extract the channel UUID from an event's `h` tag.
///
/// Returns `None` if no `h` tag is present or the value is not a valid UUID.
pub fn extract_channel_id(event: &nostr::Event) -> Option<Uuid> {
    event.tags.iter().find_map(|t| {
        let vec = t.as_slice();
        if vec.first().map(|s| s.as_str()) == Some("h") {
            vec.get(1).and_then(|v| Uuid::parse_str(v.as_str()).ok())
        } else {
            None
        }
    })
}

/// Build a git repository announcement event (kind:30617, NIP-34).
///
/// Creates or updates a repository. The `repo_id` is the unique identifier
/// (d-tag) — must be `[a-zA-Z0-9._-]{1,64}`, no leading dots, no `..`.
///
/// This is a parameterized replaceable event: publishing again with the same
/// `repo_id` updates the announcement (relay overwrites the previous one).
pub fn build_repo_announcement(
    repo_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    clone_urls: &[&str],
    web_url: Option<&str>,
    relays: &[&str],
) -> Result<EventBuilder, SdkError> {
    // Validate repo_id
    check_repo_id(repo_id)?;

    // Validate optional name
    if let Some(n) = name {
        if n.len() > 128 {
            return Err(SdkError::InvalidInput(format!(
                "name exceeds 128 characters (got {})",
                n.len()
            )));
        }
    }

    // Validate optional description
    if let Some(d) = description {
        if d.len() > 1024 {
            return Err(SdkError::InvalidInput(format!(
                "description exceeds 1024 characters (got {})",
                d.len()
            )));
        }
    }

    // Validate clone_urls
    if clone_urls.len() > 5 {
        return Err(SdkError::InvalidInput(format!(
            "too many clone_urls (max 5, got {})",
            clone_urls.len()
        )));
    }
    for url in clone_urls {
        if url.is_empty() {
            return Err(SdkError::InvalidInput("clone_url must not be empty".into()));
        }
        if url.len() > 512 {
            return Err(SdkError::InvalidInput(format!(
                "clone_url exceeds 512 characters (got {})",
                url.len()
            )));
        }
    }

    // Validate web_url
    if let Some(url) = web_url {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(SdkError::InvalidInput(format!(
                "web_url must start with http:// or https:// (got {:?})",
                url
            )));
        }
        if url.len() > 512 {
            return Err(SdkError::InvalidInput(format!(
                "web_url exceeds 512 characters (got {})",
                url.len()
            )));
        }
    }

    // Validate relays
    if relays.len() > 10 {
        return Err(SdkError::InvalidInput(format!(
            "too many relays (max 10, got {})",
            relays.len()
        )));
    }
    for relay in relays {
        if !relay.starts_with("ws://") && !relay.starts_with("wss://") {
            return Err(SdkError::InvalidInput(format!(
                "relay must start with ws:// or wss:// (got {:?})",
                relay
            )));
        }
        if relay.len() > 256 {
            return Err(SdkError::InvalidInput(format!(
                "relay exceeds 256 characters (got {})",
                relay.len()
            )));
        }
    }

    // Build tags
    let mut tags = vec![tag(&["d", repo_id])?];
    if let Some(n) = name {
        tags.push(tag(&["name", n])?);
    }
    if let Some(d) = description {
        tags.push(tag(&["description", d])?);
    }
    if !clone_urls.is_empty() {
        let mut clone_tag = vec!["clone"];
        clone_tag.extend_from_slice(clone_urls);
        tags.push(tag(&clone_tag)?);
    }
    if let Some(url) = web_url {
        tags.push(tag(&["web", url])?);
    }
    if !relays.is_empty() {
        let mut relay_tag = vec!["relays"];
        relay_tag.extend_from_slice(relays);
        tags.push(tag(&relay_tag)?);
    }

    Ok(EventBuilder::new(Kind::Custom(KIND_GIT_REPO_ANNOUNCEMENT as u16), "").tags(tags))
}

/// Repository coordinate — owner pubkey + `d`-tag identifier.
///
/// Renders as the `a`-tag value clients use to address a kind:30617
/// announcement: `30617:<owner>:<id>`.
pub struct GitRepoCoord {
    /// 64-char hex pubkey of the repo's announcing owner.
    pub owner: String,
    /// The repo's `d`-tag identifier.
    pub id: String,
}

impl GitRepoCoord {
    fn to_a_tag_value(&self) -> Result<String, SdkError> {
        let owner = check_pubkey_hex(&self.owner, "repo owner")?;
        check_repo_id(&self.id)?;
        Ok(format!("30617:{owner}:{}", self.id))
    }
}

/// Metadata for a git patch event (kind:1617, NIP-34).
#[derive(Default)]
pub struct GitPatchMeta {
    /// Earliest-unique-commit of the repo (`r` tag, `euc` marker).
    pub euc: Option<String>,
    /// Additional pubkeys to `p`-tag besides the repo owner.
    pub recipients: Vec<String>,
    /// Previous patch in a series, or the original root patch when this is
    /// the first patch of a revision — emits `["e", id, "", "reply"]`.
    pub reply_to: Option<String>,
    /// First patch in a new series — emits `["t", "root"]`.
    pub root: bool,
    /// First patch in a revision of an existing series — emits `["t", "root-revision"]`.
    pub root_revision: bool,
    /// Commit ID this patch produces when applied (`commit` tag + `r` tag).
    pub commit: Option<String>,
    /// Parent commit ID (`parent-commit` tag).
    pub parent_commit: Option<String>,
    /// PGP signature of the commit, or `Some("")` for an explicitly unsigned commit.
    pub commit_pgp_sig: Option<String>,
    /// Committer identity: `(name, email, unix-timestamp, tz-offset-minutes)`.
    pub committer: Option<(String, String, String, String)>,
}

/// Build a git patch event (kind:1617, NIP-34).
///
/// `content` is the verbatim output of `git format-patch` — not truncated.
/// NIP-34 says patches SHOULD be used when under 60KB (PRs otherwise); this
/// builder enforces that bound rather than silently truncating a patch that
/// must remain applyable.
pub fn build_git_patch(
    repo: &GitRepoCoord,
    content: &str,
    meta: &GitPatchMeta,
) -> Result<EventBuilder, SdkError> {
    if content.trim().is_empty() {
        return Err(SdkError::InvalidInput(
            "patch content must not be empty — refusing to publish an unappliable patch".into(),
        ));
    }
    check_content(content, 60 * 1024)?;
    let a_value = repo.to_a_tag_value()?;
    let owner = check_pubkey_hex(&repo.owner, "repo owner")?;

    let mut tags = vec![tag(&["a", &a_value])?];
    if let Some(ref euc) = meta.euc {
        check_commit_hex(euc, "euc")?;
        tags.push(tag(&["r", euc, "euc"])?);
    }
    tags.push(tag(&["p", &owner])?);
    for recipient in &meta.recipients {
        let pk = check_pubkey_hex(recipient, "recipient")?;
        tags.push(tag(&["p", &pk])?);
    }
    if let Some(ref prev) = meta.reply_to {
        let event_id = check_hex_exact(prev, 64, "reply_to")?;
        tags.push(tag(&["e", &event_id, "", "reply"])?);
    }
    if meta.root && meta.root_revision {
        return Err(SdkError::InvalidInput(
            "patch cannot be both --root and --root-revision".into(),
        ));
    }
    if meta.root {
        tags.push(tag(&["t", "root"])?);
    }
    if meta.root_revision {
        tags.push(tag(&["t", "root-revision"])?);
    }
    if let Some(ref commit) = meta.commit {
        check_commit_hex(commit, "commit")?;
        tags.push(tag(&["commit", commit])?);
        tags.push(tag(&["r", commit])?);
    }
    if let Some(ref parent) = meta.parent_commit {
        check_commit_hex(parent, "parent_commit")?;
        tags.push(tag(&["parent-commit", parent])?);
    }
    if let Some(ref sig) = meta.commit_pgp_sig {
        tags.push(tag(&["commit-pgp-sig", sig])?);
    }
    if let Some((ref name, ref email, ref ts, ref tz)) = meta.committer {
        tags.push(tag(&["committer", name, email, ts, tz])?);
    }

    Ok(EventBuilder::new(Kind::Custom(KIND_GIT_PATCH as u16), content).tags(tags))
}

/// Metadata for a git issue event (kind:1621, NIP-34).
#[derive(Default)]
pub struct GitIssueMeta {
    /// Labels (`t` tags).
    pub labels: Vec<String>,
    /// Additional pubkeys to `p`-tag besides the repo owner.
    pub recipients: Vec<String>,
}

/// Build a git issue event (kind:1621, NIP-34). `content` is the markdown body.
pub fn build_git_issue(
    repo: &GitRepoCoord,
    subject: &str,
    content: &str,
    meta: &GitIssueMeta,
) -> Result<EventBuilder, SdkError> {
    check_content(content, 64 * 1024)?;
    if subject.is_empty() {
        return Err(SdkError::InvalidInput("subject must not be empty".into()));
    }
    if subject.len() > 256 {
        return Err(SdkError::InvalidInput(format!(
            "subject exceeds 256 characters (got {})",
            subject.len()
        )));
    }
    let a_value = repo.to_a_tag_value()?;
    let owner = check_pubkey_hex(&repo.owner, "repo owner")?;

    let mut tags = vec![tag(&["a", &a_value])?, tag(&["p", &owner])?];
    for recipient in &meta.recipients {
        let pk = check_pubkey_hex(recipient, "recipient")?;
        tags.push(tag(&["p", &pk])?);
    }
    tags.push(tag(&["subject", subject])?);
    for label in &meta.labels {
        tags.push(tag(&["t", label])?);
    }

    Ok(EventBuilder::new(Kind::Custom(KIND_GIT_ISSUE as u16), content).tags(tags))
}

/// Status to apply to a patch or issue root (kind:1630/1631/1632/1633, NIP-34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitStatus {
    /// 1630 — Open (the default state).
    Open,
    /// 1631 — Applied/Merged for patches; Resolved for issues.
    AppliedOrResolved,
    /// 1632 — Closed.
    Closed,
    /// 1633 — Draft.
    Draft,
}

/// A reference to an applied/merged patch event for a status `q` tag,
/// optionally carrying a relay-url and/or pubkey hint per NIP-34:
/// `['q', <id>, <relay-url>, <pubkey>]`.
///
/// Parsed from the CLI's `--q <id>[:<relay-url>[:<pubkey>]]` syntax via
/// [`GitAppliedPatchRef::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitAppliedPatchRef {
    /// The applied/merged patch event id (64-char hex).
    pub id: String,
    /// Optional relay-url hint where the patch event can be found.
    pub relay: Option<String>,
    /// Optional pubkey hint of the patch's author. Only meaningful when
    /// `relay` is also set, per NIP-34's `['q', id, relay, pubkey]` shape.
    pub pubkey: Option<String>,
}

impl GitAppliedPatchRef {
    /// Parse `<id>`, `<id>:<relay-url>`, or `<id>:<relay-url>:<pubkey>`.
    ///
    /// The relay-url segment may itself contain `:` (e.g. `wss://host:port`),
    /// so splitting is bounded to at most 3 parts rather than splitting on
    /// every colon.
    pub fn parse(spec: &str) -> Result<Self, SdkError> {
        let mut parts = spec.splitn(3, ':');
        let id = parts.next().unwrap_or_default().to_string();
        let rest = parts.next();
        match rest {
            None => Ok(GitAppliedPatchRef {
                id,
                relay: None,
                pubkey: None,
            }),
            Some(_) => {
                // Re-split with the relay url glued back together, since a
                // relay URL itself contains colons (`wss://host:port`); the
                // pubkey, if present, is always the last `:`-delimited
                // segment.
                let rest_str = &spec[id.len() + 1..];
                if let Some(idx) = rest_str.rfind(':') {
                    let candidate_pubkey = &rest_str[idx + 1..];
                    if candidate_pubkey.len() == 64
                        && candidate_pubkey.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        return Ok(GitAppliedPatchRef {
                            id,
                            relay: Some(rest_str[..idx].to_string()),
                            pubkey: Some(candidate_pubkey.to_ascii_lowercase()),
                        });
                    }
                }
                Ok(GitAppliedPatchRef {
                    id,
                    relay: Some(rest_str.to_string()),
                    pubkey: None,
                })
            }
        }
    }
}

impl GitStatus {
    fn kind(self) -> u16 {
        match self {
            GitStatus::Open => KIND_GIT_STATUS_OPEN as u16,
            GitStatus::AppliedOrResolved => KIND_GIT_STATUS_MERGED as u16,
            GitStatus::Closed => KIND_GIT_STATUS_CLOSED as u16,
            GitStatus::Draft => KIND_GIT_STATUS_DRAFT as u16,
        }
    }
}

/// Metadata for a git status event (kind:1630-1633, NIP-34). Applies to a
/// patch root, a patch revision root, an issue, or a PR.
#[derive(Default)]
pub struct GitStatusMeta {
    /// The issue/PR/original-root-patch event being given a status — required.
    pub root_event: String,
    /// When a revision was the one applied/merged, its root id.
    pub accepted_revision_root: Option<String>,
    /// Repo coordinate, included as an `a` tag for subscription efficiency.
    pub repo: Option<GitRepoCoord>,
    /// Earliest-unique-commit of the repo (`r` tag, no marker).
    pub euc: Option<String>,
    /// Additional `p` tags (root author, revision author, etc.) besides the repo owner.
    pub recipients: Vec<String>,
    /// Applied/merged patch event references (`q` tags) — kind:1631 only.
    pub applied_patches: Vec<GitAppliedPatchRef>,
    /// Merge commit id — kind:1631 only.
    pub merge_commit: Option<String>,
    /// Commit ids applied to the target branch — kind:1631 only.
    pub applied_as_commits: Vec<String>,
}

/// Build a git status event (kind:1630/1631/1632/1633, NIP-34).
/// `content` is optional markdown context for the status change.
pub fn build_git_status(
    status: GitStatus,
    content: &str,
    meta: &GitStatusMeta,
) -> Result<EventBuilder, SdkError> {
    check_content(content, 64 * 1024)?;
    let root = check_hex_exact(&meta.root_event, 64, "root_event")?;

    let mut tags = vec![tag(&["e", &root, "", "root"])?];
    if let Some(ref revision) = meta.accepted_revision_root {
        let revision = check_hex_exact(revision, 64, "accepted_revision_root")?;
        tags.push(tag(&["e", &revision, "", "reply"])?);
    }
    for recipient in &meta.recipients {
        let pk = check_pubkey_hex(recipient, "recipient")?;
        tags.push(tag(&["p", &pk])?);
    }
    if let Some(ref repo) = meta.repo {
        let a_value = repo.to_a_tag_value()?;
        tags.push(tag(&["a", &a_value])?);
    }
    if let Some(ref euc) = meta.euc {
        check_commit_hex(euc, "euc")?;
        tags.push(tag(&["r", euc])?);
    }

    if status != GitStatus::AppliedOrResolved
        && (!meta.applied_patches.is_empty()
            || meta.merge_commit.is_some()
            || !meta.applied_as_commits.is_empty())
    {
        return Err(SdkError::InvalidInput(
            "applied_patches/merge_commit/applied_as_commits only apply to the merged/resolved status".into(),
        ));
    }
    for patch_ref in &meta.applied_patches {
        let patch_id = check_hex_exact(&patch_ref.id, 64, "applied_patch")?;
        match (&patch_ref.relay, &patch_ref.pubkey) {
            (None, None) => tags.push(tag(&["q", &patch_id])?),
            (Some(relay), None) => tags.push(tag(&["q", &patch_id, relay])?),
            (Some(relay), Some(pubkey)) => {
                let pubkey = check_pubkey_hex(pubkey, "applied_patch pubkey")?;
                tags.push(tag(&["q", &patch_id, relay, &pubkey])?)
            }
            (None, Some(_)) => {
                return Err(SdkError::InvalidInput(
                    "applied_patch pubkey hint requires a relay-url hint".into(),
                ))
            }
        }
    }
    if let Some(ref merge_commit) = meta.merge_commit {
        check_commit_hex(merge_commit, "merge_commit")?;
        tags.push(tag(&["merge-commit", merge_commit])?);
        tags.push(tag(&["r", merge_commit])?);
    }
    if !meta.applied_as_commits.is_empty() {
        let mut commits_tag = vec!["applied-as-commits"];
        for commit in &meta.applied_as_commits {
            check_commit_hex(commit, "applied_as_commit")?;
        }
        commits_tag.extend(meta.applied_as_commits.iter().map(String::as_str));
        tags.push(tag(&commits_tag)?);
        for commit in &meta.applied_as_commits {
            tags.push(tag(&["r", commit])?);
        }
    }

    Ok(EventBuilder::new(Kind::Custom(status.kind()), content).tags(tags))
}

/// Build a workflow definition event (kind 30620).
///
/// - `channel_id`: the channel this workflow belongs to (h-tag)
/// - `workflow_id`: unique workflow UUID (d-tag)
/// - `yaml`: workflow YAML definition as content
pub fn build_workflow_def(
    channel_id: Uuid,
    workflow_id: Uuid,
    yaml: &str,
) -> Result<EventBuilder, SdkError> {
    check_content(yaml, 64 * 1024)?;
    let tags = vec![
        tag(&["d", &workflow_id.to_string()])?,
        tag(&["h", &channel_id.to_string()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF as u16), yaml).tags(tags))
}

/// Build a workflow update event (kind 30620) for an existing workflow.
///
/// Updates an existing workflow definition in-place via the parameterized
/// replaceable event mechanism — same d-tag overwrites the previous version.
/// The h-tag (channel scope) is required by the relay for authorization.
pub fn build_workflow_update(
    channel_id: Uuid,
    workflow_id: Uuid,
    yaml: &str,
) -> Result<EventBuilder, SdkError> {
    check_content(yaml, 64 * 1024)?;
    let tags = vec![
        tag(&["d", &workflow_id.to_string()])?,
        tag(&["h", &channel_id.to_string()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF as u16), yaml).tags(tags))
}

/// Build a NIP-09 deletion event targeting a workflow definition (kind 5).
///
/// The `a`-tag addresses the parameterized replaceable event
/// `<KIND_WORKFLOW_DEF>:<pubkey>:<workflow_id>`.
pub fn build_workflow_delete(
    author_pubkey: &str,
    workflow_id: Uuid,
) -> Result<EventBuilder, SdkError> {
    let pk = check_pubkey_hex(author_pubkey, "author_pubkey")?;
    let tags = vec![tag(&[
        "a",
        &format!("{}:{pk}:{workflow_id}", KIND_WORKFLOW_DEF),
    ])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_DELETION as u16), "").tags(tags))
}

/// Build a workflow trigger event (kind 46020).
pub fn build_workflow_trigger(workflow_id: Uuid) -> Result<EventBuilder, SdkError> {
    let tags = vec![tag(&["d", &workflow_id.to_string()])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER as u16), "").tags(tags))
}

/// Build a workflow approval event — kind 46030 (grant) or 46031 (deny).
///
/// - `token_hash`: hex-encoded SHA-256 of the approval token UUID (d-tag).
///   Must be exactly 64 hex characters.
/// - `approved`: `true` emits kind 46030 (grant), `false` emits kind 46031 (deny)
/// - `note`: optional human-readable note as event content
pub fn build_workflow_approval(
    token_hash: &str,
    approved: bool,
    note: &str,
) -> Result<EventBuilder, SdkError> {
    if token_hash.len() != 64 || !token_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(
            "token_hash must be a 64-character hex SHA-256 digest".into(),
        ));
    }
    let kind = if approved {
        KIND_APPROVAL_GRANT
    } else {
        KIND_APPROVAL_DENY
    };
    let tags = vec![tag(&["d", token_hash])?];
    Ok(EventBuilder::new(Kind::Custom(kind as u16), note).tags(tags))
}

/// Build a DM open event (kind 41010).
///
/// `pubkeys` must be 1–8 hex-encoded pubkeys to include in the DM conversation.
pub fn build_dm_open(pubkeys: &[&str]) -> Result<EventBuilder, SdkError> {
    if pubkeys.is_empty() || pubkeys.len() > 8 {
        return Err(SdkError::InvalidInput(
            "dm open requires 1-8 pubkeys".into(),
        ));
    }
    let mut tags = Vec::with_capacity(pubkeys.len());
    for pk in pubkeys {
        let validated = check_pubkey_hex(pk, "pubkey")?;
        tags.push(tag(&["p", &validated])?);
    }
    Ok(EventBuilder::new(Kind::Custom(KIND_DM_OPEN as u16), "").tags(tags))
}

/// Build a DM add-member event (kind 41011).
pub fn build_dm_add_member(channel_id: Uuid, pubkey: &str) -> Result<EventBuilder, SdkError> {
    let pk = check_pubkey_hex(pubkey, "pubkey")?;
    let tags = vec![tag(&["h", &channel_id.to_string()])?, tag(&["p", &pk])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_DM_ADD_MEMBER as u16), "").tags(tags))
}

/// Build a presence update event (kind 20001).
///
/// `status` must be one of: `"online"`, `"away"`, `"offline"`.
/// The status is placed in `event.content` (relay reads it there) and also
/// in a `["status", ...]` tag for structured access.
pub fn build_presence_update(status: &str) -> Result<EventBuilder, SdkError> {
    match status {
        "online" | "away" | "offline" => {}
        _ => {
            return Err(SdkError::InvalidInput(format!(
                "status must be online, away, or offline (got: {status})"
            )))
        }
    }
    let tags = vec![tag(&["status", status])?];
    Ok(EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), status).tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventId, Keys};

    fn keys() -> Keys {
        Keys::generate()
    }

    fn sign(b: EventBuilder) -> nostr::Event {
        b.sign_with_keys(&keys()).expect("sign")
    }

    fn event_id() -> EventId {
        let k = keys();
        EventBuilder::new(Kind::Custom(1), "x")
            .tags([])
            .sign_with_keys(&k)
            .expect("sign")
            .id
    }

    fn uuid() -> Uuid {
        Uuid::new_v4()
    }

    fn tag_values(event: &nostr::Event, key: &str) -> Vec<String> {
        event
            .tags
            .iter()
            .filter_map(|t| {
                let s = t.as_slice();
                if s.first().map(|v| v.as_str()) == Some(key) {
                    s.get(1).map(|v| v.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    fn has_tag(event: &nostr::Event, key: &str, val: &str) -> bool {
        event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|v| v.as_str()) == Some(key) && s.get(1).map(|v| v.as_str()) == Some(val)
        })
    }

    #[test]
    fn message_happy_path() {
        let cid = uuid();
        let ev = sign(build_message(cid, "hello", None, &[], false, &[]).unwrap());
        assert_eq!(ev.kind.as_u16(), 9);
        assert_eq!(ev.content, "hello");
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    #[test]
    fn agent_observer_frame_happy_path() {
        let sender = keys();
        let recipient = keys();
        let agent = keys();
        let encrypted = buzz_core::observer::encrypt_observer_payload(
            &sender,
            &recipient.public_key(),
            &serde_json::json!({"type": "acp_read"}),
        )
        .unwrap();
        let ev = sign(
            build_agent_observer_frame(
                &recipient.public_key().to_hex(),
                &agent.public_key().to_hex(),
                OBSERVER_FRAME_TELEMETRY,
                &encrypted,
            )
            .unwrap(),
        );

        assert_eq!(ev.kind.as_u16(), KIND_AGENT_OBSERVER_FRAME as u16);
        assert_eq!(ev.content, encrypted);
        assert!(has_tag(&ev, "p", &recipient.public_key().to_hex()));
        assert!(has_tag(
            &ev,
            OBSERVER_AGENT_TAG,
            &agent.public_key().to_hex()
        ));
        assert!(has_tag(&ev, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY));
    }

    #[test]
    fn agent_observer_frame_rejects_plaintext_content() {
        let err = build_agent_observer_frame(
            &"a".repeat(64),
            &"b".repeat(64),
            OBSERVER_FRAME_TELEMETRY,
            "not encrypted",
        )
        .unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn message_direct_reply() {
        let cid = uuid();
        let eid = event_id();
        let tr = ThreadRef {
            root_event_id: eid,
            parent_event_id: eid,
        };
        let ev = sign(build_message(cid, "reply", Some(&tr), &[], false, &[]).unwrap());
        // Direct reply: only one e-tag with "reply" marker
        let e_tags: Vec<_> = ev
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|v| v.as_str()) == Some("e"))
            .collect();
        assert_eq!(e_tags.len(), 1);
        assert_eq!(
            e_tags[0].as_slice().get(3).map(|v| v.as_str()),
            Some("reply")
        );
    }

    #[test]
    fn message_nested_reply() {
        let cid = uuid();
        let root = event_id();
        let parent = event_id();
        let tr = ThreadRef {
            root_event_id: root,
            parent_event_id: parent,
        };
        let ev = sign(build_message(cid, "nested", Some(&tr), &[], false, &[]).unwrap());
        let e_tags: Vec<_> = ev
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|v| v.as_str()) == Some("e"))
            .collect();
        assert_eq!(e_tags.len(), 2);
        let markers: Vec<_> = e_tags
            .iter()
            .filter_map(|t| t.as_slice().get(3).map(|v| v.as_str()))
            .collect();
        assert!(markers.contains(&"root"));
        assert!(markers.contains(&"reply"));
    }

    #[test]
    fn message_broadcast_flag() {
        let cid = uuid();
        let ev = sign(build_message(cid, "hi", None, &[], true, &[]).unwrap());
        assert!(has_tag(&ev, "broadcast", "1"));
    }

    #[test]
    fn message_mentions_deduped() {
        let cid = uuid();
        let hex = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let ev = sign(build_message(cid, "hi", None, &[hex, hex], false, &[]).unwrap());
        let p_tags = tag_values(&ev, "p");
        assert_eq!(p_tags.len(), 1);
    }

    #[test]
    fn message_too_many_mentions() {
        let cid = uuid();
        let hex = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let _mentions: Vec<&str> = (0..51).map(|_| hex).collect();
        // All same hex so dedup would reduce to 1, but the check is on raw len
        // Let's use 51 distinct-ish values by varying the first char
        let hexes: Vec<String> = (0..51u8)
            .map(|i| {
                format!(
                    "{:02x}cd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd12",
                    i
                )
            })
            .collect();
        let refs: Vec<&str> = hexes.iter().map(|s| s.as_str()).collect();
        let result = build_message(cid, "hi", None, &refs, false, &[]);
        assert!(matches!(result, Err(SdkError::TooManyMentions)));
    }

    #[test]
    fn message_content_too_large() {
        let cid = uuid();
        let big = "x".repeat(64 * 1024 + 1);
        let result = build_message(cid, &big, None, &[], false, &[]);
        assert!(matches!(result, Err(SdkError::ContentTooLarge { .. })));
    }

    #[test]
    fn message_max_content_ok() {
        let cid = uuid();
        let max = "x".repeat(64 * 1024);
        assert!(build_message(cid, &max, None, &[], false, &[]).is_ok());
    }

    #[test]
    fn forum_post_happy_path() {
        let cid = uuid();
        let ev = sign(build_forum_post(cid, "post body", &[], &[]).unwrap());
        assert_eq!(ev.kind.as_u16(), 45001);
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    #[test]
    fn forum_post_content_too_large() {
        let cid = uuid();
        let big = "x".repeat(64 * 1024 + 1);
        assert!(matches!(
            build_forum_post(cid, &big, &[], &[]),
            Err(SdkError::ContentTooLarge { .. })
        ));
    }

    #[test]
    fn forum_comment_happy_path() {
        let cid = uuid();
        let eid = event_id();
        let tr = ThreadRef {
            root_event_id: eid,
            parent_event_id: eid,
        };
        let ev = sign(build_forum_comment(cid, "comment", &tr, &[], &[]).unwrap());
        assert_eq!(ev.kind.as_u16(), 45003);
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    fn good_diff_meta() -> DiffMeta {
        DiffMeta {
            repo_url: "https://github.com/example/repo".into(),
            commit_sha: "abc1234".into(),
            file_path: Some("src/main.rs".into()),
            parent_commit: None,
            branch: None,
            pr_number: None,
            language: Some("rust".into()),
            description: None,
            truncated: false,
            alt_text: None,
        }
    }

    #[test]
    fn diff_message_happy_path() {
        let cid = uuid();
        let ev = sign(build_diff_message(cid, "diff content", &good_diff_meta(), None).unwrap());
        assert_eq!(ev.kind.as_u16(), 40008);
        assert!(has_tag(&ev, "repo", "https://github.com/example/repo"));
        assert!(has_tag(&ev, "commit", "abc1234"));
        assert!(has_tag(&ev, "l", "rust"));
    }

    #[test]
    fn diff_message_bad_repo_url() {
        let cid = uuid();
        let mut meta = good_diff_meta();
        meta.repo_url = "ftp://bad.url".into();
        assert!(matches!(
            build_diff_message(cid, "x", &meta, None),
            Err(SdkError::InvalidDiffMeta(_))
        ));
    }

    #[test]
    fn diff_message_short_commit_sha() {
        let cid = uuid();
        let mut meta = good_diff_meta();
        meta.commit_sha = "abc12".into(); // only 5 chars
        assert!(matches!(
            build_diff_message(cid, "x", &meta, None),
            Err(SdkError::InvalidDiffMeta(_))
        ));
    }

    #[test]
    fn diff_message_invalid_commit_sha_chars() {
        let cid = uuid();
        let mut meta = good_diff_meta();
        meta.commit_sha = "xyz1234".into(); // 'x', 'y', 'z' not hex
        assert!(matches!(
            build_diff_message(cid, "x", &meta, None),
            Err(SdkError::InvalidDiffMeta(_))
        ));
    }

    #[test]
    fn diff_message_branch_only_source() {
        let cid = uuid();
        let mut meta = good_diff_meta();
        meta.branch = Some(("main".into(), "".into())); // target empty
        assert!(matches!(
            build_diff_message(cid, "x", &meta, None),
            Err(SdkError::InvalidDiffMeta(_))
        ));
    }

    #[test]
    fn diff_message_pr_zero() {
        let cid = uuid();
        let mut meta = good_diff_meta();
        meta.pr_number = Some(0);
        assert!(matches!(
            build_diff_message(cid, "x", &meta, None),
            Err(SdkError::InvalidDiffMeta(_))
        ));
    }

    #[test]
    fn diff_message_content_too_large() {
        let cid = uuid();
        let big = "x".repeat(60 * 1024 + 1);
        assert!(matches!(
            build_diff_message(cid, &big, &good_diff_meta(), None),
            Err(SdkError::ContentTooLarge { .. })
        ));
    }

    #[test]
    fn diff_message_all_optional_fields() {
        let cid = uuid();
        let meta = DiffMeta {
            repo_url: "https://github.com/example/repo".into(),
            commit_sha: "abc1234def".into(),
            file_path: Some("src/lib.rs".into()),
            parent_commit: Some("1234567".into()),
            branch: Some(("feature".into(), "main".into())),
            pr_number: Some(42),
            language: Some("rust".into()),
            description: Some("fix bug".into()),
            truncated: true,
            alt_text: Some("patch for bug fix".into()),
        };
        let ev = sign(build_diff_message(cid, "diff", &meta, None).unwrap());
        assert!(has_tag(&ev, "file", "src/lib.rs"));
        assert!(has_tag(&ev, "parent-commit", "1234567"));
        assert!(has_tag(&ev, "pr", "42"));
        assert!(has_tag(&ev, "truncated", "true"));
        assert!(has_tag(&ev, "alt", "patch for bug fix"));
    }

    #[test]
    fn edit_happy_path() {
        let cid = uuid();
        let eid = event_id();
        let ev = sign(build_edit(cid, eid, "new content").unwrap());
        assert_eq!(ev.kind.as_u16(), 40003);
        assert!(has_tag(&ev, "e", &eid.to_hex()));
    }

    #[test]
    fn edit_content_too_large() {
        let cid = uuid();
        let eid = event_id();
        let big = "x".repeat(64 * 1024 + 1);
        assert!(matches!(
            build_edit(cid, eid, &big),
            Err(SdkError::ContentTooLarge { .. })
        ));
    }

    #[test]
    fn delete_message_happy_path() {
        let cid = uuid();
        let eid = event_id();
        let ev = sign(build_delete_message(cid, eid).unwrap());
        assert_eq!(ev.kind.as_u16(), 9005);
        assert!(has_tag(&ev, "h", &cid.to_string()));
        assert!(has_tag(&ev, "e", &eid.to_hex()));
        assert_eq!(ev.content, "");
    }

    #[test]
    fn delete_compat_happy_path() {
        let cid = uuid();
        let eid = event_id();
        let ev = sign(build_delete_compat(cid, eid).unwrap());
        assert_eq!(ev.kind.as_u16(), 5);
        assert!(has_tag(&ev, "h", &cid.to_string()));
        assert!(has_tag(&ev, "e", &eid.to_hex()));
        assert_eq!(ev.content, "");
    }

    #[test]
    fn vote_up() {
        let cid = uuid();
        let eid = event_id();
        let ev = sign(build_vote(cid, eid, VoteDirection::Up).unwrap());
        assert_eq!(ev.kind.as_u16(), 45002);
        assert_eq!(ev.content, "+");
    }

    #[test]
    fn vote_down() {
        let cid = uuid();
        let eid = event_id();
        let ev = sign(build_vote(cid, eid, VoteDirection::Down).unwrap());
        assert_eq!(ev.content, "-");
    }

    #[test]
    fn reaction_happy_path() {
        let eid = event_id();
        let ev = sign(build_reaction(eid, "👍").unwrap());
        assert_eq!(ev.kind.as_u16(), 7);
        assert_eq!(ev.content, "👍");
    }

    #[test]
    fn reaction_emoji_too_long() {
        let eid = event_id();
        let long_emoji = "a".repeat(65);
        assert!(matches!(
            build_reaction(eid, &long_emoji),
            Err(SdkError::EmojiTooLong)
        ));
    }

    #[test]
    fn reaction_emoji_max_len_ok() {
        let eid = event_id();
        let max_emoji = "a".repeat(64);
        assert!(build_reaction(eid, &max_emoji).is_ok());
    }

    #[test]
    fn custom_emoji_reaction_happy_path() {
        let eid = event_id();
        let ev = sign(
            build_custom_emoji_reaction(eid, ":Party_Parrot:", "https://example.com/parrot.png")
                .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 7);
        assert_eq!(ev.content, ":party_parrot:");
        assert!(has_tag(&ev, "emoji", "party_parrot"));
    }

    #[test]
    fn custom_emoji_set_happy_path() {
        let ev = sign(
            build_custom_emoji_set(&[CustomEmoji {
                shortcode: "party".to_string(),
                url: "https://example.com/party.png".to_string(),
            }])
            .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 30030);
        assert!(has_tag(&ev, "d", CUSTOM_EMOJI_SET_D_TAG));
        assert!(has_tag(&ev, "emoji", "party"));
    }

    #[test]
    fn remove_reaction_happy_path() {
        let eid = event_id();
        let ev = sign(build_remove_reaction(eid).unwrap());
        assert_eq!(ev.kind.as_u16(), 5);
        assert!(has_tag(&ev, "e", &eid.to_hex()));
    }

    #[test]
    fn set_canvas_happy_path() {
        let cid = uuid();
        let ev = sign(build_set_canvas(cid, "# Canvas\nHello").unwrap());
        assert_eq!(ev.kind.as_u16(), 40100);
        assert!(has_tag(&ev, "h", &cid.to_string()));
        assert_eq!(ev.content, "# Canvas\nHello");
    }

    #[test]
    fn profile_all_fields() {
        let ev = sign(
            build_profile(
                Some("Alice"),
                Some("alice"),
                Some("https://example.com/pic.jpg"),
                Some("Hello world"),
                Some("alice@example.com"),
            )
            .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 0);
        let v: serde_json::Value = serde_json::from_str(&ev.content).unwrap();
        assert_eq!(v["display_name"], "Alice");
        assert_eq!(v["name"], "alice");
        assert_eq!(v["nip05"], "alice@example.com");
    }

    #[test]
    fn profile_some_fields() {
        let ev = sign(build_profile(Some("Bob"), None, None, None, None).unwrap());
        let v: serde_json::Value = serde_json::from_str(&ev.content).unwrap();
        assert_eq!(v["display_name"], "Bob");
        assert!(
            v.get("name").is_none()
                || !v["name"].is_null() && v.get("name") == Some(&serde_json::Value::Null)
                || !v.as_object().unwrap().contains_key("name")
        );
    }

    #[test]
    fn profile_no_fields() {
        let ev = sign(build_profile(None, None, None, None, None).unwrap());
        let v: serde_json::Value = serde_json::from_str(&ev.content).unwrap();
        assert!(v.as_object().unwrap().is_empty());
    }

    #[test]
    fn add_member_with_role() {
        let cid = uuid();
        let pubkey = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let ev = sign(build_add_member(cid, pubkey, Some(MemberRole::Admin)).unwrap());
        assert_eq!(ev.kind.as_u16(), 9000);
        assert!(has_tag(&ev, "p", pubkey));
        assert!(has_tag(&ev, "role", "admin"));
    }

    #[test]
    fn add_member_without_role() {
        let cid = uuid();
        let pubkey = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let ev = sign(build_add_member(cid, pubkey, None::<MemberRole>).unwrap());
        assert_eq!(ev.kind.as_u16(), 9000);
        assert!(tag_values(&ev, "role").is_empty());
    }

    #[test]
    fn remove_member_happy_path() {
        let cid = uuid();
        let pubkey = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
        let ev = sign(build_remove_member(cid, pubkey).unwrap());
        assert_eq!(ev.kind.as_u16(), 9001);
        assert!(has_tag(&ev, "p", pubkey));
    }

    #[test]
    fn leave_happy_path() {
        let cid = uuid();
        let ev = sign(build_leave(cid).unwrap());
        assert_eq!(ev.kind.as_u16(), 9022);
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    #[test]
    fn update_channel_name_and_about() {
        let cid = uuid();
        let ev = sign(
            build_update_channel(cid, Some("new-name"), Some("new about"), None, None).unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 9002);
        assert!(has_tag(&ev, "name", "new-name"));
        assert!(has_tag(&ev, "about", "new about"));
    }

    #[test]
    fn update_channel_visibility_and_ttl() {
        let cid = uuid();
        let ev =
            sign(build_update_channel(cid, None, None, Some("private"), Some(Some(3600))).unwrap());
        assert_eq!(ev.kind.as_u16(), 9002);
        assert!(has_tag(&ev, "visibility", "private"));
        assert!(has_tag(&ev, "ttl", "3600"));
    }

    #[test]
    fn update_channel_clears_ttl() {
        let cid = uuid();
        let ev = sign(build_update_channel(cid, None, None, None, Some(None)).unwrap());
        assert!(has_tag(&ev, "ttl", ""));
    }

    #[test]
    fn update_channel_invalid_visibility_rejected() {
        let cid = uuid();
        assert!(matches!(
            build_update_channel(cid, None, None, Some("secret"), None),
            Err(SdkError::InvalidTag(_))
        ));
    }

    #[test]
    fn update_channel_no_fields_rejected() {
        let cid = uuid();
        assert!(matches!(
            build_update_channel(cid, None, None, None, None),
            Err(SdkError::InvalidTag(_))
        ));
    }

    #[test]
    fn set_topic_happy_path() {
        let cid = uuid();
        let ev = sign(build_set_topic(cid, "Rust async patterns").unwrap());
        assert_eq!(ev.kind.as_u16(), 9002);
        assert!(has_tag(&ev, "topic", "Rust async patterns"));
    }

    #[test]
    fn set_purpose_happy_path() {
        let cid = uuid();
        let ev = sign(build_set_purpose(cid, "Team coordination").unwrap());
        assert_eq!(ev.kind.as_u16(), 9002);
        assert!(has_tag(&ev, "purpose", "Team coordination"));
    }

    #[test]
    fn create_channel_all_fields() {
        let cid = uuid();
        let ev = sign(
            build_create_channel(
                cid,
                "general",
                Some(Visibility::Open),
                Some(ChannelKind::Stream),
                Some("General chat"),
                None,
            )
            .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 9007);
        assert!(has_tag(&ev, "name", "general"));
        assert!(has_tag(&ev, "visibility", "open"));
        assert!(has_tag(&ev, "channel_type", "stream"));
        assert!(has_tag(&ev, "about", "General chat"));
    }

    #[test]
    fn create_channel_minimal() {
        let cid = uuid();
        let ev = sign(
            build_create_channel(
                cid,
                "dev",
                None::<Visibility>,
                None::<ChannelKind>,
                None,
                None,
            )
            .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 9007);
        assert!(has_tag(&ev, "name", "dev"));
    }

    #[test]
    fn create_channel_ephemeral_emits_ttl() {
        let cid = uuid();
        let ev = sign(
            build_create_channel(
                cid,
                "standup",
                Some(Visibility::Open),
                Some(ChannelKind::Stream),
                None,
                Some(3600),
            )
            .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 9007);
        assert!(has_tag(&ev, "ttl", "3600"));
    }

    #[test]
    fn join_happy_path() {
        let cid = uuid();
        let ev = sign(build_join(cid).unwrap());
        assert_eq!(ev.kind.as_u16(), 9021);
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    #[test]
    fn archive_happy_path() {
        let cid = uuid();
        let ev = sign(build_archive(cid).unwrap());
        assert_eq!(ev.kind.as_u16(), 9002);
        assert!(has_tag(&ev, "archived", "true"));
    }

    #[test]
    fn unarchive_happy_path() {
        let cid = uuid();
        let ev = sign(build_unarchive(cid).unwrap());
        assert_eq!(ev.kind.as_u16(), 9002);
        assert!(has_tag(&ev, "archived", "false"));
    }

    #[test]
    fn delete_channel_happy_path() {
        let cid = uuid();
        let ev = sign(build_delete_channel(cid).unwrap());
        assert_eq!(ev.kind.as_u16(), 9008);
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    #[test]
    fn extract_channel_id_present() {
        let cid = uuid();
        let ev = sign(build_join(cid).unwrap());
        assert_eq!(extract_channel_id(&ev), Some(cid));
    }

    #[test]
    fn extract_channel_id_absent() {
        // build_note (kind 1) is a global text note — no h tag.
        let ev = sign(build_note("hello", None).unwrap());
        assert_eq!(extract_channel_id(&ev), None);
    }

    #[test]
    fn extract_channel_id_invalid_uuid() {
        // Build an event with a malformed h-tag value
        let tags = vec![Tag::parse(["h", "not-a-uuid"]).unwrap()];
        let ev = EventBuilder::new(Kind::Custom(9), "x")
            .tags(tags)
            .sign_with_keys(&keys())
            .unwrap();
        assert_eq!(extract_channel_id(&ev), None);
    }

    #[test]
    fn build_note_happy_path() {
        let builder = build_note("hello world", None).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind, Kind::Custom(1));
        assert_eq!(event.content, "hello world");
        assert!(event.tags.is_empty());
    }

    #[test]
    fn build_note_with_reply() {
        let keys = nostr::Keys::generate();
        // Create a dummy event to get a valid EventId
        let dummy = EventBuilder::new(Kind::Custom(1), "dummy")
            .tags(vec![])
            .sign_with_keys(&keys)
            .unwrap();
        let builder = build_note("reply text", Some(dummy.id)).unwrap();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind, Kind::Custom(1));
        assert_eq!(event.content, "reply text");
        assert_eq!(event.tags.len(), 1);
        let tag = event.tags.iter().next().unwrap();
        assert_eq!(tag.as_slice()[0], "e");
        assert_eq!(tag.as_slice()[1], dummy.id.to_hex());
        assert_eq!(tag.as_slice()[3], "reply");
    }

    #[test]
    fn build_note_content_too_large() {
        let big = "x".repeat(64 * 1024 + 1);
        let err = build_note(&big, None).unwrap_err();
        assert!(matches!(err, SdkError::ContentTooLarge { .. }));
    }

    #[test]
    fn build_note_empty_content() {
        // Empty content is valid per NIP-01.
        let builder = build_note("", None).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind, Kind::Custom(1));
        assert_eq!(event.content, "");
        assert!(event.tags.is_empty());
    }

    #[test]
    fn build_contact_list_happy_path() {
        let pubkey = "a".repeat(64);
        let contacts = vec![(pubkey.as_str(), None, None)];
        let builder = build_contact_list(&contacts).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind, Kind::Custom(3));
        assert_eq!(event.content, "");
        assert_eq!(event.tags.len(), 1);
        let tag = event.tags.iter().next().unwrap();
        assert_eq!(tag.as_slice()[0], "p");
        assert_eq!(tag.as_slice()[1], pubkey);
    }

    #[test]
    fn build_contact_list_normalizes_uppercase() {
        let upper = "A".repeat(64);
        let contacts = vec![(upper.as_str(), None, None)];
        let builder = build_contact_list(&contacts).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        let tag = event.tags.iter().next().unwrap();
        assert_eq!(tag.as_slice()[1], "a".repeat(64));
    }

    #[test]
    fn build_contact_list_with_relay_and_petname() {
        let pubkey = "b".repeat(64);
        let contacts = vec![(
            pubkey.as_str(),
            Some("wss://relay.example.com"),
            Some("alice"),
        )];
        let builder = build_contact_list(&contacts).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        let tag = event.tags.iter().next().unwrap();
        assert_eq!(tag.as_slice()[0], "p");
        assert_eq!(tag.as_slice()[2], "wss://relay.example.com");
        assert_eq!(tag.as_slice()[3], "alice");
    }

    #[test]
    fn build_contact_list_empty() {
        let builder = build_contact_list(&[]).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.kind, Kind::Custom(3));
        assert!(event.tags.is_empty());
    }

    #[test]
    fn build_contact_list_rejects_short_pubkey() {
        let short = "a".repeat(63);
        let contacts = vec![(short.as_str(), None, None)];
        let err = build_contact_list(&contacts).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn build_contact_list_rejects_long_pubkey() {
        let long = "a".repeat(65);
        let contacts = vec![(long.as_str(), None, None)];
        let err = build_contact_list(&contacts).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn build_contact_list_rejects_non_hex() {
        let non_hex = "g".repeat(64);
        let contacts = vec![(non_hex.as_str(), None, None)];
        let err = build_contact_list(&contacts).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn build_contact_list_rejects_long_relay_url() {
        let pubkey = "a".repeat(64);
        let long_url = "x".repeat(2049);
        let contacts = vec![(pubkey.as_str(), Some(long_url.as_str()), None)];
        let err = build_contact_list(&contacts).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn build_contact_list_rejects_long_petname() {
        let pubkey = "a".repeat(64);
        let long_name = "x".repeat(257);
        let contacts = vec![(pubkey.as_str(), None, Some(long_name.as_str()))];
        let err = build_contact_list(&contacts).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn build_contact_list_duplicate_pubkeys() {
        let pubkey = "c".repeat(64);
        // Same pubkey twice — only one p-tag should be emitted.
        let contacts = vec![
            (pubkey.as_str(), None, None),
            (
                pubkey.as_str(),
                Some("wss://relay.example.com"),
                Some("bob"),
            ),
        ];
        let builder = build_contact_list(&contacts).unwrap();
        let keys = nostr::Keys::generate();
        let event = builder.sign_with_keys(&keys).unwrap();
        assert_eq!(event.tags.len(), 1);
        let tag = event.tags.iter().next().unwrap();
        assert_eq!(tag.as_slice()[0], "p");
        assert_eq!(tag.as_slice()[1], pubkey);
    }

    #[test]
    fn build_contact_list_too_many() {
        let pubkey = "d".repeat(64);
        // MAX_CONTACTS + 1 entries (all same pubkey — uniqueness doesn't matter,
        // the cap is checked before deduplication).
        let entry = (pubkey.as_str(), None, None);
        let contacts = vec![entry; MAX_CONTACTS + 1];
        let err = build_contact_list(&contacts).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn repo_announcement_happy_path_all_fields() {
        let ev = sign(
            build_repo_announcement(
                "my-repo",
                Some("My Repo"),
                Some("A test repository"),
                &["https://github.com/example/my-repo.git"],
                Some("https://github.com/example/my-repo"),
                &["wss://relay.example.com"],
            )
            .unwrap(),
        );
        assert_eq!(ev.kind.as_u16(), 30617);
        assert_eq!(ev.content, "");
        assert!(has_tag(&ev, "d", "my-repo"));
        assert!(has_tag(&ev, "name", "My Repo"));
        assert!(has_tag(&ev, "description", "A test repository"));
        assert!(has_tag(
            &ev,
            "clone",
            "https://github.com/example/my-repo.git"
        ));
        assert!(has_tag(&ev, "web", "https://github.com/example/my-repo"));
        // relays is a multi-value tag — check the tag key exists
        assert!(ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|v| v.as_str()) == Some("relays")
                && s.get(1).map(|v| v.as_str()) == Some("wss://relay.example.com")
        }));
    }

    #[test]
    fn repo_announcement_happy_path_minimal() {
        let ev = sign(build_repo_announcement("bare-repo", None, None, &[], None, &[]).unwrap());
        assert_eq!(ev.kind.as_u16(), 30617);
        assert_eq!(ev.content, "");
        assert!(has_tag(&ev, "d", "bare-repo"));
        // No optional tags present
        assert!(!ev
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("name")));
        assert!(!ev
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("clone")));
    }

    #[test]
    fn repo_announcement_rejects_empty_repo_id() {
        let err = build_repo_announcement("", None, None, &[], None, &[]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn repo_announcement_rejects_leading_dot() {
        let err = build_repo_announcement(".hidden", None, None, &[], None, &[]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn repo_announcement_rejects_double_dot() {
        let err = build_repo_announcement("some..repo", None, None, &[], None, &[]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn repo_announcement_rejects_repo_id_over_64_chars() {
        let long_id = "a".repeat(65);
        let err = build_repo_announcement(&long_id, None, None, &[], None, &[]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn repo_announcement_rejects_invalid_chars_in_repo_id() {
        let err = build_repo_announcement("bad repo!", None, None, &[], None, &[]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn repo_announcement_multiple_clone_urls_multi_value_tag() {
        let ev = sign(
            build_repo_announcement(
                "multi-clone",
                None,
                None,
                &[
                    "https://relay.example.com/git/abc/multi-clone",
                    "ssh://git@github.com/org/multi-clone.git",
                ],
                None,
                &[],
            )
            .unwrap(),
        );
        // clone is a multi-value tag per NIP-34: ["clone", url1, url2, ...]
        let clone_tag = ev
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|v| v.as_str()) == Some("clone"))
            .expect("clone tag missing");
        let vals: Vec<&str> = clone_tag
            .as_slice()
            .iter()
            .skip(1)
            .map(|v| v.as_str())
            .collect();
        assert_eq!(vals.len(), 2);
        assert_eq!(vals[0], "https://relay.example.com/git/abc/multi-clone");
        assert_eq!(vals[1], "ssh://git@github.com/org/multi-clone.git");
    }

    #[test]
    fn git_patch_happy_path_minimal() {
        let owner = "a".repeat(64);
        let repo = GitRepoCoord {
            owner: owner.clone(),
            id: "my-repo".to_string(),
        };
        let ev =
            sign(build_git_patch(&repo, "diff --git a/x b/x", &GitPatchMeta::default()).unwrap());
        assert_eq!(ev.kind.as_u16(), 1617);
        assert_eq!(ev.content, "diff --git a/x b/x");
        assert!(has_tag(&ev, "a", &format!("30617:{owner}:my-repo")));
        assert!(has_tag(&ev, "p", &owner));
    }

    #[test]
    fn git_patch_root_and_metadata_tags() {
        let owner = "a".repeat(64);
        let commit = "c".repeat(40);
        let parent = "d".repeat(40);
        let repo = GitRepoCoord {
            owner,
            id: "repo".to_string(),
        };
        let meta = GitPatchMeta {
            root: true,
            commit: Some(commit.clone()),
            parent_commit: Some(parent.clone()),
            ..Default::default()
        };
        let ev = sign(build_git_patch(&repo, "patch body", &meta).unwrap());
        assert!(has_tag(&ev, "t", "root"));
        assert!(has_tag(&ev, "commit", &commit));
        assert!(has_tag(&ev, "parent-commit", &parent));
        assert!(has_tag(&ev, "r", &commit));
    }

    #[test]
    fn git_patch_rejects_root_and_root_revision_together() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let meta = GitPatchMeta {
            root: true,
            root_revision: true,
            ..Default::default()
        };
        let err = build_git_patch(&repo, "x", &meta).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_patch_rejects_oversized_content() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let big = "x".repeat(60 * 1024 + 1);
        let err = build_git_patch(&repo, &big, &GitPatchMeta::default()).unwrap_err();
        assert!(matches!(err, SdkError::ContentTooLarge { .. }));
    }

    #[test]
    fn git_patch_rejects_bad_repo_owner() {
        let repo = GitRepoCoord {
            owner: "not-hex".to_string(),
            id: "repo".to_string(),
        };
        let err = build_git_patch(&repo, "x", &GitPatchMeta::default()).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_issue_happy_path() {
        let owner = "a".repeat(64);
        let repo = GitRepoCoord {
            owner: owner.clone(),
            id: "repo".to_string(),
        };
        let meta = GitIssueMeta {
            labels: vec!["bug".to_string(), "p1".to_string()],
            recipients: vec![],
        };
        let ev =
            sign(build_git_issue(&repo, "Crashes on startup", "steps to repro", &meta).unwrap());
        assert_eq!(ev.kind.as_u16(), 1621);
        assert_eq!(ev.content, "steps to repro");
        assert!(has_tag(&ev, "a", &format!("30617:{owner}:repo")));
        assert!(has_tag(&ev, "subject", "Crashes on startup"));
        assert!(has_tag(&ev, "t", "bug"));
        assert!(has_tag(&ev, "t", "p1"));
    }

    #[test]
    fn git_issue_rejects_empty_subject() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let err = build_git_issue(&repo, "", "body", &GitIssueMeta::default()).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_status_open_happy_path() {
        let root = event_id().to_hex();
        let meta = GitStatusMeta {
            root_event: root.clone(),
            ..Default::default()
        };
        let ev = sign(build_git_status(GitStatus::Open, "", &meta).unwrap());
        assert_eq!(ev.kind.as_u16(), 1630);
        assert!(has_tag(&ev, "e", &root));
    }

    #[test]
    fn git_status_merged_with_applied_patches() {
        let root = event_id().to_hex();
        let patch_id = event_id().to_hex();
        let merge_commit = "f".repeat(40);
        let meta = GitStatusMeta {
            root_event: root,
            applied_patches: vec![GitAppliedPatchRef {
                id: patch_id.clone(),
                relay: None,
                pubkey: None,
            }],
            merge_commit: Some(merge_commit.clone()),
            ..Default::default()
        };
        let ev =
            sign(build_git_status(GitStatus::AppliedOrResolved, "merged, thanks!", &meta).unwrap());
        assert_eq!(ev.kind.as_u16(), 1631);
        assert!(has_tag(&ev, "q", &patch_id));
        assert!(has_tag(&ev, "merge-commit", &merge_commit));
        assert!(has_tag(&ev, "r", &merge_commit));
    }

    #[test]
    fn git_status_rejects_merge_fields_on_non_merged_status() {
        let root = event_id().to_hex();
        let meta = GitStatusMeta {
            root_event: root,
            merge_commit: Some("f".repeat(40)),
            ..Default::default()
        };
        let err = build_git_status(GitStatus::Closed, "", &meta).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_status_rejects_bad_root_event() {
        let meta = GitStatusMeta {
            root_event: "not-an-event-id".to_string(),
            ..Default::default()
        };
        let err = build_git_status(GitStatus::Open, "", &meta).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_patch_rejects_empty_content() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let err = build_git_patch(&repo, "", &GitPatchMeta::default()).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_patch_rejects_whitespace_only_content() {
        // Regression: a failed `git format-patch | buzz patches send
        // --patch-file -` must not silently publish a whitespace-only
        // (i.e. unappliable) patch.
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let err = build_git_patch(&repo, "   \n\t\n", &GitPatchMeta::default()).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_repo_coord_rejects_invalid_repo_id_chars() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "../etc/passwd".to_string(),
        };
        let err = build_git_patch(&repo, "diff", &GitPatchMeta::default()).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_patch_rejects_short_commit_hex() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let meta = GitPatchMeta {
            commit: Some("a".to_string()),
            ..Default::default()
        };
        let err = build_git_patch(&repo, "diff", &meta).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn git_patch_accepts_full_sha1_and_sha256_commit_hex() {
        let repo = GitRepoCoord {
            owner: "a".repeat(64),
            id: "repo".to_string(),
        };
        let meta_sha1 = GitPatchMeta {
            commit: Some("c".repeat(40)),
            ..Default::default()
        };
        assert!(build_git_patch(&repo, "diff", &meta_sha1).is_ok());
        let meta_sha256 = GitPatchMeta {
            commit: Some("c".repeat(64)),
            ..Default::default()
        };
        assert!(build_git_patch(&repo, "diff", &meta_sha256).is_ok());
    }

    #[test]
    fn git_status_defaults_p_tag_to_repo_owner_when_repo_given() {
        // SDK-level: GitStatusMeta.recipients is the caller's responsibility
        // (the CLI defaults it), but verify the repo owner ends up p-tagged
        // when the CLI-style recipients list includes it.
        let owner = "a".repeat(64);
        let root = event_id().to_hex();
        let meta = GitStatusMeta {
            root_event: root,
            repo: Some(GitRepoCoord {
                owner: owner.clone(),
                id: "repo".to_string(),
            }),
            recipients: vec![owner.clone()],
            ..Default::default()
        };
        let ev = sign(build_git_status(GitStatus::Open, "", &meta).unwrap());
        assert!(has_tag(&ev, "p", &owner));
    }

    #[test]
    fn git_applied_patch_ref_parse_id_only() {
        let id = "a".repeat(64);
        let parsed = GitAppliedPatchRef::parse(&id).unwrap();
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.relay, None);
        assert_eq!(parsed.pubkey, None);
    }

    #[test]
    fn git_applied_patch_ref_parse_id_and_relay() {
        let id = "a".repeat(64);
        let spec = format!("{id}:wss://relay.example.com");
        let parsed = GitAppliedPatchRef::parse(&spec).unwrap();
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.relay, Some("wss://relay.example.com".to_string()));
        assert_eq!(parsed.pubkey, None);
    }

    #[test]
    fn git_applied_patch_ref_parse_id_relay_and_pubkey() {
        let id = "a".repeat(64);
        let pubkey = "b".repeat(64);
        let spec = format!("{id}:wss://relay.example.com:{pubkey}");
        let parsed = GitAppliedPatchRef::parse(&spec).unwrap();
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.relay, Some("wss://relay.example.com".to_string()));
        assert_eq!(parsed.pubkey, Some(pubkey));
    }

    #[test]
    fn git_status_q_tag_includes_relay_and_pubkey_hints() {
        let root = event_id().to_hex();
        let patch_id = event_id().to_hex();
        let pubkey = "b".repeat(64);
        let meta = GitStatusMeta {
            root_event: root,
            applied_patches: vec![GitAppliedPatchRef {
                id: patch_id.clone(),
                relay: Some("wss://relay.example.com".to_string()),
                pubkey: Some(pubkey.clone()),
            }],
            ..Default::default()
        };
        let ev = sign(build_git_status(GitStatus::AppliedOrResolved, "", &meta).unwrap());
        let q_tag = ev
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|v| v.as_str()) == Some("q"))
            .expect("q tag present");
        let parts = q_tag.as_slice();
        assert_eq!(parts.get(1).map(|v| v.as_str()), Some(patch_id.as_str()));
        assert_eq!(
            parts.get(2).map(|v| v.as_str()),
            Some("wss://relay.example.com")
        );
        assert_eq!(parts.get(3).map(|v| v.as_str()), Some(pubkey.as_str()));
    }

    #[test]
    fn workflow_def_happy_path() {
        let cid = uuid();
        let wid = uuid();
        let ev = sign(build_workflow_def(cid, wid, "name: test\ntrigger:\n  on: webhook").unwrap());
        assert_eq!(ev.kind.as_u16(), 30620);
        assert!(has_tag(&ev, "d", &wid.to_string()));
        assert!(has_tag(&ev, "h", &cid.to_string()));
        assert!(ev.content.contains("name: test"));
    }

    #[test]
    fn workflow_def_rejects_oversized_yaml() {
        let big = "x".repeat(65 * 1024);
        let err = build_workflow_def(uuid(), uuid(), &big).unwrap_err();
        assert!(matches!(err, SdkError::ContentTooLarge { .. }));
    }

    #[test]
    fn workflow_update_includes_h_tag() {
        let cid = uuid();
        let wid = uuid();
        let ev = sign(build_workflow_update(cid, wid, "name: updated").unwrap());
        assert_eq!(ev.kind.as_u16(), 30620);
        assert!(has_tag(&ev, "d", &wid.to_string()));
        assert!(has_tag(&ev, "h", &cid.to_string()));
    }

    #[test]
    fn workflow_update_rejects_oversized_yaml() {
        let big = "x".repeat(65 * 1024);
        let err = build_workflow_update(uuid(), uuid(), &big).unwrap_err();
        assert!(matches!(err, SdkError::ContentTooLarge { .. }));
    }

    #[test]
    fn workflow_delete_happy_path() {
        let pk = "a".repeat(64);
        let wid = uuid();
        let ev = sign(build_workflow_delete(&pk, wid).unwrap());
        assert_eq!(ev.kind.as_u16(), 5);
        let a_vals = tag_values(&ev, "a");
        assert_eq!(a_vals.len(), 1);
        assert!(a_vals[0].starts_with("30620:"));
        assert!(a_vals[0].contains(&wid.to_string()));
    }

    #[test]
    fn workflow_delete_rejects_bad_pubkey() {
        let err = build_workflow_delete("bad", uuid()).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn workflow_trigger_happy_path() {
        let wid = uuid();
        let ev = sign(build_workflow_trigger(wid).unwrap());
        assert_eq!(ev.kind.as_u16(), 46020);
        assert!(has_tag(&ev, "d", &wid.to_string()));
    }

    #[test]
    fn workflow_approval_grant() {
        let hash = "a".repeat(64);
        let ev = sign(build_workflow_approval(&hash, true, "lgtm").unwrap());
        assert_eq!(ev.kind.as_u16(), 46030);
        assert!(has_tag(&ev, "d", &hash));
        assert_eq!(ev.content, "lgtm");
    }

    #[test]
    fn workflow_approval_deny() {
        let hash = "b".repeat(64);
        let ev = sign(build_workflow_approval(&hash, false, "").unwrap());
        assert_eq!(ev.kind.as_u16(), 46031);
    }

    #[test]
    fn workflow_approval_rejects_bad_token_hash() {
        let err = build_workflow_approval("not-hex", true, "").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn workflow_approval_rejects_short_hash() {
        let short = "a".repeat(32);
        let err = build_workflow_approval(&short, true, "").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn dm_open_happy_path() {
        let pk = "a".repeat(64);
        let ev = sign(build_dm_open(&[&pk]).unwrap());
        assert_eq!(ev.kind.as_u16(), 41010);
        assert!(has_tag(&ev, "p", &pk));
    }

    #[test]
    fn dm_open_rejects_empty_pubkeys() {
        let err = build_dm_open(&[]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn dm_open_rejects_over_8_pubkeys() {
        let pk = "a".repeat(64);
        let pks: Vec<&str> = vec![pk.as_str(); 9];
        let err = build_dm_open(&pks).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn dm_open_rejects_bad_pubkey() {
        let err = build_dm_open(&["bad-hex"]).unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn dm_add_member_happy_path() {
        let cid = uuid();
        let pk = "b".repeat(64);
        let ev = sign(build_dm_add_member(cid, &pk).unwrap());
        assert_eq!(ev.kind.as_u16(), 41011);
        assert!(has_tag(&ev, "h", &cid.to_string()));
        assert!(has_tag(&ev, "p", &pk));
    }

    #[test]
    fn dm_add_member_rejects_bad_pubkey() {
        let err = build_dm_add_member(uuid(), "short").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }

    #[test]
    fn presence_update_content_is_status() {
        let ev = sign(build_presence_update("online").unwrap());
        assert_eq!(ev.kind.as_u16(), 20001);
        assert_eq!(ev.content, "online");
        assert!(has_tag(&ev, "status", "online"));
    }

    #[test]
    fn presence_update_away() {
        let ev = sign(build_presence_update("away").unwrap());
        assert_eq!(ev.content, "away");
    }

    #[test]
    fn presence_update_offline() {
        let ev = sign(build_presence_update("offline").unwrap());
        assert_eq!(ev.content, "offline");
    }

    #[test]
    fn presence_update_rejects_invalid_status() {
        let err = build_presence_update("dnd").unwrap_err();
        assert!(matches!(err, SdkError::InvalidInput(_)));
    }
}
