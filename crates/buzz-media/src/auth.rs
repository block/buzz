//! Blossom kind:24242 auth verification (BUD-11 + NIP-FI compliant).

use crate::error::MediaError;

/// Blossom kind:24242 verbs Buzz currently accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomVerb {
    Upload,
    Get,
}

impl BlossomVerb {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Get => "get",
        }
    }
}

/// Verification strictness derived from the NIP-FI mode.
///
/// `Strict` applies the full NIP-FI kind-24242 rules: 60-second proof window,
/// `expiration <= created_at + 60s`, mandatory `server` tag on all proofs, and
/// exact cardinality (exactly one each of `t`, `expiration`, `server`; at most
/// one `x`).
///
/// `Permissive` preserves the pre-NIP-FI behavior for Off-mode deployments:
/// 3600-second proof window, `server` tag optional, and duplicate tags accepted.
/// This keeps a deployed desktop that mints old-shape tokens working against an
/// Off-mode relay [FI-INV-15].
///
/// The relay call sites derive strictness from `config.nip_fi.mode`; library
/// callers without mode access may pass `Strict` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomStrictness {
    /// Full NIP-FI compliance: 60s window, mandatory `server`, exact cardinality.
    Strict,
    /// Pre-NIP-FI compatibility: 3600s window, optional `server`, tolerant cardinality.
    Permissive,
}

/// Verify common kind:24242 Blossom auth event validity.
///
/// Checks in order:
///   1. Schnorr signature
///   2. kind == 24242 and non-empty content
///   3. `t` tag matches `verb`:
///      - `Strict`: count every tag named `t` by field name; require exactly
///        one occurrence; require its content to be non-empty and equal to
///        `verb` (NIP-FI.md:658-666 — malformed/empty/duplicate instances MUST
///        be rejected as `evidence_rejected`).
///      - `Permissive`: at least one valid-valued `t` tag equal to `verb`;
///        valueless/empty tags are ignored (origin/main `found_t` semantics).
///   4. `expiration` tag present, strictly future, and within the freshness window
///   5. `created_at` bounded: not more than 5s in the future; not older than the
///      mode-selected window (60s `Strict`, 3600s `Permissive`)
///   6. `server` tag enforcement: in `Strict` mode, exactly one `server` tag is
///      required and MUST match the bound tenant host; in `Permissive`, optional
///      when-present behavior is preserved
///
/// Does NOT check verb-specific scope tags (`x` for upload, `x` OR `server`
/// for get). Call this BEFORE trusting the event's pubkey for scope resolution.
pub fn verify_blossom_auth_event_for_verb(
    auth_event: &nostr::Event,
    verb: BlossomVerb,
    server_domain: Option<&str>,
    strictness: BlossomStrictness,
) -> Result<(), MediaError> {
    let strict = strictness == BlossomStrictness::Strict;

    // 1. Verify Schnorr signature
    auth_event
        .verify()
        .map_err(|_| MediaError::InvalidSignature)?;

    // 2. Kind must be 24242
    if auth_event.kind.as_u16() != 24242 {
        return Err(MediaError::InvalidAuthKind);
    }

    // 2b. Content must be non-empty (BUD-11: "human readable string")
    if auth_event.content.trim().is_empty() {
        return Err(MediaError::InvalidAuthEvent);
    }

    // Tag accumulation with strict cardinality tracking.
    // In Strict mode: each of t/expiration/server must appear exactly once;
    // x is counted separately for upload scope checking (at most one).
    // In Permissive mode: boolean-style "found at least one" semantics,
    // matching the pre-NIP-FI behavior.
    let mut t_count: u8 = 0;
    let mut exp_count: u8 = 0;
    let mut server_count: u8 = 0;
    let mut x_count: u8 = 0;

    let mut exp_value: u64 = 0;
    // Stored as owned String to avoid lifetime entanglement across the tag iterator.
    let mut server_value: Option<String> = None;

    for tag in auth_event.tags.iter() {
        let kind = tag.kind().to_string();
        match kind.as_str() {
            "t" => {
                if strict {
                    // Strict (NIP-FI): count EVERY tag named "t" by field name,
                    // regardless of its content.  NIP-FI.md:658-666 requires
                    // that malformed, empty, duplicate, or conflicting instances
                    // be rejected as `evidence_rejected`.  We count first, gate
                    // on cardinality, then validate content.
                    t_count = t_count.saturating_add(1);
                    if t_count > 1 {
                        return Err(MediaError::DuplicateTag("t"));
                    }
                    // Exactly one t tag: its content must be non-empty and
                    // equal to the requested verb.
                    match tag.content() {
                        Some(v) if !v.is_empty() && v == verb.as_str() => {}
                        _ => return Err(MediaError::InvalidAuthVerb),
                    }
                } else {
                    // Permissive: only valid-valued matching tags count
                    // (origin/main's `found_t` semantics).  Valueless/empty
                    // tags are ignored for cardinality; wrong-verb tags reject.
                    if let Some(v) = tag.content() {
                        if v.is_empty() {
                            // Empty string value: does not satisfy the requirement.
                        } else if v != verb.as_str() {
                            return Err(MediaError::InvalidAuthVerb);
                        } else {
                            t_count = t_count.saturating_add(1);
                        }
                    }
                    // No content: tag is ignored (not counted, not rejected).
                }
            }
            "expiration" => {
                exp_count = exp_count.saturating_add(1);
                if strict && exp_count > 1 {
                    return Err(MediaError::DuplicateTag("expiration"));
                }
                if let Some(v) = tag.content() {
                    if exp_count == 1 {
                        exp_value = v.parse().unwrap_or(0);
                    }
                }
            }
            "server" => {
                server_count = server_count.saturating_add(1);
                if strict && server_count > 1 {
                    return Err(MediaError::DuplicateTag("server"));
                }
                if server_count == 1 {
                    server_value = tag.content().map(|s| s.to_owned());
                }
            }
            "x" => {
                x_count = x_count.saturating_add(1);
                if strict && x_count > 1 {
                    return Err(MediaError::DuplicateTag("x"));
                }
            }
            _ => {}
        }
    }

    // 3. t tag required (exactly one in Strict, at least one in Permissive)
    if t_count == 0 {
        return Err(MediaError::MissingTag("t"));
    }

    // 4a. Expiration must exist
    if exp_count == 0 {
        return Err(MediaError::MissingTag("expiration"));
    }
    let now = nostr::Timestamp::now().as_secs();

    // 4b. Expiration must be strictly in the future
    if exp_value <= now {
        return Err(MediaError::TokenExpired);
    }

    // 5. created_at freshness bounds.
    //
    //   Strict  (NIP-FI active modes):
    //     created_at <= now + 5s          — bounded future skew
    //     now - created_at <= 60s         — 60-second replay window
    //     expiration <= created_at + 60s  — token cannot outlive its window
    //
    //   Permissive (Off mode):
    //     created_at <= now + 5s          — future skew only
    //     now - created_at <= 3600s       — 1-hour replay window (pre-NIP-FI)
    //     expiration window not enforced
    let created = auth_event.created_at.as_secs();
    if created > now + 5 {
        return Err(MediaError::TimestampOutOfWindow);
    }
    let max_age = if strict { 60u64 } else { 3600u64 };
    if now > created + max_age {
        return Err(MediaError::TimestampOutOfWindow);
    }
    if strict && exp_value > created + 60 {
        // expiration tag must satisfy expiration <= created_at + 60s
        return Err(MediaError::TimestampOutOfWindow);
    }

    // 6. Server tag enforcement.
    //
    // `server_domain` is the host this request was bound to — the per-request
    // tenant host (`TenantContext::host()`), NOT a single process-global domain.
    // A relay process serves many tenant hosts; validating against one global
    // host would 401 every non-primary tenant's server-tagged client (the stock
    // CLI always tags its configured relay host). Comparison is done under the
    // shared [`normalize_host`] rule so a tag and the bound host agree by
    // construction across case, trailing dot, default ports, and an optional
    // URL scheme/path — exactly as every other host seam resolves tenants.
    //
    // Strict (NIP-FI active): exactly one server tag MUST be present and MUST
    // match the bound tenant host. Absent or mismatched → evidence_rejected.
    //
    // Permissive (Off mode): if server tag present, our host must appear
    // (fail-closed when present but our host is unknown); absent is accepted.
    if strict {
        match (server_count, server_value.as_deref(), server_domain) {
            (0, _, _) => {
                // Strict: server tag mandatory
                return Err(MediaError::ServerMismatch);
            }
            (_, Some(tag_host), Some(domain)) => {
                if normalize_server_host(tag_host) != normalize_server_host(domain) {
                    return Err(MediaError::ServerMismatch);
                }
            }
            (_, _, None) => {
                // Strict: bound host unknown — fail closed
                return Err(MediaError::ServerMismatch);
            }
            (_, None, _) => {
                // server tag present but empty — treat as mismatch
                return Err(MediaError::ServerMismatch);
            }
        }
    } else {
        // Permissive: validate only when server tags are present
        if server_count > 0 {
            match (server_value.as_deref(), server_domain) {
                (Some(tag_host), Some(domain)) => {
                    if normalize_server_host(tag_host) != normalize_server_host(domain) {
                        return Err(MediaError::ServerMismatch);
                    }
                }
                (_, None) => {
                    // Server tags present but we don't know our own host — reject.
                    return Err(MediaError::ServerMismatch);
                }
                (None, _) => {
                    return Err(MediaError::ServerMismatch);
                }
            }
        }
    }

    Ok(())
}

/// Verify only the `x` tag hash match on an already-admitted upload auth event.
///
/// This is the post-body hash check: the full auth event verification
/// (signature, kind, freshness, cardinality, server) was already performed at
/// the pre-body admission gate.  Re-running the full verifier after streaming
/// a potentially large body would fail for any upload that takes longer than
/// the minted token's `expiration` window (typically 60 s in Strict mode).
///
/// The ONLY thing that is unknown before the body is transferred is the
/// content hash (`x` tag).  This function confirms that the body's SHA-256
/// matches what was declared in the signed proof.
pub fn verify_upload_hash_only(auth_event: &nostr::Event, sha256: &str) -> Result<(), MediaError> {
    let has_matching_x = auth_event
        .tags
        .iter()
        .any(|tag| tag.kind().to_string() == "x" && tag.content() == Some(sha256));
    if !has_matching_x {
        return Err(MediaError::HashMismatch);
    }
    Ok(())
}

/// Verify common upload auth event validity.
///
/// Kept as the upload-shaped public wrapper for existing callers; new verb-aware
/// code should prefer [`verify_blossom_auth_event_for_verb`].
pub fn verify_blossom_auth_event(
    auth_event: &nostr::Event,
    server_domain: Option<&str>,
    strictness: BlossomStrictness,
) -> Result<(), MediaError> {
    verify_blossom_auth_event_for_verb(auth_event, BlossomVerb::Upload, server_domain, strictness)
}

/// Normalize a Blossom `server` tag value (or a bound tenant host) into the
/// canonical host form used as the community lookup key.
///
/// A `server` tag may be a bare authority (`relay.example:3100`, what the stock
/// CLI emits) or a full URL (`https://relay.example/`). We strip an optional
/// scheme and path down to the authority, then apply the one shared
/// [`buzz_core::tenant::normalize_host`] rule so the comparison agrees with how
/// the WS/HTTP/git doors resolve tenants.
fn normalize_server_host(value: &str) -> String {
    let authority = match value.split_once("://") {
        Some((_scheme, rest)) => rest.split('/').next().unwrap_or(rest),
        None => value.split('/').next().unwrap_or(value),
    };
    buzz_core::tenant::normalize_host(authority)
}

/// Verify a kind:24242 Blossom upload auth event, including the x tag hash check.
///
/// In `Strict` mode (NIP-FI active): exactly one `x` tag MUST be present and
/// MUST match `sha256`. In `Permissive` mode: at least one `x` tag must match.
pub fn verify_blossom_upload_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    strictness: BlossomStrictness,
) -> Result<(), MediaError> {
    verify_blossom_auth_event_for_verb(auth_event, BlossomVerb::Upload, server_domain, strictness)?;

    // Upload: x tag must match the body sha256.
    // Strict: exactly one x tag and it must match (duplicate x caught above).
    // Permissive: at least one x tag must match.
    let has_matching_x = auth_event
        .tags
        .iter()
        .any(|tag| tag.kind().to_string() == "x" && (tag.content() == Some(sha256)));

    if !has_matching_x {
        return Err(MediaError::HashMismatch);
    }

    Ok(())
}

/// Verify a kind:24242 Blossom get auth event for one requested blob.
///
/// BUD-01 permits either blob-scoped authorization (`x` tag matches `sha256`)
/// or server-scoped authorization (`server` tag matches this relay host). The
/// latter intentionally grants reads for all blobs on the host until expiration;
/// callers must still apply relay membership after this verifier returns.
///
/// In `Strict` mode the `server` tag is mandatory (enforced by the base verifier);
/// `x` is optional on reads. An `x` tag, when present, must be exactly one and
/// must match `sha256`.
pub fn verify_blossom_get_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    strictness: BlossomStrictness,
) -> Result<(), MediaError> {
    verify_blossom_auth_event_for_verb(auth_event, BlossomVerb::Get, server_domain, strictness)?;

    // x tag scope check for get: if an x tag is present it must match sha256.
    // In Strict mode duplicate x is already rejected above; here we check the value.
    // In Permissive mode we use the original "any matching x OR matching server" logic.
    let x_tags: Vec<&str> = auth_event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "x")
        .filter_map(|tag| tag.content())
        .collect();

    let has_matching_x = x_tags.contains(&sha256);

    let has_matching_server = match server_domain {
        Some(domain) => {
            let want = normalize_server_host(domain);
            auth_event.tags.iter().any(|tag| {
                tag.kind().to_string() == "server"
                    && tag
                        .content()
                        .map(|value| normalize_server_host(value) == want)
                        .unwrap_or(false)
            })
        }
        None => false,
    };

    if strictness == BlossomStrictness::Strict {
        // Strict: server is already validated as present+matching by the base verifier.
        // x tag, if present, must match sha256 (mismatched x → evidence_rejected).
        if !x_tags.is_empty() && !has_matching_x {
            return Err(MediaError::ServerMismatch);
        }
        // Server-scoped read (no x) is always admitted here — server was validated above.
    } else {
        // Permissive: original BUD-01 semantics — matching x OR matching server.
        if !has_matching_x && !has_matching_server {
            return Err(MediaError::InsufficientScope);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn build_valid_auth(keys: &Keys, sha256: &str) -> nostr::Event {
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn build_permissive_auth(keys: &Keys, sha256: &str) -> nostr::Event {
        // Old-shape token: no server tag, 300s expiry — valid in Permissive, rejected in Strict.
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    // ── Strict mode ──────────────────────────────────────────────────────────

    #[test]
    fn test_verify_valid_strict() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(verify_blossom_upload_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Strict
        )
        .is_ok());
    }

    #[test]
    fn test_verify_auth_event_valid_strict() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(verify_blossom_auth_event(
            &event,
            Some("relay.example"),
            BlossomStrictness::Strict
        )
        .is_ok());
    }

    // ── Permissive mode (Off-mode regression guard [FI-INV-15]) ──────────────

    #[test]
    fn test_verify_permissive_old_shape_token_admitted() {
        // Old-shape token (no server tag, 300s expiry) MUST be admitted in Permissive mode.
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_permissive_auth(&keys, &sha256);
        assert!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Permissive
            )
            .is_ok(),
            "Off-mode must admit old-shape tokens without server tag [FI-INV-15]"
        );
    }

    #[test]
    fn test_verify_permissive_long_expiry_admitted() {
        // 600s expiry token (pre-NIP-FI desktop default) MUST be admitted in Permissive mode.
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 600).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Permissive
            )
            .is_ok(),
            "Off-mode must admit 600s expiry tokens [FI-INV-15]"
        );
    }

    // ── Cardinality: Strict rejects duplicates ────────────────────────────────

    #[test]
    fn test_strict_rejects_duplicate_t_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["t", "upload"]).unwrap(), // duplicate
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::DuplicateTag("t"))
        ));
    }

    #[test]
    fn test_strict_rejects_duplicate_expiration_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(), // duplicate
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::DuplicateTag("expiration"))
        ));
    }

    #[test]
    fn test_strict_rejects_duplicate_server_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(), // duplicate
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::DuplicateTag("server"))
        ));
    }

    #[test]
    fn test_strict_rejects_duplicate_x_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(), // duplicate
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::DuplicateTag("x"))
        ));
    }

    // ── Permissive mode: duplicate tags still admitted ────────────────────────

    #[test]
    fn test_permissive_admits_duplicate_x_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(), // duplicate — permitted in Permissive
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(verify_blossom_upload_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Permissive
        )
        .is_ok());
    }

    // ── Strict: server tag mandatory ─────────────────────────────────────────

    #[test]
    fn test_strict_rejects_absent_server_on_upload() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            // no server tag
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::ServerMismatch)
            ),
            "Strict mode must reject upload proof without server tag"
        );
    }

    #[test]
    fn test_strict_rejects_absent_server_on_read() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "get"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            // no server tag
        ];
        let event = EventBuilder::new(Kind::from(24242), "Get buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_get_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::ServerMismatch)
            ),
            "Strict mode must reject read proof without server tag"
        );
    }

    // ── Strict: freshness window ──────────────────────────────────────────────

    #[test]
    fn test_strict_rejects_expiration_exceeding_60s_window() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 61).to_string(); // 61s > 60s limit
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::TimestampOutOfWindow)
            ),
            "Strict mode must reject expiration > created_at + 60s"
        );
    }

    #[test]
    fn test_strict_admits_60s_expiration() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 60).to_string(); // exactly 60s — allowed
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload buzz-media")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            )
            .is_ok(),
            "Strict mode must admit expiration == created_at + 60s"
        );
    }

    // ── Get auth ─────────────────────────────────────────────────────────────

    fn build_get_auth(keys: &Keys, tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::from(24242), "Get buzz-media")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn test_verify_get_accepts_matching_server_strict() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["server", "https://Relay.Example./media/ignored"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(verify_blossom_get_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Strict
        )
        .is_ok());
    }

    #[test]
    fn test_verify_get_accepts_matching_x_and_server_strict() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["x", &sha256]).unwrap(),
                Tag::parse(["server", "relay.example"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(verify_blossom_get_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Strict
        )
        .is_ok());
    }

    #[test]
    fn test_verify_get_strict_rejects_mismatched_x_with_valid_server() {
        // x present but wrong hash → evidence_rejected even if server matches
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let other = "b".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["x", &other]).unwrap(),
                Tag::parse(["server", "relay.example"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(matches!(
            verify_blossom_get_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_verify_get_accepts_matching_x_without_server_permissive() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["x", &sha256]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(verify_blossom_get_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Permissive
        )
        .is_ok());
    }

    #[test]
    fn test_verify_get_accepts_matching_server_without_x_permissive() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["server", "https://Relay.Example./media/ignored"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(verify_blossom_get_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Permissive
        )
        .is_ok());
    }

    #[test]
    fn test_verify_get_rejects_upload_verb() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(matches!(
            verify_blossom_get_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Permissive
            ),
            Err(MediaError::InvalidAuthVerb)
        ));
    }

    #[test]
    fn test_verify_get_requires_x_or_server_scope_permissive() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let other_hash = "b".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["x", &other_hash]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(matches!(
            verify_blossom_get_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Permissive
            ),
            Err(MediaError::InsufficientScope)
        ));
    }

    #[test]
    fn test_verify_get_rejects_wrong_server_scope() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let event = build_get_auth(
            &keys,
            vec![
                Tag::parse(["t", "get"]).unwrap(),
                Tag::parse(["server", "other.example"]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
            ],
        );
        assert!(matches!(
            verify_blossom_get_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Permissive
            ),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_verify_hash_mismatch() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        let wrong_hash = "b".repeat(64);
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &wrong_hash,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::HashMismatch)
        ));
    }

    #[test]
    fn test_verify_wrong_kind() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(27235), "wrong kind")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("relay.example"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::InvalidAuthKind)
        ));
    }

    #[test]
    fn test_server_tag_enforcement_permissive() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "other.example.com"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload scoped")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        // Should fail — server tag present but doesn't match our domain
        assert!(matches!(
            verify_blossom_upload_auth(
                &event,
                &sha256,
                Some("buzz.example.com"),
                BlossomStrictness::Permissive
            ),
            Err(MediaError::ServerMismatch)
        ));
        // Should pass when our domain matches
        assert!(verify_blossom_upload_auth(
            &event,
            &sha256,
            Some("other.example.com"),
            BlossomStrictness::Permissive
        )
        .is_ok());
        // Should fail when server_domain is None — fail closed
        assert!(matches!(
            verify_blossom_upload_auth(&event, &sha256, None, BlossomStrictness::Permissive),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_permissive_no_server_tags_always_passes() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_permissive_auth(&keys, &sha256);
        // No server tags → passes regardless of our domain in Permissive mode
        assert!(verify_blossom_upload_auth(
            &event,
            &sha256,
            Some("any.domain.com"),
            BlossomStrictness::Permissive
        )
        .is_ok());
    }

    /// A `server` tag is matched against the *bound tenant host* under the
    /// shared `normalize_host` rule, so equivalent host spellings agree.
    #[test]
    fn test_server_tag_normalized_against_bound_host() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let build = |server: &str| {
            let tags = vec![
                Tag::parse(["t", "upload"]).unwrap(),
                Tag::parse(["x", &sha256]).unwrap(),
                Tag::parse(["expiration", &exp_str]).unwrap(),
                Tag::parse(["server", server]).unwrap(),
            ];
            EventBuilder::new(Kind::from(24242), "Upload scoped")
                .tags(tags)
                .sign_with_keys(&keys)
                .unwrap()
        };

        // Non-primary tenant host with explicit non-default port.
        assert!(verify_blossom_upload_auth(
            &build("127.0.0.1:3100"),
            &sha256,
            Some("127.0.0.1:3100"),
            BlossomStrictness::Strict
        )
        .is_ok());

        // Equivalence under normalize_host
        for tag in [
            "Relay.Example:443",
            "relay.example.",
            "RELAY.EXAMPLE",
            "https://relay.example/",
        ] {
            assert!(
                verify_blossom_upload_auth(
                    &build(tag),
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                )
                .is_ok(),
                "server tag {tag:?} should match bound host relay.example"
            );
        }

        // A different tenant host still fails closed.
        assert!(matches!(
            verify_blossom_upload_auth(
                &build("127.0.0.1:3100"),
                &sha256,
                Some("127.0.0.1:3200"),
                BlossomStrictness::Strict
            ),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_empty_content_rejected() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        // Empty content — BUD-11 requires a human-readable string
        let event = EventBuilder::new(Kind::from(24242), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_auth_event(&event, Some("relay.example"), BlossomStrictness::Strict),
            Err(MediaError::InvalidAuthEvent)
        ));
    }

    // ── Finding 3: valueless / empty-string t tag must not satisfy verb binding ──

    /// A `t` tag with no content (`["t"]`) in Strict mode: counted as one occurrence,
    /// content check fires → `InvalidAuthVerb` (malformed instance, NIP-FI §cardinality).
    #[test]
    fn test_strict_rejects_valueless_t_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        // ["t"] with no second element — no content, must be rejected
        let tags = vec![
            Tag::parse(["t"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::InvalidAuthVerb)
            ),
            "valueless t tag must be rejected in Strict mode (counted once, content invalid)"
        );
    }

    #[test]
    fn test_valueless_t_tag_is_not_counted_permissive() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Permissive
                ),
                Err(MediaError::MissingTag("t"))
            ),
            "valueless t tag must not satisfy t requirement in Permissive mode"
        );
    }

    /// A `t` tag with an empty-string value (`["t", ""]`) in Strict mode: counted
    /// as one occurrence, content check fires → `InvalidAuthVerb`.
    #[test]
    fn test_strict_rejects_empty_string_t_tag() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", ""]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::InvalidAuthVerb)
            ),
            "empty-string t tag must be rejected in Strict mode (counted once, content invalid)"
        );
    }

    /// A valueless `x` tag on upload (`["x"]`) does not match any sha256.
    #[test]
    fn test_valueless_x_tag_does_not_match_hash() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x"]).unwrap(), // no value
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::HashMismatch)
            ),
            "valueless x tag must not satisfy hash requirement"
        );
    }

    // ── Finding 3 R2: mixed malformed+valid t combos must reject in Strict ────

    /// `["t"]` (valueless) + `["t","upload"]` in Strict: the malformed tag is counted
    /// and content-validated on first encounter; the exact rejection error depends on
    /// tag ordering but any error is correct — no combination may be admitted.
    /// (If malformed comes first: `InvalidAuthVerb`; if valid first: `DuplicateTag("t")`.)
    #[test]
    fn test_strict_rejects_valueless_plus_valid_t_combo() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t"]).unwrap(),           // valueless — counted in Strict
            Tag::parse(["t", "upload"]).unwrap(), // valid — but two t tags total
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload bypass attempt")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        let result = verify_blossom_upload_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Strict,
        );
        assert!(
            result.is_err(),
            "valueless+valid t combo must be rejected in Strict mode, got Ok"
        );
        // The first tag is valueless → InvalidAuthVerb fires before the second tag
        // is seen; if ordering were reversed it would be DuplicateTag("t"). Both are
        // valid evidence_rejected-class outcomes — what matters is admission is denied.
        assert!(
            matches!(
                result,
                Err(MediaError::InvalidAuthVerb) | Err(MediaError::DuplicateTag("t"))
            ),
            "expected InvalidAuthVerb or DuplicateTag(t), got unexpected error variant"
        );
    }

    /// `["t",""]` (empty-string) + `["t","upload"]` in Strict: same reasoning —
    /// any error is correct; admission must be denied.
    #[test]
    fn test_strict_rejects_empty_plus_valid_t_combo() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", ""]).unwrap(), // empty-string — counted in Strict
            Tag::parse(["t", "upload"]).unwrap(), // valid — but two t tags total
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload bypass attempt")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        let result = verify_blossom_upload_auth(
            &event,
            &sha256,
            Some("relay.example"),
            BlossomStrictness::Strict,
        );
        assert!(
            result.is_err(),
            "empty-string+valid t combo must be rejected in Strict mode, got Ok"
        );
        assert!(
            matches!(
                result,
                Err(MediaError::InvalidAuthVerb) | Err(MediaError::DuplicateTag("t"))
            ),
            "expected InvalidAuthVerb or DuplicateTag(t), got unexpected error variant"
        );
    }

    /// `["x"]` (valueless) + `["x", sha256]` (valid) in Strict: `x_count` increments
    /// unconditionally for both occurrences → `DuplicateTag("x")`.  This confirms
    /// the x arm already handles mixed malformed+valid correctly.
    #[test]
    fn test_strict_rejects_valueless_plus_valid_x_combo() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 55).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x"]).unwrap(), // valueless — counted unconditionally
            Tag::parse(["x", &sha256]).unwrap(), // valid — but makes x_count = 2
            Tag::parse(["expiration", &exp_str]).unwrap(),
            Tag::parse(["server", "relay.example"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload bypass attempt")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(
            matches!(
                verify_blossom_upload_auth(
                    &event,
                    &sha256,
                    Some("relay.example"),
                    BlossomStrictness::Strict
                ),
                Err(MediaError::DuplicateTag("x"))
            ),
            "valueless+valid x combo must be rejected as DuplicateTag in Strict mode"
        );
    }
}
