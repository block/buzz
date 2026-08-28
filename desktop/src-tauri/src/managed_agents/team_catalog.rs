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
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use super::{
    validate_agent_definition_text, validate_visible_text, AgentDefinition, RespondTo, TeamRecord,
};

/// Schema version of the 30178 content body. A reader that does not recognize
/// the value must refuse the event rather than guess at its shape.
pub const TEAM_CATALOG_SCHEMA_VERSION: u32 = 1;

// ── Size contract ────────────────────────────────────────────────────────────
//
// A 30178 event amplifies N member definitions into ONE event, so bounds that
// are immaterial for a single kind:30175 persona become load-bearing here. The
// relay's ingest ceiling is 256 KiB (`MAX_EVENT_CONTENT_BYTES`,
// `crates/buzz-relay/src/handlers/ingest.rs`), and an over-ceiling event is
// rejected AFTER being signed and durably enqueued — a permanently stuck
// pending row with no user-visible cause. Every bound below is enforced BEFORE
// the event is built, so the failure surfaces synchronously at share time.
//
// `MAX_TOTAL_BYTES` is the only bound that matters for relay acceptance; the
// per-field bounds exist so an oversized team names the specific field that
// pushed it over instead of reporting an opaque total.

/// Maximum members in one catalog projection.
pub const MAX_MEMBERS: usize = 64;
/// Maximum bytes for a team or member display name.
pub const MAX_NAME_BYTES: usize = 256;
/// Maximum bytes for the team description (display text).
pub const MAX_TEXT_BYTES: usize = 4 * 1024;
/// Maximum bytes for the team instructions — prompt content, parity with
/// `MAX_SYSTEM_PROMPT_BYTES`.
pub const MAX_INSTRUCTIONS_BYTES: usize = 16 * 1024;
/// Maximum bytes for a member's system prompt.
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 16 * 1024;
/// Maximum bytes for a member's avatar URL. Generous because the persona
/// catalog permits inline emoji data URLs, not just `https://` links.
pub const MAX_AVATAR_URL_BYTES: usize = 32 * 1024;
/// Maximum entries in a member's name pool.
pub const MAX_NAME_POOL_ENTRIES: usize = 64;
/// Maximum bytes for the whole serialized content body — the exact bytes the
/// relay counts against its 256 KiB `event.content` ceiling, inline avatar
/// base64 included. Enforcing 192 KiB here therefore guarantees relay
/// acceptance with 64 KiB of conservative headroom below that ceiling.
pub const MAX_TOTAL_BYTES: usize = 192 * 1024;

/// Maximum pixel dimension (width or height) accepted when decoding an inline
/// avatar for downscaling. Prevents decompression-bomb attacks before any
/// pixel allocation occurs. Mirrors `snapshot_avatar.rs`.
const MAX_DOWNSCALE_DECODE_DIMENSION: u32 = 2048;
/// Maximum heap allocation the image decoder may perform when materializing
/// a raster for downscaling. Mirrors `snapshot_avatar.rs`.
const MAX_DOWNSCALE_DECODE_ALLOC: u64 = 32 * 1024 * 1024;

/// Maximum bytes for a member's opaque `member_key`. A conforming key is a
/// 64-char SHA-256 hex digest; the bound is the parse-side ceiling for a
/// foreign publisher's value, which need only be opaque and unique.
pub const MAX_MEMBER_KEY_BYTES: usize = 128;
/// Maximum bytes for a member's runtime, model, or provider identifier.
pub const MAX_IDENTIFIER_BYTES: usize = 256;
/// Maximum bytes for a built-in reuse slug.
pub const MAX_BUILTIN_SLUG_BYTES: usize = 128;
/// Length of a hex-encoded SHA-256 projection hash.
pub const PROJECTION_HASH_HEX_LEN: usize = 64;

/// The JSON body stored in a kind:30178 event's content field.
///
/// Field order is pinned by declaration order: serde emits in that order, so a
/// reorder changes the content bytes and the NIP-01 event id — and the
/// freshness reconcile compares exactly those bytes, so a reorder would make
/// every shared team look stale once and republish the entire catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamCatalogContent {
    /// Schema version. First field so a reader can dispatch on it before
    /// committing to the rest of the shape.
    pub v: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Member projections in the team's own membership order — part of the
    /// canonical bytes, so a reorder is a genuine change and republishes.
    pub members: Vec<TeamCatalogMember>,
}

/// One member's safe definition, embedded in full.
///
/// Embedding is authoritative: a recipient can always rebuild this member from
/// these fields alone. `builtin_slug` / `projection_hash` are a reuse *hint*
/// and never an identity authority — see their doc comments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamCatalogMember {
    /// Stable, opaque identity of this member WITHIN this team publication.
    ///
    /// Provenance for an added member is `(owner_pubkey, team_d_tag,
    /// member_key)`, so the key must distinguish every member the publisher
    /// holds. It is a domain-separated SHA-256 over the source record's `id`
    /// (see [`member_key_for`]): deterministic, so an unchanged team rebuilds
    /// to identical bytes, while disclosing no local id.
    ///
    /// A recipient MUST treat it as opaque and MUST NOT resolve it as a
    /// kind:30175 coordinate in the publisher's namespace: the publisher may
    /// never have shared that persona individually. Hashing makes that misuse
    /// structurally impossible rather than merely forbidden.
    pub member_key: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    /// Sanitized audience mode. `allowlist` is never projected — see
    /// [`sanitized_respond_to`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    /// Clamped to 1..=32 at projection time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    /// Reuse hint: the built-in slug this member was installed from.
    ///
    /// Present only for built-in members. A recipient may substitute its own
    /// local built-in ONLY when the slug exists locally AND that built-in's
    /// current projection hash equals `projection_hash`. Any mismatch — a
    /// retired slug, a changed prompt, or a hostile slug paired with unrelated
    /// embedded fields — falls back to an ordinary copy from the embedded
    /// fields above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_slug: Option<String>,
    /// Hash of this member's own embedded projection. Meaningful only
    /// alongside `builtin_slug`; it is what makes the reuse hint exact-match
    /// gated rather than name-trusting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_hash: Option<String>,
}

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
        model: record.model.clone(),
        provider: record.provider.clone(),
        name_pool: record.name_pool.clone(),
        respond_to: sanitized_respond_to(record),
        parallelism: record.parallelism.map(|value| value.clamp(1, 32)),
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

/// Canonical JSON encoding of a content body — the single serializer.
///
/// Every byte-sensitive consumer (the size contract, the content hash, and the
/// event body) routes through this function so they can never disagree about
/// what the canonical encoding is.
pub fn team_catalog_content_json(content: &TeamCatalogContent) -> Result<String, String> {
    serde_json::to_string(content).map_err(|e| format!("failed to serialize team catalog: {e}"))
}

fn member_projection_hash(member: &TeamCatalogMember) -> String {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(member).unwrap_or_default();
    hex::encode(Sha256::digest(&json))
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

/// Validate an avatar URL against the catalog-safe allowlist.
///
/// Shared contract with `safeCatalogAvatarUrl` / `isSafeHttpUrl` in
/// `catalogRelay.ts` — the two sides must accept and reject the same inputs.
///
/// **Length metric: UTF-8 bytes** — the relay's native encoding and the same
/// unit as every other field bound here. TypeScript uses `byteLength` to match
/// (JS `value.length` counts UTF-16 code units, which diverges for non-ASCII).
///
/// Permitted forms:
/// - `http(s)://` URLs that parse cleanly via `url::Url::parse` (scheme
///   checked on the normalized value) with UTF-8 byte length ≤ 2 048. Both
///   Rust's `url` crate and the browser's `new URL()` implement the WHATWG URL
///   Standard, so parse-first runs the same algorithm on both sides —
///   including shorthand like `http:example.com` → `http://example.com/`.
/// - Inline SVG: `data:image/svg+xml,…` up to 8 192 bytes
/// - Inline raster (png/jpeg/gif/webp): `data:image/<type>;base64,<B64>` up
///   to 256 KiB with strict base64 shape
///
/// A `javascript:` URL, an arbitrary `data:` scheme, or an unparseable string
/// returns false.
pub fn is_safe_catalog_avatar_url(url: &str) -> bool {
    const INLINE_SVG_PREFIX: &str = "data:image/svg+xml,";
    const MAX_INLINE_SVG_LEN: usize = 8_192;
    const MAX_INLINE_RASTER_LEN: usize = 256 * 1_024;
    /// HTTP/HTTPS URL cap in UTF-8 bytes — same unit as TypeScript's `byteLength`.
    const MAX_HTTP_URL_BYTES: usize = 2_048;

    // Candidate HTTP/HTTPS URLs: byte cap → whitespace/paren guard → WHATWG
    // parse → scheme check. We parse rather than require a literal prefix
    // because WHATWG normalizes shorthand like `http:example.com`, which a
    // literal-prefix gate would wrongly reject.
    if !url.starts_with("data:") {
        if url.len() > MAX_HTTP_URL_BYTES {
            return false;
        }
        // Reject ECMAScript-`\s` whitespace or parentheses, matching TS's
        // pre-check `/[\s()]/u.test(value)`. Exact `\s` equivalence in Rust:
        //   ECMAScript `\s` = char::is_whitespace() − U+0085 (NEL) + U+FEFF (BOM)
        // url::Url::parse percent-encodes these rather than rejecting them, so
        // without the guard the two validators would diverge.
        if url.chars().any(|c| {
            ((c.is_whitespace() && c != '\u{0085}') || c == '\u{FEFF}') || c == '(' || c == ')'
        }) {
            return false;
        }
        // Parse with the same WHATWG algorithm as TS's `new URL()`: rejects
        // malformed authorities (https://^) and normalizes the scheme.
        if let Ok(u) = ::url::Url::parse(url) {
            if matches!(u.scheme(), "http" | "https") {
                return true;
            }
        }
        return false;
    }
    if url.starts_with(INLINE_SVG_PREFIX) {
        return url.len() <= MAX_INLINE_SVG_LEN;
    }
    // Inline raster: data:image/(png|jpeg|gif|webp);base64,<B64>
    if url.len() <= MAX_INLINE_RASTER_LEN {
        if let Some(rest) = url.strip_prefix("data:image/") {
            for mime in &["png", "jpeg", "gif", "webp"] {
                if let Some(b64_part) = rest
                    .strip_prefix(mime)
                    .and_then(|r| r.strip_prefix(";base64,"))
                {
                    // Strict base64: only [A-Za-z0-9+/] with up to 2 trailing '='
                    let trimmed = b64_part.trim_end_matches('=');
                    let padding = b64_part.len() - trimmed.len();
                    if padding <= 2
                        && trimmed
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
                        && b64_part.len() % 4 == 0
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}
fn bounded(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.len() > max {
        return Err(format!(
            "team too large to share: {label} is {} bytes (limit {max})",
            value.len()
        ));
    }
    Ok(())
}

fn non_empty(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("invalid team projection: {label} is empty"));
    }
    Ok(())
}

/// Validate one member against the v1 contract.
///
/// Every field a recipient will persist is checked here, because adoption
/// copies the projection into a local `AgentDefinition` verbatim. A field
/// bounded on the way in but unvalidated on the way out produces a record
/// accepted at add time that only fails later at mint — `parallelism` was
/// exactly that: a publisher could send `999`, adoption stored it, and minting
/// rejected it out of 1..=32. Validating at the parse boundary makes an
/// unusable team un-addable instead of add-then-broken.
fn validate_member(member: &TeamCatalogMember) -> Result<(), String> {
    let who = &member.display_name;
    non_empty(&member.member_key, "a member key")?;
    bounded(&member.member_key, MAX_MEMBER_KEY_BYTES, "a member key")?;
    non_empty(&member.display_name, "a member display name")?;
    bounded(
        &member.display_name,
        MAX_NAME_BYTES,
        "a member display name",
    )?;
    // Concealment gate on the executable-definition fields, matching the
    // invariant the persona catalog enforces at its own parse boundary
    // (`persona_catalog::parse_agent`): a member display name and prompt are
    // copied verbatim into a local persona and delivered to the ACP harness
    // (`BUZZ_ACP_SYSTEM_PROMPT`), so invisible/bidi controls could make what
    // executes differ from the reviewed text. `validate_agent_definition_text`
    // applies the display-name rule (no layout controls) and the prompt rule
    // (layout controls allowed) in one call.
    validate_agent_definition_text(
        &member.display_name,
        member.system_prompt.as_deref().unwrap_or_default(),
    )?;
    if let Some(prompt) = &member.system_prompt {
        bounded(
            prompt,
            MAX_SYSTEM_PROMPT_BYTES,
            &format!("the system prompt for '{who}'"),
        )?;
    }
    if let Some(avatar) = &member.avatar_url {
        bounded(
            avatar,
            MAX_AVATAR_URL_BYTES,
            &format!("the avatar for '{who}'"),
        )?;
        if !is_safe_catalog_avatar_url(avatar) {
            return Err(format!(
                "invalid team projection: the avatar for '{who}' uses an unsafe URL scheme (must be https, http, or an approved inline data URL)"
            ));
        }
    }
    for (value, label) in [
        (&member.runtime, "runtime"),
        (&member.model, "model"),
        (&member.provider, "provider"),
    ] {
        if let Some(value) = value {
            non_empty(value, &format!("the {label} for '{who}'"))?;
            bounded(
                value,
                MAX_IDENTIFIER_BYTES,
                &format!("the {label} for '{who}'"),
            )?;
        }
    }
    if member.name_pool.len() > MAX_NAME_POOL_ENTRIES {
        return Err(format!(
            "team too large to share: '{who}' has {} name-pool entries (limit {MAX_NAME_POOL_ENTRIES})",
            member.name_pool.len()
        ));
    }
    for name in &member.name_pool {
        non_empty(name, &format!("a name-pool entry for '{who}'"))?;
        bounded(
            name,
            MAX_NAME_BYTES,
            &format!("a name-pool entry for '{who}'"),
        )?;
        // Name-pool entries are minted verbatim as instance display names, so
        // they carry the same human-reviewed-identity contract as the member
        // display name — reject concealed controls here too.
        validate_visible_text(name, &format!("a name-pool entry for '{who}'"), false)?;
    }
    // Rejected at the boundary: an unrecognized mode must not become a local
    // definition whose audience differs from what the recipient was shown.
    if let Some(mode) = &member.respond_to {
        RespondTo::parse_wire(mode)?;
    }
    // Mirrors the 1..=32 range `mint_behavioral_defaults` enforces, so a team
    // whose members could never launch is refused at add time.
    if let Some(parallelism) = member.parallelism {
        if !(1..=32).contains(&parallelism) {
            return Err(format!(
                "invalid team projection: parallelism {parallelism} for '{who}' is out of range (must be between 1 and 32)"
            ));
        }
    }
    // The reuse hint is only meaningful as a complete, well-formed pair. A
    // half-pair or a malformed hash is a broken publisher — refuse it rather
    // than silently ignoring the hint.
    match (&member.builtin_slug, &member.projection_hash) {
        (Some(slug), Some(hash)) => {
            non_empty(slug, &format!("the built-in slug for '{who}'"))?;
            bounded(
                slug,
                MAX_BUILTIN_SLUG_BYTES,
                &format!("the built-in slug for '{who}'"),
            )?;
            if hash.len() != PROJECTION_HASH_HEX_LEN || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(format!(
                    "invalid team projection: the reuse hash for '{who}' is not a SHA-256 hex digest"
                ));
            }
            // The hash must be the hint-free projection hash of THIS member's
            // own embedded fields — not merely a well-formed digest. Without
            // this, a publisher could pair a real built-in's slug and that
            // built-in's genuine hash with arbitrary reviewed fields; the
            // recipient's `reusable_builtin` matches on (slug, hash) and would
            // install its own local built-in in place of the reviewed
            // projection. Recompute over the received member with both hint
            // fields cleared — the same input the publisher hashes — and
            // reject a mismatch. An honest publisher can never mismatch: it
            // stamps the hash from the same fields it publishes.
            let mut hint_free = member.clone();
            hint_free.builtin_slug = None;
            hint_free.projection_hash = None;
            if !member_projection_hash(&hint_free).eq_ignore_ascii_case(hash) {
                return Err(format!(
                    "invalid team projection: the reuse hash for '{who}' does not match its embedded fields"
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(format!(
                "invalid team projection: '{who}' has an incomplete built-in reuse hint"
            ))
        }
    }
    Ok(())
}

/// Enforce the size contract on a projected body.
///
/// Field bounds are checked before the total so the error names the specific
/// oversized field; the total is the backstop that actually guarantees relay
/// acceptance, because many individually-legal members still sum past the
/// ceiling.
pub fn validate_team_catalog_content(content: &TeamCatalogContent) -> Result<(), String> {
    // Non-empty trimmed name — parity with the TS reader's
    // `parsed.name.trim().length > 0`. A blank name persisted via a direct
    // backend add would be invisible in the catalog UI.
    non_empty(content.name.trim(), "the team name")?;
    bounded(&content.name, MAX_NAME_BYTES, "the team name")?;
    // The team name is rendered verbatim in the catalog UI as reviewed
    // identity, so it carries the same concealment contract as a member
    // display name: no layout controls, no invisible/bidi characters that
    // would make the displayed name differ from the reviewed bytes.
    validate_visible_text(&content.name, "the team name", false)?;
    if let Some(description) = &content.description {
        bounded(description, MAX_TEXT_BYTES, "the team description")?;
        // The description is shown verbatim in the catalog UI. It is
        // free-form prose and multiline by nature, so layout controls are
        // allowed — but concealed/bidi controls are still rejected.
        validate_visible_text(description, "the team description", true)?;
    }
    if let Some(instructions) = &content.instructions {
        bounded(
            instructions,
            MAX_INSTRUCTIONS_BYTES,
            "the team instructions",
        )?;
        // Team instructions reach the ACP harness verbatim
        // (`BUZZ_ACP_TEAM_INSTRUCTIONS`), so they are executable-definition
        // text under the same concealment contract as a member prompt. Layout
        // controls are allowed because instructions are multiline by nature.
        validate_visible_text(instructions, "the team instructions", true)?;
    }
    if content.members.len() > MAX_MEMBERS {
        return Err(format!(
            "team too large to share: {} members (limit {MAX_MEMBERS})",
            content.members.len()
        ));
    }
    // Provenance for every adopted member is `(owner_pubkey, team_d_tag,
    // member_key)`. Two members sharing a key would collapse onto one local
    // persona at adoption, silently dropping a member the recipient was shown.
    // Rejecting the publication is the only safe answer — there is no way to
    // tell which of the two the recipient meant to keep.
    let mut seen = std::collections::HashSet::with_capacity(content.members.len());
    for member in &content.members {
        validate_member(member)?;
        if !seen.insert(member.member_key.as_str()) {
            return Err(format!(
                "invalid team projection: '{}' repeats the member key '{}' of an earlier member",
                member.display_name, member.member_key
            ));
        }
    }
    let encoded = team_catalog_content_json(content)?;
    if encoded.len() > MAX_TOTAL_BYTES {
        return Err(format!(
            "team too large to share: the projection is {} bytes (limit {MAX_TOTAL_BYTES})",
            encoded.len()
        ));
    }
    Ok(())
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
///
/// Version dispatch happens before field access: a future `v: 2` body may
/// legally reshape any field, so parsing it as `v: 1` and rendering whatever
/// deserializes would present a corrupted team as a valid one.
pub fn team_catalog_content_from_event(event: &nostr::Event) -> Result<TeamCatalogContent, String> {
    let content: TeamCatalogContent = serde_json::from_str(event.content.as_ref())
        .map_err(|e| format!("failed to parse team catalog content: {e}"))?;
    if content.v != TEAM_CATALOG_SCHEMA_VERSION {
        return Err(format!(
            "unsupported team catalog schema version {} (expected {TEAM_CATALOG_SCHEMA_VERSION})",
            content.v
        ));
    }
    validate_team_catalog_content(&content)?;
    Ok(content)
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
