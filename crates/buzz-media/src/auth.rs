//! Blossom kind:24242 auth verification (BUD-11 compliant).

use crate::error::MediaError;

/// Verify kind:24242 event validity per BUD-11:
///   1. Schnorr signature
///   2. kind == 24242
///   3. `t` tag == "upload"
///   4. `expiration` tag in the future
///   5. `created_at` in the past (with 5s clock-skew tolerance)
///   6. If `server` tags present, our domain must appear in at least one
///
/// Does NOT check the `x` tag — that requires the body hash, computed later.
/// Call this BEFORE trusting the event's pubkey for scope resolution.
pub fn verify_blossom_auth_event(
    auth_event: &nostr::Event,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), MediaError> {
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

    let mut found_t = false;
    let mut found_exp = false;
    let mut server_tags: Vec<&str> = Vec::new();
    let mut exp_value: u64 = 0;

    for tag in auth_event.tags.iter() {
        let kind = tag.kind().to_string();
        match kind.as_str() {
            "t" => {
                if let Some(v) = tag.content() {
                    if v != "upload" {
                        return Err(MediaError::InvalidAuthVerb);
                    }
                    found_t = true;
                }
            }
            "expiration" => {
                if let Some(v) = tag.content() {
                    exp_value = v.parse().unwrap_or(0);
                    found_exp = true;
                }
            }
            "server" => {
                if let Some(v) = tag.content() {
                    server_tags.push(v);
                }
            }
            _ => {}
        }
    }

    // 3. t tag required
    if !found_t {
        return Err(MediaError::MissingTag("t"));
    }

    // 4. Expiration must exist and be in the future
    if !found_exp {
        return Err(MediaError::MissingTag("expiration"));
    }
    let now = nostr::Timestamp::now().as_secs();
    if exp_value <= now {
        return Err(MediaError::TokenExpired);
    }

    // 5. created_at must be recent: not in the future (5s tolerance) and not
    //    older than 10 minutes. This bounds the replay window — even if the
    //    expiration tag allows a longer lifetime, the token must have been
    //    freshly minted.
    let created = auth_event.created_at.as_secs();
    if created > now + 5 {
        return Err(MediaError::TimestampOutOfWindow);
    }
    if now > created + max_age_secs {
        return Err(MediaError::TimestampOutOfWindow);
    }

    // 6. Server tag enforcement (BUD-11 §5): if server tags present, our host must appear.
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
    // Fail closed: if the bound host is unknown, reject tokens that carry server
    // tags rather than silently accepting them.
    if !server_tags.is_empty() {
        match server_domain {
            Some(domain) => {
                let want = normalize_server_host(domain);
                let matches = server_tags
                    .iter()
                    .any(|tag| normalize_server_host(tag) == want);
                if !matches {
                    return Err(MediaError::ServerMismatch);
                }
            }
            None => {
                // Server tags present but we don't know our own host — reject.
                return Err(MediaError::ServerMismatch);
            }
        }
    }

    Ok(())
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
/// Calls [`verify_blossom_auth_event`] first, then verifies that at least one
/// `x` tag matches `sha256` (BUD-11 §6: "at least one x tag matches").
pub fn verify_blossom_upload_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), MediaError> {
    verify_blossom_auth_event(auth_event, server_domain, max_age_secs)?;

    // At least one x tag must match the body sha256 (BUD-11 §6)
    let has_matching_x = auth_event
        .tags
        .iter()
        .any(|tag| tag.kind().to_string() == "x" && (tag.content() == Some(sha256)));

    if !has_matching_x {
        return Err(MediaError::HashMismatch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn build_valid_auth(keys: &Keys, sha256: &str) -> nostr::Event {
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

    #[test]
    fn test_verify_valid() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(verify_blossom_upload_auth(&event, &sha256, None, 600).is_ok());
    }

    #[test]
    fn test_verify_auth_event_valid() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        assert!(verify_blossom_auth_event(&event, None, 600).is_ok());
    }

    #[test]
    fn test_verify_hash_mismatch() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        let wrong_hash = "b".repeat(64);
        assert!(matches!(
            verify_blossom_upload_auth(&event, &wrong_hash, None, 600),
            Err(MediaError::HashMismatch)
        ));
    }

    #[test]
    fn test_verify_wrong_kind() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(27235), "wrong kind")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_upload_auth(&event, &sha256, None, 600),
            Err(MediaError::InvalidAuthKind)
        ));
    }

    #[test]
    fn test_verify_multi_x_tags() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let other_hash = "b".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &other_hash]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::from(24242), "Upload multi-x")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        // Should pass because at least one x tag matches
        assert!(verify_blossom_upload_auth(&event, &sha256, None, 600).is_ok());
    }

    #[test]
    fn test_server_tag_enforcement() {
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
            verify_blossom_upload_auth(&event, &sha256, Some("buzz.example.com"), 600),
            Err(MediaError::ServerMismatch)
        ));
        // Should pass when our domain matches
        assert!(
            verify_blossom_upload_auth(&event, &sha256, Some("other.example.com"), 600).is_ok()
        );
        // Should fail when server_domain is None — fail closed
        assert!(matches!(
            verify_blossom_upload_auth(&event, &sha256, None, 600),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_no_server_tags_always_passes() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let event = build_valid_auth(&keys, &sha256);
        // No server tags → passes regardless of our domain
        assert!(verify_blossom_upload_auth(&event, &sha256, Some("any.domain.com"), 600).is_ok());
    }

    /// A `server` tag is matched against the *bound tenant host* under the
    /// shared `normalize_host` rule, so equivalent host spellings agree — the
    /// stock CLI's bare `host:port`, an explicit default port, a trailing dot,
    /// mixed case, and a full URL all match the same bound host. This is the
    /// regression guard for the multi-tenant media blocker: a non-primary
    /// tenant must accept its own server-tagged client.
    #[test]
    fn test_server_tag_normalized_against_bound_host() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
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

        // Non-primary tenant host with explicit non-default port (the live
        // repro: tenant B on 127.0.0.1:3100). Stock CLI tags `host:port`.
        assert!(verify_blossom_upload_auth(
            &build("127.0.0.1:3100"),
            &sha256,
            Some("127.0.0.1:3100"),
            600
        )
        .is_ok());

        // Equivalence under normalize_host: explicit default port, trailing
        // dot, mixed case, and a full URL all collapse to the bound host.
        for tag in [
            "Relay.Example:443",
            "relay.example.",
            "RELAY.EXAMPLE",
            "https://relay.example/",
        ] {
            assert!(
                verify_blossom_upload_auth(&build(tag), &sha256, Some("relay.example"), 600)
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
                600
            ),
            Err(MediaError::ServerMismatch)
        ));
    }

    #[test]
    fn test_empty_content_rejected() {
        let keys = Keys::generate();
        let sha256 = "a".repeat(64);
        let now = Timestamp::now().as_secs();
        let exp_str = (now + 300).to_string();
        let tags = vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp_str]).unwrap(),
        ];
        // Empty content — BUD-11 requires a human-readable string
        let event = EventBuilder::new(Kind::from(24242), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches!(
            verify_blossom_auth_event(&event, None, 600),
            Err(MediaError::InvalidAuthEvent)
        ));
    }
}
