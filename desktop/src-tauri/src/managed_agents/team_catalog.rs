//! Project a `TeamRecord` plus its member definitions onto a kind:30178 team
//! catalog event.
//!
//! Kind 30176 is the team's own wire body (membership by local persona id);
//! kind 30178 is the shareable catalog projection that embeds every member's
//! safe definition so a recipient can rebuild the team without reading the
//! owner's personas. They are separate kinds so an ordinary team edit
//! republishes 30176 and cannot disturb catalog share state, which lives only
//! on the 30178 head's `shared` tag.
//!
//! A pure builder plus validator — no I/O, no wiring (publication lives in
//! `commands::teams`). Field discipline is an explicit opt-IN projection over
//! the persona-catalog safe set: env vars, allowlist pubkeys, local ids, and
//! paths are structurally absent below, so no future `AgentDefinition` field
//! can leak by being forgotten.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
use image::ImageDecoder;
use nostr::{EventBuilder, Kind, Tag};
use std::io::Cursor;

#[cfg(test)]
pub(crate) use buzz_core_pkg::team_catalog::{
    is_safe_catalog_avatar_url, validate_member, MAX_IDENTIFIER_BYTES, MAX_INSTRUCTIONS_BYTES,
    MAX_MEMBERS, MAX_MEMBER_KEY_BYTES, MAX_NAME_BYTES, MAX_NAME_POOL_ENTRIES,
    MAX_SYSTEM_PROMPT_BYTES, MAX_TEXT_BYTES, MAX_TOTAL_BYTES, PROJECTION_HASH_HEX_LEN,
};
pub use buzz_core_pkg::team_catalog::{
    member_projection_hash, team_catalog_content_json, validate_team_catalog_content,
    TeamCatalogContent, TeamCatalogMember, MAX_AVATAR_URL_BYTES, TEAM_CATALOG_SCHEMA_VERSION,
};

use super::{AgentDefinition, RespondTo, TeamRecord};

/// Maximum pixel dimension (width or height) accepted when decoding an inline
/// avatar for downscaling. Prevents decompression-bomb attacks before any
/// pixel allocation occurs. Mirrors `snapshot_avatar.rs`.
const MAX_DOWNSCALE_DECODE_DIMENSION: u32 = 2048;
/// Maximum heap allocation the image decoder may perform when materializing
/// a raster for downscaling. Mirrors `snapshot_avatar.rs`.
const MAX_DOWNSCALE_DECODE_ALLOC: u64 = 32 * 1024 * 1024;

/// Resolve the members of `team` from `personas`, in the team's own
/// membership order.
///
/// Order is load-bearing: it is part of the canonical projection bytes.
/// An unresolvable id is an error, not a skip — silently publishing a team
/// with a member missing would present a different team to the community than
/// the owner sees, and the freshness reconcile treats this failure as grounds
/// for retraction.
pub fn resolve_team_members(
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<Vec<AgentDefinition>, String> {
    team.persona_ids
        .iter()
        .map(|persona_id| {
            personas
                .iter()
                .find(|record| &record.id == persona_id)
                .cloned()
                .ok_or_else(|| format!("team member {persona_id} not found"))
        })
        .collect()
}

/// There is no `respond_to_allowlist` field on [`TeamCatalogMember`], and that
/// absence is the anti-leak guarantee: an allowlist is a list of real pubkeys
/// the owner trusts, and publishing it would disclose the owner's social
/// graph. Rather than projecting an emptied list — which a recipient reading
/// `allowlist` mode with no entries would treat as "everyone" — the mode
/// itself is downgraded to `owner-only`, the most restrictive setting. A
/// recipient that wants an allowlist must author one.
fn sanitized_respond_to(record: &AgentDefinition) -> Option<String> {
    match record.respond_to.as_deref() {
        Some(mode) if mode == RespondTo::Allowlist.as_str() => {
            Some(RespondTo::OwnerOnly.as_str().to_string())
        }
        other => other.map(str::to_string),
    }
}

/// The opaque published identity of one member.
///
/// Derived from the source record's `id`, which is unique within the
/// publisher's persona store (a UUID, `builtin:<slug>`, or a pack slug). The
/// id is hashed with a domain-separation prefix rather than published raw, so
/// the key leaks no local identifier and cannot be mistaken for a resolvable
/// kind:30175 d-tag.
///
/// Deliberately NOT `persona_events::persona_d_tag`: that normalizer is
/// documented non-injective (case-folds, maps every char outside `[a-z0-9_-]`
/// to `-`, truncates to 64 bytes), so two distinct members could collide on
/// one key. Provenance is keyed on `(owner_pubkey, team_d_tag, member_key)`,
/// so a collision there is not cosmetic: on adoption both members would
/// collapse onto a single local persona. SHA-256 over the exact id keeps
/// distinct sources distinct.
pub fn member_key_for(record: &AgentDefinition) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"buzz:team-catalog:member-key:v1\0");
    hasher.update(record.id.as_bytes());
    hex::encode(hasher.finalize())
}

/// Downscale an oversized inline raster data URL to fit within `MAX_AVATAR_URL_BYTES`.
///
/// Tries successively smaller maximum dimensions (256 → 192 → 128 → 96 → 64)
/// and returns the first PNG data URL that fits. Returns `None` if the input is
/// not a decodable raster data URL or no dimension produces a small enough result.
fn downscale_raster_avatar(url: &str) -> Option<String> {
    if !url.starts_with("data:image/") {
        return None;
    }
    let bytes = crate::managed_agents::agent_snapshot::decode_avatar_data_url(url)?;
    // Use a bounded decoder to reject decompression bombs before pixel
    // allocation. `image::load_from_memory` imposes no dimension ceiling and
    // allows the decoder's default 512 MiB allocation budget.
    let reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DOWNSCALE_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_DOWNSCALE_DECODE_DIMENSION);
    limits.max_alloc = Some(MAX_DOWNSCALE_DECODE_ALLOC);
    decoder.set_limits(limits).ok()?;
    let img = image::DynamicImage::from_decoder(decoder).ok()?;
    for &max_dim in &[256u32, 192, 128, 96, 64] {
        let resized = if img.width().max(img.height()) > max_dim {
            img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3)
        } else {
            img.clone()
        };
        let mut png = Vec::new();
        if resized
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
        {
            let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&png));
            if data_url.len() <= MAX_AVATAR_URL_BYTES {
                return Some(data_url);
            }
        }
    }
    None
}

/// Project one member definition, without the built-in reuse hint.
fn member_projection(record: &AgentDefinition) -> TeamCatalogMember {
    // Built-in members: oversized avatars are silently stripped. Downscaling
    // would change the projection bytes and break the reuse-hint hash, which
    // must stay recomputable from the recipient's pristine local copy.
    //
    // Non-built-in members: oversized inline raster data URLs are downscaled
    // so the share succeeds. If decoding fails or no dimension fits, the
    // avatar falls through unchanged and `validate_member` surfaces the
    // deterministic "avatar too large" error.
    let is_builtin = builtin_catalog_slug(record).is_some();
    let avatar_url = record
        .avatar_url
        .as_deref()
        .filter(|url| !is_builtin || url.len() <= MAX_AVATAR_URL_BYTES)
        .map(|url| {
            if !is_builtin && url.len() > MAX_AVATAR_URL_BYTES {
                downscale_raster_avatar(url).unwrap_or_else(|| url.to_string())
            } else {
                url.to_string()
            }
        });

    TeamCatalogMember {
        member_key: member_key_for(record),
        display_name: record.display_name.clone(),
        // Mirrors `persona_event_content`: always `Some`, including for an
        // empty prompt, so the encoding does not depend on emptiness.
        system_prompt: Some(record.system_prompt.clone()),
        avatar_url,
        runtime: record.runtime.clone(),
        acp_command: record
            .acp_command
            .clone()
            .filter(|command| super::is_portable_acp_command(command)),
        model: record.model.clone(),
        provider: record.provider.clone(),
        name_pool: record.name_pool.clone(),
        respond_to: sanitized_respond_to(record),
        parallelism: record.parallelism.map(|value| value.clamp(1, 32)),
        session_policy: record.session_policy,
        builtin_slug: None,
        projection_hash: None,
    }
}

/// The canonical catalog slug of a local built-in, or `None` for any record
/// that is not one.
///
/// Real built-ins have ids like `builtin:fizz` and `source_team_persona_slug:
/// None`, so keying the reuse hint on `source_team_persona_slug` matched no
/// real built-in on either side. The `builtin:` id prefix is the actual
/// canonical identity, identical across installs — exactly what a
/// cross-install reuse hint needs.
pub fn builtin_catalog_slug(record: &AgentDefinition) -> Option<&str> {
    if !record.is_builtin {
        return None;
    }
    record
        .id
        .strip_prefix("builtin:")
        .filter(|slug| !slug.is_empty())
}

/// Project a member and attach the built-in reuse hint when applicable.
///
/// The hash is computed over the member projection with both hint fields
/// still absent, so the recipient — which recomputes it from its own local
/// built-in — derives the same value without needing to know the publisher's
/// slug. A hash that covered the slug would be self-referential and could
/// never match across installs.
fn member_projection_with_reuse_hint(record: &AgentDefinition) -> TeamCatalogMember {
    let mut member = member_projection(record);
    if let Some(slug) = builtin_catalog_slug(record) {
        member.projection_hash = Some(member_projection_hash(&member));
        member.builtin_slug = Some(slug.to_string());
    }
    member
}

/// The projection hash a recipient computes for one of its OWN local records,
/// to compare against a published member's `projection_hash`.
///
/// This is the reader half of the built-in reuse hint: the publisher stamps
/// `projection_hash` over the hint-free projection, and the recipient
/// recomputes it here from its own local built-in. Equality means the two
/// installs hold a byte-identical definition, which is the only condition
/// under which substituting the local record for the published one is safe.
pub fn local_member_projection_hash(record: &AgentDefinition) -> String {
    member_projection_hash(&member_projection(record))
}

/// Project a team and its resolved members onto a validated 30178 body.
///
/// `members` are supplied already resolved and ordered by the caller (the
/// team's own `persona_ids` order) because resolution needs the persona store
/// and this module stays pure.
///
/// Returns `Err` when the size contract is violated, so a share attempt fails
/// synchronously with a deterministic reason instead of enqueuing an event the
/// relay will refuse.
pub fn build_team_catalog_content(
    team: &TeamRecord,
    members: &[AgentDefinition],
) -> Result<TeamCatalogContent, String> {
    let content = TeamCatalogContent {
        v: TEAM_CATALOG_SCHEMA_VERSION,
        name: team.name.clone(),
        description: team.description.clone(),
        instructions: team.instructions.clone(),
        members: members
            .iter()
            .map(member_projection_with_reuse_hint)
            .collect(),
    };
    validate_team_catalog_content(&content)?;
    Ok(content)
}

/// Build an unsigned kind:30178 event for a team catalog projection.
///
/// The `d` tag is the team's id, matching its kind:30176 coordinate, so the
/// two heads for one team address consistently. `shared` is tagged only when
/// true: the relay's read gate keys off the tag's presence
/// (`SHARED_GATED_KINDS`), and an untagged head is the durable "published but
/// not discoverable" state that unshare produces.
///
/// Returns an `EventBuilder`; the caller sets `created_at`, signs, and submits.
pub fn build_team_catalog_event(
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared: bool,
) -> Result<EventBuilder, String> {
    let content = build_team_catalog_content(team, members)?;
    let content_json = team_catalog_content_json(&content)?;
    let mut tags =
        vec![Tag::parse(["d", team.id.as_str()]).map_err(|e| format!("invalid d-tag: {e}"))?];
    if shared {
        tags.push(Tag::parse(["shared", "true"]).map_err(|e| format!("invalid shared tag: {e}"))?);
    }
    Ok(EventBuilder::new(Kind::Custom(KIND_TEAM_CATALOG as u16), content_json).tags(tags))
}

/// Parse a kind:30178 event body, rejecting an unrecognized schema version.
pub fn team_catalog_content_from_event(event: &nostr::Event) -> Result<TeamCatalogContent, String> {
    buzz_core_pkg::team_catalog::parse_team_catalog_content(event.content.as_ref())
}

/// Build a NIP-09 deletion (kind:5) targeting a team's kind:30178 projection.
///
/// Mirrors `team_events::build_team_delete` but at the 30178 coordinate: a
/// single `a`-tag and no `e`-tag, because an `e`-tag routes the relay to the
/// event-id deletion path and leaves the replaceable coordinate live. Deleting
/// a shared team must retract the catalog entry for every reader, not just
/// this client.
pub fn build_team_catalog_delete(
    d_tag: &str,
    owner_pubkey_hex: &str,
) -> Result<EventBuilder, String> {
    let coord = format!("{KIND_TEAM_CATALOG}:{owner_pubkey_hex}:{d_tag}");
    let tag = Tag::parse(["a", coord.as_str()]).map_err(|e| format!("invalid a-tag: {e}"))?;
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(vec![tag]))
}

/// Purge the retained 30178 head at `d_tag` and enqueue a kind:5 tombstone.
///
/// Called from the direct delete path, the boot reconcile (orphaned shared
/// heads), and immediate retraction when a team can no longer be projected —
/// all hold the db path and keys but cannot share a single
/// `tombstone_team_catalog_at`.
///
/// Timestamp-domination invariant: the head this tombstone retracts may itself
/// be future-dated (`monotonic_created_at` bumps a same-second re-publish past
/// the prior head), and the relay only soft-deletes coordinate versions with
/// `created_at <=` the tombstone's (NIP-09 replay protection). So the kind:5 is
/// signed with `monotonic_created_at(Some(head.created_at))` — strictly past
/// the retained head — read inside the transaction. Signing at wall-clock `now`
/// would let a future-dated head survive its own tombstone, and because we then
/// purge the local row (the only retry witness), the team would stay publicly
/// discoverable forever. With no head, fall back to `monotonic_created_at(None)`.
///
/// The two SQLite operations (DELETE retained row + INSERT tombstone) run in a
/// single transaction. A kill between them would otherwise leave the relay
/// head shared indefinitely — the A3/I3 failure mode. Reading the head's
/// `created_at` inside the same `BEGIN IMMEDIATE` closes the read-then-sign
/// race: no concurrent writer can bump the head between the read and the purge.
/// Splitting the shared logic here also avoids a cross-module layering violation.
pub fn tombstone_team_catalog_coordinate(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    d_tag: &str,
) -> Result<(), String> {
    use crate::managed_agents::persona_events::monotonic_created_at;
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
        RetainedEvent,
    };
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let pubkey = keys.public_key().to_hex();

    let conn = open_retention_db(db_path)?;
    // Single transaction (see the crash and domination invariants above).
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("failed to begin tombstone transaction: {e}"))?;
    let result = (|| -> Result<(), String> {
        // Read the head's created_at inside the transaction, then sign the
        // kind:5 strictly past it so the relay cannot reject the deletion.
        let prior_head =
            get_retained_event(&conn, KIND_TEAM_CATALOG, &pubkey, d_tag)?.map(|row| row.created_at);
        let event = build_team_catalog_delete(d_tag, &pubkey)?
            .custom_created_at(monotonic_created_at(prior_head))
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign team catalog tombstone: {e}"))?;
        let tombstone = RetainedEvent {
            kind: KIND_DELETE,
            pubkey: pubkey.clone(),
            // Key by the target coordinate so the 30176 and 30178 tombstones for
            // one team occupy distinct rows.
            d_tag: tombstone_retention_d_tag(KIND_TEAM_CATALOG, d_tag),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        };
        conn.execute(
            "DELETE FROM persona_events
             WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
            rusqlite::params![KIND_TEAM_CATALOG, &pubkey, d_tag],
        )
        .map_err(|e| format!("failed to purge retained 30178 head: {e}"))?;
        retain_event(&conn, &tombstone)
    })();
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("failed to commit tombstone transaction: {e}")),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests;
