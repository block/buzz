//! Wire contract and validation for kind:30178 team catalog bodies.
//!
//! Pure data and checks shared by the desktop (publish and adopt boundaries)
//! and the CLI, so a catalog listing matches what the desktop can adopt.

use serde::{Deserialize, Serialize};

use crate::definition_validation::{
    validate_agent_definition_text, validate_portable_acp_command, validate_visible_text,
};

/// Defines whether one ACP conversation is shared by a channel or isolated per thread.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpSessionPolicy {
    /// Share one ACP conversation across all threads in a channel.
    #[default]
    Channel,
    /// Keep a separate ACP conversation for each channel thread.
    Thread,
}

impl<'de> Deserialize<'de> for AcpSessionPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value.as_str() {
            Some("thread") => Self::Thread,
            _ => Self::Channel,
        })
    }
}

impl AcpSessionPolicy {
    /// Whether this is the default channel-wide policy.
    pub const fn is_channel(&self) -> bool {
        matches!(self, Self::Channel)
    }

    /// The wire string for this policy.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Thread => "thread",
        }
    }
}

/// Reject a `respond_to` mode the harness does not recognize.
pub fn validate_respond_to_wire(value: &str) -> Result<(), String> {
    match value {
        "owner-only" | "allowlist" | "anyone" => Ok(()),
        other => Err(respond_to_error(other)),
    }
}

/// The error for an unrecognized `respond_to` mode.
pub fn respond_to_error(value: &str) -> String {
    format!(
        "definition respond_to '{value}' is not a recognized mode (expected 'owner-only', 'allowlist', or 'anyone')"
    )
}

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
    /// Team name.
    pub name: String,
    /// Team description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Team instructions delivered to the harness.
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
    /// (see `member_key_for`): deterministic, so an unchanged team rebuilds
    /// to identical bytes, while disclosing no local id.
    ///
    /// A recipient MUST treat it as opaque and MUST NOT resolve it as a
    /// kind:30175 coordinate in the publisher's namespace: the publisher may
    /// never have shared that persona individually. Hashing makes that misuse
    /// structurally impossible rather than merely forbidden.
    pub member_key: String,
    /// Member display name.
    pub display_name: String,
    /// Member system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Member avatar URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Member runtime identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Portable ACP transport alias, omitted for legacy machine-local commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_command: Option<String>,
    /// Member model identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Member provider identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Names minted for instances of this member.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    /// Sanitized audience mode. `allowlist` is never projected — see
    /// `sanitized_respond_to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    /// Clamped to 1..=32 at projection time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    /// ACP conversation boundary for instances created from this member.
    #[serde(default, skip_serializing_if = "AcpSessionPolicy::is_channel")]
    pub session_policy: AcpSessionPolicy,
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

/// Parse and validate a kind:30178 event body.
///
/// Version dispatch happens before field access: a future `v: 2` body may
/// legally reshape any field, so parsing it as `v: 1` and rendering whatever
/// deserializes would present a corrupted team as a valid one.
pub fn parse_team_catalog_content(content: &str) -> Result<TeamCatalogContent, String> {
    let content: TeamCatalogContent = serde_json::from_str(content)
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

/// Canonical JSON encoding of a content body — the single serializer.
///
/// Every byte-sensitive consumer (the size contract, the content hash, and the
/// event body) routes through this function so they can never disagree about
/// what the canonical encoding is.
pub fn team_catalog_content_json(content: &TeamCatalogContent) -> Result<String, String> {
    serde_json::to_string(content).map_err(|e| format!("failed to serialize team catalog: {e}"))
}

/// SHA-256 hex digest of a member's canonical JSON projection.
pub fn member_projection_hash(member: &TeamCatalogMember) -> String {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(member).unwrap_or_default();
    hex::encode(Sha256::digest(&json))
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
pub fn validate_member(member: &TeamCatalogMember) -> Result<(), String> {
    validate_portable_acp_command(member.acp_command.as_deref())?;
    let who = &member.display_name;
    non_empty(&member.member_key, "a member key")?;
    bounded(&member.member_key, MAX_MEMBER_KEY_BYTES, "a member key")?;
    non_empty(&member.display_name, "a member display name")?;
    bounded(
        &member.display_name,
        MAX_NAME_BYTES,
        "a member display name",
    )?;
    // Concealment gate on the executable-definition fields: a member display
    // name and prompt are copied verbatim into a local persona and delivered
    // to the ACP harness
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
        validate_respond_to_wire(mode)?;
    }
    // Instances launch with 1..=32 parallelism, so a team whose members could
    // never launch is refused at add time.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn member(overrides: Value) -> Value {
        let mut base = json!({"member_key": "k", "display_name": "One", "system_prompt": "Do it."});
        base.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        base
    }

    fn team(members: Vec<Value>) -> Value {
        json!({"v": 1, "name": "T", "members": members})
    }

    fn parse(body: &Value) -> Result<TeamCatalogContent, String> {
        parse_team_catalog_content(&body.to_string())
    }

    fn reuse_hint(fields: Value) -> Value {
        let hint_free: TeamCatalogMember = serde_json::from_value(member(fields.clone())).unwrap();
        let mut with_hint = member(fields);
        with_hint["builtin_slug"] = json!("fizz");
        with_hint["projection_hash"] = json!(member_projection_hash(&hint_free));
        with_hint
    }

    #[test]
    fn team_validation_enforces_the_v1_contract() {
        let long = |n: usize| "a".repeat(n);
        let cases: Vec<(&str, Value, Option<&str>)> = vec![
            ("minimal", team(vec![member(json!({}))]), None),
            ("empty team", team(vec![]), None),
            (
                "emoji sequence",
                team(vec![member(json!({"display_name": "Dev 👩‍💻"}))]),
                None,
            ),
            (
                "future version",
                json!({"v": 2, "name": "T", "members": []}),
                Some("unsupported"),
            ),
            (
                "missing version",
                json!({"name": "T", "members": []}),
                Some("failed to parse"),
            ),
            (
                "member without key",
                team(vec![json!({"display_name": "One"})]),
                Some("failed to parse"),
            ),
            (
                "blank team name",
                json!({"v": 1, "name": " ", "members": []}),
                Some("team name is empty"),
            ),
            (
                "concealed team name",
                json!({"v": 1, "name": "T\u{200B}", "members": []}),
                Some("the team name contains"),
            ),
            (
                "bidi team instructions",
                json!({"v": 1, "name": "T", "instructions": "a\u{202E}b", "members": []}),
                Some("the team instructions contains"),
            ),
            (
                "oversized team description",
                json!({"v": 1, "name": "T", "description": long(MAX_TEXT_BYTES + 1), "members": []}),
                Some("the team description"),
            ),
            (
                "too many members",
                team(
                    (0..=MAX_MEMBERS)
                        .map(|i| member(json!({"member_key": format!("k{i}")})))
                        .collect(),
                ),
                Some("members (limit 64)"),
            ),
            (
                "duplicate member key",
                team(vec![member(json!({})), member(json!({}))]),
                Some("repeats the member key"),
            ),
            (
                "concealed member name",
                team(vec![member(json!({"display_name": "Re\u{200B}v"}))]),
                Some("prohibited invisible"),
            ),
            (
                "concealed member prompt",
                team(vec![member(json!({"system_prompt": "run\u{2066}x"}))]),
                Some("prohibited invisible"),
            ),
            (
                "concealed name pool entry",
                team(vec![member(json!({"name_pool": ["Al\u{202E}ias"]}))]),
                Some("name-pool entry"),
            ),
            (
                "oversized member key",
                team(vec![member(
                    json!({"member_key": long(MAX_MEMBER_KEY_BYTES + 1)}),
                )]),
                Some("a member key"),
            ),
            (
                "oversized member prompt",
                team(vec![member(
                    json!({"system_prompt": long(MAX_SYSTEM_PROMPT_BYTES + 1)}),
                )]),
                Some("the system prompt for"),
            ),
            (
                "nonportable acp command",
                team(vec![member(json!({"acp_command": "/bin/sh"}))]),
                Some("ACP command must be"),
            ),
            (
                "unsafe avatar",
                team(vec![member(json!({"avatar_url": "javascript:alert(1)"}))]),
                Some("unsafe URL scheme"),
            ),
            (
                "unknown respond_to",
                team(vec![member(json!({"respond_to": "everyone"}))]),
                Some("not a recognized mode"),
            ),
            (
                "allowlist respond_to",
                team(vec![member(json!({"respond_to": "allowlist"}))]),
                None,
            ),
            (
                "parallelism zero",
                team(vec![member(json!({"parallelism": 0}))]),
                Some("out of range"),
            ),
            (
                "parallelism 33",
                team(vec![member(json!({"parallelism": 33}))]),
                Some("out of range"),
            ),
            (
                "parallelism 32",
                team(vec![member(json!({"parallelism": 32}))]),
                None,
            ),
            (
                "half reuse hint",
                team(vec![member(json!({"builtin_slug": "fizz"}))]),
                Some("incomplete built-in reuse hint"),
            ),
            (
                "malformed reuse hash",
                team(vec![member(
                    json!({"builtin_slug": "fizz", "projection_hash": "xyz"}),
                )]),
                Some("not a SHA-256 hex digest"),
            ),
            (
                "reuse hash of other fields",
                team(vec![member(
                    json!({"builtin_slug": "fizz", "projection_hash": "0".repeat(64)}),
                )]),
                Some("does not match its embedded fields"),
            ),
            (
                "matching reuse hash",
                team(vec![reuse_hint(json!({}))]),
                None,
            ),
        ];
        for (label, body, expected) in cases {
            let result = parse(&body);
            match expected {
                None => assert!(result.is_ok(), "{label}: {result:?}"),
                Some(fragment) => {
                    let error = result.expect_err(label);
                    assert!(error.contains(fragment), "{label}: {error}");
                }
            }
        }
    }

    #[test]
    fn avatar_urls_follow_the_catalog_allowlist() {
        for (url, safe) in [
            ("https://x.test/a.png", true),
            ("http:example.com", true),
            ("data:image/svg+xml,<svg/>", true),
            ("data:image/png;base64,AAAA", true),
            ("data:image/png;base64,AAA", false),
            ("data:text/html;base64,AAAA", false),
            ("javascript:alert(1)", false),
            ("https://x.test/a b.png", false),
            ("https://x.test/a(1).png", false),
        ] {
            assert_eq!(is_safe_catalog_avatar_url(url), safe, "{url}");
        }
    }
}
